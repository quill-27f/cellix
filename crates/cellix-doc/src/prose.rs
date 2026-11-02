//! Spelling and grammar for writing. Harper checks plain English, so what isn't prose is
//! blanked out first, keeping every other char where it was, and what it finds still lands on
//! the right words in the source.

use std::collections::HashSet;
use std::sync::Arc;

use harper_core::language_detection::is_doc_likely_english;
use harper_core::linting::{LintGroup, LintKind, Linter, Suggestion};
use harper_core::spell::FstDictionary;
use harper_core::{Dialect, Document};

/// One thing to look at, as a range of chars in the source.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Issue {
    pub start: usize,
    pub end: usize,
    pub message: String,
    /// What it could be instead.
    pub fixes: Vec<String>,
    pub spelling: bool,
}

/// What a document is written in, which says what isn't prose.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Markdown,
    Latex,
}

fn blank(chars: &mut [char], from: usize, to: usize) {
    let to = to.min(chars.len());
    for c in &mut chars[from..to] {
        if *c != '\n' {
            *c = ' ';
        }
    }
}

fn find_from(chars: &[char], from: usize, wanted: &[char]) -> Option<usize> {
    (from..chars.len().saturating_sub(wanted.len() - 1)).find(|at| chars[*at..].starts_with(wanted))
}

fn markdown(text: &str) -> Vec<char> {
    let mut chars: Vec<char> = text.chars().collect();
    let mut at = 0;
    let mut line_start = true;
    let mut fenced = false;
    while at < chars.len() {
        let c = chars[at];
        if line_start {
            let end = chars[at..]
                .iter()
                .position(|c| *c == '\n')
                .map_or(chars.len(), |n| at + n);
            let line: String = chars[at..end].iter().collect();
            let trimmed = line.trim_start();
            let fence = trimmed.starts_with("```");
            fenced ^= fence;
            let table_rule = line.contains('-') && line.chars().all(|c| "|-: ".contains(c));
            if fence || fenced || table_rule {
                blank(&mut chars, at, end);
                at = end;
                line_start = false;
                continue;
            }
            // A line's marker isn't a word.
            let marker = trimmed
                .chars()
                .take_while(|c| "#>-*+ ".contains(*c) || c.is_ascii_digit() || *c == '.')
                .count();
            let indent = line.chars().count() - trimmed.chars().count();
            blank(&mut chars, at, at + indent + marker);
            at += indent + marker;
            line_start = false;
            continue;
        }
        match c {
            '\n' => {
                line_start = true;
                at += 1;
            }
            '`' | '$' => {
                let double = chars.get(at + 1) == Some(&c);
                let wanted = if double { vec![c, c] } else { vec![c] };
                match find_from(&chars, at + wanted.len(), &wanted) {
                    Some(close) => {
                        blank(&mut chars, at, close + wanted.len());
                        at = close + wanted.len();
                    }
                    None => at += 1,
                }
            }
            // A link keeps its words, and loses its address. An image loses both.
            ']' if chars.get(at + 1) == Some(&'(') => {
                let close = find_from(&chars, at, &[')']).unwrap_or(at);
                blank(&mut chars, at, close + 1);
                at = close + 1;
            }
            '*' | '_' | '|' | '[' | ']' | '{' | '}' | '~' | '=' | '<' | '>' | '^' => {
                chars[at] = ' ';
                at += 1;
            }
            'h' if chars[at..].starts_with(&['h', 't', 't', 'p']) => {
                let end = chars[at..]
                    .iter()
                    .position(|c| c.is_whitespace())
                    .map_or(chars.len(), |n| at + n);
                blank(&mut chars, at, end);
                at = end;
            }
            _ => at += 1,
        }
    }
    chars
}

/// Commands whose argument is words, not a name.
const WORDS_INSIDE: [&str; 12] = [
    "section",
    "subsection",
    "subsubsection",
    "chapter",
    "paragraph",
    "caption",
    "title",
    "textbf",
    "textit",
    "emph",
    "underline",
    "footnote",
];
/// Environments with no prose in them.
const NOT_PROSE: [&str; 13] = [
    "equation",
    "align",
    "gather",
    "multline",
    "displaymath",
    "verbatim",
    "lstlisting",
    "minted",
    "tikzpicture",
    "tikzcd",
    "tabular",
    "thebibliography",
    "algorithmic",
];

fn latex(text: &str) -> Vec<char> {
    let mut chars: Vec<char> = text.chars().collect();
    let begin: Vec<char> = "\\begin{document}".chars().collect();
    let mut at = find_from(&chars, 0, &begin).map_or(0, |found| {
        blank(&mut chars, 0, found + begin.len());
        found + begin.len()
    });
    while at < chars.len() {
        match chars[at] {
            '%' if at == 0 || chars[at - 1] != '\\' => {
                let end = chars[at..]
                    .iter()
                    .position(|c| *c == '\n')
                    .map_or(chars.len(), |n| at + n);
                blank(&mut chars, at, end);
                at = end;
            }
            '$' => match find_from(&chars, at + 1, &['$']) {
                Some(close) => {
                    blank(&mut chars, at, close + 1);
                    at = close + 1;
                }
                None => at += 1,
            },
            '\\' => {
                let name: String = chars[at + 1..]
                    .iter()
                    .take_while(|c| c.is_ascii_alphabetic() || **c == '*')
                    .collect();
                let mut end = at + 1 + name.chars().count().max(1);
                if name == "begin" || name == "end" {
                    let close = find_from(&chars, end, &['}']).unwrap_or(end);
                    let env: String = chars[end..close]
                        .iter()
                        .filter(|c| !"{*".contains(**c))
                        .collect();
                    if name == "begin" && NOT_PROSE.contains(&env.as_str()) {
                        let wanted: Vec<char> = format!("\\end{{{env}").chars().collect();
                        let stop = find_from(&chars, close, &wanted).unwrap_or(close);
                        blank(&mut chars, at, stop);
                        at = stop;
                        continue;
                    }
                    blank(&mut chars, at, close + 1);
                    at = close + 1;
                    continue;
                }
                if name.is_empty() && chars.get(at + 1) == Some(&'[') {
                    let close = find_from(&chars, at, &['\\', ']']).unwrap_or(at);
                    blank(&mut chars, at, close + 2);
                    at = close + 2;
                    continue;
                }
                if chars.get(end) == Some(&'[')
                    && let Some(close) = find_from(&chars, end, &[']'])
                {
                    end = close + 1;
                }
                let keeps = WORDS_INSIDE.contains(&name.trim_end_matches('*'));
                let has_argument = chars.get(end) == Some(&'{');
                if keeps && has_argument {
                    blank(&mut chars, at, end + 1);
                    at = end + 1;
                } else if has_argument {
                    let close = find_from(&chars, end, &['}']).unwrap_or(end);
                    blank(&mut chars, at, close + 1);
                    at = close + 1;
                } else {
                    blank(&mut chars, at, end);
                    at = end;
                }
            }
            '{' | '}' | '~' | '&' => {
                chars[at] = ' ';
                at += 1;
            }
            _ => at += 1,
        }
    }
    chars
}

/// The prose of a document, the same length as its source.
pub fn prose(text: &str, source: Source) -> String {
    let chars = match source {
        Source::Markdown => markdown(text),
        Source::Latex => latex(text),
    };
    chars.into_iter().collect()
}

/// Kinds of finding worth a writer's attention. Spacing and capitals are left out, since
/// the blanks above would set them off.
fn worth_showing(kind: LintKind) -> bool {
    matches!(
        kind,
        LintKind::Spelling
            | LintKind::Typo
            | LintKind::Grammar
            | LintKind::Agreement
            | LintKind::Repetition
            | LintKind::Malapropism
            | LintKind::Eggcorn
            | LintKind::WordOrder
            | LintKind::Usage
    )
}

/// Words that aren't English to begin with: names like ImageNet and CIFAR, anything with a
/// digit in it, a letter or two left by a formula, and what keyboards call their keys.
fn technical(word: &str) -> bool {
    const KEYS: [&str; 4] = ["Ctrl", "Cmd", "Esc", "Alt"];
    let mut letters = word.chars();
    let first = letters.next();
    let inner_capital = letters.any(char::is_uppercase);
    word.chars().count() <= 2
        || inner_capital
        || first.is_some_and(|c| c.is_ascii_digit())
        || word.chars().any(|c| c.is_ascii_digit())
        || KEYS.contains(&word)
}

/// Whether a suggestion is only the other side of the Atlantic: minimises for minimizes,
/// colour for color. Either is right, and the writer knows which they mean.
fn regional(word: &str, fix: &Suggestion) -> bool {
    let Suggestion::ReplaceWith(fix) = fix else {
        return false;
    };
    let fix: String = fix.iter().collect::<String>().to_lowercase();
    let word = word.to_lowercase();
    let plain = |text: &str| {
        text.replace("ise", "ize")
            .replace("isa", "iza")
            .replace("yse", "yze")
            .replace("our", "or")
            .replace("tre", "ter")
            .replace("ll", "l")
    };
    plain(&word) == plain(&fix)
}

/// Does the checking. Starting it takes a moment, so it lives on its own thread.
pub struct Checker {
    dictionary: Arc<FstDictionary>,
    group: LintGroup,
    /// Words the writer has said are right, in lower case.
    pub known: HashSet<String>,
}

impl Checker {
    pub fn new(known: HashSet<String>) -> Self {
        let dictionary = FstDictionary::curated();
        let group = LintGroup::new_curated(dictionary.clone(), Dialect::American);
        Self {
            dictionary,
            group,
            known,
        }
    }

    /// What's wrong with a document, or nothing if it doesn't read as English.
    pub fn check(&mut self, text: &str, source: Source) -> Vec<Issue> {
        let cleaned = prose(text, source);
        let chars: Vec<char> = cleaned.chars().collect();
        let document = Document::new_plain_english_curated(&cleaned);
        if !is_doc_likely_english(&document, self.dictionary.as_ref()) {
            return Vec::new();
        }
        let mut issues = Vec::new();
        for lint in self.group.lint(&document) {
            if !worth_showing(lint.lint_kind) {
                continue;
            }
            let (start, end) = (lint.span.start, lint.span.end.min(chars.len()));
            let word: String = chars[start..end].iter().collect();
            let spelling = lint.lint_kind == LintKind::Spelling;
            if spelling && (self.known.contains(&word.to_lowercase()) || technical(&word)) {
                continue;
            }
            if spelling && lint.suggestions.iter().any(|fix| regional(&word, fix)) {
                continue;
            }
            let fixes = lint
                .suggestions
                .iter()
                .filter_map(|fix| match fix {
                    Suggestion::ReplaceWith(text) => Some(text.iter().collect()),
                    Suggestion::Remove => Some(String::new()),
                    Suggestion::InsertAfter(_) => None,
                })
                .take(5)
                .collect();
            issues.push(Issue {
                start,
                end,
                message: lint.message,
                fixes,
                spelling,
            });
        }
        issues
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prose_stays_where_it_was_and_the_rest_goes() {
        let text = "# Title\n\nSome `code` and $x^2$ and [a link](http://x.y) here.\n\n```\nfn broken\n```\n";
        let cleaned = prose(text, Source::Markdown);
        assert_eq!(cleaned.chars().count(), text.chars().count());
        assert!(cleaned.contains("Title"));
        assert!(cleaned.contains("a link"));
        for gone in ["code", "x^2", "http", "broken", "#"] {
            assert!(!cleaned.contains(gone), "{gone}: {cleaned}");
        }
    }

    #[test]
    fn a_papers_commands_go_and_its_words_stay() {
        let text = "\\documentclass{article}\n\\usepackage{tikz}\n\\begin{document}\n\\section{Intro words}\nText \\cite{knuth} with \\emph{stress} and $x$.\n\\begin{equation}\nE = mc^2\n\\end{equation}\n% a comment\n\\end{document}\n";
        let cleaned = prose(text, Source::Latex);
        assert_eq!(cleaned.chars().count(), text.chars().count());
        assert!(cleaned.contains("Intro words") && cleaned.contains("stress"));
        for gone in ["documentclass", "knuth", "mc^2", "comment", "tikz"] {
            assert!(!cleaned.contains(gone), "{gone}: {cleaned}");
        }
    }

    #[test]
    fn a_misspelling_is_found_with_its_fix() {
        let mut checker = Checker::new(HashSet::new());
        let text = "This sentence has a mispelled word in the middle of it.\n";
        let issues = checker.check(text, Source::Markdown);
        let issue = issues.iter().find(|i| i.spelling).expect("found");
        let word: String = text
            .chars()
            .skip(issue.start)
            .take(issue.end - issue.start)
            .collect();
        assert_eq!(word, "mispelled");
        assert!(
            issue.fixes.iter().any(|f| f == "misspelled"),
            "{:?}",
            issue.fixes
        );
    }

    #[test]
    fn a_word_the_writer_knows_is_left_alone() {
        let known = HashSet::from(["mispelled".to_string()]);
        let mut checker = Checker::new(known);
        let text = "This sentence has a mispelled word in the middle of it.\n";
        assert!(
            checker
                .check(text, Source::Markdown)
                .iter()
                .all(|i| !i.spelling)
        );
    }

    #[test]
    fn names_and_keys_are_not_misspellings() {
        let mut checker = Checker::new(HashSet::new());
        let text =
            "We trained on ImageNet and CIFAR-10, drew it in TikZ, and pressed Ctrl to go.\n";
        let found = checker.check(text, Source::Markdown);
        assert!(found.iter().all(|i| !i.spelling), "{found:?}");
    }

    #[test]
    fn british_spelling_is_not_a_mistake() {
        let mut checker = Checker::new(HashSet::new());
        let text = "This minimises the loss, and the colour of the emphasised centre stays.\n";
        let found = checker.check(text, Source::Markdown);
        assert!(found.iter().all(|i| !i.spelling), "{found:?}");
    }

    #[test]
    fn what_is_not_prose_is_not_checked() {
        let mut checker = Checker::new(HashSet::new());
        let text = "A plain sentence with `qwrtyzz` in code and nothing wrong around it.\n";
        assert!(checker.check(text, Source::Markdown).is_empty());
    }
}
