//! Checkpoints of the working tree, so review can ask what changed in an agent's turn and
//! a turn can be undone. They live in private refs and never touch the branch or the index.

use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime};

use crate::git::{Change, git, parse_name_status, parse_numstat};

const PREFIX: &str = "refs/cellix/turns/";
/// Untracked files bigger than this stay out of checkpoints. Hashing a project's data dumps on
/// every prompt made the prompt hook outlast the time an agent gives it, and each checkpoint
/// copied them into `.git`.
pub const LARGEST_UNTRACKED: u64 = 10 * 1024 * 1024;
/// A lock on the checkpoint index older than this was left by a git that was stopped.
const STALE_LOCK: Duration = Duration::from_secs(10 * 60);
/// How long a checkpoint waits for another one to finish with the shared index.
const LOCK_WAIT: Duration = Duration::from_millis(1500);
/// How many numbers a checkpoint tries, when others take them first.
const TRIES: u64 = 16;

/// One checkpoint: the state of the tree when a turn began.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Turn {
    pub commit: String,
    pub number: u64,
    pub label: String,
}

/// Runs git with a private index, so a snapshot can't disturb what's staged, giving it `input`.
fn git_with_index_fed(root: &Path, index: &Path, args: &[&str], input: &[u8]) -> Option<String> {
    let mut child = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .env("GIT_INDEX_FILE", index)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut stdin = child.stdin.take()?;
    let fed = stdin.write_all(input).is_ok();
    drop(stdin);
    let output = child.wait_with_output().ok()?;
    (fed && output.status.success())
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn git_with_index(root: &Path, index: &Path, args: &[&str]) -> Option<String> {
    git_with_index_fed(root, index, args, &[])
}

/// The tree object for the working directory as it stands, ignored files and untracked files
/// over `LARGEST_UNTRACKED` aside.
fn snapshot_tree(root: &Path) -> Option<String> {
    let dir = PathBuf::from(git(root, &["rev-parse", "--absolute-git-dir"])?.trim());
    // Kept from one checkpoint to the next, so git knows each file's size and time and hashes
    // only what changed. Rebuilt from scratch, every file was read again on every prompt.
    let index = dir.join("cellix-index");
    let lock = dir.join("cellix-index.lock");
    if let Some(tree) = snapshot_with(root, &dir, &index) {
        return Some(tree);
    }
    // Another checkpoint has it, as when two agents prompt at once, or had it a moment ago
    // and has let go since: this one waits a little for the lock to go, and tries again.
    let deadline = SystemTime::now() + LOCK_WAIT;
    while lock.exists() && SystemTime::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    if let Some(tree) = snapshot_with(root, &dir, &index) {
        return Some(tree);
    }
    // Still not: it works from an index of its own, which nothing else can hold, so a
    // checkpoint is only ever lost to git itself. Named for the process and thread, since
    // checkpoints can run side by side in one.
    let own = dir.join(format!(
        "cellix-index-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = fs::copy(&index, &own);
    // A copy taken while the other was writing can't be read; then this one starts afresh.
    let tree = snapshot_with(root, &dir, &own).or_else(|| {
        let _ = fs::remove_file(&own);
        snapshot_with(root, &dir, &own)
    });
    // With nobody writing the shared one, this one's takes its place in one step. If the
    // shared one couldn't be read, that mends it; either way the next checkpoint has every
    // file's size and time to go by. While another holds it, the shared one is left alone.
    if tree.is_some() && !lock.exists() {
        let _ = fs::rename(&own, &index);
    }
    let _ = fs::remove_file(&own);
    tree
}

fn snapshot_with(root: &Path, dir: &Path, index: &Path) -> Option<String> {
    let lock = dir.join("cellix-index.lock");
    if fs::metadata(&lock)
        .and_then(|m| m.modified())
        .is_ok_and(|at| SystemTime::now().duration_since(at).unwrap_or_default() > STALE_LOCK)
    {
        let _ = fs::remove_file(&lock);
    }
    if !index.exists() {
        // The repository's own index already knows the tracked files, so the first checkpoint
        // hashes only what has changed since they were staged.
        let seeded = fs::copy(dir.join("index"), index).is_ok();
        if !seeded && git(root, &["rev-parse", "--verify", "-q", "HEAD"]).is_some() {
            git_with_index(root, index, &["read-tree", "HEAD"])?;
        }
    }
    // What's already in it, tracked or added by an earlier checkpoint, as it stands now.
    git_with_index(root, index, &["add", "-u", "."])?;
    let untracked = git_with_index_fed(
        root,
        index,
        &["ls-files", "-z", "-o", "--exclude-standard", "."],
        &[],
    )?;
    let small: Vec<&str> = untracked
        .split('\0')
        .filter(|path| !path.is_empty())
        .filter(|path| {
            fs::symlink_metadata(root.join(path)).is_ok_and(|m| m.len() <= LARGEST_UNTRACKED)
        })
        .collect();
    if !small.is_empty() {
        let mut paths = small.join("\0").into_bytes();
        paths.push(0);
        let args = ["add", "--pathspec-from-file=-", "--pathspec-file-nul"];
        git_with_index_fed(root, index, &args, &paths)?;
    }
    git_with_index(root, index, &["write-tree"])
}

/// Every checkpoint, oldest first.
pub fn turns(root: &Path) -> Vec<Turn> {
    let format = "--format=%(refname)%00%(objectname)%00%(contents:subject)";
    let Some(output) = git(root, &["for-each-ref", "--sort=refname", format, PREFIX]) else {
        return Vec::new();
    };
    output
        .lines()
        .filter_map(|line| {
            let mut parts = line.split('\0');
            let number = parts.next()?.strip_prefix(PREFIX)?.parse().ok()?;
            Some(Turn {
                number,
                commit: parts.next()?.to_string(),
                label: parts.next().unwrap_or("").to_string(),
            })
        })
        .collect()
}

/// Records the tree. Nothing is added when it hasn't changed since the last checkpoint.
pub fn checkpoint(root: &Path, label: &str) -> Option<Turn> {
    let tree = snapshot_tree(root)?;
    let previous = turns(root);
    if let Some(last) = previous.last() {
        let spec = format!("{}^{{tree}}", last.commit);
        if git(root, &["rev-parse", &spec]).is_some_and(|t| t.trim() == tree) {
            return Some(last.clone());
        }
    }
    let identity = [
        "-c",
        "user.name=Cellix",
        "-c",
        "user.email=cellix@localhost",
    ];
    let mut args = identity.to_vec();
    args.extend(["commit-tree", &tree, "-m", label]);
    let commit = git(root, &args)?.trim().to_string();
    let number = claim(root, &commit, previous.last().map_or(1, |t| t.number + 1))?;
    Some(Turn {
        commit,
        number,
        label: label.to_string(),
    })
}

/// Gives `commit` the first checkpoint number from `first` that no other has. A ref is made
/// only where none is yet, so two checkpoints taken at once, as two agents prompting
/// together, each get a number, and neither replaces the other.
fn claim(root: &Path, commit: &str, first: u64) -> Option<u64> {
    (first..first + TRIES).find(|number| {
        let name = format!("{PREFIX}{number:010}");
        git(root, &["update-ref", &name, commit, ""]).is_some()
    })
}

/// Files that differ between a checkpoint and the tree now.
pub fn changed_since(root: &Path, commit: &str) -> Vec<Change> {
    let Some(tree) = snapshot_tree(root) else {
        return Vec::new();
    };
    let args = ["diff-tree", "-r", "--name-status", "-z", commit, &tree];
    git(root, &args).map_or_else(Vec::new, |output| parse_name_status(&output))
}

/// Files that differ between two checkpoints, as one turn changed them.
pub fn changed_between(root: &Path, from: &str, to: &str) -> Vec<Change> {
    let args = ["diff-tree", "-r", "--name-status", "-z", from, to];
    git(root, &args).map_or_else(Vec::new, |output| parse_name_status(&output))
}

/// Lines added and removed by file between two checkpoints.
pub fn numstat_between(root: &Path, from: &str, to: &str) -> HashMap<PathBuf, (usize, usize)> {
    let text = git(root, &["diff", "--numstat", from, to]).unwrap_or_default();
    parse_numstat(&text)
}

/// A file as it was at a commit, or empty if it didn't exist.
pub fn text_at(root: &Path, commit: &str, path: &Path) -> String {
    let spec = format!("{commit}:./{}", path.display());
    git(root, &["show", &spec]).unwrap_or_default()
}

/// Puts the tree back as it was at a checkpoint, after checkpointing it as it is now, so
/// the undo can itself be undone. Returns how many files changed.
pub fn restore(root: &Path, commit: &str) -> Result<usize, String> {
    checkpoint(root, "Before undoing a turn").ok_or("Could not save the current state")?;
    let changes = changed_since(root, commit);
    for change in &changes {
        let target = root.join(&change.path);
        let spec = format!("{commit}:./{}", change.path.display());
        let shown = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(["show", &spec])
            .output()
            .map_err(|e| e.to_string())?;
        if shown.status.success() {
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            fs::write(&target, shown.stdout).map_err(|e| e.to_string())?;
        } else {
            fs::remove_file(&target).map_err(|e| e.to_string())?;
        }
    }
    Ok(changes.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::tests::repo;

    fn write(root: &Path, name: &str, text: &str) {
        fs::write(root.join(name), text).unwrap();
    }

    #[test]
    fn a_checkpoint_taken_meanwhile_is_neither_replaced_nor_its_index_lost() {
        let dir = repo();
        let root = dir.path();
        write(root, "kept.txt", "first\n");
        let first = checkpoint(root, "agent one").unwrap();
        // Another agent's checkpoint took the next number after this one looked.
        write(root, "kept.txt", "second\n");
        let other = checkpoint(root, "agent two").unwrap();
        assert_eq!(
            claim(root, &first.commit, other.number),
            Some(other.number + 1)
        );
        let kept = turns(root);
        assert!(
            kept.iter()
                .any(|k| k.number == other.number && k.commit == other.commit)
        );
        // And while another checkpoint holds the shared index, this one is still taken, and
        // the shared index, which saves hashing every file again, is kept.
        let git_dir = root.join(".git");
        fs::write(git_dir.join("cellix-index.lock"), "").unwrap();
        write(root, "kept.txt", "third\n");
        let third = checkpoint(root, "agent three");
        assert!(third.is_some(), "taken from a copy of its own");
        assert!(git_dir.join("cellix-index").exists());
        fs::remove_file(git_dir.join("cellix-index.lock")).unwrap();
    }

    #[test]
    fn checkpoints_taken_at_once_each_keep_their_own() {
        let dir = repo();
        write(dir.path(), "kept.txt", "a turn begins\n");
        let root = dir.path().to_path_buf();
        let taken: Vec<Turn> = std::thread::scope(|scope| {
            let runs: Vec<_> = (0..4)
                .map(|n| {
                    let root = &root;
                    scope.spawn(move || checkpoint(root, &format!("agent {n}")))
                })
                .collect();
            runs.into_iter()
                .map(|run| run.join().unwrap())
                .collect::<Option<_>>()
        })
        .expect("every checkpoint is taken");
        let kept = turns(&root);
        for turn in &taken {
            assert!(
                kept.iter()
                    .any(|k| k.number == turn.number && k.commit == turn.commit),
                "{turn:?} was replaced: {kept:?}"
            );
        }
        let git_dir = root.join(".git");
        assert!(
            git_dir.join("cellix-index").exists(),
            "the shared index is kept"
        );
        assert!(
            !fs::read_dir(&git_dir)
                .unwrap()
                .flatten()
                .any(|e| e.file_name().to_string_lossy().starts_with("cellix-index-"))
        );
    }

    #[test]
    fn a_checkpoint_sees_what_changed_after_it() {
        let dir = repo();
        write(dir.path(), "kept.txt", "before the turn\n");
        let turn = checkpoint(dir.path(), "first prompt").unwrap();
        write(dir.path(), "kept.txt", "after the turn\n");
        write(dir.path(), "made.txt", "new\n");
        let changes = changed_since(dir.path(), &turn.commit);
        assert_eq!(changes.len(), 2, "{changes:?}");
        let old = text_at(dir.path(), &turn.commit, Path::new("kept.txt"));
        assert_eq!(old, "before the turn\n");
    }

    #[test]
    fn checkpoints_leave_the_index_and_branch_alone() {
        let dir = repo();
        write(dir.path(), "kept.txt", "changed\n");
        let head = git(dir.path(), &["rev-parse", "HEAD"]);
        checkpoint(dir.path(), "x").unwrap();
        assert_eq!(git(dir.path(), &["rev-parse", "HEAD"]), head);
        assert_eq!(
            git(dir.path(), &["diff", "--cached", "--name-only"]).unwrap(),
            ""
        );
    }

    #[test]
    fn a_large_untracked_file_stays_out_of_checkpoints() {
        let dir = repo();
        write(dir.path(), "notes.txt", "small\n");
        let big = vec![b'x'; LARGEST_UNTRACKED as usize + 1];
        fs::write(dir.path().join("dump.bin"), big).unwrap();
        let turn = checkpoint(dir.path(), "prompt").unwrap();
        let files = git(dir.path(), &["ls-tree", "-r", "--name-only", &turn.commit]).unwrap();
        assert!(files.lines().any(|f| f == "notes.txt"), "{files}");
        assert!(!files.lines().any(|f| f == "dump.bin"), "{files}");
        // Nor does it show as a change the agent made.
        write(dir.path(), "notes.txt", "edited\n");
        let changes = changed_since(dir.path(), &turn.commit);
        assert_eq!(changes.len(), 1, "{changes:?}");
    }

    #[test]
    fn the_checkpoint_index_is_kept_and_follows_the_tree() {
        let dir = repo();
        write(dir.path(), "made.txt", "one\n");
        let first = checkpoint(dir.path(), "one").unwrap();
        let git_dir = git(dir.path(), &["rev-parse", "--absolute-git-dir"]).unwrap();
        assert!(Path::new(git_dir.trim()).join("cellix-index").exists());
        // A file an earlier checkpoint took in, then deleted, leaves the next one.
        fs::remove_file(dir.path().join("made.txt")).unwrap();
        write(dir.path(), "kept.txt", "changed again\n");
        let second = checkpoint(dir.path(), "two").unwrap();
        let files = git(
            dir.path(),
            &["ls-tree", "-r", "--name-only", &second.commit],
        )
        .unwrap();
        assert!(!files.lines().any(|f| f == "made.txt"), "{files}");
        assert_eq!(
            text_at(dir.path(), &second.commit, Path::new("kept.txt")),
            "changed again\n"
        );
        assert_eq!(
            text_at(dir.path(), &first.commit, Path::new("made.txt")),
            "one\n"
        );
    }

    #[test]
    fn an_unchanged_tree_adds_no_checkpoint() {
        let dir = repo();
        checkpoint(dir.path(), "one").unwrap();
        checkpoint(dir.path(), "two").unwrap();
        assert_eq!(turns(dir.path()).len(), 1);
    }

    #[test]
    fn undoing_a_turn_can_be_undone() {
        let dir = repo();
        write(dir.path(), "kept.txt", "good\n");
        let turn = checkpoint(dir.path(), "prompt").unwrap();
        write(dir.path(), "kept.txt", "agent edit\n");
        write(dir.path(), "made.txt", "agent file\n");
        assert_eq!(restore(dir.path(), &turn.commit), Ok(2));
        assert_eq!(
            fs::read_to_string(dir.path().join("kept.txt")).unwrap(),
            "good\n"
        );
        assert!(!dir.path().join("made.txt").exists());

        let undo_point = turns(dir.path()).last().unwrap().commit.clone();
        restore(dir.path(), &undo_point).unwrap();
        let kept = fs::read_to_string(dir.path().join("kept.txt")).unwrap();
        assert_eq!(kept, "agent edit\n");
        assert!(dir.path().join("made.txt").exists());
    }
}
