//! Prose being typed into a box, as a comment is: a caret that moves by letter, word and
//! line, and the deletes a Mac or a shell has taught everyone, Option+Backspace among them.

use cellix_input::{Command, Direction, Key, KeyEvent, simple_keymap};
use unicode_segmentation::UnicodeSegmentation;

/// Applies a key to `text` with its caret at byte `caret`. Returns whether the key meant
/// anything here; Enter and Esc are the box's own to handle.
pub fn key(text: &mut String, caret: &mut usize, key: KeyEvent) -> bool {
    *caret = on_boundary(text, *caret);
    let mods = key.mods;
    match key.key {
        // As in a shell. Elsewhere Ctrl+W closes the tab, but not while writing.
        _ if takes_word(key) => {
            let to = word_left(text, *caret);
            delete(text, caret, to);
            return true;
        }
        // Option+Backspace, however the terminal sends it.
        Key::Backspace if mods.alt || mods.ctrl => {
            let to = word_left(text, *caret);
            delete(text, caret, to);
            return true;
        }
        Key::Char(c) if !mods.ctrl && !mods.alt => {
            text.insert(*caret, c);
            *caret += c.len_utf8();
            return true;
        }
        _ => {}
    }
    let Some(command) = simple_keymap(key) else {
        return false;
    };
    let at = *caret;
    match command {
        Command::Move(direction, _) => {
            *caret = match direction {
                Direction::Left => previous(text, at),
                Direction::Right => next(text, at),
                Direction::WordLeft => word_left(text, at),
                Direction::WordRight => word_right(text, at),
                Direction::LineStart | Direction::LineColumnZero => line_start(text, at),
                Direction::LineEnd => line_end(text, at),
                Direction::Up => vertical(text, at, false),
                Direction::Down => vertical(text, at, true),
                Direction::Top | Direction::PageUp => 0,
                Direction::Bottom | Direction::PageDown => text.len(),
                _ => at,
            };
        }
        // There's no selection in a box this small, so Ctrl+A goes to the line's start, as
        // it does in a Mac text field.
        Command::SelectAll => *caret = line_start(text, at),
        Command::DeleteBackward => delete(text, caret, previous(text, at)),
        Command::DeleteForward => delete(text, caret, next(text, at)),
        Command::DeleteWordLeft => delete(text, caret, word_left(text, at)),
        Command::DeleteWordRight => delete(text, caret, word_right(text, at)),
        Command::DeleteToLineStart => delete(text, caret, line_start(text, at)),
        Command::DeleteToLineEnd => delete(text, caret, line_end(text, at)),
        _ => return false,
    }
    true
}

/// Whether a key is Ctrl+W, which a box takes as a shell does, for the word before the caret,
/// however else it's bound.
pub fn takes_word(key: KeyEvent) -> bool {
    matches!(key.key, Key::Char('w' | 'W')) && key.mods.ctrl
}

/// Puts `inserted` in at the caret and moves the caret past it.
pub fn insert(text: &mut String, caret: &mut usize, inserted: &str) {
    *caret = on_boundary(text, *caret);
    text.insert_str(*caret, inserted);
    *caret += inserted.len();
}

/// Where a caret kept as `back`, how much of the text stands after it, is: in bytes from
/// the text's start. A box one line tall keeps its caret that way, so one opened with words
/// in it, or given them afresh, has its caret after them; and one given fewer words than
/// stood after its caret has it after them too.
pub fn caret_of(text: &str, back: usize) -> usize {
    match text.len().checked_sub(back) {
        Some(at) => on_boundary(text, at),
        None => text.len(),
    }
}

/// `key`, for a box one line tall that keeps its caret as `back` (`caret_of`). Up, Down and
/// the page keys aren't its own: they step through what it finds, or move what it's over.
pub fn line_key(text: &mut String, back: &mut usize, key: KeyEvent) -> bool {
    if matches!(key.key, Key::Up | Key::Down | Key::PageUp | Key::PageDown) {
        return false;
    }
    let mut caret = caret_of(text, *back);
    let taken = self::key(text, &mut caret, key);
    *back = text.len() - caret;
    taken
}

/// `insert`, for a box that keeps its caret as `back`.
pub fn line_insert(text: &mut String, back: &mut usize, inserted: &str) {
    let mut caret = caret_of(text, *back);
    insert(text, &mut caret, inserted);
    *back = text.len() - caret;
}

/// Puts a caret kept as `back` at byte `caret` of the text, as a press on a letter does.
pub fn set_caret(text: &str, back: &mut usize, caret: usize) {
    *back = text.len() - on_boundary(text, caret);
}

/// Removes what's between the caret and `to`, either side of it, leaving the caret at the
/// start of the gap.
fn delete(text: &mut String, caret: &mut usize, to: usize) {
    let (start, end) = ((*caret).min(to), (*caret).max(to));
    text.replace_range(start..end, "");
    *caret = start;
}

fn on_boundary(text: &str, at: usize) -> usize {
    let mut at = at.min(text.len());
    while !text.is_char_boundary(at) {
        at -= 1;
    }
    at
}

/// The start of what reads as one letter before `at`: an emoji built of several code points,
/// a flag or a letter with its accent, is one press of Backspace or the arrow.
fn previous(text: &str, at: usize) -> usize {
    text[..at]
        .grapheme_indices(true)
        .next_back()
        .map_or(0, |(i, _)| i)
}

fn next(text: &str, at: usize) -> usize {
    text[at..]
        .graphemes(true)
        .next()
        .map_or(at, |letter| at + letter.len())
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '\''
}

/// The start of the word before the caret, past any spaces or marks between.
fn word_left(text: &str, at: usize) -> usize {
    let mut i = at;
    let before = |i: usize| text[..i].chars().next_back();
    while let Some(c) = before(i).filter(|c| !is_word(*c)) {
        i -= c.len_utf8();
    }
    while let Some(c) = before(i).filter(|c| is_word(*c)) {
        i -= c.len_utf8();
    }
    i
}

/// The end of the word after the caret, past any spaces or marks between.
fn word_right(text: &str, at: usize) -> usize {
    let mut i = at;
    let after = |i: usize| text[i..].chars().next();
    while let Some(c) = after(i).filter(|c| !is_word(*c)) {
        i += c.len_utf8();
    }
    while let Some(c) = after(i).filter(|c| is_word(*c)) {
        i += c.len_utf8();
    }
    i
}

fn line_start(text: &str, at: usize) -> usize {
    text[..at].rfind('\n').map_or(0, |i| i + 1)
}

fn line_end(text: &str, at: usize) -> usize {
    text[at..].find('\n').map_or(text.len(), |i| at + i)
}

/// The same column on the line above or below, or its end when it's shorter.
fn vertical(text: &str, at: usize, down: bool) -> usize {
    let start = line_start(text, at);
    let column = text[start..at].chars().count();
    let target = if down {
        let end = line_end(text, at);
        if end == text.len() {
            return text.len();
        }
        end + 1
    } else {
        if start == 0 {
            return 0;
        }
        line_start(text, start - 1)
    };
    let end = line_end(text, target);
    text[target..end]
        .char_indices()
        .nth(column)
        .map_or(end, |(i, _)| target + i)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cellix_input::Mods;

    fn press(text: &mut String, caret: &mut usize, key: Key, mods: Mods) {
        super::key(text, caret, KeyEvent { key, mods });
    }

    fn typed(text: &mut String, caret: &mut usize, words: &str) {
        for c in words.chars() {
            press(text, caret, Key::Char(c), Mods::default());
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

    #[test]
    fn option_backspace_and_ctrl_w_take_a_word() {
        let (mut text, mut caret) = (String::new(), 0);
        typed(&mut text, &mut caret, "fix the cache ");
        press(&mut text, &mut caret, Key::Backspace, ALT);
        assert_eq!(text, "fix the ");
        press(&mut text, &mut caret, Key::Char('w'), CTRL);
        assert_eq!(text, "fix ");
        // Cmd+Backspace arrives as Ctrl+U, and takes the line.
        press(&mut text, &mut caret, Key::Char('u'), CTRL);
        assert_eq!((text.as_str(), caret), ("", 0));
    }

    #[test]
    fn the_caret_moves_and_typing_goes_in_where_it_is() {
        let (mut text, mut caret) = (String::new(), 0);
        typed(&mut text, &mut caret, "one three");
        // Option+Left, as the terminal sends it.
        press(&mut text, &mut caret, Key::Char('b'), ALT);
        assert_eq!(caret, 4);
        typed(&mut text, &mut caret, "two ");
        assert_eq!(text, "one two three");
        press(&mut text, &mut caret, Key::Home, Mods::default());
        assert_eq!(caret, 0);
        press(&mut text, &mut caret, Key::Right, ALT);
        assert_eq!(caret, 3);
        press(&mut text, &mut caret, Key::Delete, ALT);
        assert_eq!(text, "one three");
        press(&mut text, &mut caret, Key::End, Mods::default());
        assert_eq!(caret, text.len());
        press(&mut text, &mut caret, Key::Left, Mods::default());
        press(&mut text, &mut caret, Key::Backspace, Mods::default());
        assert_eq!(text, "one thre");
    }

    #[test]
    fn a_box_of_one_line_keeps_its_caret_from_its_end() {
        let (mut text, mut back) = ("pricing page".to_string(), 0);
        let press = |text: &mut String, back: &mut usize, key, mods| {
            line_key(text, back, KeyEvent { key, mods })
        };
        press(&mut text, &mut back, Key::Left, ALT);
        assert_eq!(caret_of(&text, back), 8, "before \"page\"");
        line_insert(&mut text, &mut back, "the ");
        assert_eq!((text.as_str(), back), ("pricing the page", 4));
        // Up and Down are the list's, or the matches', not the box's.
        assert!(!press(&mut text, &mut back, Key::Up, Mods::default()));
        assert_eq!(back, 4);
        // Words given afresh keep the caret as far from their end.
        text = "billing".to_string();
        assert_eq!(caret_of(&text, back), 3);
        set_caret(&text, &mut back, 0);
        assert_eq!(back, 7);
        press(&mut text, &mut back, Key::Delete, Mods::default());
        assert_eq!((text.as_str(), caret_of(&text, back)), ("illing", 0));
    }

    #[test]
    fn up_and_down_keep_the_column_and_letters_stay_whole() {
        let mut text = "first line\nab\nlast line".to_string();
        let mut caret = 8;
        press(&mut text, &mut caret, Key::Down, Mods::default());
        assert_eq!(caret, 13, "the end of the shorter line");
        press(&mut text, &mut caret, Key::Down, Mods::default());
        assert_eq!(&text[caret..], "st line", "column 2 of the last");
        let (mut text, mut caret) = ("café".to_string(), 5);
        press(&mut text, &mut caret, Key::Backspace, Mods::default());
        assert_eq!(text, "caf");
    }
}
