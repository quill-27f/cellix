//! Reviewing agent work: what changed, how it lines up, and what the reviewer said.

mod diff;
mod git;
mod history;

pub use diff::{
    FileDiff, Hunk, LineChange, Row, RowKind, Side, hunk_patch, line_changes, rank_changes,
    reject_hunk,
};
pub use git::{
    Change, Worktree, blame_line, changed_files, commit, current_branch, head_text, index_text,
    numstat, stage_patch, staged_count, worktrees,
};
pub use history::{Turn, changed_since, checkpoint, restore, text_at, turns};

use serde::{Deserialize, Serialize};

/// A reviewer's note on one line. `line` counts from 1, like the gutter does.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Comment {
    pub file: String,
    pub line: usize,
    pub text: String,
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
        };
        assert_eq!(
            append_comments("# Plan\nstep\n", &[comment]),
            "# Plan\nstep\n\n## Review comments\n\n- Line 2: Use a queue\n"
        );
        assert_eq!(append_comments("same", &[]), "same");
    }
}
