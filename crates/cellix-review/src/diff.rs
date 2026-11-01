use similar::{ChangeTag, DiffOp, TextDiff};

const CONTEXT_LINES: usize = 3;

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
                            out.rows.push(Row::Line { left, right, kind });
                        }
                    }
                }
            }
        }
        out
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

#[cfg(test)]
mod tests {
    use super::*;

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
