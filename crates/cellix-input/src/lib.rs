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
    Middle,
    Right,
    Up,
    Drag,
    /// The pointer moved with no button held.
    Move,
    ScrollUp,
    ScrollDown,
    /// The wheel tipped sideways, or a trackpad moving across. Only what can move sideways
    /// uses these: elsewhere a trackpad's drift as it scrolls down would jitter the view.
    ScrollLeft,
    ScrollRight,
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

/// Markdown formatting, as the page's toolbar offers it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Bold,
    Italic,
    Code,
    Heading,
    Bullet,
    Task,
    Quote,
    Strikethrough,
    /// Takes a line's heading, list or quote marker away.
    PlainText,
    CodeBlock,
    Table,
    Numbered,
    /// A heading of this level, or plain text at zero.
    Level(u8),
    /// A line across the page.
    Rule,
    Link,
}

/// Anything the user can ask for. Each entry is also a command-palette item unless it
/// needs arguments.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    Move(Direction, bool),
    Newline,
    /// Types one character, as a key would.
    Type(char),
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
    /// Moves the keyboard between the file and the side pane.
    FocusAgent,
    /// Builds the paper in view, and shows the PDF.
    BuildPdf,
    /// Writes the document as a PDF beside it.
    ExportPdf,
    /// The plan in review against the version before it, as suggestions.
    ComparePlanVersions,
    /// Scrolls the PDF to the line the caret is on.
    ShowInPdf,
    /// The side pane, or the editor in front, takes the window, or gives it back.
    MaximizePane,
    MaximizeEditor,
    /// Wraps long lines of code at the pane's width, or stops: Alt+Z.
    ToggleWordWrap,
    /// Folds the region the caret is in, or opens the fold: Ctrl+Shift+[ and ].
    Fold,
    Unfold,
    FoldAll,
    UnfoldAll,
    ToggleComment,
    MoveLineUp,
    MoveLineDown,
    DuplicateLine,
    /// VS Code's Ctrl+Shift+K.
    DeleteLine,
    /// A new line below this one, or above, without splitting it: Ctrl+Enter, Ctrl+Shift+Enter.
    InsertLineBelow,
    InsertLineAbove,
    /// Every place the selection or word appears, as cursors: Ctrl+Shift+L.
    SelectAllMatches,
    /// A cursor on the line above or below as well: Ctrl+Alt+Up and Down.
    AddCursorAbove,
    AddCursorBelow,
    /// The next suggestion in the document, or the one before: Alt+F5, Shift+Alt+F5.
    NextChange,
    PreviousChange,
    /// The next problem in the file, or the one before: F8, Shift+F8.
    NextProblem,
    PreviousProblem,
    GoToLine,
    DeleteWordLeft,
    DeleteWordRight,
    DeleteToLineStart,
    DeleteToLineEnd,
    FindInFile,
    /// Find with the replace row open.
    ReplaceInFile,
    Format(Format),
    SendToAgent,
    MentionSelection,
    AcceptProposal,
    RejectProposal,
    ApprovePlan,
    SendBackPlan,
    ShowAgents,
    CloseDock,
    /// Shows the side pane with what it showed last, or puts it away.
    TogglePanel,
    /// Shows the document's headings in the sidebar.
    ShowOutline,
    /// Shows what's changed, to stage and commit, in the sidebar.
    ShowSourceControl,
    /// Brings a terminal up in the pane, starting one if there is none, or puts it away.
    ToggleTerminal,
    /// Starts another terminal in the pane.
    NewTerminal,
    /// Brings the comments up in the pane, or puts them away.
    ShowComments,
    /// Brings the pane's browser up, or starts one, or puts it away.
    ShowBrowser,
    /// Brings the list of agents up in the pane, or puts it away.
    ListAgents,
    /// Runs the notebook cell the caret is in.
    RunCell,
    /// Clears what every cell printed.
    ClearOutputs,
    RunAllCells,
    /// Stops what the notebook's kernel is doing.
    InterruptKernel,
    /// Starts the notebook's kernel again, forgetting what it held.
    RestartKernel,
    /// Opens the guide to getting started.
    ShowGuide,
    /// Starts a new document.
    NewDocument,
    /// Keeps typing in this document as suggested changes.
    ToggleTracking,
    AcceptChange,
    RejectChange,
    /// Turns spelling and grammar checks on or off.
    ToggleSpelling,
    AcceptAllChanges,
    RejectAllChanges,
    ReviewPreviousTurn,
    UndoTurn,
    Commit,
    OpenClaude,
    OpenCodex,
    /// Moves to the next workspace in the bar along the bottom.
    NextWorkspace,
    PreviousWorkspace,
    /// Opens another folder beside this one, as a workspace of its own.
    OpenWorkspace,
    OpenGemini,
    OpenSettings,
    SwitchBranch,
    GoToDefinition,
    /// The language server's fixes and refactors at the caret: Ctrl+. in VS Code.
    QuickFix,
    /// What the language server says about the symbol at the caret.
    ShowHover,
    /// Back to where the last jump left from, as VS Code's Go Back and Vim's Ctrl+O.
    GoBack,
    GoForward,
    GoToReferences,
    FormatDocument,
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
    /// The start of the next word, as Vim's `w`.
    NextWordStart,
    /// To a character on the line: forward or back, and stopping just before it.
    FindChar(char, bool, bool),
}
