//! The rows a code view shows: a line each, unless folds hide some lines or word wrap
//! breaks a long line over several rows.

use cellix_core::Rope;
use unicode_width::UnicodeWidthChar;

use crate::fold::Range;

/// One row on screen: chars `start..end` of a line, counted from the line's start.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CodeRow {
    pub line: usize,
    pub start: usize,
    /// `usize::MAX` for the rest of the line.
    pub end: usize,
    /// Columns of space before a wrapped line's later rows, so they keep its indent.
    pub indent: u16,
}

impl CodeRow {
    /// Whether this is the first row of its line.
    pub fn first(&self) -> bool {
        self.start == 0
    }
}

/// Every row of a file, or just its line count when each line is one row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Rows {
    Lines(usize),
    Laid {
        rows: Vec<CodeRow>,
        /// Each line's first row, or `usize::MAX` where a fold hides it.
        first: Vec<usize>,
    },
}

/// The display width of `c` at `column`, with tabs to the next stop.
pub fn width_of(c: char, column: usize, tab: usize) -> usize {
    if c == '\t' {
        tab - column % tab
    } else {
        c.width().unwrap_or(0)
    }
}

impl Rows {
    /// Lays out `rope` with `hidden` lines folded away and lines wrapped at `wrap` columns.
    pub fn lay_out(rope: &Rope, hidden: &[Range], wrap: Option<usize>, tab: usize) -> Self {
        let lines = rope.len_lines();
        if hidden.is_empty() && wrap.is_none() {
            return Self::Lines(lines);
        }
        let mut rows = Vec::with_capacity(lines);
        let mut first = vec![usize::MAX; lines];
        let mut folds = hidden.iter().peekable();
        let mut line = 0;
        while line < lines {
            first[line] = rows.len();
            match wrap {
                Some(width) => wrap_line(rope, line, width.max(1), tab, &mut rows),
                None => rows.push(CodeRow {
                    line,
                    start: 0,
                    end: usize::MAX,
                    indent: 0,
                }),
            }
            while folds.peek().is_some_and(|f| f.end < line) {
                folds.next();
            }
            line = match folds.peek() {
                Some(fold) if fold.start == line => fold.end + 1,
                _ => line + 1,
            };
        }
        Self::Laid { rows, first }
    }

    pub fn len(&self) -> usize {
        match self {
            Self::Lines(lines) => *lines,
            Self::Laid { rows, .. } => rows.len(),
        }
    }

    pub fn get(&self, index: usize) -> Option<CodeRow> {
        match self {
            Self::Lines(lines) => (index < *lines).then_some(CodeRow {
                line: index,
                start: 0,
                end: usize::MAX,
                indent: 0,
            }),
            Self::Laid { rows, .. } => rows.get(index).copied(),
        }
    }

    /// The row that shows char `column` of `line`, or the row a fold over it stands on.
    pub fn row_of(&self, line: usize, column: usize) -> usize {
        match self {
            Self::Lines(lines) => line.min(lines.saturating_sub(1)),
            Self::Laid { rows, first } => {
                let line = line.min(first.len().saturating_sub(1));
                // A hidden line shows as the fold's own row, the nearest shown line above.
                let shown = (0..=line).rev().find(|l| first[*l] != usize::MAX);
                let Some(shown) = shown else {
                    return 0;
                };
                let mut index = first[shown];
                if shown == line {
                    while rows
                        .get(index + 1)
                        .is_some_and(|r| r.line == line && r.start <= column)
                    {
                        index += 1;
                    }
                }
                index
            }
        }
    }

    /// Whether a line is on screen at all, rather than inside a fold.
    pub fn shows(&self, line: usize) -> bool {
        match self {
            Self::Lines(lines) => line < *lines,
            Self::Laid { first, .. } => first.get(line).is_some_and(|r| *r != usize::MAX),
        }
    }

    /// The rows of `line`, as a range of row numbers.
    pub fn rows_of(&self, line: usize) -> std::ops::Range<usize> {
        match self {
            Self::Lines(_) => line..line + 1,
            Self::Laid { rows, first } => {
                let Some(&start) = first.get(line).filter(|r| **r != usize::MAX) else {
                    return 0..0;
                };
                let end = rows[start..]
                    .iter()
                    .position(|r| r.line != line)
                    .map_or(rows.len(), |n| start + n);
                start..end
            }
        }
    }
}

/// Breaks one line into rows of at most `width` columns, after a space where there is one.
/// Later rows keep the line's indent, unless it would leave too little room.
fn wrap_line(rope: &Rope, line: usize, width: usize, tab: usize, rows: &mut Vec<CodeRow>) {
    let text = rope.line(line);
    let chars: Vec<char> = text
        .chars()
        .take_while(|c| *c != '\n' && *c != '\r')
        .collect();
    let mut indent = 0;
    for c in &chars {
        if *c == ' ' || *c == '\t' {
            indent += width_of(*c, indent, tab);
        } else {
            break;
        }
    }
    let indent = if indent * 2 > width { 0 } else { indent };
    let mut start = 0;
    let mut column = 0;
    let mut room = width;
    let mut space = None;
    let mut used = 0;
    let mut at = 0;
    while at < chars.len() {
        let c = chars[at];
        let w = width_of(c, column, tab);
        if used + w > room && at > start {
            // Break after the last space, unless that leaves the row empty of words.
            let cut = space.filter(|s| *s > start).unwrap_or(at);
            rows.push(CodeRow {
                line,
                start,
                end: cut,
                indent: if start == 0 { 0 } else { indent as u16 },
            });
            start = cut;
            room = width - indent;
            used = 0;
            space = None;
            at = cut;
            continue;
        }
        used += w;
        column += w;
        if c == ' ' {
            space = Some(at + 1);
        }
        at += 1;
    }
    rows.push(CodeRow {
        line,
        start,
        end: usize::MAX,
        indent: if start == 0 { 0 } else { indent as u16 },
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(text: &str, hidden: &[Range], wrap: Option<usize>) -> Vec<(usize, usize, usize)> {
        let laid = Rows::lay_out(&Rope::from_str(text), hidden, wrap, 4);
        (0..laid.len())
            .map(|i| laid.get(i).unwrap())
            .map(|r| (r.line, r.start, r.end))
            .collect()
    }

    #[test]
    fn long_lines_wrap_after_a_space_and_keep_their_indent() {
        let laid = Rows::lay_out(&Rope::from_str("  aaa bbb ccc\nx\n"), &[], Some(8), 4);
        let got: Vec<CodeRow> = (0..laid.len()).map(|i| laid.get(i).unwrap()).collect();
        assert_eq!(got[0].end, 6, "after `aaa `");
        assert_eq!((got[1].start, got[1].indent), (6, 2));
        assert_eq!(
            (got[2].line, got[2].start),
            (0, 10),
            "`bbb ` fills the second"
        );
        assert_eq!(got[3].line, 1);
        assert_eq!(laid.row_of(0, 7), 1);
        assert_eq!(laid.row_of(1, 0), 3);
        assert_eq!(laid.rows_of(0), 0..3);
        // A word longer than the row breaks where it must.
        assert_eq!(rows("abcdefghij", &[], Some(4)).len(), 3);
    }

    #[test]
    fn folded_lines_are_left_out() {
        let fold = Range { start: 0, end: 2 };
        assert_eq!(
            rows("a {\n  b\n}\nc\n", &[fold], None),
            vec![(0, 0, usize::MAX), (3, 0, usize::MAX), (4, 0, usize::MAX)]
        );
        let laid = Rows::lay_out(&Rope::from_str("a {\n  b\n}\nc\n"), &[fold], None, 4);
        assert!(!laid.shows(1));
        assert_eq!(laid.row_of(1, 0), 0, "inside the fold is its header's row");
        assert_eq!(laid.row_of(3, 0), 1);
        assert_eq!(
            Rows::lay_out(&Rope::from_str("a\nb"), &[], None, 4),
            Rows::Lines(2)
        );
    }
}
