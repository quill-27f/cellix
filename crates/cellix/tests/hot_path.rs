//! Drawing runs on every frame, on the loop that reads keys: a file read or a process started
//! there is a stall the person feels, and grows with their project. This reads the drawing code
//! (every `fn draw…` and `fn paint…`, with comments, strings and tests blanked out) and fails
//! on any call that reaches the disk or another process. What drawing needs from those is
//! fetched beside the loop and kept, as the tree and git are.

use std::path::{Path, PathBuf};

/// Calls drawing must never make, and why each is here.
const FORBIDDEN: &[(&str, &str)] = &[
    ("std::fs", "the disk"),
    ("fs::", "the disk"),
    ("File::", "the disk"),
    (".metadata(", "the disk"),
    ("read_dir(", "the disk"),
    ("read_to_string(", "the disk"),
    ("canonicalize(", "the disk"),
    ("Command::new", "another process"),
    (
        "process_group_leader",
        "the terminal's foreground, a system call",
    ),
    ("thread::spawn", "a thread a frame"),
];

/// The crates whose drawing is checked.
const CRATES: &[&str] = &["cellix", "cellix-render"];

fn sources(dir: &Path, found: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            sources(&path, found);
        } else if path.extension().is_some_and(|e| e == "rs") {
            found.push(path);
        }
    }
}

/// The text with comments, string and character literals blanked to spaces, keeping every
/// line where it was, so a forbidden word in a doc comment or a message isn't a call.
fn blanked(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    let blank = |c: char| if c == '\n' { '\n' } else { ' ' };
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        if c == '/' && next == Some('/') {
            while i < chars.len() && chars[i] != '\n' {
                out.push(' ');
                i += 1;
            }
        } else if c == '/' && next == Some('*') {
            let mut depth = 0;
            while i < chars.len() {
                if chars[i] == '/' && chars.get(i + 1) == Some(&'*') {
                    depth += 1;
                    out.push_str("  ");
                    i += 2;
                } else if chars[i] == '*' && chars.get(i + 1) == Some(&'/') {
                    depth -= 1;
                    out.push_str("  ");
                    i += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    out.push(blank(chars[i]));
                    i += 1;
                }
            }
        } else if c == 'r' && (next == Some('"') || next == Some('#')) && !ident(&chars, i) {
            // A raw string: r"…", r#"…"#.
            let mut j = i + 1;
            let mut hashes = 0;
            while chars.get(j) == Some(&'#') {
                hashes += 1;
                j += 1;
            }
            if chars.get(j) != Some(&'"') {
                out.push(c);
                i += 1;
                continue;
            }
            let end: String = std::iter::once('"')
                .chain(std::iter::repeat_n('#', hashes))
                .collect();
            let rest: String = chars[j + 1..].iter().collect();
            let length = rest
                .find(&end)
                .map_or(rest.chars().count(), |at| rest[..at].chars().count());
            let stop = (j + 1 + length + 1 + hashes).min(chars.len());
            for &c in &chars[i..stop] {
                out.push(blank(c));
            }
            i = stop;
        } else if c == '"' {
            out.push(' ');
            i += 1;
            while i < chars.len() && chars[i] != '"' {
                if chars[i] == '\\' {
                    out.push(' ');
                    i += 1;
                }
                if i < chars.len() {
                    out.push(blank(chars[i]));
                    i += 1;
                }
            }
            out.push(' ');
            i += 1;
        } else if c == '\'' && is_char_literal(&chars, i) {
            // `'\''` and `'\u{2502}'` close at the first quote after the escape's first letter.
            let stop = if chars[i + 1] == '\\' {
                (i + 3..chars.len())
                    .find(|&j| chars[j] == '\'')
                    .map_or(i + 1, |j| j + 1)
            } else {
                i + 3
            };
            for &c in &chars[i..stop] {
                out.push(blank(c));
            }
            i = stop;
        } else {
            out.push(c);
            i += 1;
        }
    }
    out
}

/// Whether the character before `i` continues an identifier, as in `for` or `bar"`.
fn ident(chars: &[char], i: usize) -> bool {
    i > 0 && (chars[i - 1].is_alphanumeric() || chars[i - 1] == '_')
}

/// A quote that opens a character literal, not a lifetime: `'a'`, `'\n'`, `'\''`.
fn is_char_literal(chars: &[char], i: usize) -> bool {
    matches!(
        (chars.get(i + 1), chars.get(i + 2)),
        (Some('\\'), _) | (Some(_), Some('\''))
    )
}

/// The text before the file's test module: a `#[cfg(test)]` on a `mod`.
fn without_tests(text: &str) -> &str {
    let mut from = 0;
    while let Some(at) = text[from..].find("#[cfg(test)]") {
        let start = from + at;
        let after = text[start + "#[cfg(test)]".len()..].trim_start();
        if after.starts_with("mod ") {
            return &text[..start];
        }
        from = start + 1;
    }
    text
}

/// Each `fn draw…` and `fn paint…` in `text`, as its name, first line and body.
fn drawing(text: &str) -> Vec<(String, usize, String)> {
    let mut found = Vec::new();
    let mut from = 0;
    while let Some(at) = text[from..].find("fn ") {
        let start = from + at;
        from = start + 3;
        if start > 0
            && text[..start]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_alphanumeric() || c == '_')
        {
            continue;
        }
        let name: String = text[start + 3..]
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        if !(name.starts_with("draw") || name.starts_with("paint")) {
            continue;
        }
        // The body: from the first brace after the signature to the one that closes it. A
        // `;` first is a declaration in a trait, with no body; one inside brackets, as in
        // `[u8; 4]`, is part of a type.
        let rest = &text[start..];
        let mut nested = 0i32;
        let open = rest.char_indices().find(|&(_, c)| {
            match c {
                '(' | '[' => nested += 1,
                ')' | ']' => nested -= 1,
                _ => {}
            }
            nested == 0 && (c == '{' || c == ';')
        });
        let Some((open, '{')) = open else {
            continue;
        };
        let mut depth = 0;
        let mut end = rest.len();
        for (index, c) in rest[open..].char_indices() {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        end = open + index + 1;
                        break;
                    }
                }
                _ => {}
            }
        }
        let line = text[..start].lines().count() + 1;
        found.push((name, line, rest[..end].to_string()));
    }
    found
}

#[test]
fn drawing_never_reaches_the_disk_or_another_process() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut files = Vec::new();
    for name in CRATES {
        sources(&root.join(name).join("src"), &mut files);
    }
    files.sort();
    let mut bad = Vec::new();
    let mut checked = 0;
    for file in &files {
        let text = std::fs::read_to_string(file).unwrap();
        let code = blanked(without_tests(&text));
        for (name, line, body) in drawing(&code) {
            checked += 1;
            for (call, reaches) in FORBIDDEN {
                if body.contains(call) {
                    let shown = file
                        .strip_prefix(&root)
                        .unwrap_or(file)
                        .display()
                        .to_string();
                    bad.push(format!(
                        "{shown}:{line} fn {name} calls {call}, which reaches {reaches}"
                    ));
                }
            }
        }
    }
    assert!(
        checked > 120,
        "only {checked} drawing functions found: the scan is broken"
    );
    assert!(
        bad.is_empty(),
        "drawing must not wait on the disk or a process:\n{}",
        bad.join("\n")
    );
}

/// The scan itself: comments, strings and tests don't count; a body does.
#[test]
fn the_scan_finds_a_call_in_a_body_and_nowhere_else() {
    let text = r##"
/// Calls std::fs::read in a comment.
fn draw_one(grid: &mut Grid) {
    let label = "std::fs::read";
    let c = '"';
    let r = r#"Command::new("x")"#;
    grid.put(label);
}
fn paint_two() {
    let text = std::fs::read_to_string("a").unwrap();
}
fn layout() { std::fs::read("b"); }
#[cfg(test)]
mod tests { fn draw_test() { std::fs::read("c"); } }
"##;
    let code = blanked(without_tests(text));
    let found = drawing(&code);
    let names: Vec<&str> = found.iter().map(|(n, ..)| n.as_str()).collect();
    assert_eq!(names, ["draw_one", "paint_two"]);
    assert!(!found[0].2.contains("fs::") && !found[0].2.contains("Command::new"));
    assert!(found[1].2.contains("std::fs"));
}
