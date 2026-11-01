//! The language registry and syntax highlighting. A language is one row in `SPECS`.

pub mod complete;
pub mod lsp;

use std::cell::OnceCell;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread;

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
const CAPTURES: [(&str, Token); 18] = [
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
    ("property", Token::Attribute),
    ("label", Token::Attribute),
    ("tag", Token::Keyword),
    ("module", Token::Type),
    ("variable.builtin", Token::Constant),
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

fn hcl() -> tree_sitter::Language {
    tree_sitter_hcl::LANGUAGE.into()
}
fn yaml() -> tree_sitter::Language {
    tree_sitter_yaml::LANGUAGE.into()
}
fn toml() -> tree_sitter::Language {
    tree_sitter_toml_ng::LANGUAGE.into()
}
fn html() -> tree_sitter::Language {
    tree_sitter_html::LANGUAGE.into()
}
fn css() -> tree_sitter::Language {
    tree_sitter_css::LANGUAGE.into()
}
fn c() -> tree_sitter::Language {
    tree_sitter_c::LANGUAGE.into()
}
fn cpp() -> tree_sitter::Language {
    tree_sitter_cpp::LANGUAGE.into()
}
fn java() -> tree_sitter::Language {
    tree_sitter_java::LANGUAGE.into()
}
fn ruby() -> tree_sitter::Language {
    tree_sitter_ruby::LANGUAGE.into()
}
fn sql() -> tree_sitter::Language {
    tree_sitter_sequel::LANGUAGE.into()
}

/// The grammar ships no highlight query, so Terraform's is here. Earlier patterns win.
const HCL_HIGHLIGHTS: &str = r#"
(comment) @comment
(block (identifier) @keyword)
(block (string_lit (template_literal) @type))
(attribute (identifier) @property)
(function_call (identifier) @function)
(numeric_lit) @number
(bool_lit) @boolean
(null_lit) @constant
(string_lit) @string
(quoted_template) @string
(heredoc_template) @string
(variable_expr (identifier) @variable.builtin)
"#;

fn typescript_queries() -> (String, &'static str, &'static str) {
    let highlights = format!(
        "{}\n{}",
        tree_sitter_typescript::HIGHLIGHTS_QUERY,
        tree_sitter_javascript::HIGHLIGHT_QUERY
    );
    (highlights, "", tree_sitter_typescript::LOCALS_QUERY)
}

const SPECS: [Spec; 19] = [
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
    Spec {
        name: "Terraform",
        extensions: &["tf", "tfvars", "hcl", "tfbackend"],
        language: hcl,
        queries: || (HCL_HIGHLIGHTS.into(), "", ""),
    },
    Spec {
        name: "YAML",
        extensions: &["yml", "yaml"],
        language: yaml,
        queries: || (tree_sitter_yaml::HIGHLIGHTS_QUERY.into(), "", ""),
    },
    Spec {
        name: "TOML",
        extensions: &["toml"],
        language: toml,
        queries: || (tree_sitter_toml_ng::HIGHLIGHTS_QUERY.into(), "", ""),
    },
    Spec {
        name: "HTML",
        extensions: &["html", "htm"],
        language: html,
        queries: || (tree_sitter_html::HIGHLIGHTS_QUERY.into(), "", ""),
    },
    Spec {
        name: "CSS",
        extensions: &["css", "scss"],
        language: css,
        queries: || (tree_sitter_css::HIGHLIGHTS_QUERY.into(), "", ""),
    },
    Spec {
        name: "C",
        extensions: &["c", "h"],
        language: c,
        queries: || (tree_sitter_c::HIGHLIGHT_QUERY.into(), "", ""),
    },
    Spec {
        name: "C++",
        extensions: &["cc", "cpp", "cxx", "hpp", "hh"],
        language: cpp,
        queries: || {
            let highlights = format!(
                "{}\n{}",
                tree_sitter_cpp::HIGHLIGHT_QUERY,
                tree_sitter_c::HIGHLIGHT_QUERY
            );
            (highlights, "", "")
        },
    },
    Spec {
        name: "Java",
        extensions: &["java"],
        language: java,
        queries: || (tree_sitter_java::HIGHLIGHTS_QUERY.into(), "", ""),
    },
    Spec {
        name: "Ruby",
        extensions: &["rb"],
        language: ruby,
        queries: || {
            (
                tree_sitter_ruby::HIGHLIGHTS_QUERY.into(),
                "",
                tree_sitter_ruby::LOCALS_QUERY,
            )
        },
    },
    Spec {
        name: "SQL",
        extensions: &["sql"],
        language: sql,
        queries: || (tree_sitter_sequel::HIGHLIGHTS_QUERY.into(), "", ""),
    },
];

/// Files known by name rather than by extension.
const NAMES: [(&str, &str); 16] = [
    (".bashrc", "sh"),
    (".bash_profile", "sh"),
    (".zshrc", "sh"),
    (".zprofile", "sh"),
    (".profile", "sh"),
    ("Cargo.lock", "toml"),
    ("Pipfile", "toml"),
    ("Gemfile", "rb"),
    ("Rakefile", "rb"),
    ("Vagrantfile", "rb"),
    (".eslintrc", "json"),
    (".prettierrc", "json"),
    ("terraform.tfstate", "json"),
    ("flake.lock", "json"),
    ("uv.lock", "toml"),
    ("poetry.lock", "toml"),
];

/// Languages simple enough to read a line at a time, with no grammar to load.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Lines {
    Env,
    Docker,
    Ini,
    Ignore,
}

fn lines_for(path: &Path) -> Option<Lines> {
    let name = path.file_name()?.to_str()?;
    let extension = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    if name.starts_with(".env") || extension == "env" {
        Some(Lines::Env)
    } else if name == "Dockerfile" || name.starts_with("Dockerfile.") || extension == "dockerfile" {
        Some(Lines::Docker)
    } else if matches!(extension, "ini" | "cfg" | "conf" | "properties") || name == ".editorconfig"
    {
        Some(Lines::Ini)
    } else if matches!(
        name,
        ".gitignore" | ".dockerignore" | ".npmignore" | ".terraformignore"
    ) {
        Some(Lines::Ignore)
    } else {
        None
    }
}

/// How a value after `=` looks.
fn value_token(value: &str) -> Token {
    let bare = value.trim();
    if matches!(
        bare.to_lowercase().as_str(),
        "true" | "false" | "yes" | "no" | "on" | "off"
    ) {
        Token::Constant
    } else if !bare.is_empty() && bare.parse::<f64>().is_ok() {
        Token::Number
    } else {
        Token::String
    }
}

fn highlight_lines(kind: Lines, text: &str) -> Vec<Span> {
    let mut spans = Vec::new();
    let mut byte = 0;
    for line in text.split_inclusive('\n') {
        let body = line.trim_end();
        let indent = body.len() - body.trim_start().len();
        let content = &body[indent..];
        let start = byte + indent;
        let end = byte + body.len();
        let mut push = |from: usize, to: usize, token: Token| {
            if from < to {
                spans.push(Span {
                    start: from,
                    end: to,
                    token,
                });
            }
        };
        if content.starts_with('#') || (kind == Lines::Ini && content.starts_with(';')) {
            push(start, end, Token::Comment);
        } else if kind == Lines::Ignore {
            if content.starts_with('!') {
                push(start, start + 1, Token::Keyword);
            }
        } else if kind == Lines::Docker {
            let word = content.split_whitespace().next().unwrap_or("");
            if !word.is_empty() && word.chars().all(|c| c.is_ascii_uppercase()) {
                push(start, start + word.len(), Token::Keyword);
            }
        } else if kind == Lines::Ini && content.starts_with('[') {
            push(start, end, Token::Heading);
        } else {
            let prefix = if content.starts_with("export ") {
                "export ".len()
            } else {
                0
            };
            push(start, start + prefix.saturating_sub(1), Token::Keyword);
            if let Some(equals) = content.find('=') {
                let key_end = start + content[..equals].trim_end().len();
                push(start + prefix, key_end, Token::Attribute);
                push(start + equals + 1, end, value_token(&content[equals + 1..]));
            }
        }
        byte += line.len();
    }
    spans
}

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
        let name = path.file_name()?.to_str()?;
        let extension = NAMES
            .iter()
            .find(|(known, _)| *known == name)
            .map(|(_, extension)| *extension)
            .or_else(|| path.extension()?.to_str())?;
        SPECS.iter().position(|s| s.extensions.contains(&extension))
    }

    pub fn language_name(path: &Path) -> Option<&'static str> {
        match lines_for(path) {
            Some(Lines::Env) => Some("Env"),
            Some(Lines::Docker) => Some("Dockerfile"),
            Some(Lines::Ini) => Some("INI"),
            Some(Lines::Ignore) => Some("Ignore"),
            None => Some(SPECS[Self::spec_for(path)?].name),
        }
    }

    pub fn languages() -> impl Iterator<Item = &'static str> {
        SPECS.iter().map(|s| s.name)
    }

    /// What starts a comment that runs to the end of the line, if the language has one.
    pub fn line_comment(path: &Path) -> Option<&'static str> {
        match Self::language_name(path)? {
            "Rust" | "JavaScript" | "TypeScript" | "TSX" | "Go" | "C" | "C++" | "Java" => {
                Some("//")
            }
            "Python" | "Shell" | "Terraform" | "YAML" | "TOML" | "Ruby" | "Env" | "Dockerfile"
            | "Ignore" => Some("#"),
            "SQL" => Some("--"),
            "INI" => Some(";"),
            _ => None,
        }
    }

    /// Spans in byte order, or nothing for an unknown language or a very large file.
    pub fn highlight(&self, path: &Path, text: &str) -> Vec<Span> {
        if text.len() > MAX_HIGHLIGHT_BYTES {
            return Vec::new();
        }
        if let Some(kind) = lines_for(path) {
            return highlight_lines(kind, text);
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

/// One file to highlight, tagged with the revision it was read at.
type Job = (PathBuf, String, u64);

/// Highlights away from the drawing thread. tree-sitter-highlight reparses the whole file,
/// which is felt on every keystroke in a large one. The thread keeps only the newest job,
/// and ends when this is dropped.
pub struct Worker {
    jobs: Sender<Job>,
    done: Receiver<(u64, Vec<Span>)>,
}

impl Default for Worker {
    fn default() -> Self {
        let (jobs, inbox) = channel::<Job>();
        let (outbox, done) = channel();
        thread::spawn(move || {
            let registry = Registry::default();
            while let Ok(mut job) = inbox.recv() {
                while let Ok(newer) = inbox.try_recv() {
                    job = newer;
                }
                let spans = registry.highlight(&job.0, &job.1);
                if outbox.send((job.2, spans)).is_err() {
                    return;
                }
            }
        });
        Self { jobs, done }
    }
}

impl Worker {
    pub fn request(&self, path: &Path, text: String, revision: u64) {
        let _ = self.jobs.send((path.to_path_buf(), text, revision));
    }

    /// The newest finished result, if there's one.
    pub fn finished(&self) -> Option<(u64, Vec<Span>)> {
        self.done.try_iter().last()
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
            (
                "main.tf",
                "resource \"aws_s3_bucket\" \"b\" {\n  acl = \"private\"\n}\n",
                "resource",
                Token::Keyword,
            ),
            (
                "main.tf",
                "resource \"aws_s3_bucket\" \"b\" {\n}\n",
                "aws_s3_bucket",
                Token::Type,
            ),
            (
                "main.tf",
                "bucket = \"logs\" # keep\n",
                "keep",
                Token::Comment,
            ),
            (
                "terraform.tfvars",
                "region = \"eu\"\ncount = 3\n",
                "region",
                Token::Attribute,
            ),
            ("terraform.tfvars", "count = 3\n", "3", Token::Number),
            (
                "a.yml",
                "name: cellix\nport: 80\n",
                "name",
                Token::Attribute,
            ),
            ("Cargo.toml", "[package]\nversion = 1\n", "1", Token::Number),
            ("a.html", "<div>x</div>", "div", Token::Keyword),
            (
                "a.c",
                "int main(void) { return 0; }\n",
                "return",
                Token::Keyword,
            ),
            ("a.java", "class A {}\n", "class", Token::Keyword),
            ("a.rb", "def f\nend\n", "def", Token::Keyword),
            ("a.sql", "SELECT 1;\n", "SELECT", Token::Keyword),
            (".zshrc", "if true; then echo; fi\n", "if", Token::Keyword),
        ];
        for (path, text, word, token) in cases {
            assert_eq!(token_of(path, text, word), Some(token), "{path}");
        }
    }

    #[test]
    fn env_files_highlight_each_part() {
        let text = "# Secrets\nexport API_KEY=abc\nPORT=8080\nDEBUG=true\n";
        for path in [".env", ".env.local", "prod.env"] {
            assert_eq!(
                token_of(path, text, "Secrets"),
                Some(Token::Comment),
                "{path}"
            );
            assert_eq!(token_of(path, text, "export"), Some(Token::Keyword));
            assert_eq!(token_of(path, text, "API_KEY"), Some(Token::Attribute));
            assert_eq!(token_of(path, text, "abc"), Some(Token::String));
            assert_eq!(token_of(path, text, "8080"), Some(Token::Number));
            assert_eq!(token_of(path, text, "true"), Some(Token::Constant));
        }
    }

    #[test]
    fn simple_formats_highlight_by_line() {
        assert_eq!(
            token_of("Dockerfile", "FROM rust\n", "FROM"),
            Some(Token::Keyword)
        );
        assert_eq!(token_of("a.ini", "[core]\n", "core"), Some(Token::Heading));
        assert_eq!(token_of(".gitignore", "# x\n", "x"), Some(Token::Comment));
    }

    #[test]
    fn files_are_known_by_name() {
        let name = |path| Registry::language_name(Path::new(path));
        assert_eq!(name(".env.production"), Some("Env"));
        assert_eq!(name("terraform.tfvars"), Some("Terraform"));
        assert_eq!(name("Cargo.lock"), Some("TOML"));
        assert_eq!(name("Dockerfile.prod"), Some("Dockerfile"));
    }

    #[test]
    fn languages_know_their_comments() {
        assert_eq!(Registry::line_comment(Path::new("a.rs")), Some("//"));
        assert_eq!(Registry::line_comment(Path::new("main.tf")), Some("#"));
        assert_eq!(Registry::line_comment(Path::new("a.md")), None);
    }

    #[test]
    fn the_worker_highlights_away_from_the_caller() {
        let worker = Worker::default();
        worker.request(Path::new("a.rs"), "fn main() {}".into(), 7);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let result = loop {
            if let Some(result) = worker.finished() {
                break result;
            }
            assert!(std::time::Instant::now() < deadline);
            thread::sleep(std::time::Duration::from_millis(5));
        };
        assert_eq!(result.0, 7);
        assert_eq!(result.1[0].token, Token::Keyword);
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
