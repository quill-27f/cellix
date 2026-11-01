use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// A file that differs from HEAD. `status` is `M`, `A`, `D` or `?`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Change {
    pub path: PathBuf,
    pub status: char,
}

pub(crate) fn git(root: &Path, args: &[&str]) -> Option<String> {
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

/// Parses `--name-status -z` output.
pub(crate) fn parse_name_status(output: &str) -> Vec<Change> {
    let mut changes = Vec::new();
    let mut fields = output.split('\0');
    while let (Some(status), Some(path)) = (fields.next(), fields.next()) {
        let Some(letter) = status.chars().next() else {
            break;
        };
        changes.push(Change {
            path: PathBuf::from(path),
            status: letter,
        });
    }
    changes
}

/// The file as staged, or empty if it isn't in the index.
pub fn index_text(root: &Path, path: &Path) -> String {
    let spec = format!(":./{}", path.display());
    git(root, &["show", &spec]).unwrap_or_default()
}

/// How many files are staged.
pub fn staged_count(root: &Path) -> usize {
    git(root, &["diff", "--cached", "--name-only"]).map_or(0, |o| o.lines().count())
}

/// Stages one change by applying a patch to the index, leaving the rest of the file alone.
pub fn stage_patch(root: &Path, path: &Path, patch: &str) -> Result<(), String> {
    let file = path.to_string_lossy();
    if !root.join(path).exists() || index_text(root, path).is_empty() {
        // A new file has to be known to the index before a patch can apply to it.
        let _ = git(root, &["add", "--intent-to-add", "--", &file]);
    }
    let mut child = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["apply", "--cached", "--whitespace=nowarn", "--recount", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(patch.as_bytes())
            .map_err(|e| e.to_string())?;
    }
    let output = child.wait_with_output().map_err(|e| e.to_string())?;
    if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}

/// Commits what's staged. Returns git's first line of output.
pub fn commit(root: &Path, message: &str) -> Result<String, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["commit", "-q", "-m", message])
        .output()
        .map_err(|e| e.to_string())?;
    if output.status.success() {
        Ok(format!("Committed: {message}"))
    } else {
        let error = String::from_utf8_lossy(&output.stderr);
        Err(error
            .lines()
            .next()
            .unwrap_or("Could not commit")
            .to_string())
    }
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
pub(crate) mod tests {
    use super::*;
    use std::fs;

    pub(crate) fn repo() -> tempfile::TempDir {
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
    fn accepting_a_hunk_stages_only_it() {
        let dir = repo();
        let old: String = (0..30).map(|i| format!("{i}\n")).collect();
        fs::write(dir.path().join("long.txt"), &old).unwrap();
        git(dir.path(), &["add", "."]).unwrap();
        git(dir.path(), &["commit", "-q", "-m", "long"]).unwrap();
        let new = old
            .replace("\n2\n", "\ntwo\n")
            .replace("27\n", "twenty-seven\n");
        fs::write(dir.path().join("long.txt"), &new).unwrap();
        let path = Path::new("long.txt");
        let diff = crate::FileDiff::new(&old, &new);
        let patch = crate::hunk_patch(path, &old, &new, &diff.hunks[0]);
        stage_patch(dir.path(), path, &patch).unwrap();
        let staged = index_text(dir.path(), path);
        assert!(staged.contains("two\n"), "{staged}");
        assert!(staged.contains("\n27\n"), "the other hunk stays unstaged");
        assert_eq!(
            fs::read_to_string(dir.path().join("long.txt")).unwrap(),
            new
        );
        // The second hunk still applies, now that the file has moved on.
        let patch = crate::hunk_patch(path, &old, &new, &diff.hunks[1]);
        stage_patch(dir.path(), path, &patch).unwrap();
        assert_eq!(index_text(dir.path(), path), new);
    }

    #[test]
    fn a_new_file_can_be_staged() {
        let dir = repo();
        fs::write(dir.path().join("made.txt"), "x\ny\n").unwrap();
        let path = Path::new("made.txt");
        let diff = crate::FileDiff::new("", "x\ny\n");
        let patch = crate::hunk_patch(path, "", "x\ny\n", &diff.hunks[0]);
        stage_patch(dir.path(), path, &patch).unwrap();
        assert_eq!(index_text(dir.path(), path), "x\ny\n");
    }

    #[test]
    fn commit_records_what_is_staged() {
        let dir = repo();
        fs::write(dir.path().join("kept.txt"), "two\n").unwrap();
        git(dir.path(), &["add", "."]).unwrap();
        assert_eq!(staged_count(dir.path()), 1);
        commit(dir.path(), "Say two").unwrap();
        assert_eq!(staged_count(dir.path()), 0);
        assert!(changed_files(dir.path()).is_empty());
    }

    #[test]
    fn outside_a_repository_nothing_changed() {
        let dir = tempfile::tempdir().unwrap();
        assert!(changed_files(dir.path()).is_empty());
    }
}
