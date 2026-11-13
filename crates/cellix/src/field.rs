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
        Key::Char('w' | 'W') if mods.ctrl => {
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

/// Puts `inserted` in at the caret and moves the caret past it.
pub fn insert(text: &mut String, caret: &mut usize, inserted: &str) {
    *caret = on_boundary(text, *caret);
    text.insert_str(*caret, inserted);
    *caret += inserted.len();
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
