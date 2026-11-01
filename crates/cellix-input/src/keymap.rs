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
        Key::Enter => Newline,
        Key::Tab => Indent,
        Key::BackTab => Outdent,
        Key::Backspace => DeleteBackward,
        Key::Delete => DeleteForward,
        Key::Esc => Cancel,
        Key::F(1) => CommandPalette,
        Key::F(2) => RenameSymbol,
        Key::F(12) => GoToDefinition,
        Key::Char(' ') if mods.ctrl => Complete,
        Key::Char('p') if mods.alt => CommandPalette,
        Key::Char('c') if mods.alt => AddComment,
        Key::Char(c) if mods.ctrl => match c.to_ascii_lowercase() {
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
            'f' => FindInProject,
            'b' => ToggleSidebar,
            'g' => OpenDiff,
            '\\' => SplitPane,
            'o' => FocusNextPane,
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
    fn plain_characters_are_text() {
        assert_eq!(simple_keymap(KeyEvent::plain(Key::Char('s'))), None);
    }

    #[test]
    fn ctrl_keys_are_commands() {
        assert_eq!(simple_keymap(KeyEvent::ctrl('s')), Some(Command::Save));
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
