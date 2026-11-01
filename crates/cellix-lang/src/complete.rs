//! Completion that needs no language server, so every file type gets some. Candidates come
//! from the language, from words in open files, and for `.env` files from their siblings.
//! A language server's answers join these when one is running.

use std::collections::HashSet;
use std::fs;
use std::path::Path;

/// Files larger than this aren't scanned for words on every keystroke.
const MAX_SCAN_BYTES: usize = 1024 * 1024;
const MIN_WORD: usize = 3;
pub const MAX_SHOWN: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Server,
    Keyword,
    Key,
    Word,
}

impl Source {
    pub fn label(self) -> &'static str {
        match self {
            Self::Server => "symbol",
            Self::Keyword => "keyword",
            Self::Key => "key",
            Self::Word => "word",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub label: String,
    pub source: Source,
}

const TERRAFORM: &[&str] = &[
    "resource",
    "data",
    "variable",
    "output",
    "locals",
    "module",
    "provider",
    "terraform",
    "required_providers",
    "required_version",
    "backend",
    "source",
    "version",
    "count",
    "for_each",
    "depends_on",
    "lifecycle",
    "provisioner",
    "connection",
    "moved",
    "import",
    "dynamic",
    "description",
    "type",
    "default",
    "sensitive",
    "nullable",
    "validation",
    "condition",
    "error_message",
    "value",
    "string",
    "number",
    "bool",
    "list",
    "map",
    "set",
    "object",
    "tuple",
    "any",
    "null",
    "true",
    "false",
    "var",
    "local",
    "each",
    "self",
    "path",
    "create_before_destroy",
    "prevent_destroy",
    "ignore_changes",
    "length",
    "lookup",
    "merge",
    "concat",
    "format",
    "join",
    "split",
    "jsonencode",
    "jsondecode",
    "yamlencode",
    "tolist",
    "toset",
    "tomap",
    "tostring",
    "tonumber",
    "try",
    "can",
    "coalesce",
    "contains",
    "element",
    "keys",
    "values",
    "flatten",
    "distinct",
    "replace",
    "lower",
    "upper",
    "trimspace",
    "file",
    "templatefile",
];
const RUST: &[&str] = &[
    "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum", "extern",
    "false", "impl", "let", "loop", "match", "mod", "move", "mut", "pub", "ref", "return", "self",
    "Self", "static", "struct", "super", "trait", "true", "type", "unsafe", "use", "where",
    "while", "String", "Option", "Result", "Vec", "Some", "None", "Ok", "Err",
];
const PYTHON: &[&str] = &[
    "and", "assert", "async", "await", "break", "class", "continue", "def", "del", "elif", "else",
    "except", "False", "finally", "for", "from", "global", "if", "import", "lambda", "None",
    "nonlocal", "not", "pass", "raise", "return", "True", "try", "while", "with", "yield",
];
const SCRIPT: &[&str] = &[
    "async",
    "await",
    "break",
    "case",
    "catch",
    "class",
    "const",
    "continue",
    "default",
    "delete",
    "else",
    "export",
    "extends",
    "false",
    "finally",
    "for",
    "from",
    "function",
    "import",
    "instanceof",
    "interface",
    "let",
    "new",
    "null",
    "return",
    "switch",
    "this",
    "throw",
    "true",
    "try",
    "type",
    "typeof",
    "undefined",
    "var",
    "void",
    "while",
    "yield",
];
const GO: &[&str] = &[
    "break",
    "case",
    "chan",
    "const",
    "continue",
    "default",
    "defer",
    "else",
    "fallthrough",
    "false",
    "for",
    "func",
    "go",
    "goto",
    "import",
    "interface",
    "map",
    "nil",
    "package",
    "range",
    "return",
    "select",
    "struct",
    "switch",
    "true",
    "type",
    "var",
];
const SHELL: &[&str] = &[
    "case", "done", "echo", "elif", "else", "esac", "export", "exit", "for", "function", "local",
    "printf", "read", "return", "then", "until", "while",
];
const DOCKER: &[&str] = &[
    "FROM",
    "RUN",
    "CMD",
    "ENTRYPOINT",
    "COPY",
    "ADD",
    "ENV",
    "ARG",
    "EXPOSE",
    "WORKDIR",
    "USER",
    "VOLUME",
    "LABEL",
    "HEALTHCHECK",
    "SHELL",
    "STOPSIGNAL",
    "ONBUILD",
];
const SQL: &[&str] = &[
    "SELECT",
    "FROM",
    "WHERE",
    "GROUP",
    "ORDER",
    "HAVING",
    "LIMIT",
    "OFFSET",
    "INSERT",
    "INTO",
    "VALUES",
    "UPDATE",
    "DELETE",
    "CREATE",
    "TABLE",
    "INDEX",
    "ALTER",
    "DROP",
    "JOIN",
    "LEFT",
    "RIGHT",
    "INNER",
    "OUTER",
    "NULL",
    "PRIMARY",
    "FOREIGN",
    "REFERENCES",
    "DISTINCT",
];

pub fn keywords(language: &str) -> &'static [&'static str] {
    match language {
        "Terraform" => TERRAFORM,
        "Rust" => RUST,
        "Python" => PYTHON,
        "JavaScript" | "TypeScript" | "TSX" => SCRIPT,
        "Go" => GO,
        "Shell" => SHELL,
        "Dockerfile" => DOCKER,
        "SQL" => SQL,
        _ => &[],
    }
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// The words in some text, at their first appearance.
pub fn words(text: &str) -> Vec<&str> {
    if text.len() > MAX_SCAN_BYTES {
        return Vec::new();
    }
    let mut seen = HashSet::new();
    text.split(|c| !is_word(c))
        .filter(|word| word.chars().count() >= MIN_WORD)
        .filter(|word| !word.starts_with(|c: char| c.is_ascii_digit()))
        .filter(|word| seen.insert(*word))
        .collect()
}

/// The keys set in the `.env` files beside this one, such as `.env.example`, which is what
/// a new `.env` usually needs filling in from.
pub fn env_keys(path: &Path) -> Vec<String> {
    let Some(entries) = path.parent().and_then(|dir| fs::read_dir(dir).ok()) else {
        return Vec::new();
    };
    let mut keys = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let is_env = name.to_string_lossy().starts_with(".env");
        if !is_env || entry.path() == path {
            continue;
        }
        let Ok(text) = fs::read_to_string(entry.path()) else {
            continue;
        };
        for line in text.lines() {
            let line = line.trim_start().trim_start_matches("export ");
            if let Some((key, _)) = line.split_once('=')
                && !key.starts_with('#')
                && !key.is_empty()
                && !keys.iter().any(|known| known == key)
            {
                keys.push(key.to_string());
            }
        }
    }
    keys
}

/// How well a candidate fits what's typed, lower being better: an exact-case start, then
/// any-case start, then letters in order. None if it doesn't fit, or adds nothing.
fn fit(label: &str, typed: &str) -> Option<u8> {
    if label == typed {
        return None;
    }
    if label.starts_with(typed) {
        return Some(0);
    }
    let (label, typed) = (label.to_lowercase(), typed.to_lowercase());
    if label.starts_with(&typed) {
        return Some(1);
    }
    let mut letters = label.chars();
    typed.chars().all(|c| letters.any(|l| l == c)).then_some(2)
}

/// The best candidates for what's typed, keeping their order within each level of fit, so
/// sources are tried in the order given. A label appears once, under its first source.
pub fn rank(typed: &str, candidates: Vec<Candidate>) -> Vec<Candidate> {
    let mut seen = HashSet::new();
    let mut fitting: Vec<(u8, Candidate)> = candidates
        .into_iter()
        .filter(|c| seen.insert(c.label.clone()))
        .filter_map(|c| fit(&c.label, typed).map(|level| (level, c)))
        .collect();
    fitting.sort_by_key(|(level, _)| *level);
    fitting.truncate(MAX_SHOWN);
    fitting.into_iter().map(|(_, c)| c).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidates(labels: &[&str]) -> Vec<Candidate> {
        labels
            .iter()
            .map(|label| Candidate {
                label: label.to_string(),
                source: Source::Word,
            })
            .collect()
    }

    fn labels(ranked: Vec<Candidate>) -> Vec<String> {
        ranked.into_iter().map(|c| c.label).collect()
    }

    #[test]
    fn starts_come_before_scattered_letters() {
        let ranked = rank("res", candidates(&["my_result", "Resource", "resource"]));
        assert_eq!(labels(ranked), ["resource", "Resource", "my_result"]);
    }

    #[test]
    fn what_is_already_typed_is_not_offered() {
        assert!(rank("done", candidates(&["done"])).is_empty());
    }

    #[test]
    fn the_first_source_of_a_label_wins() {
        let mut all = vec![Candidate {
            label: "resource".into(),
            source: Source::Keyword,
        }];
        all.extend(candidates(&["resource"]));
        assert_eq!(rank("re", all)[0].source, Source::Keyword);
    }

    #[test]
    fn words_skip_numbers_and_short_ones() {
        assert_eq!(words("an alpha 123 beta_2 alpha"), ["alpha", "beta_2"]);
    }

    #[test]
    fn env_files_offer_their_siblings_keys() {
        let dir = tempfile::tempdir().unwrap();
        let example = "# Keys\nAPI_KEY=\nexport DB_URL=postgres://x\nAPI_KEY=again\n";
        fs::write(dir.path().join(".env.example"), example).unwrap();
        fs::write(dir.path().join("other.txt"), "NOT_THIS=1\n").unwrap();
        let keys = env_keys(&dir.path().join(".env"));
        assert_eq!(keys, ["API_KEY", "DB_URL"]);
    }

    #[test]
    fn terraform_has_its_keywords() {
        assert!(keywords("Terraform").contains(&"resource"));
        assert!(keywords("Env").is_empty());
    }
}
