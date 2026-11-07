//! `.editorconfig`: the indentation and line rules a project sets for its files, read from
//! the file's folder up to the nearest `root = true`.

use std::path::Path;

/// How a file is indented.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Indentation {
    pub tabs: bool,
    /// Columns one level takes, and how wide a tab is drawn.
    pub size: usize,
}

impl Default for Indentation {
    fn default() -> Self {
        Self {
            tabs: false,
            size: 4,
        }
    }
}

impl Indentation {
    /// What one level of indentation types.
    pub fn unit(&self) -> String {
        if self.tabs {
            "\t".to_string()
        } else {
            " ".repeat(self.size)
        }
    }

    /// As the status bar shows it, as VS Code does.
    pub fn label(&self) -> String {
        if self.tabs {
            format!("Tab Size: {}", self.size)
        } else {
            format!("Spaces: {}", self.size)
        }
    }
}

/// The line endings a file is saved with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Eol {
    Lf,
    Crlf,
    Cr,
}

impl Eol {
    pub fn text(self) -> &'static str {
        match self {
            Self::Lf => "\n",
            Self::Crlf => "\r\n",
            Self::Cr => "\r",
        }
    }
}

/// What applies to one file. `None` leaves the editor's own way.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Rules {
    pub indent_tabs: Option<bool>,
    pub indent_size: Option<usize>,
    pub tab_width: Option<usize>,
    pub final_newline: Option<bool>,
    pub trim_trailing: Option<bool>,
    pub eol: Option<Eol>,
}

impl Rules {
    /// The rules for `path`, from every `.editorconfig` above it.
    pub fn for_file(path: &Path) -> Self {
        let mut files = Vec::new();
        let mut folder = path.parent();
        while let Some(dir) = folder {
            if let Ok(text) = std::fs::read_to_string(dir.join(".editorconfig")) {
                let root = is_root(&text);
                files.push((dir.to_path_buf(), text));
                if root {
                    break;
                }
            }
            folder = dir.parent();
        }
        let mut rules = Self::default();
        // Nearer files win, so the farthest is read first.
        for (dir, text) in files.iter().rev() {
            let Ok(relative) = path.strip_prefix(dir) else {
                continue;
            };
            let relative = relative.to_string_lossy().replace('\\', "/");
            rules.read(text, &relative);
        }
        rules
    }

    /// Takes in the sections of one file that match `relative`, its path from that file's
    /// folder.
    fn read(&mut self, text: &str, relative: &str) {
        let mut matching = false;
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with(['#', ';']) {
                continue;
            }
            if let Some(section) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
                matching = section_matches(section, relative);
                continue;
            }
            if !matching {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let key = key.trim().to_ascii_lowercase();
            let value = value.trim().to_ascii_lowercase();
            let flag = match value.as_str() {
                "true" => Some(true),
                "false" => Some(false),
                _ => None,
            };
            match key.as_str() {
                "indent_style" => {
                    self.indent_tabs = match value.as_str() {
                        "tab" => Some(true),
                        "space" => Some(false),
                        _ => None,
                    };
                }
                "indent_size" if value == "tab" => self.indent_size = None,
                "indent_size" => self.indent_size = value.parse().ok().filter(|n| *n > 0),
                "tab_width" => self.tab_width = value.parse().ok().filter(|n| *n > 0),
                "insert_final_newline" => self.final_newline = flag,
                "trim_trailing_whitespace" => self.trim_trailing = flag,
                "end_of_line" => {
                    self.eol = match value.as_str() {
                        "lf" => Some(Eol::Lf),
                        "crlf" => Some(Eol::Crlf),
                        "cr" => Some(Eol::Cr),
                        _ => None,
                    };
                }
                _ => {}
            }
        }
    }

    /// The indentation these rules ask for, on top of `default`.
    pub fn indentation(&self, default: Indentation) -> Indentation {
        let tabs = self.indent_tabs.unwrap_or(default.tabs);
        let size = if tabs {
            self.tab_width.or(self.indent_size)
        } else {
            self.indent_size.or(self.tab_width)
        };
        Indentation {
            tabs,
            size: size.unwrap_or(default.size),
        }
    }

    /// The changes saving makes to `text`, as (start, end, replacement) in chars, in order:
    /// trailing spaces taken off, line endings made one kind, the last newline added or
    /// taken away.
    pub fn edits(&self, text: &str) -> Vec<(usize, usize, String)> {
        let mut edits = Vec::new();
        if self.trim_trailing != Some(true) && self.final_newline.is_none() && self.eol.is_none() {
            return edits;
        }
        let eol = self.eol.map(Eol::text);
        let chars: Vec<char> = text.chars().collect();
        let mut first_ending = None;
        let mut start = 0;
        while start < chars.len() {
            let end = (start..chars.len())
                .find(|&i| matches!(chars[i], '\n' | '\r'))
                .unwrap_or(chars.len());
            if self.trim_trailing == Some(true) {
                let kept = (start..end)
                    .rev()
                    .find(|&i| !matches!(chars[i], ' ' | '\t'))
                    .map_or(start, |i| i + 1);
                if kept < end {
                    edits.push((kept, end, String::new()));
                }
            }
            if end == chars.len() {
                break;
            }
            let crlf = chars[end] == '\r' && chars.get(end + 1) == Some(&'\n');
            let after = end + if crlf { 2 } else { 1 };
            let ending: String = chars[end..after].iter().collect();
            first_ending.get_or_insert(ending.clone());
            if let Some(eol) = eol
                && ending != eol
            {
                edits.push((end, after, eol.to_string()));
            }
            start = after;
        }
        let ends_open = chars.last().is_some_and(|c| !matches!(c, '\n' | '\r'));
        match self.final_newline {
            Some(true) if ends_open => {
                let ending = eol.map(str::to_string).or(first_ending);
                edits.push((
                    chars.len(),
                    chars.len(),
                    ending.unwrap_or_else(|| "\n".into()),
                ));
            }
            Some(false) => {
                let mut cut = chars.len();
                while cut > 0 && matches!(chars[cut - 1], '\n' | '\r') {
                    cut -= 1;
                }
                // The endings being cut are no longer changed, only taken away.
                edits.retain(|(s, _, _)| *s < cut);
                if cut < chars.len() {
                    edits.push((cut, chars.len(), String::new()));
                }
            }
            _ => {}
        }
        edits
    }

    /// `text` as these rules would save it.
    #[cfg(test)]
    fn apply(&self, text: &str) -> String {
        let mut chars: Vec<char> = text.chars().collect();
        for (start, end, with) in self.edits(text).into_iter().rev() {
            chars.splice(start..end, with.chars());
        }
        chars.into_iter().collect()
    }
}

fn is_root(text: &str) -> bool {
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            return false;
        }
        if let Some((key, value)) = line.split_once('=')
            && key.trim().eq_ignore_ascii_case("root")
        {
            return value.trim().eq_ignore_ascii_case("true");
        }
    }
    false
}

/// Whether a section's glob takes in `relative`. A glob without a `/` matches the file's
/// name in any folder; with one, the path from the `.editorconfig`'s folder.
fn section_matches(glob: &str, relative: &str) -> bool {
    let glob = glob.trim();
    if glob.contains('/') {
        let glob = glob.strip_prefix('/').unwrap_or(glob);
        glob_matches(glob, relative)
    } else {
        let name = relative.rsplit('/').next().unwrap_or(relative);
        glob_matches(glob, name)
    }
}

/// EditorConfig's globs: `*` within a folder, `**` across folders, `?`, `[abc]`, `[!abc]`,
/// `{a,b}` and `{1..3}`.
fn glob_matches(glob: &str, text: &str) -> bool {
    expand_braces(glob).iter().any(|g| {
        matches_from(
            &g.chars().collect::<Vec<_>>(),
            &text.chars().collect::<Vec<_>>(),
        )
    })
}

/// Every glob a pattern with braces stands for.
fn expand_braces(glob: &str) -> Vec<String> {
    let Some(open) = glob.find('{') else {
        return vec![glob.to_string()];
    };
    let mut depth = 0;
    let mut close = None;
    for (i, c) in glob[open..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    close = Some(open + i);
                    break;
                }
            }
            _ => {}
        }
    }
    let Some(close) = close else {
        return vec![glob.to_string()];
    };
    let (head, inner, tail) = (&glob[..open], &glob[open + 1..close], &glob[close + 1..]);
    let choices: Vec<String> = match inner.split_once("..") {
        Some((a, b)) if a.parse::<i64>().is_ok() && b.parse::<i64>().is_ok() => {
            let (a, b) = (a.parse::<i64>().unwrap(), b.parse::<i64>().unwrap());
            (a.min(b)..=a.max(b)).map(|n| n.to_string()).collect()
        }
        _ => split_top(inner),
    };
    let mut out = Vec::new();
    for choice in choices {
        for rest in expand_braces(&format!("{choice}{tail}")) {
            out.push(format!("{head}{rest}"));
        }
    }
    out
}

/// Splits on commas outside nested braces.
fn split_top(text: &str) -> Vec<String> {
    let mut parts = vec![String::new()];
    let mut depth = 0;
    for c in text.chars() {
        match c {
            '{' => depth += 1,
            '}' => depth -= 1,
            ',' if depth == 0 => {
                parts.push(String::new());
                continue;
            }
            _ => {}
        }
        parts.last_mut().expect("never empty").push(c);
    }
    parts
}

fn matches_from(glob: &[char], text: &[char]) -> bool {
    let Some(&first) = glob.first() else {
        return text.is_empty();
    };
    match first {
        '*' if glob.get(1) == Some(&'*') => {
            let rest = &glob[2..];
            // `**/` also matches no folder at all.
            if rest.first() == Some(&'/') && matches_from(&rest[1..], text) {
                return true;
            }
            (0..=text.len()).any(|i| matches_from(rest, &text[i..]))
        }
        '*' => {
            let rest = &glob[1..];
            for i in 0..=text.len() {
                if matches_from(rest, &text[i..]) {
                    return true;
                }
                if text.get(i) == Some(&'/') {
                    break;
                }
            }
            false
        }
        '?' => text.first().is_some_and(|c| *c != '/') && matches_from(&glob[1..], &text[1..]),
        '[' => {
            let Some(end) = glob.iter().position(|c| *c == ']').filter(|e| *e > 1) else {
                return text.first() == Some(&'[') && matches_from(&glob[1..], &text[1..]);
            };
            let Some(&c) = text.first() else {
                return false;
            };
            let class = &glob[1..end];
            let (negate, class) = match class.first() {
                Some('!' | '^') => (true, &class[1..]),
                _ => (false, class),
            };
            let mut found = false;
            let mut i = 0;
            while i < class.len() {
                if i + 2 < class.len() && class[i + 1] == '-' {
                    found |= class[i] <= c && c <= class[i + 2];
                    i += 3;
                } else {
                    found |= class[i] == c;
                    i += 1;
                }
            }
            found != negate && matches_from(&glob[end + 1..], &text[1..])
        }
        '\\' if glob.len() > 1 => {
            text.first() == Some(&glob[1]) && matches_from(&glob[2..], &text[1..])
        }
        c => text.first() == Some(&c) && matches_from(&glob[1..], &text[1..]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn globs_match_as_editorconfig_says() {
        assert!(glob_matches("*.rs", "main.rs"));
        assert!(!glob_matches("*.rs", "main.rsx"));
        assert!(glob_matches("*.{js,ts}", "app.ts"));
        assert!(glob_matches("lib/**.js", "lib/a/b/c.js"));
        assert!(glob_matches("**/Makefile", "Makefile"));
        assert!(glob_matches("file[0-9].txt", "file3.txt"));
        assert!(!glob_matches("file[!0-9].txt", "file3.txt"));
        assert!(glob_matches("v{1..3}.md", "v2.md"));
        assert!(!glob_matches("*.md", "docs/a.md/x"));
        assert!(section_matches("*.md", "docs/notes/a.md"));
        assert!(section_matches("docs/*.md", "docs/a.md"));
        assert!(!section_matches("docs/*.md", "src/docs/a.md"));
    }

    #[test]
    fn nearer_files_win_and_root_stops_the_walk() {
        let dir = tempfile::tempdir().unwrap();
        let top = dir.path();
        std::fs::write(top.join(".editorconfig"), "[*]\nindent_size = 8\n").unwrap();
        let project = top.join("project");
        std::fs::create_dir_all(project.join("src")).unwrap();
        std::fs::write(
            project.join(".editorconfig"),
            "root = true\n\n[*]\nindent_style = space\nindent_size = 2\n\
             insert_final_newline = true\n\n[Makefile]\nindent_style = tab\n",
        )
        .unwrap();
        std::fs::write(
            project.join("src/.editorconfig"),
            "[*.rs]\nindent_size = 4\ntrim_trailing_whitespace = true\nend_of_line = crlf\n",
        )
        .unwrap();
        let rust = Rules::for_file(&project.join("src/main.rs"));
        assert_eq!(
            rust.indentation(Indentation::default()),
            Indentation {
                tabs: false,
                size: 4
            }
        );
        assert_eq!(rust.final_newline, Some(true));
        assert_eq!(rust.trim_trailing, Some(true));
        assert_eq!(rust.eol, Some(Eol::Crlf));
        let make = Rules::for_file(&project.join("Makefile"));
        assert_eq!(
            make.indentation(Indentation::default()),
            Indentation {
                tabs: true,
                size: 2
            }
        );
        // The 8 above the root is never read.
        let readme = Rules::for_file(&project.join("README.md"));
        assert_eq!(readme.indentation(Indentation::default()).size, 2);
    }

    #[test]
    fn saving_trims_ends_lines_and_adds_the_last_newline() {
        let rules = Rules {
            trim_trailing: Some(true),
            final_newline: Some(true),
            eol: Some(Eol::Crlf),
            ..Rules::default()
        };
        assert_eq!(rules.apply("a  \nb\t\r\nc"), "a\r\nb\r\nc\r\n");
        let strip = Rules {
            final_newline: Some(false),
            ..Rules::default()
        };
        assert_eq!(strip.apply("a\n\n"), "a");
        assert_eq!(Rules::default().apply("keep  \n"), "keep  \n");
    }
}
