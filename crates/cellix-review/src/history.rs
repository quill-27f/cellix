//! Checkpoints of the working tree, so review can ask what changed in an agent's turn and
//! a turn can be undone. They live in private refs and never touch the branch or the index.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::git::{Change, git, parse_name_status};

const PREFIX: &str = "refs/cellix/turns/";

/// One checkpoint: the state of the tree when a turn began.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Turn {
    pub commit: String,
    pub number: u64,
    pub label: String,
}

/// Runs git with a private index, so a snapshot can't disturb what's staged.
fn git_with_index(root: &Path, index: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .env("GIT_INDEX_FILE", index)
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// The tree object for the working directory as it stands, ignored files aside.
fn snapshot_tree(root: &Path) -> Option<String> {
    let dir = git(root, &["rev-parse", "--absolute-git-dir"])?;
    let index = PathBuf::from(dir.trim()).join("cellix-index");
    let _ = fs::remove_file(&index);
    if git(root, &["rev-parse", "--verify", "-q", "HEAD"]).is_some() {
        git_with_index(root, &index, &["read-tree", "HEAD"])?;
    }
    git_with_index(root, &index, &["add", "-A", "."])?;
    let tree = git_with_index(root, &index, &["write-tree"]);
    let _ = fs::remove_file(&index);
    tree
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
    let number = previous.last().map_or(1, |t| t.number + 1);
    let name = format!("{PREFIX}{number:010}");
    git(root, &["update-ref", &name, &commit])?;
    Some(Turn {
        commit,
        number,
        label: label.to_string(),
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
