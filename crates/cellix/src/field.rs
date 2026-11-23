//! Prose being typed into a box, as a comment is. Every box in the app is a `TextBox`: the
//! file's own buffer and selections (`cellix_core::Buffer`, `Selections`), so a box and the
//! file agree on what a letter, a word, a line and a selection are, and a key does in a box
//! what it does in the file: Shift with a move selects, typing replaces what's selected,
//! Ctrl+Z undoes as the file groups it, and the deletes a Mac or a shell has taught work.

use std::fmt;
use std::hash::{Hash, Hasher};
use std::ops::Deref;

use cellix_core::{Buffer, Motion, Selection, Selections};
use cellix_input::{Command, Direction, Key, KeyEvent, simple_keymap};

/// The file's motion for a direction, `page` lines to a page. The file and every box move
/// through this one table.
pub fn motion(direction: Direction, page: usize) -> Motion {
    match direction {
        Direction::Left => Motion::Left,
        Direction::Right => Motion::Right,
        Direction::Up => Motion::Up,
        Direction::Down => Motion::Down,
        Direction::WordLeft => Motion::WordLeft,
        Direction::WordRight => Motion::WordRight,
        Direction::LineStart => Motion::LineStart,
        Direction::LineColumnZero => Motion::LineColumnZero,
        Direction::LineEnd => Motion::LineEnd,
        Direction::Top => Motion::Top,
        Direction::Bottom => Motion::Bottom,
        Direction::PageUp => Motion::PageUp(page),
        Direction::PageDown => Motion::PageDown(page),
        Direction::NextWordStart => Motion::NextWordStart,
        Direction::FindChar(c, forward, till) => Motion::FindChar(c, forward, till),
    }
}

/// How many lines a page is in a box: a box shows a few rows, so Page Up and Down go most
/// of the way through what's written, as they do through a short file.
const BOX_PAGE: usize = 8;

/// Text being written in a box, with a caret or a selection and the file's undo.
#[derive(Clone, Default)]
pub struct TextBox {
    /// The buffer's text, kept beside it so a box reads as the `str` it holds.
    text: String,
    buffer: Buffer,
    selections: Selections,
}

impl TextBox {
    /// A box holding `text`, its caret after it.
    pub fn new(text: impl Into<String>) -> Self {
        let text = text.into();
        let buffer = Buffer::from_text(&text);
        let end = buffer.rope().len_chars();
        Self {
            text,
            buffer,
            selections: Selections::single(Selection::cursor(end)),
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    /// Gives the box words afresh, as a list's history or a prefix changing does: the caret
    /// keeps as far from their end as it stood, so one at the end stays there, nothing is
    /// selected, and what was typed before can't be undone back into it.
    pub fn set(&mut self, text: impl Into<String>) {
        let text = text.into();
        if text == self.text {
            return;
        }
        let back = self.back();
        *self = Self::new(text);
        self.set_back(back);
    }

    pub fn clear(&mut self) {
        self.set(String::new());
    }

    /// Where the caret is, in bytes: the selection's head, where it's being made from.
    pub fn caret(&self) -> usize {
        self.byte_of(self.selections.primary().head)
    }

    /// How much of the text stands after the caret, in bytes.
    pub fn back(&self) -> usize {
        self.text.len() - self.caret()
    }

    /// Puts the caret that many bytes from the end, nothing selected.
    pub fn set_back(&mut self, back: usize) {
        let at = self.text.len().saturating_sub(back);
        self.set_caret(at);
    }

    /// What's selected, as bytes from its start to its end, when anything is.
    pub fn selection(&self) -> Option<(usize, usize)> {
        let s = self.selections.primary();
        (!s.is_empty()).then(|| (self.byte_of(s.start()), self.byte_of(s.end())))
    }

    /// Where the selection is anchored and where its head is, in bytes.
    pub fn ends(&self) -> (usize, usize) {
        let s = self.selections.primary();
        (self.byte_of(s.anchor), self.byte_of(s.head))
    }

    pub fn selected(&self) -> Option<&str> {
        self.selection().map(|(start, end)| &self.text[start..end])
    }

    /// Puts the caret at byte `at`, nothing selected, as a press on a letter does.
    pub fn set_caret(&mut self, at: usize) {
        let at = self.char_of(at);
        self.selections.set_single(Selection::cursor(at));
    }

    /// Selects from byte `anchor` to byte `head`, the caret at the head.
    pub fn select(&mut self, anchor: usize, head: usize) {
        let (anchor, head) = (self.char_of(anchor), self.char_of(head));
        self.selections.set_single(Selection {
            anchor,
            head,
            goal: None,
        });
    }

    /// Moves the caret to byte `at`, keeping where the selection was anchored: Shift+click,
    /// or a drag.
    pub fn extend_to(&mut self, at: usize) {
        let anchor = self.selections.primary().anchor;
        let head = self.char_of(at);
        self.selections.set_single(Selection {
            anchor,
            head,
            goal: None,
        });
    }

    /// Selects the word at byte `at`, as a double click does, by the file's idea of a word.
    pub fn select_word_at(&mut self, at: usize) {
        let (start, end) = self.buffer.word_at(self.char_of(at));
        self.selections.set_single(Selection {
            anchor: start,
            head: end,
            goal: None,
        });
    }

    /// Selects the line at byte `at`, as a triple click does: in a box of one line, all of it.
    pub fn select_line_at(&mut self, at: usize) {
        let rope = self.buffer.rope();
        let line = rope.char_to_line(self.char_of(at));
        let start = rope.line_to_char(line);
        let end = if line + 1 < rope.len_lines() {
            rope.line_to_char(line + 1)
        } else {
            rope.len_chars()
        };
        self.selections.set_single(Selection {
            anchor: start,
            head: end,
            goal: None,
        });
    }

    /// Puts `inserted` in place of what's selected, or at the caret, and the caret after it:
    /// a paste, or words a list put in. One step of undo.
    pub fn insert(&mut self, inserted: &str) {
        self.buffer.replace(&mut self.selections, inserted);
        self.sync();
    }

    /// Types a letter, over what's selected, grouped for undo with the letters around it as
    /// the file groups them.
    pub fn type_char(&mut self, c: char) {
        self.buffer
            .type_text(&mut self.selections, c.encode_utf8(&mut [0; 4]));
        self.sync();
    }

    /// Replaces bytes `start..end` with `text`, the caret after it, as one step of undo.
    pub fn replace(&mut self, start: usize, end: usize, text: &str) {
        let (start, end) = (self.char_of(start), self.char_of(end));
        self.buffer
            .replace_range(&mut self.selections, start, end, text);
        self.selections
            .set_single(Selection::cursor(start + text.chars().count()));
        self.sync();
    }

    /// Takes away what's selected. False when nothing was.
    pub fn delete_selection(&mut self) -> bool {
        if self.selection().is_none() {
            return false;
        }
        self.buffer.replace(&mut self.selections, "");
        self.sync();
        true
    }

    /// Applies a key. Returns whether it meant anything here; Enter, Esc and Tab are the
    /// box's own to handle, and copying and pasting are the app's, which has the clipboard.
    pub fn key(&mut self, key: KeyEvent) -> bool {
        self.key_as(key, simple_keymap(key))
    }

    /// `key`, with the command the app's keymap makes of it.
    pub fn key_as(&mut self, key: KeyEvent, command: Option<Command>) -> bool {
        let mods = key.mods;
        match key.key {
            // As in a shell. Elsewhere Ctrl+W closes the tab, but not while writing.
            _ if takes_word(key) => return self.run(Command::DeleteWordLeft),
            // Option+Backspace, however the terminal sends it.
            Key::Backspace if mods.alt || mods.ctrl => {
                return self.run(Command::DeleteWordLeft);
            }
            Key::Char(c) if !mods.ctrl && !mods.alt => {
                self.type_char(c);
                return true;
            }
            Key::Enter | Key::Esc | Key::Tab | Key::BackTab => return false,
            _ => {}
        }
        command.is_some_and(|command| self.run(command))
    }

    /// `key` for a box one line tall: Up, Down and the page keys aren't its own, they step
    /// through what it finds, or move what it's over.
    pub fn line_key(&mut self, key: KeyEvent) -> bool {
        if matches!(key.key, Key::Up | Key::Down | Key::PageUp | Key::PageDown) {
            return false;
        }
        self.key(key)
    }

    /// Runs one of the file's editing commands on the box, as the file runs it. False for
    /// what isn't editing.
    pub fn run(&mut self, command: Command) -> bool {
        match command {
            Command::Move(direction, extend) => {
                let motion = motion(direction, BOX_PAGE);
                self.selections
                    .apply_motion(self.buffer.rope(), motion, extend);
            }
            Command::Type(c) => self.type_char(c),
            Command::Newline => {
                self.buffer.type_text(&mut self.selections, "\n");
                self.sync();
            }
            Command::SelectAll => {
                let len = self.buffer.rope().len_chars();
                self.selections.set_single(Selection {
                    anchor: 0,
                    head: len,
                    goal: None,
                });
            }
            Command::DeleteBackward => {
                self.buffer.delete_backward(&mut self.selections);
                self.sync();
            }
            Command::DeleteForward => {
                self.buffer.delete_forward(&mut self.selections);
                self.sync();
            }
            Command::DeleteWordLeft => self.delete_to(Motion::WordLeft),
            Command::DeleteWordRight => self.delete_to(Motion::WordRight),
            Command::DeleteToLineStart => self.delete_to(Motion::LineColumnZero),
            Command::DeleteToLineEnd => self.delete_to(Motion::LineEnd),
            Command::Undo => {
                self.buffer.undo(&mut self.selections);
                self.sync();
            }
            Command::Redo => {
                self.buffer.redo(&mut self.selections);
                self.sync();
            }
            // What's selected goes, the caret staying where the selection's head was.
            Command::Cancel => {
                let head = self.selections.primary().head;
                self.selections.set_single(Selection::cursor(head));
            }
            _ => return false,
        }
        true
    }

    /// Deletes what's selected, or from the caret to where `motion` takes it, as the file's
    /// word and line deletes do.
    fn delete_to(&mut self, motion: Motion) {
        let primary = self.selections.primary();
        let (start, end) = if primary.is_empty() {
            let mut moved = self.selections.clone();
            moved.apply_motion(self.buffer.rope(), motion, false);
            let target = moved.primary().head;
            (target.min(primary.head), target.max(primary.head))
        } else {
            (primary.start(), primary.end())
        };
        self.buffer
            .replace_range(&mut self.selections, start, end, "");
        self.selections.set_single(Selection::cursor(start));
        self.sync();
    }

    fn sync(&mut self) {
        self.text = self.buffer.text();
    }

    /// The byte where char `at` starts.
    fn byte_of(&self, at: usize) -> usize {
        let rope = self.buffer.rope();
        rope.char_to_byte(at.min(rope.len_chars()))
    }

    /// The char a byte falls in, a byte inside a letter counting as the letter's start.
    fn char_of(&self, at: usize) -> usize {
        let mut at = at.min(self.text.len());
        while !self.text.is_char_boundary(at) {
            at -= 1;
        }
        self.buffer.rope().byte_to_char(at)
    }
}

impl Deref for TextBox {
    type Target = str;

    fn deref(&self) -> &str {
        &self.text
    }
}

impl AsRef<str> for TextBox {
    fn as_ref(&self) -> &str {
        &self.text
    }
}

impl From<String> for TextBox {
    fn from(text: String) -> Self {
        Self::new(text)
    }
}

impl From<&str> for TextBox {
    fn from(text: &str) -> Self {
        Self::new(text)
    }
}

impl fmt::Display for TextBox {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.text)
    }
}

impl fmt::Debug for TextBox {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.text, f)
    }
}

/// Two boxes are the same when their words are, as two strings are: where the caret is, and
/// the undo behind them, aren't what they hold. What draws a box hashes its `ends` as well.
impl PartialEq for TextBox {
    fn eq(&self, other: &Self) -> bool {
        self.text == other.text
    }
}

impl Eq for TextBox {}

impl Hash for TextBox {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.text.hash(state);
    }
}

impl PartialEq<str> for TextBox {
    fn eq(&self, other: &str) -> bool {
        self.text == other
    }
}

impl PartialEq<&str> for TextBox {
    fn eq(&self, other: &&str) -> bool {
        self.text == *other
    }
}

impl PartialEq<String> for TextBox {
    fn eq(&self, other: &String) -> bool {
        self.text == *other
    }
}

impl PartialEq<TextBox> for &str {
    fn eq(&self, other: &TextBox) -> bool {
        *self == other.text
    }
}

impl PartialEq<TextBox> for String {
    fn eq(&self, other: &TextBox) -> bool {
        *self == other.text
    }
}

/// Whether a key is Ctrl+W, which a box takes as a shell does, for the word before the caret,
/// however else it's bound.
pub fn takes_word(key: KeyEvent) -> bool {
    matches!(key.key, Key::Char('w' | 'W')) && key.mods.ctrl
}

#[cfg(test)]
mod tests {
    use super::*;
    use cellix_input::Mods;

    fn press(field: &mut TextBox, key: Key, mods: Mods) {
        field.key(KeyEvent { key, mods });
    }

    fn typed(field: &mut TextBox, words: &str) {
        for c in words.chars() {
            press(field, Key::Char(c), Mods::default());
        }
    }

    const ALT: Mods = Mods {
        alt: true,
        ctrl: false,
        shift: false,
    };
    const CTRL: Mods = Mods {
        alt: false,
        ctrl: true,
        shift: false,
    };
    const SHIFT: Mods = Mods {
        alt: false,
        ctrl: false,
        shift: true,
    };

    #[test]
    fn option_backspace_and_ctrl_w_take_a_word() {
        let mut field = TextBox::default();
        typed(&mut field, "fix the cache ");
        press(&mut field, Key::Backspace, ALT);
        assert_eq!(field, "fix the ");
        press(&mut field, Key::Char('w'), CTRL);
        assert_eq!(field, "fix ");
        // Cmd+Backspace arrives as Ctrl+U, and takes the line.
        press(&mut field, Key::Char('u'), CTRL);
        assert_eq!((field.text(), field.caret()), ("", 0));
    }

    #[test]
    fn the_caret_moves_and_typing_goes_in_where_it_is() {
        let mut field = TextBox::default();
        typed(&mut field, "one three");
        // Option+Left, as the terminal sends it.
        press(&mut field, Key::Char('b'), ALT);
        assert_eq!(field.caret(), 4);
        typed(&mut field, "two ");
        assert_eq!(field, "one two three");
        press(&mut field, Key::Home, Mods::default());
        assert_eq!(field.caret(), 0);
        press(&mut field, Key::Right, ALT);
        assert_eq!(field.caret(), 3);
        press(&mut field, Key::Delete, ALT);
        assert_eq!(field, "one three");
        press(&mut field, Key::End, Mods::default());
        assert_eq!(field.caret(), field.len());
        press(&mut field, Key::Left, Mods::default());
        press(&mut field, Key::Backspace, Mods::default());
        assert_eq!(field, "one thre");
    }

    #[test]
    fn a_box_of_one_line_keeps_its_caret_from_its_end() {
        let mut field = TextBox::new("pricing page");
        let press = |field: &mut TextBox, key, mods| field.line_key(KeyEvent { key, mods });
        press(&mut field, Key::Left, ALT);
        assert_eq!(field.caret(), 8, "before \"page\"");
        field.insert("the ");
        assert_eq!((field.text(), field.back()), ("pricing the page", 4));
        // Up and Down are the list's, or the matches', not the box's.
        assert!(!press(&mut field, Key::Up, Mods::default()));
        assert_eq!(field.back(), 4);
        // Words given afresh keep the caret as far from their end.
        field.set("billing");
        assert_eq!(field.caret(), 3);
        field.set_caret(0);
        assert_eq!(field.back(), 7);
        press(&mut field, Key::Delete, Mods::default());
        assert_eq!((field.text(), field.caret()), ("illing", 0));
    }

    #[test]
    fn up_and_down_keep_the_column_and_letters_stay_whole() {
        let mut field = TextBox::new("first line\nab\nlast line");
        field.set_caret(8);
        press(&mut field, Key::Down, Mods::default());
        assert_eq!(field.caret(), 13, "the end of the shorter line");
        // As in the file, the column it set out from, past the shorter line: it went to the
        // shorter line's column until the box moved as the file does.
        press(&mut field, Key::Down, Mods::default());
        assert_eq!(&field[field.caret()..], "e", "column 8 of the last");
        let mut field = TextBox::new("café");
        field.set_caret(5);
        press(&mut field, Key::Backspace, Mods::default());
        assert_eq!(field, "caf");
    }

    /// The box selects as the file does: Shift with a move, Ctrl+A for all of it; what's
    /// typed replaces the selection, and undo takes back a word's typing at a time.
    #[test]
    fn shift_selects_typing_replaces_it_and_undo_goes_back_by_words() {
        let mut field = TextBox::default();
        typed(&mut field, "fix the cache");
        press(&mut field, Key::Left, Mods { shift: true, ..ALT });
        assert_eq!(field.selected(), Some("cache"));
        typed(&mut field, "queue");
        assert_eq!(field, "fix the queue");
        press(&mut field, Key::Home, SHIFT);
        assert_eq!(field.selected(), Some("fix the queue"));
        press(&mut field, Key::Right, Mods::default());
        assert_eq!((field.selection(), field.caret()), (None, 13), "to its end");
        press(&mut field, Key::Char('a'), CTRL);
        assert_eq!(field.selection(), Some((0, 13)));
        press(&mut field, Key::Backspace, Mods::default());
        assert_eq!(field, "");
        press(&mut field, Key::Char('z'), CTRL);
        assert_eq!(field, "fix the queue");
        press(&mut field, Key::Char('z'), CTRL);
        assert_eq!(field, "fix the cache", "the word typed over the selection");
        press(&mut field, Key::Char('y'), CTRL);
        assert_eq!(field, "fix the queue");
    }
}
