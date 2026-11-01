use std::collections::HashSet;
use std::path::Path;

use similar::{ChangeTag, DiffOp, TextDiff};

use crate::git::Change;

const CONTEXT_LINES: usize = 3;
/// Shorter lines repeat too often for a match to mean they moved.
const MIN_MOVED_LENGTH: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowKind {
    Same,
    Removed,
    Added,
    Changed,
}

/// One side of a row: its 1-based line number and text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Side {
    pub number: usize,
    pub text: String,
    /// Char ranges that differ from the other side of a changed line.
    pub changed: Vec<(usize, usize)>,
    /// This line appears on the other side too, somewhere else in the file.
    pub moved: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Row {
    /// Starts a hunk, and carries its index in `FileDiff::hunks`.
    Header(usize),
    Line {
        left: Option<Side>,
        right: Option<Side>,
        kind: RowKind,
    },
}

/// A changed region, in 0-based line ranges, including its context lines.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Hunk {
    pub old_start: usize,
    pub old_len: usize,
    pub new_start: usize,
    pub new_len: usize,
}

/// Two versions of a file lined up for side-by-side display.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FileDiff {
    pub rows: Vec<Row>,
    pub hunks: Vec<Hunk>,
}

fn clean(line: &str) -> String {
    line.trim_end_matches(['\n', '\r']).to_string()
}

impl FileDiff {
    pub fn new(old: &str, new: &str) -> Self {
        let diff = TextDiff::from_lines(old, new);
        let old_lines: Vec<&str> = old.split_inclusive('\n').collect();
        let new_lines: Vec<&str> = new.split_inclusive('\n').collect();
        let side = |lines: &[&str], i: usize| Side {
            number: i + 1,
            text: clean(lines[i]),
            changed: Vec::new(),
            moved: false,
        };
        let mut out = Self::default();
        for group in diff.grouped_ops(CONTEXT_LINES) {
            let (Some(first), Some(last)) = (group.first(), group.last()) else {
                continue;
            };
            let (old_range, new_range) = (first.old_range(), first.new_range());
            let (old_end, new_end) = (last.old_range().end, last.new_range().end);
            out.rows.push(Row::Header(out.hunks.len()));
            out.hunks.push(Hunk {
                old_start: old_range.start,
                old_len: old_end - old_range.start,
                new_start: new_range.start,
                new_len: new_end - new_range.start,
            });
            for op in &group {
                let (old_at, new_at) = (op.old_range(), op.new_range());
                match *op {
                    DiffOp::Equal { len, .. } => {
                        for i in 0..len {
                            out.rows.push(Row::Line {
                                left: Some(side(&old_lines, old_at.start + i)),
                                right: Some(side(&new_lines, new_at.start + i)),
                                kind: RowKind::Same,
                            });
                        }
                    }
                    _ => {
                        // Changed lines sit beside each other; the shorter side pads.
                        for i in 0..old_at.len().max(new_at.len()) {
                            let left =
                                (i < old_at.len()).then(|| side(&old_lines, old_at.start + i));
                            let right =
                                (i < new_at.len()).then(|| side(&new_lines, new_at.start + i));
                            let kind = match (&left, &right) {
                                (Some(_), Some(_)) => RowKind::Changed,
                                (Some(_), None) => RowKind::Removed,
                                _ => RowKind::Added,
                            };
                            let (mut left, mut right) = (left, right);
                            if let (Some(l), Some(r)) = (&mut left, &mut right) {
                                (l.changed, r.changed) = changed_words(&l.text, &r.text);
                            }
                            out.rows.push(Row::Line { left, right, kind });
                        }
                    }
                }
            }
        }
        out.mark_moved();
        out
    }

    /// Lines removed in one place and added in another are moves, not rewrites.
    fn mark_moved(&mut self) {
        let worth = |text: &str| text.trim().len() >= MIN_MOVED_LENGTH;
        let collect = |rows: &[Row], left: bool| -> HashSet<String> {
            rows.iter()
                .filter_map(|row| match row {
                    Row::Line {
                        left: l,
                        right: r,
                        kind,
                    } if *kind != RowKind::Same => {
                        if left {
                            l.as_ref()
                        } else {
                            r.as_ref()
                        }
                    }
                    _ => None,
                })
                .map(|side| side.text.trim().to_string())
                .filter(|text| worth(text))
                .collect()
        };
        let (removed, added) = (collect(&self.rows, true), collect(&self.rows, false));
        for row in &mut self.rows {
            if let Row::Line { left, right, kind } = row
                && *kind != RowKind::Same
            {
                if let Some(side) = left {
                    side.moved = added.contains(side.text.trim());
                }
                if let Some(side) = right {
                    side.moved = removed.contains(side.text.trim());
                }
            }
        }
    }

    /// Added and removed line counts, for the file list.
    pub fn stat(old: &str, new: &str) -> (usize, usize) {
        let diff = TextDiff::from_lines(old, new);
        let mut stat = (0, 0);
        for change in diff.iter_all_changes() {
            match change.tag() {
                ChangeTag::Insert => stat.0 += 1,
                ChangeTag::Delete => stat.1 += 1,
                ChangeTag::Equal => {}
            }
        }
        stat
    }
}

/// Splits a line into identifiers and single other characters, so `a.pop_front()` differs
/// from `a.pop_back()` by one word, not the whole expression.
fn tokens(line: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = None;
    for (at, c) in line.char_indices() {
        let word = c.is_alphanumeric() || c == '_';
        match (word, start) {
            (true, None) => start = Some(at),
            (false, _) => {
                if let Some(from) = start.take() {
                    out.push(&line[from..at]);
                }
                out.push(&line[at..at + c.len_utf8()]);
            }
            (true, Some(_)) => {}
        }
    }
    if let Some(from) = start {
        out.push(&line[from..]);
    }
    out
}

/// Char ranges within a line.
type Ranges = Vec<(usize, usize)>;

/// The changed parts of a line, as char ranges on each side. A line that changed almost
/// entirely gets none, since marking all of it says nothing.
fn changed_words(old: &str, new: &str) -> (Ranges, Ranges) {
    let (old_tokens, new_tokens) = (tokens(old), tokens(new));
    let diff = TextDiff::from_slices(&old_tokens, &new_tokens);
    let (mut left, mut right) = (Vec::new(), Vec::new());
    let (mut at_old, mut at_new, mut same) = (0, 0, 0);
    for change in diff.iter_all_changes() {
        let length = change.value().chars().count();
        match change.tag() {
            ChangeTag::Equal => {
                at_old += length;
                at_new += length;
                same += length;
            }
            ChangeTag::Delete => {
                push_range(&mut left, at_old, at_old + length);
                at_old += length;
            }
            ChangeTag::Insert => {
                push_range(&mut right, at_new, at_new + length);
                at_new += length;
            }
        }
    }
    if same * 3 < at_old.max(at_new) {
        return (Vec::new(), Vec::new());
    }
    (left, right)
}

fn push_range(ranges: &mut Ranges, start: usize, end: usize) {
    match ranges.last_mut() {
        Some(last) if last.1 == start => last.1 = end,
        _ => ranges.push((start, end)),
    }
}

/// One hunk as a patch, for staging. Context lines let it apply after other hunks moved
/// the file's line numbers.
pub fn hunk_patch(path: &Path, old: &str, new: &str, hunk: &Hunk) -> String {
    let old_lines: Vec<&str> = old.split_inclusive('\n').collect();
    let new_lines: Vec<&str> = new.split_inclusive('\n').collect();
    let old_part = old_lines[hunk.old_start..hunk.old_start + hunk.old_len].concat();
    let new_part = new_lines[hunk.new_start..hunk.new_start + hunk.new_len].concat();
    let name = path.display();
    let mut patch = format!(
        "--- a/{name}\n+++ b/{name}\n@@ -{},{} +{},{} @@\n",
        hunk.old_start + usize::from(hunk.old_len > 0),
        hunk.old_len,
        hunk.new_start + usize::from(hunk.new_len > 0),
        hunk.new_len
    );
    let diff = TextDiff::from_lines(&old_part, &new_part);
    for change in diff.iter_all_changes() {
        patch.push(match change.tag() {
            ChangeTag::Equal => ' ',
            ChangeTag::Delete => '-',
            ChangeTag::Insert => '+',
        });
        patch.push_str(change.value());
        if !change.value().ends_with('\n') {
            patch.push_str("\n\\ No newline at end of file\n");
        }
    }
    patch
}

fn risk(change: &Change) -> i32 {
    const GENERATED: [&str; 7] = [
        ".lock",
        "-lock.json",
        ".min.js",
        ".snap",
        "dist/",
        "target/",
        "vendor/",
    ];
    const SENSITIVE: [&str; 9] = [
        "auth",
        "secret",
        "token",
        "password",
        "migration",
        "payment",
        "security",
        ".github/",
        "dockerfile",
    ];
    const DOCS: [&str; 4] = [".md", "docs/", "test", "spec"];
    let path = change.path.to_string_lossy().to_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| path.contains(w));
    if has(&GENERATED) {
        0
    } else if has(&SENSITIVE) {
        3
    } else if has(&DOCS) {
        1
    } else {
        2
    }
}

/// Orders files so the ones most worth reading come first and generated ones last.
pub fn rank_changes(changes: &mut [Change]) {
    changes.sort_by(|a, b| risk(b).cmp(&risk(a)).then_with(|| a.path.cmp(&b.path)));
}

/// `new` with one hunk put back as it was in `old`.
pub fn reject_hunk(old: &str, new: &str, hunk: &Hunk) -> String {
    let old_lines: Vec<&str> = old.split_inclusive('\n').collect();
    let new_lines: Vec<&str> = new.split_inclusive('\n').collect();
    let mut out = String::new();
    out.extend(new_lines[..hunk.new_start].iter().copied());
    out.extend(
        old_lines[hunk.old_start..hunk.old_start + hunk.old_len]
            .iter()
            .copied(),
    );
    out.extend(new_lines[hunk.new_start + hunk.new_len..].iter().copied());
    out
}

/// How a line differs from the last commit, for the editor's margin.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineChange {
    Added,
    Modified,
    /// Lines were removed just above this one.
    DeletedAbove,
}

/// What changed, by zero-based line of `new`.
pub fn line_changes(old: &str, new: &str) -> Vec<(usize, LineChange)> {
    let mut changes = Vec::new();
    for op in TextDiff::from_lines(old, new).ops() {
        match *op {
            DiffOp::Equal { .. } => {}
            DiffOp::Insert {
                new_index, new_len, ..
            } => changes.extend((new_index..new_index + new_len).map(|l| (l, LineChange::Added))),
            DiffOp::Replace {
                new_index, new_len, ..
            } => {
                let lines = new_index..new_index + new_len;
                changes.extend(lines.map(|l| (l, LineChange::Modified)));
            }
            DiffOp::Delete { new_index, .. } => changes.push((new_index, LineChange::DeletedAbove)),
        }
    }
    changes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_changes_name_each_kind() {
        let old = "a\nb\nc\nd\n";
        let new = "a\nB\nc\nnew\n";
        assert_eq!(
            line_changes(old, new),
            [(1, LineChange::Modified), (3, LineChange::Modified)]
        );
        assert_eq!(line_changes("a\n", "a\nb\n"), [(1, LineChange::Added)]);
        assert_eq!(
            line_changes("a\nb\n", "b\n"),
            [(0, LineChange::DeletedAbove)]
        );
    }

    fn lines(diff: &FileDiff) -> Vec<(Option<usize>, Option<usize>, RowKind)> {
        diff.rows
            .iter()
            .filter_map(|row| match row {
                Row::Line { left, right, kind } => Some((
                    left.as_ref().map(|s| s.number),
                    right.as_ref().map(|s| s.number),
                    *kind,
                )),
                Row::Header(_) => None,
            })
            .collect()
    }

    #[test]
    fn side_by_side_diff_aligns_hunks() {
        let diff = FileDiff::new("a\nb\nc\n", "a\nB\nnew\nc\n");
        assert_eq!(
            lines(&diff),
            vec![
                (Some(1), Some(1), RowKind::Same),
                (Some(2), Some(2), RowKind::Changed),
                (None, Some(3), RowKind::Added),
                (Some(3), Some(4), RowKind::Same),
            ]
        );
        assert_eq!(diff.hunks.len(), 1);
    }

    #[test]
    fn distant_changes_are_separate_hunks() {
        let old: String = (0..30).map(|i| format!("{i}\n")).collect();
        let new = old
            .replace("\n2\n", "\ntwo\n")
            .replace("27\n", "twenty-seven\n");
        assert_eq!(FileDiff::new(&old, &new).hunks.len(), 2);
    }

    #[test]
    fn reject_restores_only_that_hunk() {
        let old: String = (0..30).map(|i| format!("{i}\n")).collect();
        let new = old
            .replace("\n2\n", "\ntwo\n")
            .replace("27\n", "twenty-seven\n");
        let diff = FileDiff::new(&old, &new);
        let result = reject_hunk(&old, &new, &diff.hunks[0]);
        assert!(result.contains("\n2\n"));
        assert!(result.contains("twenty-seven"));
        assert_eq!(
            reject_hunk(&old, &result, &FileDiff::new(&old, &result).hunks[0]),
            old
        );
    }

    #[test]
    fn changed_words_are_marked() {
        let diff = FileDiff::new("let count = 1;\n", "let total = 1;\n");
        let Row::Line {
            left: Some(l),
            right: Some(r),
            ..
        } = &diff.rows[1]
        else {
            panic!("{:?}", diff.rows);
        };
        assert_eq!(l.changed, [(4, 9)]);
        assert_eq!(r.changed, [(4, 9)]);
    }

    #[test]
    fn identifiers_are_words_inside_an_expression() {
        let diff = FileDiff::new("    a.pop_front()\n", "    a.pop_back()\n");
        let Row::Line {
            left: Some(l),
            right: Some(r),
            ..
        } = &diff.rows[1]
        else {
            panic!("{:?}", diff.rows);
        };
        assert_eq!(l.changed, [(6, 15)]);
        assert_eq!(r.changed, [(6, 14)]);
    }

    #[test]
    fn a_rewritten_line_marks_no_words() {
        let diff = FileDiff::new("alpha beta gamma\n", "one two three four\n");
        let Row::Line { left: Some(l), .. } = &diff.rows[1] else {
            panic!();
        };
        assert!(l.changed.is_empty());
    }

    #[test]
    fn moved_lines_are_recognised() {
        let old = "fn first() {}\na\nb\nc\nd\ne\nf\ng\nh\ni\nfn second() {}\n";
        let new = "fn second() {}\na\nb\nc\nd\ne\nf\ng\nh\ni\nfn first() {}\n";
        let diff = FileDiff::new(old, new);
        let moved = diff
            .rows
            .iter()
            .filter(|row| matches!(row, Row::Line { left: Some(s), .. } if s.moved))
            .count();
        assert_eq!(moved, 2, "{:?}", diff.rows);
    }

    #[test]
    fn risky_files_come_first() {
        let change = |p: &str| Change {
            path: p.into(),
            status: 'M',
        };
        let mut files = [
            change("Cargo.lock"),
            change("README.md"),
            change("src/app.rs"),
            change("src/auth.rs"),
        ];
        rank_changes(&mut files);
        let names: Vec<String> = files.iter().map(|c| c.path.display().to_string()).collect();
        assert_eq!(
            names,
            ["src/auth.rs", "src/app.rs", "README.md", "Cargo.lock"]
        );
    }

    #[test]
    fn a_new_file_is_all_added() {
        let diff = FileDiff::new("", "x\ny\n");
        assert!(lines(&diff).iter().all(|l| l.2 == RowKind::Added));
        assert_eq!(reject_hunk("", "x\ny\n", &diff.hunks[0]), "");
    }

    #[test]
    fn stat_counts_lines() {
        assert_eq!(FileDiff::stat("a\nb\n", "a\nc\nd\n"), (2, 1));
    }
}
