//! Counting a document's words as a reader would: the prose, without markup, code or math.

use crate::changes::{Syntax, settle_all};

/// Words in a Markdown document or a LaTeX paper, with pending suggestions taken as accepted.
pub fn count(text: &str, latex: bool) -> usize {
    let prose = if latex {
        latex_prose(&settle_all(text, Syntax::Latex, true))
    } else {
        markdown_prose(&settle_all(text, Syntax::Critic, true))
    };
    prose
        .split_whitespace()
        .filter(|word| word.chars().any(char::is_alphanumeric))
        .count()
}

/// Markdown without its front matter, code blocks, comments and link targets.
fn markdown_prose(text: &str) -> String {
    let (_, body) = crate::front::split(text);
    let mut out = String::with_capacity(body.len());
    let mut fence: Option<&str> = None;
    let mut comment = false;
    for line in body.lines() {
        let trimmed = line.trim_start();
        if let Some(open) = fence {
            if trimmed.starts_with(open) {
                fence = None;
            }
            continue;
        }
        if let Some(open) = ["```", "~~~"].into_iter().find(|f| trimmed.starts_with(f)) {
            fence = Some(open);
            continue;
        }
        let mut rest = line;
        while !rest.is_empty() {
            if comment {
                match rest.find("-->") {
                    Some(end) => {
                        rest = &rest[end + 3..];
                        comment = false;
                    }
                    None => rest = "",
                }
            } else if let Some(start) = rest.find("<!--") {
                out.push_str(&without_targets(&rest[..start]));
                rest = &rest[start + 4..];
                comment = true;
            } else {
                out.push_str(&without_targets(rest));
                rest = "";
            }
        }
        out.push('\n');
    }
    out
}

/// A line with each link's `(target)` taken out, keeping the text in its brackets.
fn without_targets(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(at) = rest.find("](") {
        out.push_str(&rest[..at]);
        out.push(' ');
        rest = match rest[at..].find(')') {
            Some(close) => &rest[at + close + 1..],
            None => "",
        };
    }
    out.push_str(rest);
    out
}

/// Commands whose argument isn't prose: names, keys and paths.
const NOT_PROSE: [&str; 22] = [
    "label",
    "ref",
    "eqref",
    "cref",
    "Cref",
    "autoref",
    "pageref",
    "cite",
    "citep",
    "citet",
    "nocite",
    "input",
    "include",
    "usepackage",
    "documentclass",
    "bibliography",
    "bibliographystyle",
    "includegraphics",
    "url",
    "newcommand",
    "renewcommand",
    "hspace",
];

/// Environments that hold math.
const MATH: [&str; 12] = [
    "equation",
    "equation*",
    "align",
    "align*",
    "gather",
    "gather*",
    "multline",
    "multline*",
    "displaymath",
    "eqnarray",
    "eqnarray*",
    "math",
];

/// A paper's body without its comments, commands, math and keys.
fn latex_prose(text: &str) -> String {
    let mut plain = String::with_capacity(text.len());
    for line in text.lines() {
        plain.push_str(uncommented(line));
        plain.push('\n');
    }
    let body = match plain.find("\\begin{document}") {
        Some(start) => &plain[start + "\\begin{document}".len()..],
        None => &plain,
    };
    let body = body
        .find("\\end{document}")
        .map_or(body, |end| &body[..end]);
    let chars: Vec<char> = body.chars().collect();
    let mut out = String::with_capacity(body.len());
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '$' => {
                let double = chars.get(i + 1) == Some(&'$');
                let open = if double { 2 } else { 1 };
                i += open;
                while i < chars.len() && !(chars[i] == '$' && chars[i - 1] != '\\') {
                    i += 1;
                }
                i += open;
                out.push(' ');
            }
            '\\' => {
                let name: String = chars[i + 1..]
                    .iter()
                    .take_while(|c| c.is_ascii_alphabetic())
                    .collect();
                if name.is_empty() {
                    match chars.get(i + 1) {
                        // Display math.
                        Some('[') => {
                            i = find(&chars, i + 2, "\\]").map_or(chars.len(), |end| end + 2);
                            out.push(' ');
                        }
                        Some(c) if c.is_ascii_punctuation() && *c != '\\' => {
                            out.push(*c);
                            i += 2;
                        }
                        _ => {
                            out.push(' ');
                            i += 2;
                        }
                    }
                    continue;
                }
                i += 1 + name.chars().count();
                out.push(' ');
                if name == "begin" || name == "end" {
                    let (env, after) = argument(&chars, i);
                    i = after;
                    if name == "begin" && MATH.contains(&env.as_str()) {
                        let end = format!("\\end{{{env}}}");
                        i = find(&chars, i, &end)
                            .map_or(chars.len(), |at| at + end.chars().count());
                    }
                } else if NOT_PROSE.contains(&name.as_str()) {
                    while chars.get(i) == Some(&'[') {
                        i = chars[i..]
                            .iter()
                            .position(|c| *c == ']')
                            .map_or(chars.len(), |at| i + at + 1);
                    }
                    let (_, after) = argument(&chars, i);
                    i = after;
                }
            }
            '{' | '}' => i += 1,
            '~' => {
                out.push(' ');
                i += 1;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    out
}

/// A line up to its `%` comment, if it has one.
fn uncommented(line: &str) -> &str {
    let bytes = line.as_bytes();
    for (at, b) in bytes.iter().enumerate() {
        if *b == b'%' && (at == 0 || bytes[at - 1] != b'\\') {
            return &line[..at];
        }
    }
    line
}

/// Where `text` next starts, from `from` on.
fn find(chars: &[char], from: usize, text: &str) -> Option<usize> {
    let wanted: Vec<char> = text.chars().collect();
    (from..chars.len()).find(|at| chars[*at..].starts_with(&wanted))
}

/// The braced argument starting at `at`, if there is one, and where it ends.
fn argument(chars: &[char], at: usize) -> (String, usize) {
    if chars.get(at) != Some(&'{') {
        return (String::new(), at);
    }
    let mut depth = 0;
    for (n, c) in chars[at..].iter().enumerate() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return (chars[at + 1..at + n].iter().collect(), at + n + 1);
                }
            }
            _ => {}
        }
    }
    (String::new(), chars.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_counts_its_prose_not_its_markup() {
        let text = "---\ntitle: X\n---\n# Heading here\n\n- one **two** [three](http://x.y/z)\n\n```\ncode words here\n```\n<!-- a note -->\nEnd {--gone--}{++new++}.\n";
        assert_eq!(count(text, false), 7, "Heading here one two three End new");
    }

    #[test]
    fn latex_counts_the_body_without_commands_math_or_keys() {
        let text = "\\documentclass{article}\n\\usepackage{amsmath}\n\\begin{document}\n\\section{Intro}\\label{sec:a}\nSome \\emph{nice} text, see~\\ref{sec:a} and $x^2$ % a comment here\n\\begin{equation}\na = b\n\\end{equation}\n100\\% done.\n\\end{document}\n";
        assert_eq!(
            count(text, true),
            8,
            "Intro Some nice text see and 100% done"
        );
    }
}
