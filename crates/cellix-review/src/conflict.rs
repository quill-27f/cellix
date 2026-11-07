//! Merge conflicts: the `<<<<<<<` blocks git leaves in a file, and the merge or rebase that
//! left them. Agents and rebases produce these constantly, so each can be settled in place.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::git::git;

/// One conflict in a file. Lines count from 0 and name the marker lines themselves.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Conflict {
    /// The `<<<<<<<` line.
    pub start: usize,
    /// The `|||||||` line of a diff3 conflict, which opens the common ancestor's version.
    pub base: Option<usize>,
    /// The `=======` line.
    pub middle: usize,
    /// The `>>>>>>>` line.
    pub end: usize,
    /// What git wrote after the markers: `HEAD`, a branch, or a commit and its subject.
    pub current: String,
    pub incoming: String,
}

impl Conflict {
    /// Lines of the version that was checked out.
    pub fn current_lines(&self) -> std::ops::Range<usize> {
        self.start + 1..self.base.unwrap_or(self.middle)
    }

    /// Lines of the version coming in.
    pub fn incoming_lines(&self) -> std::ops::Range<usize> {
        self.middle + 1..self.end
    }
}

/// Which side of a conflict to keep.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Resolution {
    Current,
    Incoming,
    /// Current, then incoming.
    Both,
}

fn marker<'a>(line: &'a str, mark: &str) -> Option<&'a str> {
    let rest = line.strip_prefix(mark)?;
    let rest = rest.trim_end_matches(['\n', '\r']);
    // Seven marks and then the end of the line or a space: eight `<` is text, not a marker.
    match rest.chars().next() {
        None => Some(""),
        Some(' ') => Some(rest[1..].trim()),
        Some(_) => None,
    }
}

/// Every whole conflict in a text, in order. A block missing its end isn't one.
pub fn conflicts(text: &str) -> Vec<Conflict> {
    let mut found = Vec::new();
    let mut open: Option<Conflict> = None;
    for (index, line) in text.split_inclusive('\n').enumerate() {
        if let Some(label) = marker(line, "<<<<<<<") {
            open = Some(Conflict {
                start: index,
                base: None,
                middle: usize::MAX,
                end: usize::MAX,
                current: label.to_string(),
                incoming: String::new(),
            });
            continue;
        }
        let Some(conflict) = open.as_mut() else {
            continue;
        };
        if conflict.middle == usize::MAX {
            if conflict.base.is_none() && marker(line, "|||||||").is_some() {
                conflict.base = Some(index);
            } else if marker(line, "=======").is_some_and(str::is_empty) {
                conflict.middle = index;
            }
        } else if let Some(label) = marker(line, ">>>>>>>") {
            conflict.end = index;
            conflict.incoming = label.to_string();
            found.extend(open.take());
        }
    }
    found
}

pub fn has_conflicts(text: &str) -> bool {
    !conflicts(text).is_empty()
}

/// The two sides of a conflict as text, for comparing them.
pub fn sides(text: &str, conflict: &Conflict) -> (String, String) {
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let take = |range: std::ops::Range<usize>| lines.get(range).unwrap_or_default().concat();
    (
        take(conflict.current_lines()),
        take(conflict.incoming_lines()),
    )
}

/// The text with one conflict settled and every other line as it was.
pub fn resolve(text: &str, conflict: &Conflict, how: Resolution) -> String {
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    if conflict.end >= lines.len() {
        return text.to_string();
    }
    let take = |range: std::ops::Range<usize>| lines.get(range).unwrap_or_default().concat();
    let mut kept = match how {
        Resolution::Current => take(conflict.current_lines()),
        Resolution::Incoming => take(conflict.incoming_lines()),
        Resolution::Both => {
            let mut both = take(conflict.current_lines());
            if !both.is_empty() && !both.ends_with('\n') {
                both.push('\n');
            }
            both + &take(conflict.incoming_lines())
        }
    };
    // The end marker's line ending belongs to what's kept, so a conflict on the last line
    // of a file without one doesn't gain or lose it.
    let after = &lines[conflict.end + 1..];
    if after.is_empty() && !lines[conflict.end].ends_with('\n') && kept.ends_with('\n') {
        kept.pop();
        if kept.ends_with('\r') {
            kept.pop();
        }
    }
    take(0..conflict.start) + &kept + &after.concat()
}

/// What kind of operation stopped with conflicts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operation {
    Merge,
    Rebase,
    CherryPick,
    Revert,
}

impl Operation {
    pub fn verb(self) -> &'static str {
        match self {
            Operation::Merge => "Merging",
            Operation::Rebase => "Rebasing",
            Operation::CherryPick => "Cherry-picking",
            Operation::Revert => "Reverting",
        }
    }

    fn command(self) -> &'static str {
        match self {
            Operation::Merge => "merge",
            Operation::Rebase => "rebase",
            Operation::CherryPick => "cherry-pick",
            Operation::Revert => "revert",
        }
    }
}

/// A merge, rebase, cherry-pick or revert under way.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Underway {
    pub operation: Operation,
    /// The branch or commit coming in, as plainly as git can name it.
    pub incoming: String,
}

fn git_path(root: &Path, name: &str) -> Option<PathBuf> {
    let path = PathBuf::from(git(root, &["rev-parse", "--git-path", name])?.trim());
    Some(if path.is_absolute() {
        path
    } else {
        root.join(path)
    })
}

/// Names a commit by a branch that points at it, else by its short hash.
fn name_of(root: &Path, commit: &str) -> String {
    let commit = commit.trim();
    let named = git(root, &["name-rev", "--name-only", "--no-undefined", commit]);
    match named.map(|n| n.trim().to_string()) {
        Some(name) if !name.is_empty() && !name.contains('~') && !name.contains('^') => {
            name.trim_start_matches("remotes/").to_string()
        }
        _ => commit.get(..7).unwrap_or(commit).to_string(),
    }
}

/// The merge, rebase, cherry-pick or revert waiting to be finished, if there is one.
pub fn underway(root: &Path) -> Option<Underway> {
    let read = |name: &str| std::fs::read_to_string(git_path(root, name)?).ok();
    for dir in ["rebase-merge", "rebase-apply"] {
        if git_path(root, dir).is_some_and(|p| p.is_dir()) {
            let onto = read(&format!("{dir}/onto")).unwrap_or_default();
            let head = read(&format!("{dir}/head-name")).unwrap_or_default();
            let head = head.trim().trim_start_matches("refs/heads/").to_string();
            let incoming = if onto.trim().is_empty() {
                head
            } else {
                name_of(root, &onto)
            };
            return Some(Underway {
                operation: Operation::Rebase,
                incoming,
            });
        }
    }
    let heads = [
        ("MERGE_HEAD", Operation::Merge),
        ("CHERRY_PICK_HEAD", Operation::CherryPick),
        ("REVERT_HEAD", Operation::Revert),
    ];
    for (name, operation) in heads {
        if let Some(commit) = read(name) {
            let first = commit.lines().next().unwrap_or_default();
            return Some(Underway {
                operation,
                incoming: name_of(root, first),
            });
        }
    }
    None
}

/// Gives up on what's under way and puts things back as they were before it.
pub fn abort(root: &Path, operation: Operation) -> Result<(), String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args([operation.command(), "--abort"])
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

/// Goes on with a rebase, cherry-pick or revert once its conflicts are staged. A merge is
/// finished by committing.
pub fn proceed(root: &Path, operation: Operation) -> Result<(), String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args([operation.command(), "--continue"])
        // Keep the message git prepared rather than open an editor nobody can see.
        .env("GIT_EDITOR", "true")
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

/// The message git prepared for the merge commit, if there's one waiting.
pub fn merge_message(root: &Path) -> Option<String> {
    let text = std::fs::read_to_string(git_path(root, "MERGE_MSG")?).ok()?;
    let kept: Vec<&str> = text.lines().filter(|l| !l.starts_with('#')).collect();
    let message = kept.join("\n").trim().to_string();
    (!message.is_empty()).then_some(message)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::git::tests::repo;
    use std::fs;

    const TWO: &str = "\
keep
<<<<<<< HEAD
mine
=======
theirs
>>>>>>> feature
middle
<<<<<<< HEAD
a
||||||| base
o
=======
b
c
>>>>>>> feature
end
";

    #[test]
    fn conflicts_are_found_with_their_labels() {
        let found = conflicts(TWO);
        assert_eq!(found.len(), 2);
        assert_eq!((found[0].start, found[0].middle, found[0].end), (1, 3, 5));
        assert_eq!(found[0].current, "HEAD");
        assert_eq!(found[0].incoming, "feature");
        assert_eq!(found[1].base, Some(9));
        assert_eq!(found[1].current_lines(), 8..9);
        assert_eq!(found[1].incoming_lines(), 12..14);
        assert_eq!(sides(TWO, &found[1]), ("a\n".into(), "b\nc\n".into()));
    }

    #[test]
    fn each_resolution_keeps_its_side() {
        let first = &conflicts(TWO)[0];
        let current = resolve(TWO, first, Resolution::Current);
        assert!(
            current.starts_with("keep\nmine\nmiddle\n<<<<<<< HEAD\n"),
            "{current}"
        );
        let incoming = resolve(TWO, first, Resolution::Incoming);
        assert!(incoming.starts_with("keep\ntheirs\nmiddle\n"), "{incoming}");
        let both = resolve(TWO, first, Resolution::Both);
        assert!(both.starts_with("keep\nmine\ntheirs\nmiddle\n"), "{both}");
        // The other conflict is untouched and still found.
        assert_eq!(conflicts(&both).len(), 1);
        let last = &conflicts(&both)[0];
        let done = resolve(&both, last, Resolution::Incoming);
        assert_eq!(done, "keep\nmine\ntheirs\nmiddle\nb\nc\nend\n");
        assert!(!has_conflicts(&done));
    }

    #[test]
    fn text_that_only_looks_like_a_marker_is_left_alone() {
        let text = "<<<<<<<< eight\n=======\n>>>>>>> x\n";
        assert!(conflicts(text).is_empty());
        let unfinished = "<<<<<<< HEAD\na\n=======\nb\n";
        assert!(conflicts(unfinished).is_empty());
    }

    #[test]
    fn a_last_line_without_a_newline_stays_so() {
        let text = "<<<<<<< HEAD\na\n=======\nb\n>>>>>>> x";
        let conflict = &conflicts(text)[0];
        assert_eq!(resolve(text, conflict, Resolution::Incoming), "b");
        let crlf = "x\r\n<<<<<<< HEAD\r\na\r\n=======\r\nb\r\n>>>>>>> x\r\ny\r\n";
        let conflict = &conflicts(crlf)[0];
        assert_eq!(
            resolve(crlf, conflict, Resolution::Both),
            "x\r\na\r\nb\r\ny\r\n"
        );
    }

    pub(crate) fn run(dir: &Path, args: &[&str]) -> bool {
        Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .unwrap()
            .status
            .success()
    }

    /// A repository stopped in a merge with a conflict in kept.txt.
    pub(crate) fn conflicted() -> tempfile::TempDir {
        let dir = repo();
        let path = dir.path();
        assert!(run(path, &["checkout", "-q", "-b", "feature"]));
        fs::write(path.join("kept.txt"), "theirs\n").unwrap();
        assert!(run(path, &["commit", "-qam", "Theirs"]));
        assert!(run(path, &["checkout", "-q", "-"]));
        fs::write(path.join("kept.txt"), "mine\n").unwrap();
        assert!(run(path, &["commit", "-qam", "Mine"]));
        assert!(
            !run(path, &["merge", "-q", "feature"]),
            "the merge conflicts"
        );
        dir
    }

    #[test]
    fn a_real_merge_is_seen_and_settled() {
        let dir = conflicted();
        let path = dir.path();
        let under = underway(path).expect("a merge is under way");
        assert_eq!(under.operation, Operation::Merge);
        assert_eq!(under.incoming, "feature");
        assert!(merge_message(path).unwrap().contains("feature"));
        let entries = crate::status_entries(path);
        let kept = entries
            .iter()
            .find(|e| e.path == Path::new("kept.txt"))
            .unwrap();
        assert!(kept.is_conflicted(), "{kept:?}");
        assert!(!kept.is_staged() && !kept.is_unstaged());
        let text = fs::read_to_string(path.join("kept.txt")).unwrap();
        let conflict = &conflicts(&text)[0];
        assert_eq!(conflict.incoming, "feature");
        fs::write(
            path.join("kept.txt"),
            resolve(&text, conflict, Resolution::Incoming),
        )
        .unwrap();
        crate::stage(path, &[PathBuf::from("kept.txt")]).unwrap();
        let entries = crate::status_entries(path);
        assert!(!entries.iter().any(|e| e.is_conflicted()));
        crate::commit(path, &merge_message(path).unwrap()).unwrap();
        assert!(underway(path).is_none());
        assert_eq!(
            fs::read_to_string(path.join("kept.txt")).unwrap(),
            "theirs\n"
        );
    }

    #[test]
    fn a_merge_can_be_abandoned() {
        let dir = conflicted();
        abort(dir.path(), Operation::Merge).unwrap();
        assert!(underway(dir.path()).is_none());
        let text = fs::read_to_string(dir.path().join("kept.txt")).unwrap();
        assert_eq!(text, "mine\n");
    }

    #[test]
    fn a_rebase_is_seen_and_continued() {
        let dir = repo();
        let path = dir.path();
        assert!(run(path, &["checkout", "-q", "-b", "feature"]));
        fs::write(path.join("kept.txt"), "theirs\n").unwrap();
        assert!(run(path, &["commit", "-qam", "Theirs"]));
        assert!(run(path, &["checkout", "-q", "-"]));
        fs::write(path.join("kept.txt"), "mine\n").unwrap();
        assert!(run(path, &["commit", "-qam", "Mine"]));
        assert!(run(path, &["checkout", "-q", "feature"]));
        assert!(!run(path, &["rebase", "-q", "-"]), "the rebase conflicts");
        let under = underway(path).expect("a rebase is under way");
        assert_eq!(under.operation, Operation::Rebase);
        fs::write(path.join("kept.txt"), "both\n").unwrap();
        crate::stage(path, &[PathBuf::from("kept.txt")]).unwrap();
        proceed(path, Operation::Rebase).unwrap();
        assert!(underway(path).is_none());
    }
}
