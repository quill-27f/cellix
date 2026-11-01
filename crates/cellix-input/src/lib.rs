//! Input events and the commands they map to. Editing modes are key maps over the same
//! commands, so views never depend on which mode is active.

mod keymap;
mod vim;

pub use keymap::simple_keymap;
pub use vim::{Vim, VimAction, VimMode};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Char(char),
    Enter,
    Tab,
    BackTab,
    Backspace,
    Delete,
    Esc,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    PageUp,
    PageDown,
    F(u8),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mods {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyEvent {
    pub key: Key,
    pub mods: Mods,
}

impl KeyEvent {
    pub fn plain(key: Key) -> Self {
        Self {
            key,
            mods: Mods::default(),
        }
    }

    pub fn ctrl(c: char) -> Self {
        Self {
            key: Key::Char(c),
            mods: Mods {
                ctrl: true,
                ..Mods::default()
            },
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseKind {
    Left,
    Right,
    Up,
    Drag,
    /// The pointer moved with no button held.
    Move,
    ScrollUp,
    ScrollDown,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MouseEvent {
    pub kind: MouseKind,
    pub x: u16,
    pub y: u16,
    /// `x` with the fraction of a cell, where the terminal reports pixels. Proportional
    /// text needs this: a cell can hold two thin letters.
    pub fine_x: f32,
    pub mods: Mods,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    Key(KeyEvent),
    Mouse(MouseEvent),
    Paste(String),
    Resize(u16, u16),
    /// Periodic wake-up for background work such as checking files on disk.
    Tick,
}

/// Anything the user can ask for. Each entry is also a command-palette item unless it
/// needs arguments.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    Move(Direction, bool),
    Newline,
    Indent,
    Outdent,
    DeleteBackward,
    DeleteForward,
    Undo,
    Redo,
    Copy,
    Cut,
    Paste,
    SelectAll,
    SelectNextMatch,
    Save,
    CloseTab,
    NextTab,
    PreviousTab,
    SplitPane,
    FocusNextPane,
    ToggleSidebar,
    QuickOpen,
    CommandPalette,
    FindInProject,
    OpenDiff,
    AddComment,
    BlameLine,
    ReplaceInProject,
    MarkdownPreview,
    OpenInBrowser,
    ReviewPreviousTurn,
    UndoTurn,
    Commit,
    OpenClaude,
    OpenCodex,
    GoToDefinition,
    Complete,
    RenameSymbol,
    /// Selects this many whole lines from the cursor.
    SelectLines(usize),
    /// Pastes after the cursor, or below it for a copied line.
    PasteLine,
    Cancel,
    Quit,
}

/// A cursor movement, independent of the rope model.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
    WordLeft,
    WordRight,
    LineStart,
    /// Column zero, where `LineStart` stops at the indent first.
    LineColumnZero,
    LineEnd,
    Top,
    Bottom,
    PageUp,
    PageDown,
}
