use std::collections::HashMap;
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
        // This runs all the time, beside whatever else is using the repository. Left to
        // itself, `git status` takes the index lock, and a commit at that moment is refused.
        .env("GIT_OPTIONAL_LOCKS", "0")
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

/// A file git knows something about: what's staged, and what isn't.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub path: PathBuf,
    /// X and Y of `git status`, with `?` for a file git doesn't track.
    pub index: char,
    pub worktree: char,
}

impl Entry {
    pub fn is_untracked(&self) -> bool {
        self.index == '?'
    }

    /// Both sides changed it and git couldn't merge them: `UU`, `AA`, `DD` and the rest.
    pub fn is_conflicted(&self) -> bool {
        matches!(
            (self.index, self.worktree),
            ('U', _) | (_, 'U') | ('A', 'A') | ('D', 'D')
        )
    }

    pub fn is_staged(&self) -> bool {
        !self.is_conflicted() && !matches!(self.index, ' ' | '?')
    }

    pub fn is_unstaged(&self) -> bool {
        !self.is_conflicted() && (self.is_untracked() || self.worktree != ' ')
    }

    /// The letter it shows in one list or the other, as VS Code writes them.
    pub fn letter(&self, staged: bool) -> char {
        if self.is_conflicted() {
            return 'C';
        }
        match (self.is_untracked(), staged) {
            (true, _) => 'U',
            (false, true) => self.index,
            (false, false) => self.worktree,
        }
    }
}

/// Every changed file, with both sides of its status.
pub fn status_entries(root: &Path) -> Vec<Entry> {
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
    let mut entries = Vec::new();
    let mut fields = output.split('\0');
    while let Some(entry) = fields.next() {
        let mut chars = entry.chars();
        let (Some(index), Some(worktree)) = (chars.next(), chars.next()) else {
            continue;
        };
        if entry.len() < 4 {
            continue;
        }
        if matches!(index, 'R' | 'C') {
            fields.next();
        }
        entries.push(Entry {
            path: PathBuf::from(&entry[3..]),
            index,
            worktree,
        });
    }
    entries
}

fn run_git(root: &Path, args: &[&str], paths: &[PathBuf]) -> Result<(), String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .args(paths)
        // Nothing here can answer a prompt, so one mustn't hang waiting.
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .map_err(|e| e.to_string())?;
    if output.status.success() {
        return Ok(());
    }
    let said = String::from_utf8_lossy(&output.stderr);
    let first = said.lines().find(|line| !line.trim().is_empty());
    Err(first.unwrap_or("git refused").trim().to_string())
}

/// Stages whole files.
pub fn stage(root: &Path, paths: &[PathBuf]) -> Result<(), String> {
    run_git(root, &["add", "-A", "--"], paths)
}

/// Takes files back out of what's staged, leaving their changes alone.
pub fn unstage(root: &Path, paths: &[PathBuf]) -> Result<(), String> {
    if git(root, &["rev-parse", "--verify", "-q", "HEAD"]).is_some() {
        run_git(root, &["reset", "-q", "HEAD", "--"], paths)
    } else {
        // Before the first commit there's nothing to go back to.
        run_git(root, &["rm", "-q", "--cached", "--"], paths)
    }
}

/// Throws a file's changes away. A file git never knew about simply goes.
pub fn discard(root: &Path, path: &Path, untracked: bool) -> Result<(), String> {
    if untracked {
        return std::fs::remove_file(root.join(path)).map_err(|e| e.to_string());
    }
    run_git(root, &["checkout", "--"], &[path.to_path_buf()])
}

/// Commits to push and to pull, or None with no upstream.
pub fn ahead_behind(root: &Path) -> Option<(usize, usize)> {
    let counts = git(
        root,
        &["rev-list", "--left-right", "--count", "HEAD...@{upstream}"],
    )?;
    let mut numbers = counts.split_whitespace().map(str::parse::<usize>);
    Some((numbers.next()?.ok()?, numbers.next()?.ok()?))
}

pub fn has_remote(root: &Path) -> bool {
    git(root, &["remote"]).is_some_and(|names| !names.trim().is_empty())
}

pub fn push(root: &Path) -> Result<(), String> {
    run_git(root, &["push", "-q"], &[])
}

pub fn pull(root: &Path) -> Result<(), String> {
    run_git(root, &["pull", "-q", "--ff-only"], &[])
}

/// Pushes a branch that has no upstream yet, and tracks it.
pub fn publish(root: &Path, branch: &str) -> Result<(), String> {
    run_git(root, &["push", "-q", "-u", "origin", branch], &[])
}

/// The checked-out branch, or None outside a repository or on a detached HEAD.
pub fn current_branch(root: &Path) -> Option<String> {
    let name = git(root, &["symbolic-ref", "--short", "-q", "HEAD"])?;
    Some(name.trim().to_string())
}

/// A local branch, and when it last moved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Branch {
    pub name: String,
    pub when: String,
}

/// Local branches, most recently moved first.
pub fn branches(root: &Path) -> Vec<Branch> {
    let format = "--format=%(refname:short)%09%(committerdate:relative)";
    let args = [
        "for-each-ref",
        "--sort=-committerdate",
        format,
        "refs/heads",
    ];
    let Some(output) = git(root, &args) else {
        return Vec::new();
    };
    output
        .lines()
        .filter_map(|line| line.split_once('\t'))
        .map(|(name, when)| Branch {
            name: name.to_string(),
            when: when.to_string(),
        })
        .collect()
}

fn switch(root: &Path, args: &[&str]) -> Result<(), String> {
    let done = Command::new("git")
        .current_dir(root)
        .arg("switch")
        .args(args)
        .output()
        .map_err(|e| e.to_string())?;
    if done.status.success() {
        return Ok(());
    }
    let said = String::from_utf8_lossy(&done.stderr);
    let first = said.lines().find(|l| !l.trim().is_empty());
    Err(first.unwrap_or("git refused").trim().to_string())
}

/// Moves to a branch. Git refuses when that would lose changes, and says why.
pub fn switch_branch(root: &Path, name: &str) -> Result<(), String> {
    switch(root, &[name])
}

/// Starts a branch here and moves to it.
pub fn create_branch(root: &Path, name: &str) -> Result<(), String> {
    switch(root, &["-c", name])
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

/// Lines added and removed in each file since a commit, or HEAD for None.
pub fn numstat(root: &Path, base: Option<&str>) -> HashMap<PathBuf, (usize, usize)> {
    let text = git(root, &["diff", "--numstat", base.unwrap_or("HEAD")]).unwrap_or_default();
    let mut stats = HashMap::new();
    for line in text.lines() {
        let mut parts = line.splitn(3, '\t');
        let (Some(added), Some(removed), Some(path)) = (parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        if let (Ok(added), Ok(removed)) = (added.parse(), removed.parse()) {
            stats.insert(PathBuf::from(path), (added, removed));
        }
    }
    stats
}

/// A checkout of this repository.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Worktree {
    pub path: PathBuf,
    pub branch: String,
    pub changes: usize,
}

/// Every worktree, each with how many files differ from its HEAD.
pub fn worktrees(root: &Path) -> Vec<Worktree> {
    let Some(output) = git(root, &["worktree", "list", "--porcelain"]) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for block in output.split("\n\n") {
        let field = |name: &str| block.lines().find_map(|l| l.strip_prefix(name));
        let Some(path) = field("worktree ") else {
            continue;
        };
        let path = PathBuf::from(path);
        let branch = field("branch refs/heads/")
            .unwrap_or("detached")
            .to_string();
        let changes = changed_files(&path).len();
        found.push(Worktree {
            path,
            branch,
            changes,
        });
    }
    found
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

    #[test]
    fn reading_never_takes_the_lock_a_commit_needs() {
        let dir = repo();
        let told = ["-c", "alias.told=!printenv GIT_OPTIONAL_LOCKS", "told"];
        let told = git(dir.path(), &told);
        assert_eq!(told.as_deref().map(str::trim), Some("0"));
    }

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
    fn numstat_counts_lines_by_file() {
        let dir = repo();
        fs::write(dir.path().join("kept.txt"), "two\nthree\n").unwrap();
        let stats = numstat(dir.path(), None);
        assert_eq!(stats[Path::new("kept.txt")], (2, 1));
    }

    #[test]
    fn worktrees_are_listed_with_their_changes() {
        let dir = repo();
        let other = tempfile::tempdir().unwrap();
        let path = other.path().join("wt");
        let target = path.to_string_lossy();
        git(
            dir.path(),
            &["worktree", "add", "-q", "-b", "agent-a", &target],
        )
        .unwrap();
        fs::write(path.join("kept.txt"), "edited there\n").unwrap();
        let found = worktrees(dir.path());
        assert_eq!(found.len(), 2, "{found:?}");
        let agent = found.iter().find(|w| w.branch == "agent-a").unwrap();
        assert_eq!(agent.changes, 1);
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
