//! The language registry and syntax highlighting. A language is one row in `SPECS`.

pub mod lsp;

use std::cell::OnceCell;
use std::path::Path;

use tree_sitter_highlight::{HighlightConfiguration, HighlightEvent, Highlighter};

/// What a span of source is, independent of any theme.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Token {
    Keyword,
    String,
    Comment,
    Function,
    Type,
    Number,
    Constant,
    Attribute,
    Heading,
}

/// tree-sitter capture names, matched by longest dotted prefix.
const CAPTURES: [(&str, Token); 13] = [
    ("keyword", Token::Keyword),
    ("string", Token::String),
    ("comment", Token::Comment),
    ("function", Token::Function),
    ("constructor", Token::Type),
    ("type", Token::Type),
    ("number", Token::Number),
    ("constant", Token::Constant),
    ("boolean", Token::Constant),
    ("attribute", Token::Attribute),
    ("text.title", Token::Heading),
    ("text.literal", Token::String),
    ("text.uri", Token::Attribute),
];

/// A highlighted byte range.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
    pub token: Token,
}

struct Spec {
    name: &'static str,
    extensions: &'static [&'static str],
    language: fn() -> tree_sitter::Language,
    queries: fn() -> (String, &'static str, &'static str),
}

fn rust() -> tree_sitter::Language {
    tree_sitter_rust::LANGUAGE.into()
}
fn python() -> tree_sitter::Language {
    tree_sitter_python::LANGUAGE.into()
}
fn javascript() -> tree_sitter::Language {
    tree_sitter_javascript::LANGUAGE.into()
}
fn typescript() -> tree_sitter::Language {
    tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()
}
fn tsx() -> tree_sitter::Language {
    tree_sitter_typescript::LANGUAGE_TSX.into()
}
fn json() -> tree_sitter::Language {
    tree_sitter_json::LANGUAGE.into()
}
fn go() -> tree_sitter::Language {
    tree_sitter_go::LANGUAGE.into()
}
fn bash() -> tree_sitter::Language {
    tree_sitter_bash::LANGUAGE.into()
}
fn markdown() -> tree_sitter::Language {
    tree_sitter_md::LANGUAGE.into()
}

fn typescript_queries() -> (String, &'static str, &'static str) {
    let highlights = format!(
        "{}\n{}",
        tree_sitter_typescript::HIGHLIGHTS_QUERY,
        tree_sitter_javascript::HIGHLIGHT_QUERY
    );
    (highlights, "", tree_sitter_typescript::LOCALS_QUERY)
}

const SPECS: [Spec; 9] = [
    Spec {
        name: "Rust",
        extensions: &["rs"],
        language: rust,
        queries: || (tree_sitter_rust::HIGHLIGHTS_QUERY.into(), "", ""),
    },
    Spec {
        name: "Python",
        extensions: &["py", "pyi"],
        language: python,
        queries: || (tree_sitter_python::HIGHLIGHTS_QUERY.into(), "", ""),
    },
    Spec {
        name: "JavaScript",
        extensions: &["js", "mjs", "cjs", "jsx"],
        language: javascript,
        queries: || {
            let highlights = format!(
                "{}\n{}",
                tree_sitter_javascript::JSX_HIGHLIGHT_QUERY,
                tree_sitter_javascript::HIGHLIGHT_QUERY
            );
            (highlights, "", tree_sitter_javascript::LOCALS_QUERY)
        },
    },
    Spec {
        name: "TypeScript",
        extensions: &["ts", "mts", "cts"],
        language: typescript,
        queries: typescript_queries,
    },
    Spec {
        name: "TSX",
        extensions: &["tsx"],
        language: tsx,
        queries: typescript_queries,
    },
    Spec {
        name: "JSON",
        extensions: &["json"],
        language: json,
        queries: || (tree_sitter_json::HIGHLIGHTS_QUERY.into(), "", ""),
    },
    Spec {
        name: "Go",
        extensions: &["go"],
        language: go,
        queries: || (tree_sitter_go::HIGHLIGHTS_QUERY.into(), "", ""),
    },
    Spec {
        name: "Shell",
        extensions: &["sh", "bash", "zsh"],
        language: bash,
        queries: || (tree_sitter_bash::HIGHLIGHT_QUERY.into(), "", ""),
    },
    Spec {
        name: "Markdown",
        extensions: &["md", "markdown"],
        language: markdown,
        queries: || (tree_sitter_md::HIGHLIGHT_QUERY_BLOCK.into(), "", ""),
    },
];

/// Highlighting skips files this large, where a full parse per edit would be felt.
const MAX_HIGHLIGHT_BYTES: usize = 2 * 1024 * 1024;

/// Compiles each language's queries the first time a file needs them.
#[derive(Default)]
pub struct Registry {
    configs: [OnceCell<Option<HighlightConfiguration>>; SPECS.len()],
}

fn build(spec: &Spec) -> Option<HighlightConfiguration> {
    let (highlights, injections, locals) = (spec.queries)();
    let mut config = HighlightConfiguration::new(
        (spec.language)(),
        spec.name,
        &highlights,
        injections,
        locals,
    )
    .ok()?;
    let names: Vec<&str> = CAPTURES.iter().map(|(name, _)| *name).collect();
    config.configure(&names);
    Some(config)
}

impl Registry {
    fn spec_for(path: &Path) -> Option<usize> {
        let extension = path.extension()?.to_str()?;
        SPECS.iter().position(|s| s.extensions.contains(&extension))
    }

    pub fn language_name(path: &Path) -> Option<&'static str> {
        Some(SPECS[Self::spec_for(path)?].name)
    }

    pub fn languages() -> impl Iterator<Item = &'static str> {
        SPECS.iter().map(|s| s.name)
    }

    /// Spans in byte order, or nothing for an unknown language or a very large file.
    pub fn highlight(&self, path: &Path, text: &str) -> Vec<Span> {
        if text.len() > MAX_HIGHLIGHT_BYTES {
            return Vec::new();
        }
        let Some(index) = Self::spec_for(path) else {
            return Vec::new();
        };
        let config = self.configs[index].get_or_init(|| build(&SPECS[index]));
        let Some(config) = config else {
            return Vec::new();
        };
        let mut highlighter = Highlighter::new();
        let Ok(events) = highlighter.highlight(config, text.as_bytes(), None, None, |_| None)
        else {
            return Vec::new();
        };
        let mut spans = Vec::new();
        let mut stack = Vec::new();
        for event in events.flatten() {
            match event {
                HighlightEvent::HighlightStart(h) => stack.push(CAPTURES[h.0].1),
                HighlightEvent::HighlightEnd => {
                    stack.pop();
                }
                HighlightEvent::Source { start, end } => {
                    if let Some(token) = stack.last() {
                        spans.push(Span {
                            start,
                            end,
                            token: *token,
                        });
                    }
                }
            }
        }
        spans
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn token_of(path: &str, text: &str, word: &str) -> Option<Token> {
        let at = text.find(word).unwrap();
        Registry::default()
            .highlight(Path::new(path), text)
            .iter()
            .find(|s| s.start <= at && at < s.end)
            .map(|s| s.token)
    }

    #[test]
    fn every_language_compiles_its_queries() {
        for spec in &SPECS {
            assert!(build(spec).is_some(), "{} failed to build", spec.name);
        }
    }

    #[test]
    fn each_language_highlights_a_sample() {
        let cases = [
            ("a.rs", "fn main() {}", "fn", Token::Keyword),
            ("a.py", "def f():\n    pass\n", "def", Token::Keyword),
            ("a.js", "const x = 1;\n", "const", Token::Keyword),
            ("a.ts", "const x: string = '';\n", "const", Token::Keyword),
            ("a.go", "package main\n", "package", Token::Keyword),
            ("a.sh", "if true; then echo; fi\n", "if", Token::Keyword),
            ("a.json", "{\"a\": 1}", "1", Token::Number),
        ];
        for (path, text, word, token) in cases {
            assert_eq!(token_of(path, text, word), Some(token), "{path}");
        }
    }

    #[test]
    fn unknown_files_have_no_spans() {
        assert!(
            Registry::default()
                .highlight(Path::new("a.xyz"), "fn")
                .is_empty()
        );
        assert_eq!(Registry::language_name(Path::new("a.rs")), Some("Rust"));
    }
}
