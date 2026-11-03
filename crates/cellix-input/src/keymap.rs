use crate::{Command, Direction, Key, KeyEvent};

/// VS Code-style keys. Returns None for plain characters, which the view inserts.
pub fn simple_keymap(event: KeyEvent) -> Option<Command> {
    use Command::*;
    let KeyEvent { key, mods } = event;
    let shift = mods.shift;
    let word = mods.ctrl || mods.alt;
    Some(match key {
        Key::Left if word => Move(Direction::WordLeft, shift),
        Key::Right if word => Move(Direction::WordRight, shift),
        Key::Left => Move(Direction::Left, shift),
        Key::Right => Move(Direction::Right, shift),
        Key::Up if mods.ctrl && mods.alt => AddCursorAbove,
        Key::Down if mods.ctrl && mods.alt => AddCursorBelow,
        Key::Down if mods.alt && shift => DuplicateLine,
        Key::Up if mods.alt => MoveLineUp,
        Key::Down if mods.alt => MoveLineDown,
        Key::Up => Move(Direction::Up, shift),
        Key::Down => Move(Direction::Down, shift),
        Key::Home if mods.ctrl => Move(Direction::Top, shift),
        Key::End if mods.ctrl => Move(Direction::Bottom, shift),
        Key::Home => Move(Direction::LineStart, shift),
        Key::End => Move(Direction::LineEnd, shift),
        Key::PageUp if mods.ctrl => PreviousTab,
        Key::PageDown if mods.ctrl => NextTab,
        Key::PageUp => Move(Direction::PageUp, shift),
        Key::PageDown => Move(Direction::PageDown, shift),
        Key::Char('`') if mods.ctrl && mods.shift => CloseDock,
        Key::Char('~') if mods.ctrl => CloseDock,
        Key::Char('`') if mods.ctrl => FocusAgent,
        Key::Char('b' | 'B') if mods.ctrl && mods.alt => BuildPdf,
        Key::Char('j' | 'J') if mods.ctrl && mods.alt => ShowInPdf,
        Key::Char('j') if mods.ctrl => ToggleTerminal,
        Key::Enter if mods.ctrl && shift => InsertLineAbove,
        Key::Enter if mods.ctrl => InsertLineBelow,
        Key::Enter => Newline,
        // Where the terminal tells Ctrl+Tab from Tab, it changes tab as in VS Code.
        Key::Tab if mods.ctrl => NextTab,
        Key::BackTab if mods.ctrl => PreviousTab,
        Key::Tab => Indent,
        Key::BackTab => Outdent,
        Key::Backspace if word => DeleteWordLeft,
        Key::Delete if word => DeleteWordRight,
        Key::Backspace => DeleteBackward,
        Key::Delete => DeleteForward,
        // Before Option+Right below, which is the same letter without Shift.
        Key::Char('F') if mods.alt => FormatDocument,
        Key::Char('f') if mods.alt && shift => FormatDocument,
        // What terminals send for Option+Left and Option+Right.
        Key::Char('b') if mods.alt => Move(Direction::WordLeft, shift),
        Key::Char('f') if mods.alt => Move(Direction::WordRight, shift),
        Key::Char('z' | 'Z') if mods.ctrl && mods.shift => Redo,
        // Terminals without the Kitty protocol send Ctrl+/ as Ctrl+_ or Ctrl+7.
        // Ctrl+- and Ctrl+Shift+-, as in VS Code, where the terminal tells them apart.
        Key::Char('-') if mods.ctrl && !shift => GoBack,
        Key::Char('_' | '-') if mods.ctrl && shift => GoForward,
        Key::Char('/' | '_' | '7') if mods.ctrl => ToggleComment,
        Key::Esc => Cancel,
        Key::F(1) => CommandPalette,
        Key::F(2) => RenameSymbol,
        Key::F(8) if shift => PreviousProblem,
        Key::F(8) => NextProblem,
        Key::F(12) if shift => GoToReferences,
        Key::F(12) => GoToDefinition,
        Key::Char(' ') if mods.ctrl => Complete,
        Key::Char('p') if mods.alt => CommandPalette,
        Key::Char('c') if mods.alt => AddComment,
        // As in VS Code. Telling these from Ctrl+P needs the Kitty keyboard protocol.
        Key::Char('p' | 'P') if mods.ctrl && mods.shift => CommandPalette,
        Key::Char('f' | 'F') if mods.ctrl && mods.shift => FindInProject,
        Key::Char('k' | 'K') if mods.ctrl && mods.shift => DeleteLine,
        Key::Char('l' | 'L') if mods.ctrl && mods.shift => SelectAllMatches,
        Key::Char(c) if mods.ctrl => match c.to_ascii_lowercase() {
            // Ghostty sends these for Cmd+Backspace and Cmd+Right, and they're the
            // Mac's own text shortcuts everywhere else.
            'l' => GoToLine,
            'u' => DeleteToLineStart,
            'k' => DeleteToLineEnd,
            'e' => Move(Direction::LineEnd, shift),
            'q' => Quit,
            's' => Save,
            'z' => Undo,
            'y' => Redo,
            'c' => Copy,
            'x' => Cut,
            'v' => Paste,
            'a' => SelectAll,
            'd' => SelectNextMatch,
            'w' => CloseTab,
            'p' => QuickOpen,
            'f' => FindInFile,
            // Ctrl+H is Backspace in too many terminals.
            'r' => ReplaceInFile,
            'b' => ToggleSidebar,
            'g' => OpenDiff,
            // Without the kitty protocol Ctrl+\ arrives as the byte 0x1c, read as Ctrl+4.
            '\\' | '4' => SplitPane,
            'o' => FocusNextPane,
            'n' => NewDocument,
            ',' => OpenSettings,
            ']' => Indent,
            // Ctrl+[ only arrives as itself with the kitty protocol; elsewhere it's Escape.
            '[' => Outdent,
            _ => return None,
        },
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Mods;

    #[test]
    fn vs_codes_everyday_shortcuts_are_here() {
        let key = |key, ctrl, shift, alt| {
            simple_keymap(KeyEvent {
                key,
                mods: Mods { ctrl, shift, alt },
            })
        };
        let ctrl = |c| key(Key::Char(c), true, false, false);
        let ctrl_shift = |c| key(Key::Char(c), true, true, false);
        assert_eq!(ctrl('a'), Some(Command::SelectAll));
        assert_eq!(ctrl('n'), Some(Command::NewDocument));
        assert_eq!(ctrl(','), Some(Command::OpenSettings));
        assert_eq!(ctrl(']'), Some(Command::Indent));
        assert_eq!(ctrl_shift('K'), Some(Command::DeleteLine));
        assert_eq!(ctrl_shift('L'), Some(Command::SelectAllMatches));
        assert_eq!(
            key(Key::Enter, true, false, false),
            Some(Command::InsertLineBelow)
        );
        assert_eq!(
            key(Key::Enter, true, true, false),
            Some(Command::InsertLineAbove)
        );
        assert_eq!(
            key(Key::Up, true, false, true),
            Some(Command::AddCursorAbove)
        );
        assert_eq!(
            key(Key::Down, true, false, true),
            Some(Command::AddCursorBelow)
        );
        assert_eq!(
            key(Key::F(8), false, false, false),
            Some(Command::NextProblem)
        );
        assert_eq!(key(Key::Tab, true, false, false), Some(Command::NextTab));
        assert_eq!(key(Key::Tab, false, false, false), Some(Command::Indent));
    }

    #[test]
    fn ctrl_backslash_splits_in_either_encoding() {
        let ctrl = |c| KeyEvent {
            key: Key::Char(c),
            mods: Mods {
                ctrl: true,
                ..Mods::default()
            },
        };
        assert_eq!(simple_keymap(ctrl('\\')), Some(Command::SplitPane));
        assert_eq!(simple_keymap(ctrl('4')), Some(Command::SplitPane));
    }

    #[test]
    fn option_shift_f_formats_and_option_f_still_moves() {
        let alt = |c, shift| KeyEvent {
            key: Key::Char(c),
            mods: Mods {
                alt: true,
                shift,
                ..Mods::default()
            },
        };
        assert_eq!(simple_keymap(alt('F', true)), Some(Command::FormatDocument));
        assert_eq!(
            simple_keymap(alt('F', false)),
            Some(Command::FormatDocument)
        );
        assert_eq!(
            simple_keymap(alt('f', false)),
            Some(Command::Move(Direction::WordRight, false))
        );
    }

    #[test]
    fn plain_characters_are_text() {
        assert_eq!(simple_keymap(KeyEvent::plain(Key::Char('s'))), None);
    }

    #[test]
    fn ctrl_keys_are_commands() {
        assert_eq!(simple_keymap(KeyEvent::ctrl('s')), Some(Command::Save));
    }

    #[test]
    fn ctrl_p_opens_files_and_ctrl_shift_p_opens_commands() {
        assert_eq!(simple_keymap(KeyEvent::ctrl('p')), Some(Command::QuickOpen));
        let event = KeyEvent {
            key: Key::Char('P'),
            mods: Mods {
                ctrl: true,
                shift: true,
                ..Mods::default()
            },
        };
        assert_eq!(simple_keymap(event), Some(Command::CommandPalette));
    }

    #[test]
    fn mac_text_shortcuts_work() {
        let alt = |key| KeyEvent {
            key,
            mods: Mods {
                alt: true,
                ..Mods::default()
            },
        };
        assert_eq!(
            simple_keymap(alt(Key::Backspace)),
            Some(Command::DeleteWordLeft)
        );
        assert_eq!(
            simple_keymap(alt(Key::Delete)),
            Some(Command::DeleteWordRight)
        );
        let word_left = Some(Command::Move(Direction::WordLeft, false));
        assert_eq!(simple_keymap(alt(Key::Char('b'))), word_left);
        assert_eq!(
            simple_keymap(KeyEvent::ctrl('u')),
            Some(Command::DeleteToLineStart)
        );
        assert_eq!(simple_keymap(KeyEvent::ctrl('z')), Some(Command::Undo));
    }

    #[test]
    fn shift_extends_selection() {
        let event = KeyEvent {
            key: Key::Left,
            mods: Mods {
                shift: true,
                ..Mods::default()
            },
        };
        assert_eq!(
            simple_keymap(event),
            Some(Command::Move(Direction::Left, true))
        );
    }
}
