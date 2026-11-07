use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use ropey::Rope;

use crate::selection::{Selection, Selections, is_word, line_len};

#[derive(Clone, Debug)]
struct Edit {
    at: usize,
    removed: String,
    inserted: String,
}

#[derive(Clone, Debug)]
struct Txn {
    edits: Vec<Edit>,
    before: Selections,
    after: Selections,
    typing: bool,
}

/// A text document with undo history and an optional file behind it.
#[derive(Debug, Default)]
pub struct Buffer {
    rope: Rope,
    path: Option<PathBuf>,
    undo: Vec<Txn>,
    redo: Vec<Txn>,
    /// History length at the last save, or None once that state is unreachable.
    saved_at: Option<usize>,
    disk_mtime: Option<SystemTime>,
    revision: u64,
}

impl Buffer {
    pub fn from_text(text: &str) -> Self {
        Self {
            rope: Rope::from_str(text),
            saved_at: Some(0),
            ..Self::default()
        }
    }

    pub fn open(path: &Path) -> io::Result<Self> {
        let mut buffer = Self::from_text(&fs::read_to_string(path)?);
        buffer.disk_mtime = fs::metadata(path)?.modified().ok();
        buffer.path = Some(path.to_path_buf());
        Ok(buffer)
    }

    /// Opens a file through something that reads it into other text, like a notebook.
    pub fn open_as(path: &Path, text: &str) -> io::Result<Self> {
        let mut buffer = Self::from_text(text);
        buffer.disk_mtime = fs::metadata(path)?.modified().ok();
        buffer.path = Some(path.to_path_buf());
        Ok(buffer)
    }

    pub fn rope(&self) -> &Rope {
        &self.rope
    }

    pub fn text(&self) -> String {
        self.rope.to_string()
    }

    /// Changes whenever the text does, so views can cache derived data.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    pub fn set_path(&mut self, path: PathBuf) {
        self.path = Some(path);
    }

    pub fn is_dirty(&self) -> bool {
        self.saved_at != Some(self.undo.len())
    }

    pub fn save(&mut self) -> io::Result<()> {
        self.save_as(&self.rope.to_string())
    }

    /// Saves `text` as what this buffer stands for on disk.
    pub fn save_as(&mut self, text: &str) -> io::Result<()> {
        let path = self
            .path
            .as_ref()
            .ok_or_else(|| io::Error::other("buffer has no file"))?;
        // Write beside the target and rename, so a crash can't truncate the file.
        let tmp = path.with_extension("cellix-tmp");
        fs::write(&tmp, text)?;
        if let Ok(meta) = fs::metadata(path) {
            fs::set_permissions(&tmp, meta.permissions())?;
        }
        fs::rename(&tmp, path)?;
        self.disk_mtime = fs::metadata(path)?.modified().ok();
        self.saved_at = Some(self.undo.len());
        Ok(())
    }

    /// True when the file on disk is newer than what this buffer loaded or saved.
    pub fn changed_on_disk(&self) -> bool {
        let Some(path) = &self.path else { return false };
        let on_disk = fs::metadata(path).and_then(|m| m.modified()).ok();
        on_disk.is_some() && on_disk != self.disk_mtime
    }

    /// Treats the file on disk as seen, so its change is reported once.
    pub fn acknowledge_disk(&mut self) {
        if let Some(path) = &self.path {
            self.disk_mtime = fs::metadata(path).and_then(|m| m.modified()).ok();
        }
    }

    /// Replaces the contents from disk. The reload itself can be undone.
    pub fn reload(&mut self, selections: &mut Selections) -> io::Result<()> {
        let path = self
            .path
            .clone()
            .ok_or_else(|| io::Error::other("buffer has no file"))?;
        let text = fs::read_to_string(&path)?;
        self.reload_as(selections, text)
    }

    /// Replaces the contents with what the file on disk comes to.
    pub fn reload_as(&mut self, selections: &mut Selections, text: String) -> io::Result<()> {
        let path = self
            .path
            .clone()
            .ok_or_else(|| io::Error::other("buffer has no file"))?;
        let before = selections.clone();
        self.push_edits(
            vec![Edit {
                at: 0,
                removed: self.rope.to_string(),
                inserted: String::new(),
            }],
            before.clone(),
        );
        self.rope = Rope::from_str(&text);
        self.revision += 1;
        let txn = self.undo.last_mut().expect("just pushed");
        txn.edits[0].inserted = text;
        selections.clamp(self.rope.len_chars());
        txn.after = selections.clone();
        self.saved_at = Some(self.undo.len());
        self.disk_mtime = fs::metadata(&path)?.modified().ok();
        Ok(())
    }

    /// Puts back edits that weren't saved, as one change that can be undone, so the buffer
    /// holds them and is unsaved again.
    pub fn restore_unsaved(&mut self, selections: &mut Selections, text: &str) {
        if self.rope == text {
            return;
        }
        let before = selections.clone();
        self.push_edits(
            vec![Edit {
                at: 0,
                removed: self.rope.to_string(),
                inserted: text.to_string(),
            }],
            before,
        );
        self.rope = Rope::from_str(text);
        self.revision += 1;
        selections.clamp(self.rope.len_chars());
        if let Some(txn) = self.undo.last_mut() {
            txn.after = selections.clone();
        }
    }

    fn push_edits(&mut self, edits: Vec<Edit>, before: Selections) {
        self.redo.clear();
        if self.saved_at.is_some_and(|at| at > self.undo.len()) {
            self.saved_at = None;
        }
        self.undo.push(Txn {
            edits,
            after: before.clone(),
            before,
            typing: false,
        });
    }

    /// Replaces each selection's range with what `text` returns for it.
    fn edit_each(
        &mut self,
        selections: &mut Selections,
        typing: bool,
        range: impl Fn(&Selection, &Rope) -> (usize, usize),
        text: impl Fn(usize) -> String,
    ) {
        self.revision += 1;
        let before = selections.clone();
        let mut edits = Vec::new();
        let mut placed = Vec::new();
        let mut shift = 0isize;
        for (i, s) in before.iter().enumerate() {
            let (start, end) = range(s, &self.rope);
            let at = (start as isize + shift) as usize;
            let removed: String = self.rope.slice(at..at + (end - start)).to_string();
            let inserted = text(i);
            self.rope.remove(at..at + (end - start));
            self.rope.insert(at, &inserted);
            let new_len = inserted.chars().count();
            placed.push(Selection::cursor(at + new_len));
            shift += new_len as isize - (end - start) as isize;
            edits.push(Edit {
                at,
                removed,
                inserted,
            });
        }
        selections.replace_all(placed);
        let merge = typing
            && self
                .undo
                .last()
                .is_some_and(|last| last.typing && last.after == before)
            && self.redo.is_empty()
            && self.saved_at != Some(self.undo.len());
        if merge {
            let last = self.undo.last_mut().expect("checked above");
            last.edits.extend(edits);
            last.after = selections.clone();
        } else {
            self.push_edits(edits, before);
            let last = self.undo.last_mut().expect("just pushed");
            last.after = selections.clone();
            last.typing = typing;
        }
    }

    /// Inserts typed text. Consecutive typing undoes as one step, except that a
    /// newline or space ends the current word's group.
    pub fn type_text(&mut self, selections: &mut Selections, text: &str) {
        let keeps_group = !text.contains(['\n', ' ']);
        let (owned, chars) = (text.to_string(), keeps_group);
        self.edit_each(
            selections,
            chars,
            |s, _| (s.start(), s.end()),
            |_| owned.clone(),
        );
    }

    /// Replaces each selection with `text`, as one undo step.
    pub fn replace(&mut self, selections: &mut Selections, text: &str) {
        let owned = text.to_string();
        self.edit_each(
            selections,
            false,
            |s, _| (s.start(), s.end()),
            |_| owned.clone(),
        );
    }

    /// Replaces each selection with its own text. `texts` must match their count.
    pub fn replace_each(&mut self, selections: &mut Selections, texts: &[String]) {
        assert_eq!(texts.len(), selections.len());
        self.edit_each(
            selections,
            false,
            |s, _| (s.start(), s.end()),
            |i| texts[i].clone(),
        );
    }

    pub fn delete_backward(&mut self, selections: &mut Selections) {
        self.edit_each(
            selections,
            false,
            |s, _| {
                if s.is_empty() {
                    (s.head.saturating_sub(1), s.head)
                } else {
                    (s.start(), s.end())
                }
            },
            |_| String::new(),
        );
    }

    pub fn delete_forward(&mut self, selections: &mut Selections) {
        self.edit_each(
            selections,
            false,
            |s, rope| {
                if s.is_empty() {
                    (s.head, (s.head + 1).min(rope.len_chars()))
                } else {
                    (s.start(), s.end())
                }
            },
            |_| String::new(),
        );
    }

    /// Replaces a char range directly, such as for an LSP rename or a rejected hunk.
    pub fn replace_range(
        &mut self,
        selections: &mut Selections,
        start: usize,
        end: usize,
        text: &str,
    ) {
        let owned = text.to_string();
        let one = Selections::single(Selection {
            anchor: start,
            head: end,
            goal: None,
        });
        let mut scratch = one;
        self.edit_each(
            &mut scratch,
            false,
            |s, _| (s.start(), s.end()),
            |_| owned.clone(),
        );
        self.undo.last_mut().expect("just pushed").before = selections.clone();
        selections.clamp(self.rope.len_chars());
        self.undo.last_mut().expect("just pushed").after = selections.clone();
    }

    /// Makes several changes as one undo step, as saving does to tidy a file. `edits` are
    /// (start, end, text) in chars of the text as it is, in order and apart. Selections move
    /// with the text around them.
    pub fn edit_ranges(&mut self, selections: &mut Selections, edits: &[(usize, usize, String)]) {
        if edits.is_empty() {
            return;
        }
        self.revision += 1;
        let before = selections.clone();
        let mut done = Vec::new();
        // From the end, so each range still means what it did.
        for (start, end, text) in edits.iter().rev() {
            let removed = self.rope.slice(*start..*end).to_string();
            self.rope.remove(*start..*end);
            self.rope.insert(*start, text);
            if start == end {
                selections.insert_after(*start, text.chars().count());
            } else {
                selections.shift((*start, *end), text.chars().count());
            }
            done.push(Edit {
                at: *start,
                removed,
                inserted: text.clone(),
            });
        }
        self.push_edits(done, before);
        self.undo.last_mut().expect("just pushed").after = selections.clone();
    }

    /// The char index of a line and column, clamped to the text.
    pub fn position(&self, line: usize, column: usize) -> usize {
        let line = line.min(self.rope.len_lines().saturating_sub(1));
        self.rope.line_to_char(line) + column.min(line_len(&self.rope, line))
    }

    /// Selects the word under a cursor, or the next match of the selected text. This is
    /// the Ctrl+D behaviour.
    pub fn select_next_match(&self, selections: &mut Selections) {
        let primary = selections.primary();
        if primary.is_empty() {
            let (start, end) = self.word_at(primary.head);
            if start < end {
                selections.set_single(Selection {
                    anchor: start,
                    head: end,
                    goal: None,
                });
            }
            return;
        }
        let needle = self.rope.slice(primary.start()..primary.end()).to_string();
        let haystack = self.rope.to_string();
        let byte_from = self.rope.char_to_byte(primary.end());
        let found = haystack[byte_from..]
            .find(&needle)
            .map(|i| i + byte_from)
            .or_else(|| haystack.find(&needle));
        if let Some(byte) = found {
            let start = self.rope.byte_to_char(byte);
            selections.add(Selection {
                anchor: start,
                head: start + needle.chars().count(),
                goal: None,
            });
        }
    }

    pub fn word_at(&self, at: usize) -> (usize, usize) {
        let len = self.rope.len_chars();
        let (mut start, mut end) = (at, at);
        while start > 0 && is_word(self.rope.char(start - 1)) {
            start -= 1;
        }
        while end < len && is_word(self.rope.char(end)) {
            end += 1;
        }
        (start, end)
    }

    /// Changes a char range as part of the last edit, so one undo takes back both: for what
    /// an edit sets off, as a numbered list counting again once an item goes. Selections
    /// after the range move with it. Refused, returning false, when the last edit isn't the
    /// newest state, as after an undo, or when it was saved.
    pub fn amend(
        &mut self,
        selections: &mut Selections,
        (start, end): (usize, usize),
        text: &str,
    ) -> bool {
        let saved = self.saved_at == Some(self.undo.len());
        if !self.redo.is_empty() || self.undo.is_empty() || saved {
            return false;
        }
        let removed = self.rope.slice(start..end).to_string();
        self.rope.remove(start..end);
        self.rope.insert(start, text);
        self.revision += 1;
        selections.shift((start, end), text.chars().count());
        let last = self.undo.last_mut().expect("checked above");
        last.edits.push(Edit {
            at: start,
            removed,
            inserted: text.to_string(),
        });
        last.after = selections.clone();
        last.typing = false;
        true
    }

    pub fn undo(&mut self, selections: &mut Selections) -> bool {
        let Some(txn) = self.undo.pop() else {
            return false;
        };
        self.revision += 1;
        for e in txn.edits.iter().rev() {
            self.rope.remove(e.at..e.at + e.inserted.chars().count());
            self.rope.insert(e.at, &e.removed);
        }
        *selections = txn.before.clone();
        selections.clamp(self.rope.len_chars());
        self.redo.push(txn);
        true
    }

    pub fn redo(&mut self, selections: &mut Selections) -> bool {
        let Some(txn) = self.redo.pop() else {
            return false;
        };
        self.revision += 1;
        for e in &txn.edits {
            self.rope.remove(e.at..e.at + e.removed.chars().count());
            self.rope.insert(e.at, &e.inserted);
        }
        *selections = txn.after.clone();
        selections.clamp(self.rope.len_chars());
        self.undo.push(txn);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(s: &Selections) -> usize {
        s.primary().head
    }

    #[test]
    fn undo_redo_round_trip() {
        let mut b = Buffer::from_text("hello");
        let mut s = Selections::single(Selection::cursor(5));
        b.replace(&mut s, " world");
        assert_eq!(b.text(), "hello world");
        b.delete_backward(&mut s);
        assert_eq!(b.text(), "hello worl");
        assert!(b.undo(&mut s));
        assert!(b.undo(&mut s));
        assert_eq!(b.text(), "hello");
        assert_eq!(at(&s), 5);
        assert!(!b.is_dirty());
        assert!(b.redo(&mut s));
        assert!(b.redo(&mut s));
        assert_eq!(b.text(), "hello worl");
        assert!(!b.redo(&mut s));
    }

    #[test]
    fn typing_groups_until_a_space() {
        let mut b = Buffer::from_text("");
        let mut s = Selections::default();
        for c in ["a", "b", " ", "c"] {
            b.type_text(&mut s, c);
        }
        assert_eq!(b.text(), "ab c");
        b.undo(&mut s);
        assert_eq!(b.text(), "ab ");
        b.undo(&mut s);
        assert_eq!(b.text(), "ab");
        b.undo(&mut s);
        assert_eq!(b.text(), "");
    }

    #[test]
    fn multicursor_types_at_each_cursor() {
        let mut b = Buffer::from_text("foo\nfoo\nfoo\n");
        let mut s = Selections::default();
        b.select_next_match(&mut s);
        b.select_next_match(&mut s);
        b.select_next_match(&mut s);
        assert_eq!(s.len(), 3);
        b.type_text(&mut s, "bar");
        assert_eq!(b.text(), "bar\nbar\nbar\n");
        b.undo(&mut s);
        assert_eq!(b.text(), "foo\nfoo\nfoo\n");
    }

    #[test]
    fn dirty_state_follows_save_point() {
        let mut b = Buffer::from_text("x");
        let mut s = Selections::default();
        assert!(!b.is_dirty());
        b.replace(&mut s, "y");
        assert!(b.is_dirty());
        b.undo(&mut s);
        assert!(!b.is_dirty());
    }

    #[test]
    fn multibyte_text_edits_by_char() {
        let mut b = Buffer::from_text("héllo");
        let mut s = Selections::single(Selection::cursor(2));
        b.delete_backward(&mut s);
        assert_eq!(b.text(), "hllo");
        b.undo(&mut s);
        assert_eq!(b.text(), "héllo");
    }

    #[test]
    fn an_amended_edit_undoes_as_one() {
        let mut buffer = Buffer::from_text("1. a\n2. b\n3. c\n");
        let mut selections = Selections::single(Selection {
            anchor: 5,
            head: 10,
            goal: None,
        });
        buffer.replace(&mut selections, "");
        assert_eq!(buffer.text(), "1. a\n3. c\n");
        assert!(buffer.amend(&mut selections, (5, 6), "2"));
        assert_eq!(buffer.text(), "1. a\n2. c\n");
        assert_eq!(
            selections.primary().head,
            5,
            "the caret stays at the line's start"
        );
        buffer.undo(&mut selections);
        assert_eq!(buffer.text(), "1. a\n2. b\n3. c\n");
        // After an undo the last edit isn't the newest, so nothing is added to it.
        assert!(!buffer.amend(&mut selections, (0, 1), "9"));
        buffer.redo(&mut selections);
        assert_eq!(buffer.text(), "1. a\n2. c\n");
    }
}
