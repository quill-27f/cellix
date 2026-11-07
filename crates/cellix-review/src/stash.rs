//! Stashes: changes put aside, listed, and brought back.
//!
//! Every worktree of a repository shares one stash list, and an agent in another worktree may
//! push or pop while Cellix shows it. So a stash is named by its commit, and its place in the
//! list is looked up again at the moment it's popped or dropped.

use std::path::Path;
use std::process::Command;

use crate::git::{Change, git, parse_name_status};

/// One stash, newest first in the list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stash {
    pub hash: String,
    /// What git says it is: "WIP on main: 51ad7e0 Bound the queue", or the message given.
    pub message: String,
    pub time: i64,
}

impl Stash {
    /// The message without git's "On main: " or "WIP on main: " in front.
    pub fn title(&self) -> &str {
        let message = self.message.as_str();
        for lead in ["WIP on ", "On "] {
            if let Some((_, rest)) = message
                .strip_prefix(lead)
                .and_then(|rest| rest.split_once(": "))
            {
                return rest;
            }
        }
        message
    }
}

fn run(root: &Path, args: &[&str]) -> Result<(), String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .map_err(|e| e.to_string())?;
    if output.status.success() {
        return Ok(());
    }
    let said = String::from_utf8_lossy(&output.stderr);
    let said = if said.trim().is_empty() {
        String::from_utf8_lossy(&output.stdout).into_owned()
    } else {
        said.into_owned()
    };
    let first = said.lines().find(|line| !line.trim().is_empty());
    Err(first.unwrap_or("git refused").trim().to_string())
}

/// Every stash, newest first.
pub fn stashes(root: &Path) -> Vec<Stash> {
    let Some(output) = git(root, &["stash", "list", "--format=%H%x00%ct%x00%gs"]) else {
        return Vec::new();
    };
    output
        .lines()
        .filter_map(|line| {
            let mut parts = line.splitn(3, '\0');
            Some(Stash {
                hash: parts.next()?.to_string(),
                time: parts.next()?.parse().unwrap_or(0),
                message: parts.next()?.to_string(),
            })
        })
        .collect()
}

/// What to put aside.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Keep {
    /// Every change to a tracked file, staged or not.
    #[default]
    Tracked,
    /// Those and files git doesn't track.
    Untracked,
    /// Only what's staged.
    Staged,
}

/// Puts changes aside, with a message if one is given.
pub fn stash(root: &Path, message: &str, keep: Keep) -> Result<(), String> {
    let mut args = vec!["stash", "push"];
    match keep {
        Keep::Tracked => {}
        Keep::Untracked => args.push("--include-untracked"),
        Keep::Staged => args.push("--staged"),
    }
    let message = message.trim();
    if !message.is_empty() {
        args.extend(["-m", message]);
    }
    let before = stashes(root).first().map(|s| s.hash.clone());
    run(root, &args)?;
    // Git says nothing went aside, and succeeds, when there was nothing to put aside.
    if stashes(root).first().map(|s| s.hash.clone()) == before {
        return Err("There are no changes to stash".to_string());
    }
    Ok(())
}

/// Where a stash stands in the list now, as git names it.
fn place(root: &Path, hash: &str) -> Result<String, String> {
    let index = stashes(root)
        .iter()
        .position(|s| s.hash == hash)
        .ok_or("That stash is gone")?;
    Ok(format!("stash@{{{index}}}"))
}

/// Brings a stash's changes back and keeps it in the list.
pub fn apply_stash(root: &Path, hash: &str) -> Result<(), String> {
    let name = place(root, hash)?;
    run(root, &["stash", "apply", "--index", &name])
        .or_else(|_| run(root, &["stash", "apply", &name]))
}

/// Brings a stash's changes back and takes it off the list.
pub fn pop_stash(root: &Path, hash: &str) -> Result<(), String> {
    apply_stash(root, hash)?;
    drop_stash(root, hash)
}

/// Throws a stash away.
pub fn drop_stash(root: &Path, hash: &str) -> Result<(), String> {
    let name = place(root, hash)?;
    run(root, &["stash", "drop", "-q", &name])
}

/// The files a stash changed, untracked ones included.
pub fn stash_files(root: &Path, hash: &str) -> Vec<Change> {
    let mut changes = git(
        root,
        &[
            "diff",
            "--no-renames",
            "--name-status",
            "-z",
            &format!("{hash}^1"),
            hash,
        ],
    )
    .map_or_else(Vec::new, |o| parse_name_status(&o));
    // Untracked files sit in a third parent of their own.
    let third = format!("{hash}^3");
    if git(root, &["rev-parse", "--verify", "-q", &third]).is_some() {
        let args = [
            "diff-tree",
            "-r",
            "--root",
            "--no-commit-id",
            "--name-status",
            "-z",
            &third,
        ];
        changes.extend(git(root, &args).map_or_else(Vec::new, |o| parse_name_status(&o)));
    }
    changes
}

/// A file before and after a stash: as the commit it was made on had it, and as stashed.
pub fn stash_sides(root: &Path, hash: &str, path: &Path) -> (String, String) {
    let before = crate::text_at(root, &format!("{hash}^1"), path);
    let mut after = crate::text_at(root, hash, path);
    if after.is_empty() {
        after = crate::text_at(root, &format!("{hash}^3"), path);
    }
    (before, after)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::tests::repo;
    use std::fs;

    #[test]
    fn changes_go_aside_and_come_back() {
        let dir = repo();
        let path = dir.path();
        fs::write(path.join("kept.txt"), "two\n").unwrap();
        stash(path, "Halfway", Keep::Tracked).unwrap();
        assert_eq!(fs::read_to_string(path.join("kept.txt")).unwrap(), "one\n");
        let list = stashes(path);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].title(), "Halfway");
        let files = stash_files(path, &list[0].hash);
        assert_eq!(files.len(), 1);
        let (before, after) = stash_sides(path, &list[0].hash, Path::new("kept.txt"));
        assert_eq!((before.as_str(), after.as_str()), ("one\n", "two\n"));
        pop_stash(path, &list[0].hash).unwrap();
        assert_eq!(fs::read_to_string(path.join("kept.txt")).unwrap(), "two\n");
        assert!(stashes(path).is_empty());
    }

    #[test]
    fn untracked_files_go_aside_when_asked() {
        let dir = repo();
        let path = dir.path();
        fs::write(path.join("new.txt"), "n\n").unwrap();
        assert!(
            stash(path, "", Keep::Tracked).is_err(),
            "nothing tracked changed"
        );
        stash(path, "", Keep::Untracked).unwrap();
        assert!(!path.join("new.txt").exists());
        let list = stashes(path);
        assert!(list[0].title().contains("first"), "{}", list[0].message);
        let files = stash_files(path, &list[0].hash);
        assert!(
            files.iter().any(|c| c.path == Path::new("new.txt")),
            "{files:?}"
        );
        apply_stash(path, &list[0].hash).unwrap();
        assert!(path.join("new.txt").exists());
        assert_eq!(stashes(path).len(), 1, "apply keeps it");
        drop_stash(path, &list[0].hash).unwrap();
        assert!(stashes(path).is_empty());
    }

    #[test]
    fn a_stash_is_found_by_its_commit_after_others_come_and_go() {
        let dir = repo();
        let path = dir.path();
        fs::write(path.join("kept.txt"), "a\n").unwrap();
        stash(path, "A", Keep::Tracked).unwrap();
        let a = stashes(path)[0].hash.clone();
        fs::write(path.join("kept.txt"), "b\n").unwrap();
        stash(path, "B", Keep::Tracked).unwrap();
        // A is stash@{1} now; dropping it by its commit leaves B.
        drop_stash(path, &a).unwrap();
        let left = stashes(path);
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].title(), "B");
        assert!(drop_stash(path, &a).is_err(), "it's gone");
    }

    #[test]
    fn only_staged_changes_can_go_aside() {
        let dir = repo();
        let path = dir.path();
        fs::write(path.join("kept.txt"), "staged\n").unwrap();
        crate::stage(path, &[std::path::PathBuf::from("kept.txt")]).unwrap();
        fs::write(path.join("gone.txt"), "not staged\n").unwrap();
        if stash(path, "", Keep::Staged).is_err() {
            // Git before 2.35 has no --staged.
            return;
        }
        assert_eq!(fs::read_to_string(path.join("kept.txt")).unwrap(), "one\n");
        assert_eq!(
            fs::read_to_string(path.join("gone.txt")).unwrap(),
            "not staged\n"
        );
    }
}
