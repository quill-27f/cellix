//! Gentle checks on what an agent works from. They say what Cellix can tell from the files
//! alone and never stop the agent. Whether two instructions really disagree in meaning is for
//! the agent to judge; these only notice the same command told to run and never to run.

use std::path::{Path, PathBuf};

use super::{Agent, Context, Instruction, ItemKind, LARGE_CHARS, estimate_tokens};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CheckKind {
    /// Two files say opposite things about the same command.
    Opposite,
    /// An `@import` names a file that isn't there.
    MissingImport,
    /// The agent reads only part of the file.
    CutOff,
    /// A large file loaded into every session.
    Large,
    /// A skill or subagent the agent can't tell when to use.
    NoDescription,
    /// A hook runs a file that isn't there.
    MissingFile,
    /// The project's instructions are in a file this agent doesn't read.
    NotRead,
}

impl CheckKind {
    pub fn label(self) -> &'static str {
        match self {
            CheckKind::Opposite => "Say opposite things",
            CheckKind::MissingImport => "Import not found",
            CheckKind::CutOff => "Cut off",
            CheckKind::Large => "Large",
            CheckKind::NoDescription => "No description",
            CheckKind::MissingFile => "Missing file",
            CheckKind::NotRead => "Not read by this agent",
        }
    }

    /// Whether it changes what the agent does, rather than how well.
    pub fn warns(self) -> bool {
        matches!(
            self,
            CheckKind::Opposite | CheckKind::MissingImport | CheckKind::CutOff | CheckKind::NotRead
        )
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Check {
    pub kind: CheckKind,
    pub message: String,
    pub path: PathBuf,
    /// 1-based.
    pub line: Option<usize>,
    /// The other side, for two files that disagree.
    pub other: Option<(PathBuf, usize)>,
}

const NEGATIVE: [&str; 8] = [
    "never ",
    "don't ",
    "do not ",
    "avoid ",
    "must not ",
    "mustn't ",
    "should not ",
    "shouldn't ",
];

pub fn checks(agent: Agent, project: &Path, context: &Context) -> Vec<Check> {
    let mut out = Vec::new();
    let present: Vec<&Instruction> = context
        .instructions
        .iter()
        .filter(|i| i.exists() && i.later.is_none())
        .collect();
    out.extend(opposites(&present));
    for file in &context.instructions {
        if let (Some((by, line)), false) = (&file.imported_by, file.exists()) {
            out.push(Check {
                kind: CheckKind::MissingImport,
                message: format!(
                    "{} doesn't exist, so nothing is loaded there",
                    short(&file.path, project)
                ),
                path: by.clone(),
                line: Some(*line),
                other: None,
            });
        }
        if let Some(limit) = file.limit
            && file.lines() > limit
        {
            out.push(Check {
                kind: CheckKind::CutOff,
                message: format!(
                    "{} has {} lines; {} reads the first {limit}",
                    name(&file.path),
                    file.lines(),
                    agent.name()
                ),
                path: file.path.clone(),
                line: Some(limit + 1),
                other: None,
            });
        }
        if file
            .text
            .as_ref()
            .is_some_and(|t| t.chars().count() > LARGE_CHARS)
            && file.later.is_none()
        {
            out.push(Check {
                kind: CheckKind::Large,
                message: format!(
                    "{} is {} lines (≈{}k tokens), loaded into every session",
                    short(&file.path, project),
                    file.lines(),
                    estimate_tokens(file.loaded()) / 1000
                ),
                path: file.path.clone(),
                line: None,
                other: None,
            });
        }
    }
    for item in &context.items {
        if matches!(item.kind, ItemKind::Skill | ItemKind::Subagent) && item.about.trim().is_empty()
        {
            out.push(Check {
                kind: CheckKind::NoDescription,
                message: format!(
                    "{} {}: the agent sees only its name, so can't tell when to use it",
                    capital(item.kind.label()),
                    item.name
                ),
                path: item.path.clone(),
                line: None,
                other: None,
            });
        }
        if item.kind == ItemKind::Hook {
            for script in scripts(&item.about, project) {
                if !script.exists() {
                    out.push(Check {
                        kind: CheckKind::MissingFile,
                        message: format!(
                            "Hook {} runs {}, which isn't there",
                            item.name,
                            short(&script, project)
                        ),
                        path: item.path.clone(),
                        line: item.line,
                        other: None,
                    });
                }
            }
        }
    }
    out.extend(not_read(agent, project, context));
    out
}

fn capital(s: &str) -> String {
    let mut c = s.chars();
    c.next()
        .map(|f| f.to_uppercase().chain(c).collect())
        .unwrap_or_default()
}

fn name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// A path as the user would say it: inside the project, from its top; at home, with `~`.
pub(crate) fn short(path: &Path, project: &Path) -> String {
    if let Ok(rel) = path.strip_prefix(project) {
        return rel.display().to_string();
    }
    if let Some(home) = std::env::var_os("HOME")
        && let Ok(rel) = path.strip_prefix(&home)
    {
        return format!("~/{}", rel.display());
    }
    path.display().to_string()
}

/// The commands a line names in backticks, lower case. A one-word span after a longer one
/// borrows its first word, so "Never run `cargo build`, `test`" names `cargo test` too.
fn commands(line: &str) -> Vec<String> {
    let spans: Vec<&str> = line.split('`').skip(1).step_by(2).collect();
    let mut out = Vec::new();
    let mut head: Option<String> = None;
    for span in spans {
        let words: Vec<String> = span.split_whitespace().map(str::to_lowercase).collect();
        match words.len() {
            0 => {}
            1 => {
                if let Some(h) = &head {
                    out.push(format!("{h} {}", words[0]));
                }
                out.push(words[0].clone());
            }
            _ => {
                head = Some(words[0].clone());
                out.push(words.join(" "));
            }
        }
    }
    out.retain(|c| c.len() > 2);
    out
}

fn negative(line: &str) -> bool {
    let lower = format!(" {}", line.to_lowercase().replace(['*', '_'], ""));
    NEGATIVE.iter().any(|n| lower.contains(&format!(" {n}")))
}

/// Lines outside fenced code, numbered from 1.
fn prose_lines(text: &str) -> Vec<(usize, &str)> {
    let mut fenced = false;
    let mut out = Vec::new();
    for (i, line) in text.lines().enumerate() {
        if line.trim_start().starts_with("```") || line.trim_start().starts_with("~~~") {
            fenced = !fenced;
            continue;
        }
        if !fenced {
            out.push((i + 1, line));
        }
    }
    out
}

fn opposites(files: &[&Instruction]) -> Vec<Check> {
    struct Said<'a> {
        file: usize,
        line: usize,
        negative: bool,
        command: String,
        path: &'a Path,
    }
    let mut said = Vec::new();
    for (f, file) in files.iter().enumerate() {
        for (n, line) in prose_lines(file.loaded()) {
            let neg = negative(line);
            for command in commands(line) {
                said.push(Said {
                    file: f,
                    line: n,
                    negative: neg,
                    command,
                    path: &file.path,
                });
            }
        }
    }
    let mut out: Vec<Check> = Vec::new();
    for no in said.iter().filter(|s| s.negative) {
        for yes in said
            .iter()
            .filter(|s| !s.negative && s.file != no.file && s.command == no.command)
        {
            // Only commands, not single words like `test` that turn up everywhere.
            if !yes.command.contains(' ') {
                continue;
            }
            let pair = (yes.path.to_path_buf(), no.path.to_path_buf());
            if out.iter().any(|c| {
                (
                    c.path.clone(),
                    c.other.as_ref().map(|o| o.0.clone()).unwrap_or_default(),
                ) == pair
                    && c.message.contains(&yes.command)
            }) {
                continue;
            }
            out.push(Check {
                kind: CheckKind::Opposite,
                message: format!(
                    "“{}”: run in {}, never in {}",
                    yes.command,
                    name(yes.path),
                    name(no.path)
                ),
                path: yes.path.to_path_buf(),
                line: Some(yes.line),
                other: Some((no.path.to_path_buf(), no.line)),
            });
        }
    }
    out
}

/// Script files a hook's command runs: words that look like a path to a script, with the
/// project's folder put in for `$CLAUDE_PROJECT_DIR`.
fn scripts(command: &str, project: &Path) -> Vec<PathBuf> {
    let project_text = project.to_string_lossy();
    let expanded = command
        .replace("\"$CLAUDE_PROJECT_DIR\"", &project_text)
        .replace("${CLAUDE_PROJECT_DIR}", &project_text)
        .replace("$CLAUDE_PROJECT_DIR", &project_text);
    expanded
        .split(|c: char| c.is_whitespace() || c == ';' || c == '&' || c == '|')
        .map(|w| w.trim_matches(|c| c == '"' || c == '\''))
        .filter(|w| {
            let script = [".sh", ".py", ".js", ".mjs", ".ts", ".rb", ".pl"]
                .iter()
                .any(|e| w.ends_with(e));
            script && (w.contains('/') || w.starts_with('.'))
        })
        .filter(|w| !w.contains('$') && !w.contains('*'))
        .map(|w| {
            let path = Path::new(w);
            if path.is_absolute() {
                path.to_path_buf()
            } else {
                project.join(path)
            }
        })
        .collect()
}

/// The project has instructions, but in a file this agent doesn't read.
fn not_read(agent: Agent, project: &Path, context: &Context) -> Vec<Check> {
    let has = |n: &str| project.join(n).is_file();
    let project_files = context
        .instructions
        .iter()
        .any(|i| i.exists() && i.path.starts_with(project) && i.imported_by.is_none());
    let note = |message: &str, path: &str| Check {
        kind: CheckKind::NotRead,
        message: message.to_string(),
        path: project.join(path),
        line: None,
        other: None,
    };
    match agent {
        Agent::Claude if has("AGENTS.md") && !project_files => {
            vec![note(
                "Claude Code reads CLAUDE.md; this project's instructions are in AGENTS.md",
                "AGENTS.md",
            )]
        }
        Agent::Codex if has("CLAUDE.md") && !has("AGENTS.md") => {
            vec![note(
                "Codex reads AGENTS.md; this project's instructions are in CLAUDE.md",
                "CLAUDE.md",
            )]
        }
        Agent::Gemini if has("AGENTS.md") && !project_files => {
            vec![note(
                "Gemini CLI reads GEMINI.md; this project's instructions are in AGENTS.md",
                "AGENTS.md",
            )]
        }
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_short_command_borrows_the_one_before() {
        assert_eq!(
            commands("**Never run `cargo build`, `test`, `check` directly.**"),
            ["cargo build", "cargo test", "test", "cargo check", "check"]
        );
        assert!(negative("**Never run `cargo build` directly.**"));
        assert!(!negative("Run tests with `cargo test` first."));
    }

    #[test]
    fn hook_scripts_are_found_in_the_project() {
        let found = scripts(
            "bash \"$CLAUDE_PROJECT_DIR\"/harness/hooks/guard.sh && echo ok",
            Path::new("/p"),
        );
        assert_eq!(found, [PathBuf::from("/p/harness/hooks/guard.sh")]);
        assert!(scripts("harness/x status", Path::new("/p")).is_empty());
    }
}
