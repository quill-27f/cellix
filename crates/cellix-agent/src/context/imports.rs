//! `@path` imports in instruction files, as Claude Code and Gemini CLI read them: an `@` at
//! the start of a line or after a space, then a path, outside code spans and fenced blocks.

use std::path::{Path, PathBuf};

/// Imports are followed this many files deep and no further.
pub const IMPORT_DEPTH: usize = 5;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Import {
    /// 1-based.
    pub line: usize,
    pub target: String,
}

/// The imports in `text`, in order. Ones that can't be files, like `@types/node` in prose,
/// are left out: a path counts only with an extension or a leading `./`, `../`, `~/` or `/`.
pub fn imports(text: &str) -> Vec<Import> {
    let mut out = Vec::new();
    let mut fence: Option<&str> = None;
    for (i, line) in text.lines().enumerate() {
        let trimmed = line.trim_start();
        for mark in ["```", "~~~"] {
            if trimmed.starts_with(mark) {
                fence = match fence {
                    Some(open) if open == mark => None,
                    None => Some(mark),
                    other => other,
                };
            }
        }
        if fence.is_some() || trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            continue;
        }
        let mut in_code = false;
        let mut prev = ' ';
        let chars: Vec<char> = line.chars().collect();
        let mut j = 0;
        while j < chars.len() {
            let c = chars[j];
            if c == '`' {
                in_code = !in_code;
            } else if c == '@' && !in_code && prev.is_whitespace() {
                let rest: String = chars[j + 1..]
                    .iter()
                    .take_while(|c| !c.is_whitespace() && **c != '`')
                    .collect();
                let target = rest
                    .trim_end_matches([',', ';', ':', ')', '.', '!', '?'])
                    .to_string();
                if looks_like_file(&target) {
                    out.push(Import {
                        line: i + 1,
                        target,
                    });
                }
                j += rest.chars().count();
            }
            prev = c;
            j += 1;
        }
    }
    out
}

fn looks_like_file(target: &str) -> bool {
    if target.is_empty() {
        return false;
    }
    let explicit = ["./", "../", "~/", "/"]
        .iter()
        .any(|p| target.starts_with(p));
    let name = target.rsplit('/').next().unwrap_or(target);
    let extension = name.rsplit_once('.').is_some_and(|(stem, ext)| {
        !stem.is_empty() && !ext.is_empty() && ext.chars().all(|c| c.is_ascii_alphanumeric())
    });
    explicit || extension
}

/// Where an import points, from the folder of the file it's in. `~` is the home folder.
pub fn resolve(target: &str, dir: &Path) -> PathBuf {
    if let Some(rest) = target.strip_prefix("~/")
        && let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))
    {
        return PathBuf::from(home).join(rest);
    }
    let path = Path::new(target);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        normalize(&dir.join(path))
    }
}

/// `a/./b/../c` as `a/c`, without touching the disk, so a missing file still has a fair name.
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn targets(text: &str) -> Vec<(usize, String)> {
        imports(text)
            .into_iter()
            .map(|i| (i.line, i.target))
            .collect()
    }

    #[test]
    fn imports_are_found_outside_code_only() {
        let text = "@AGENTS.md\nSee @docs/STYLE.md, and mail me@host.com.\n`@not/this.md`\n```\n@nor/this.md\n```\nInstall @types/node\n@~/notes.md";
        let want = [(1, "AGENTS.md"), (2, "docs/STYLE.md"), (8, "~/notes.md")];
        assert_eq!(targets(text), want.map(|(l, t)| (l, t.to_string())));
    }

    #[test]
    fn a_relative_import_resolves_from_its_file() {
        assert_eq!(
            resolve("../b.md", Path::new("/p/docs")),
            PathBuf::from("/p/b.md")
        );
        assert_eq!(
            resolve("./x/y.md", Path::new("/p")),
            PathBuf::from("/p/x/y.md")
        );
    }
}
