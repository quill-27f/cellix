use ropey::Rope;

/// A cursor, or a selected range when `anchor` differs from `head`. Positions are
/// char indices into the rope.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Selection {
    pub anchor: usize,
    pub head: usize,
    /// Column to return to when moving through shorter lines.
    pub goal: Option<usize>,
}

impl Selection {
    pub fn cursor(at: usize) -> Self {
        Self {
            anchor: at,
            head: at,
            goal: None,
        }
    }

    pub fn start(&self) -> usize {
        self.anchor.min(self.head)
    }

    pub fn end(&self) -> usize {
        self.anchor.max(self.head)
    }

    pub fn is_empty(&self) -> bool {
        self.anchor == self.head
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Motion {
    Left,
    Right,
    Up,
    Down,
    WordLeft,
    WordRight,
    LineStart,
    LineColumnZero,
    LineEnd,
    Top,
    Bottom,
    PageUp(usize),
    PageDown(usize),
    /// The start of the next word, as Vim's `w`.
    NextWordStart,
    /// To a character on the same line: forward or back, and stopping just before it.
    FindChar(char, bool, bool),
}

/// One or more selections, kept sorted and non-overlapping. The last one added is
/// primary: it is what the view follows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Selections {
    items: Vec<Selection>,
    primary: usize,
}

impl Default for Selections {
    fn default() -> Self {
        Self::single(Selection::cursor(0))
    }
}

impl Selections {
    pub fn single(selection: Selection) -> Self {
        Self {
            items: vec![selection],
            primary: 0,
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = &Selection> {
        self.items.iter()
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn primary(&self) -> Selection {
        self.items[self.primary]
    }

    pub fn set_single(&mut self, selection: Selection) {
        *self = Self::single(selection);
    }

    pub fn add(&mut self, selection: Selection) {
        self.items.push(selection);
        self.primary = self.items.len() - 1;
        self.normalize();
    }

    pub fn replace_all(&mut self, items: Vec<Selection>) {
        self.items = items;
        self.primary = self.items.len() - 1;
        self.normalize();
    }

    /// Moves each selection for `start..end` having become `inserted` chars long: places
    /// after it shift by the difference, and places inside it stay inside.
    pub(crate) fn shift(&mut self, (start, end): (usize, usize), inserted: usize) {
        let place = |at: usize| {
            if at >= end {
                at + inserted - (end - start)
            } else if at > start {
                start + (at - start).min(inserted)
            } else {
                at
            }
        };
        for s in &mut self.items {
            s.anchor = place(s.anchor);
            s.head = place(s.head);
        }
        self.normalize();
    }

    pub fn clamp(&mut self, len: usize) {
        for s in &mut self.items {
            s.anchor = s.anchor.min(len);
            s.head = s.head.min(len);
        }
        self.normalize();
    }

    fn normalize(&mut self) {
        let primary = self.items[self.primary];
        self.items.sort_by_key(Selection::start);
        self.items.dedup_by(|b, a| {
            if b.start() <= a.end() && (b.start() < a.end() || a.is_empty() && b.is_empty()) {
                let (start, end) = (a.start(), a.end().max(b.end()));
                *a = Selection {
                    anchor: start,
                    head: end,
                    goal: None,
                };
                true
            } else {
                false
            }
        });
        self.primary = self
            .items
            .iter()
            .position(|s| s.start() <= primary.head && primary.head <= s.end())
            .unwrap_or(self.items.len() - 1);
    }

    /// Moves every selection. With `extend`, anchors stay put.
    pub fn apply_motion(&mut self, rope: &Rope, motion: Motion, extend: bool) {
        for s in &mut self.items {
            let collapse_to = if !extend && !s.is_empty() {
                match motion {
                    Motion::Left => Some(s.start()),
                    Motion::Right => Some(s.end()),
                    _ => None,
                }
            } else {
                None
            };
            let vertical = matches!(
                motion,
                Motion::Up | Motion::Down | Motion::PageUp(_) | Motion::PageDown(_)
            );
            let goal = s.goal.unwrap_or_else(|| column(rope, s.head));
            let head = collapse_to.unwrap_or_else(|| step(rope, s.head, goal, motion));
            s.head = head;
            if !extend {
                s.anchor = head;
            }
            s.goal = vertical.then_some(goal);
        }
        self.normalize();
    }
}

fn column(rope: &Rope, at: usize) -> usize {
    at - rope.line_to_char(rope.char_to_line(at))
}

/// Length of a line without its line ending.
pub(crate) fn line_len(rope: &Rope, line: usize) -> usize {
    let slice = rope.line(line);
    let mut len = slice.len_chars();
    while len > 0 && matches!(slice.char(len - 1), '\n' | '\r') {
        len -= 1;
    }
    len
}

fn at_line(rope: &Rope, line: usize, goal: usize) -> usize {
    rope.line_to_char(line) + goal.min(line_len(rope, line))
}

fn step(rope: &Rope, at: usize, goal: usize, motion: Motion) -> usize {
    let line = rope.char_to_line(at);
    let last_line = rope.len_lines() - 1;
    match motion {
        Motion::Left => at.saturating_sub(1),
        Motion::Right => (at + 1).min(rope.len_chars()),
        Motion::Up if line == 0 => 0,
        Motion::Up => at_line(rope, line - 1, goal),
        Motion::Down if line == last_line => rope.len_chars(),
        Motion::Down => at_line(rope, line + 1, goal),
        Motion::PageUp(n) => at_line(rope, line.saturating_sub(n), goal),
        Motion::PageDown(n) => at_line(rope, (line + n).min(last_line), goal),
        Motion::LineStart => {
            // First press goes to the indent, the next to column zero.
            let start = rope.line_to_char(line);
            let indent = rope
                .line(line)
                .chars()
                .take_while(|c| *c == ' ' || *c == '\t')
                .count();
            if at == start + indent {
                start
            } else {
                start + indent
            }
        }
        Motion::LineColumnZero => rope.line_to_char(line),
        Motion::LineEnd => rope.line_to_char(line) + line_len(rope, line),
        Motion::Top => 0,
        Motion::Bottom => rope.len_chars(),
        Motion::WordLeft => {
            let mut i = at;
            while i > 0 && !is_word(rope.char(i - 1)) {
                i -= 1;
            }
            while i > 0 && is_word(rope.char(i - 1)) {
                i -= 1;
            }
            i
        }
        Motion::WordRight => {
            let len = rope.len_chars();
            let mut i = at;
            while i < len && !is_word(rope.char(i)) {
                i += 1;
            }
            while i < len && is_word(rope.char(i)) {
                i += 1;
            }
            i
        }
        Motion::NextWordStart => {
            let len = rope.len_chars();
            let mut i = at;
            while i < len && is_word(rope.char(i)) {
                i += 1;
            }
            while i < len && !is_word(rope.char(i)) && rope.char(i) != '\n' {
                i += 1;
            }
            i
        }
        Motion::FindChar(wanted, forward, till) => {
            let line = rope.char_to_line(at);
            let (start, end) = (rope.line_to_char(line), line_end(rope, line));
            let found = if forward {
                (at + 1..end).find(|&i| rope.char(i) == wanted)
            } else {
                (start..at).rev().find(|&i| rope.char(i) == wanted)
            };
            match (found, till) {
                (Some(i), false) => i,
                (Some(i), true) if forward => i - 1,
                (Some(i), true) => i + 1,
                (None, _) => at,
            }
        }
    }
}

/// The last character of a line, before its newline.
fn line_end(rope: &Rope, line: usize) -> usize {
    let next = rope.line_to_char((line + 1).min(rope.len_lines()));
    let has_newline = next > 0 && rope.char(next - 1) == '\n' && line + 1 < rope.len_lines();
    if has_newline { next - 1 } else { next }
}

pub(crate) fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_word_start_and_find_char_stay_where_vim_puts_them() {
        let rope = Rope::from_str("one two_three four\nnext\n");
        let mut s = Selections::single(Selection::cursor(0));
        s.apply_motion(&rope, Motion::NextWordStart, false);
        assert_eq!(s.primary().head, 4);
        s.apply_motion(&rope, Motion::FindChar('f', true, false), false);
        assert_eq!(s.primary().head, 14);
        s.apply_motion(&rope, Motion::FindChar('o', true, true), false);
        assert_eq!(s.primary().head, 14, "the next o is right beside it");
        s.apply_motion(&rope, Motion::FindChar('x', true, false), false);
        assert_eq!(s.primary().head, 14, "not on the line, so it stays");
        s.apply_motion(&rope, Motion::FindChar('t', false, false), false);
        assert_eq!(s.primary().head, 8);
    }

    #[test]
    fn vertical_motion_keeps_goal_column() {
        let rope = Rope::from_str("long line\nab\nanother long\n");
        let mut s = Selections::single(Selection::cursor(8));
        s.apply_motion(&rope, Motion::Down, false);
        assert_eq!(s.primary().head, 12);
        s.apply_motion(&rope, Motion::Down, false);
        assert_eq!(s.primary().head, 13 + 8);
    }

    #[test]
    fn overlapping_selections_merge() {
        let mut s = Selections::single(Selection {
            anchor: 0,
            head: 5,
            goal: None,
        });
        s.add(Selection {
            anchor: 3,
            head: 8,
            goal: None,
        });
        assert_eq!(s.len(), 1);
        assert_eq!((s.primary().start(), s.primary().end()), (0, 8));
    }
}
