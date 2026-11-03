//! Reviewing agent work: what changed, how it lines up, and what the reviewer said.

mod diff;
mod git;
mod history;
pub mod threads;

pub use diff::{
    FileDiff, Hunk, LineChange, Row, RowKind, Side, hunk_patch, line_changes, rank_changes,
    reject_hunk,
};
pub use git::{
    Branch, Change, Entry, Worktree, ahead_behind, blame_line, branches, changed_files, commit,
    create_branch, current_branch, discard, has_remote, head_text, index_text, numstat, publish,
    pull, push, stage, stage_patch, staged_count, status_entries, switch_branch, unstage,
    worktrees,
};
pub use history::{Turn, changed_since, checkpoint, restore, text_at, turns};
pub use threads::{Reply, Thread, YOU};

use serde::{Deserialize, Serialize};

/// A reviewer's note on one line. `line` counts from 1, like the gutter does.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Comment {
    pub file: String,
    pub line: usize,
    pub text: String,
    /// What it's about: the words selected, or the line. An agent acts on it without opening
    /// the file, and it still says which code is meant once the lines move.
    #[serde(default)]
    pub quote: String,
    /// On a line the change took out, so `line` counts in the version before it.
    #[serde(default)]
    pub removed: bool,
}

impl Comment {
    /// How an agent reads it, on one line.
    pub fn message(&self) -> String {
        let place = if self.removed {
            format!(
                "{} line {} (a line your change removed)",
                self.file, self.line
            )
        } else {
            format!("{} line {}", self.file, self.line)
        };
        let quote = self.quote.trim();
        if quote.is_empty() {
            format!("{place}: {}", self.text)
        } else {
            format!("{place}, on \"{quote}\": {}", self.text)
        }
    }
}

/// Appends comments to a document, so an agent that only reads the file back, as with
/// `$VISUAL`, still receives them.
pub fn append_comments(document: &str, comments: &[Comment]) -> String {
    if comments.is_empty() {
        return document.to_string();
    }
    let mut out = document.trim_end().to_string();
    out.push_str("\n\n## Review comments\n\n");
    for comment in comments {
        out.push_str(&format!("- Line {}: {}\n", comment.line, comment.text));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comments_become_a_section() {
        let comment = Comment {
            file: "plan.md".into(),
            line: 2,
            text: "Use a queue".into(),
            ..Comment::default()
        };
        assert_eq!(
            append_comments("# Plan\nstep\n", &[comment]),
            "# Plan\nstep\n\n## Review comments\n\n- Line 2: Use a queue\n"
        );
        assert_eq!(append_comments("same", &[]), "same");
    }
}
