use std::path::{Path, PathBuf};
use std::process::Command;

/// A file that differs from HEAD. `status` is `M`, `A`, `D` or `?`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Change {
    pub path: PathBuf,
    pub status: char,
}

fn git(root: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Paths relative to `root`. Empty outside a repository.
pub fn changed_files(root: &Path) -> Vec<Change> {
    let Some(output) = git(
        root,
        &[
            "status",
            "--porcelain=v1",
            "-z",
            "--untracked-files=all",
            ".",
        ],
    ) else {
        return Vec::new();
    };
    let mut changes = Vec::new();
    let mut fields = output.split('\0');
    while let Some(entry) = fields.next() {
        if entry.len() < 4 {
            continue;
        }
        let code = &entry[..2];
        // A rename's second field is the old path, which isn't needed here.
        if code.starts_with('R') || code.starts_with('C') {
            fields.next();
        }
        let status = match code {
            "??" => '?',
            c if c.contains('D') => 'D',
            c if c.starts_with('A') => 'A',
            _ => 'M',
        };
        changes.push(Change {
            path: PathBuf::from(&entry[3..]),
            status,
        });
    }
    changes
}

/// The checked-out branch, or None outside a repository or on a detached HEAD.
pub fn current_branch(root: &Path) -> Option<String> {
    let name = git(root, &["symbolic-ref", "--short", "-q", "HEAD"])?;
    Some(name.trim().to_string())
}

/// The file as committed, or empty if it's new.
pub fn head_text(root: &Path, path: &Path) -> String {
    let spec = format!("HEAD:./{}", path.display());
    git(root, &["show", &spec]).unwrap_or_default()
}

/// "abc1234 Author: summary" for a 1-based line.
pub fn blame_line(root: &Path, path: &Path, line: usize) -> Option<String> {
    let range = format!("{line},{line}");
    let file = path.to_string_lossy();
    let output = git(root, &["blame", "--porcelain", "-L", &range, "--", &file])?;
    let hash = output.split_whitespace().next()?.get(..7)?.to_string();
    let field = |key: &str| {
        output
            .lines()
            .find_map(|l| l.strip_prefix(key))
            .map(str::to_string)
    };
    if hash == "0000000" {
        return Some("Not committed yet".into());
    }
    Some(format!(
        "{hash} {}: {}",
        field("author ")?,
        field("summary ")?
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let run = |args: &[&str]| {
            assert!(
                Command::new("git")
                    .arg("-C")
                    .arg(dir.path())
                    .args(args)
                    .output()
                    .unwrap()
                    .status
                    .success()
            );
        };
        run(&["init", "-q"]);
        run(&["config", "user.email", "test@example.com"]);
        run(&["config", "user.name", "Tester"]);
        fs::write(dir.path().join("kept.txt"), "one\n").unwrap();
        fs::write(dir.path().join("gone.txt"), "bye\n").unwrap();
        run(&["add", "."]);
        run(&["commit", "-q", "-m", "first"]);
        dir
    }

    #[test]
    fn status_lists_every_kind_of_change() {
        let dir = repo();
        fs::write(dir.path().join("kept.txt"), "two\n").unwrap();
        fs::remove_file(dir.path().join("gone.txt")).unwrap();
        fs::create_dir(dir.path().join("sub")).unwrap();
        fs::write(dir.path().join("sub/new file.txt"), "x\n").unwrap();
        let mut changes = changed_files(dir.path());
        changes.sort_by(|a, b| a.path.cmp(&b.path));
        let summary: Vec<(String, char)> = changes
            .iter()
            .map(|c| (c.path.display().to_string(), c.status))
            .collect();
        assert_eq!(
            summary,
            vec![
                ("gone.txt".to_string(), 'D'),
                ("kept.txt".to_string(), 'M'),
                ("sub/new file.txt".to_string(), '?'),
            ]
        );
    }

    #[test]
    fn head_text_reads_the_commit() {
        let dir = repo();
        fs::write(dir.path().join("kept.txt"), "two\n").unwrap();
        assert_eq!(head_text(dir.path(), Path::new("kept.txt")), "one\n");
        assert_eq!(head_text(dir.path(), Path::new("missing.txt")), "");
    }

    #[test]
    fn blame_names_the_commit() {
        let dir = repo();
        let blame = blame_line(dir.path(), Path::new("kept.txt"), 1).unwrap();
        assert!(blame.ends_with("Tester: first"), "{blame}");
    }

    #[test]
    fn outside_a_repository_nothing_changed() {
        let dir = tempfile::tempdir().unwrap();
        assert!(changed_files(dir.path()).is_empty());
    }
}
