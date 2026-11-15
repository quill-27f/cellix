use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

use cellix_input::{Command, Format, KeyEvent};
use cellix_render::{Bounds, Font, Grid, Icon, Measure, Rect, Rgb, Style};

use crate::theme::Theme;

const WIDTH: u16 = 64;
/// Text in the search field, as a percentage of the interface font.
const INTERFACE_SIZE_PERCENT: u8 = 100;
const ROWS: u16 = 12;
/// Space from the panel's edge to the icon and the text of the list, which line up: two
/// columns, as text stands inside whatever holds it.
const TEXT_INSET: f32 = 2.0;
/// How far a floating panel reaches past its rows: not at all, so it ends on a row line and
/// never cuts through the text rows above and below it. A row's own air keeps its text off
/// the edge.
const PANEL_PADDING: f32 = 0.0;
/// Rows above the list: the field's, and one of air, shared out above the field, between it
/// and the list, and under the list.
const HEADER_ROWS: u16 = 2;
/// How far the field's box stands inside its row, above and below, as every text box does.
const BOX_AIR: f32 = 0.11;
/// The band behind the picked row, as a menu's is: 26 px of a 32 px row, and a column in
/// from each side, where the field's box stands too.
const BAND_HEIGHT: f32 = 26.0 / 32.0;
const BAND_INSET: f32 = 1.0;
const BAND_AIR: f32 = (1.0 - BAND_HEIGHT) / 2.0;
/// The air above the field's box, between the box and the first row's band, and under the
/// last row's band: the header's spare row and what the box and the bands leave of their
/// own rows, in three equal parts. The panel still starts and ends on a row line.
const AIR: f32 = (1.0 + 2.0 * BOX_AIR + 2.0 * BAND_AIR) / 3.0;
/// How far down its row the field is painted, and how far up theirs the rows of the list
/// are, to stand that far apart. Only paint can show it: in cells the field has the first
/// row, the list starts on the third, and the row between them is the air.
const FIELD_DROP: f32 = AIR - BOX_AIR;
const LIST_LIFT: f32 = AIR - BAND_AIR;
/// How tall the caret is, in rows.
const CARET: f32 = 0.56;
/// The least the thumb at the list's edge is, in rows: less is a speck.
const THUMB: f32 = 1.2;
/// Rows the list moves for a notch of the wheel.
const NOTCH: isize = 2;
/// Air between what's typed and the status at the field's right end, in cells.
const STATUS_GAP: f32 = 1.5;
/// Space between the caret and the hint in an empty field.
const HINT_GAP: f32 = 0.45;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Item {
    File(PathBuf, String),
    Command(&'static str, Command),
    /// A branch to switch to, and when it last moved.
    Branch(String, String),
    NewBranch,
    /// A blank row with a line through it, between groups.
    Gap,
    /// A key to cite, and where it comes from.
    Citation(String, String),
    /// A picture, how it's shown, and whether it takes the place of the one at the caret.
    Image(PathBuf, String, bool),
    /// A picture to add to the comment being written, and how it's shown.
    Attach(PathBuf, String),
    /// A folder to open as a workspace, as it's written, and a word about it.
    Folder(PathBuf, String, &'static str),
    /// A symbol to go to, in this file or elsewhere in the workspace.
    Symbol(Box<SymbolRow>),
    /// A task to run, from Run Task.
    Task(crate::tasks::Task),
    /// A group's name, over its rows, as RECENTLY RUN or CARGO.
    Header(String),
    /// A folder or file from Open Recent, as `~` writes it, and whether it's a folder.
    Recent(PathBuf, String, bool),
    /// A choice in one of git's lists: what it says, what stands at its right, and what it does.
    Git(String, String, GitPick),
    /// A line of the file in view to go to, as Quick Open's `:120` asks, and how it's said.
    Line(String, Place),
}

/// A place in a file, as a compiler, a search or a chat names one after its path. Both
/// count from one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Place {
    pub line: usize,
    pub column: Option<usize>,
}

/// What picking a row of one of git's lists does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GitPick {
    /// Compare the file in view with a branch or commit, and what to call it.
    CompareRef(String, String),
    CompareClipboard,
    /// Apply, pop or drop a stash, by its commit.
    Stash(crate::source::StashAction, String),
    /// A commit that changed a file, and the file as it was called there.
    FileCommit(Box<cellix_review::Commit>, PathBuf),
}

/// A row of Go to Symbol.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SymbolRow {
    pub name: String,
    /// The protocol's SymbolKind, which picks its icon and its group.
    pub kind: u64,
    /// Dim, after the name: what it sits in.
    pub detail: String,
    /// At the right, for a symbol in another file: where it is.
    pub place: String,
    pub path: PathBuf,
    pub line: usize,
    pub column: usize,
}

/// Where a kind of symbol is listed in Go to Symbol, and what its group is called, as VS
/// Code names them.
pub fn kind_group(kind: u64) -> (u8, &'static str) {
    match kind {
        2..=4 => (0, "modules"),
        5 => (1, "classes"),
        23 => (2, "structs"),
        10 => (3, "enumerations"),
        11 => (4, "interfaces"),
        19 => (5, "objects"),
        26 => (6, "type parameters"),
        8 => (7, "fields"),
        7 => (8, "properties"),
        22 => (9, "enum members"),
        14 => (10, "constants"),
        13 => (11, "variables"),
        9 => (12, "constructors"),
        6 => (13, "methods"),
        12 => (14, "functions"),
        _ => (15, "symbols"),
    }
}

/// A kind's icon, what the cell tier shows instead, and its colour: VS Code's purple for
/// what runs, orange for types, blue for fields and teal for the rest.
pub fn kind_icon(kind: u64, theme: &Theme) -> (Icon, &'static str, Rgb) {
    let light = theme.is_light();
    let pick = |dark: Rgb, lightened: Rgb| if light { lightened } else { dark };
    match kind {
        6 | 9 | 12 | 24 | 25 => (
            Icon::SymbolFunction,
            "ƒ",
            pick(Rgb(0xcb, 0xa6, 0xf7), Rgb(0x88, 0x39, 0xef)),
        ),
        5 | 10 | 11 | 19 | 23 | 26 => (
            Icon::SymbolType,
            "◆",
            pick(Rgb(0xfa, 0xb3, 0x87), Rgb(0xd2, 0x4f, 0x0b)),
        ),
        7 | 8 | 20 => (Icon::SymbolField, "▪", theme.accent),
        _ => (
            Icon::SymbolValue,
            "≡",
            pick(Rgb(0x94, 0xe2, 0xd5), Rgb(0x17, 0x92, 0x99)),
        ),
    }
}

impl Item {
    /// A row that only divides the others, which the keyboard and filtering step over.
    pub fn divider(&self) -> bool {
        matches!(self, Self::Gap | Self::Header(_))
    }

    fn label(&self) -> &str {
        match self {
            Self::File(_, label)
            | Self::Image(_, label, _)
            | Self::Attach(_, label)
            | Self::Folder(_, label, _)
            | Self::Recent(_, label, _) => label,
            Self::Command(label, _) => label,
            Self::Task(task) => &task.label,
            Self::Header(name) => name,
            Self::Branch(name, _)
            | Self::Citation(name, _)
            | Self::Git(name, _, _)
            | Self::Line(name, _) => name,
            Self::Symbol(symbol) => &symbol.name,
            Self::NewBranch => "New branch…",
            Self::Gap => "",
        }
    }

    /// What sits at the right of the row.
    fn keys(&self) -> Option<&str> {
        match self {
            Self::File(..)
            | Self::Image(..)
            | Self::Attach(..)
            | Self::NewBranch
            | Self::Gap
            | Self::Line(..)
            | Self::Recent(..) => None,
            Self::Command(_, command) => keys_of(*command),
            Self::Branch(_, when) | Self::Citation(_, when) | Self::Git(_, when, _) => Some(when),
            Self::Folder(_, _, note) => (!note.is_empty()).then_some(note),
            Self::Symbol(symbol) => (!symbol.place.is_empty()).then_some(&symbol.place),
            Self::Task(task) => task.default_build.then_some("default build · Ctrl+Shift+B"),
            Self::Header(_) => None,
        }
    }
}

/// A command's usual keys, shown beside it.
pub fn keys_of(command: Command) -> Option<&'static str> {
    Some(match command {
        Command::Save => "Ctrl+S",
        Command::Cut => "Ctrl+X",
        Command::Copy => "Ctrl+C",
        Command::Paste => "Ctrl+V",
        Command::CommandPalette => "Ctrl+Shift+P",
        Command::Undo => "Ctrl+Z",
        Command::Redo => "Ctrl+Y",
        Command::SelectAll => "Ctrl+A",
        Command::CloseTab => "Ctrl+W",
        Command::FindInFile => "Ctrl+F",
        Command::GoBack => "Ctrl+-",
        Command::QuickFix => "Ctrl+.",
        Command::GoForward => "Ctrl+Shift+-",
        Command::ReplaceInFile => "Ctrl+R",
        Command::FindInProject => "Ctrl+Shift+F",
        Command::OpenDiff => "Ctrl+G",
        Command::ToggleSidebar => "Ctrl+B",
        Command::ToggleComment => "Ctrl+/",
        Command::ToggleWordWrap => "Alt+Z",
        Command::Fold => "Ctrl+Shift+[",
        Command::Unfold => "Ctrl+Shift+]",
        Command::GoToLine => "Ctrl+L",
        Command::GoToDefinition => "F12",
        Command::GoToSymbol => "Ctrl+Shift+O",
        Command::GoToWorkspaceSymbol => "Ctrl+T",
        Command::RunBuildTask => "Ctrl+Shift+B",
        Command::GoToReferences => "Shift+F12",
        Command::FormatDocument => "Alt+Shift+F",
        Command::RenameSymbol => "F2",
        Command::AddComment => "Alt+C",
        Command::AskAgent => "Alt+A",
        Command::SplitPane => "Ctrl+\\",
        Command::Quit => "Ctrl+Q",
        _ => return None,
    })
}

pub const COMMANDS: [(&str, Command); 122] = [
    ("Save", Command::Save),
    ("Undo", Command::Undo),
    ("Redo", Command::Redo),
    ("Select All", Command::SelectAll),
    ("Close Tab", Command::CloseTab),
    ("Next Tab", Command::NextTab),
    ("Previous Tab", Command::PreviousTab),
    ("Split Pane", Command::SplitPane),
    ("Focus Next Pane", Command::FocusNextPane),
    ("Toggle Sidebar", Command::ToggleSidebar),
    ("Find in Project", Command::FindInProject),
    ("Review Changes", Command::OpenDiff),
    ("Add Comment", Command::AddComment),
    ("Ask Agent", Command::AskAgent),
    ("Blame Line", Command::BlameLine),
    ("Replace in Project", Command::ReplaceInProject),
    ("Toggle Markdown Source", Command::MarkdownPreview),
    ("Open in Browser", Command::OpenInBrowser),
    ("Focus Agent Pane", Command::FocusAgent),
    ("Build PDF", Command::BuildPdf),
    ("Export to PDF", Command::ExportPdf),
    ("Compare Plan Versions", Command::ComparePlanVersions),
    ("Show Line in PDF", Command::ShowInPdf),
    ("Toggle Maximized Side Pane", Command::MaximizePane),
    ("Toggle Maximized Editor", Command::MaximizeEditor),
    ("Find in File", Command::FindInFile),
    ("Go Back", Command::GoBack),
    ("Next Change", Command::NextChange),
    ("Previous Change", Command::PreviousChange),
    ("Show Hover", Command::ShowHover),
    ("Quick Fix…", Command::QuickFix),
    ("Go Forward", Command::GoForward),
    ("Replace in File", Command::ReplaceInFile),
    ("Toggle Comment", Command::ToggleComment),
    ("View: Toggle Word Wrap", Command::ToggleWordWrap),
    ("Fold", Command::Fold),
    ("Unfold", Command::Unfold),
    ("Fold All", Command::FoldAll),
    ("Unfold All", Command::UnfoldAll),
    ("Duplicate Line", Command::DuplicateLine),
    ("Move Line Up", Command::MoveLineUp),
    ("Go to Line", Command::GoToLine),
    ("Format: Bold", Command::Format(Format::Bold)),
    ("Format: Italic", Command::Format(Format::Italic)),
    ("Format: Inline Code", Command::Format(Format::Code)),
    ("Format: Heading", Command::Format(Format::Heading)),
    ("Format: Bulleted List", Command::Format(Format::Bullet)),
    ("Format: Task List", Command::Format(Format::Task)),
    ("Format: Quote", Command::Format(Format::Quote)),
    ("Send Comments to Agent", Command::SendToAgent),
    ("Send Selection to Agent", Command::MentionSelection),
    ("Accept Proposed Change", Command::AcceptProposal),
    ("Reject Proposed Change", Command::RejectProposal),
    ("Approve Plan", Command::ApprovePlan),
    ("Send Plan Back", Command::SendBackPlan),
    ("Show Agents", Command::ShowAgents),
    ("Close Side Pane", Command::CloseDock),
    ("Toggle Side Pane", Command::TogglePanel),
    ("Show Outline", Command::ShowOutline),
    ("Show Source Control", Command::ShowSourceControl),
    ("Show Agent Context", Command::ShowAgentContext),
    ("Toggle Terminal", Command::ToggleTerminal),
    ("New Terminal", Command::NewTerminal),
    ("Show Comments", Command::ShowComments),
    ("Show Inbox", Command::ShowInbox),
    ("Show Browser", Command::ShowBrowser),
    ("List Agents", Command::ListAgents),
    ("Run Cell", Command::RunCell),
    ("Run All Cells", Command::RunAllCells),
    ("Clear All Outputs", Command::ClearOutputs),
    ("Interrupt Kernel", Command::InterruptKernel),
    ("Restart Kernel", Command::RestartKernel),
    ("Getting Started", Command::ShowGuide),
    ("New Document", Command::NewDocument),
    ("Toggle Track Changes", Command::ToggleTracking),
    ("Toggle Spell Check", Command::ToggleSpelling),
    ("Accept Change", Command::AcceptChange),
    ("Reject Change", Command::RejectChange),
    ("Accept All Changes", Command::AcceptAllChanges),
    ("Reject All Changes", Command::RejectAllChanges),
    ("Review Previous Agent Turn", Command::ReviewPreviousTurn),
    ("Undo Agent Turn", Command::UndoTurn),
    ("Show Agent Task", Command::ShowAgentTask),
    ("Commit Staged Changes", Command::Commit),
    ("Open Claude Code", Command::OpenClaude),
    ("Open Codex", Command::OpenCodex),
    ("Open Folder as a Workspace", Command::OpenWorkspace),
    ("Open Recent…", Command::OpenRecent),
    ("Run Task…", Command::RunTask),
    ("Run Build Task", Command::RunBuildTask),
    ("Rerun Last Task", Command::RerunLastTask),
    ("Next Workspace", Command::NextWorkspace),
    ("Previous Workspace", Command::PreviousWorkspace),
    ("Open Settings", Command::OpenSettings),
    ("Switch Branch", Command::SwitchBranch),
    ("Open Gemini CLI", Command::OpenGemini),
    ("Open Agent Transcript", Command::OpenTranscript),
    ("Go to Definition", Command::GoToDefinition),
    ("Go to Symbol in Editor", Command::GoToSymbol),
    ("Go to Symbol in Workspace", Command::GoToWorkspaceSymbol),
    ("Go to References", Command::GoToReferences),
    ("Format Document", Command::FormatDocument),
    ("Complete", Command::Complete),
    ("Rename Symbol", Command::RenameSymbol),
    // Git
    ("Stash Changes", Command::Stash),
    ("Stash Including Untracked", Command::StashUntracked),
    ("Stash Staged Changes", Command::StashStaged),
    ("Pop Latest Stash", Command::PopStash),
    ("Apply Stash…", Command::ApplyStash),
    ("Pop Stash…", Command::PickStashToPop),
    ("Drop Stash…", Command::DropStash),
    ("Compare With…", Command::CompareWith),
    ("Show Git History", Command::ShowHistory),
    ("Show File History", Command::FileHistory),
    ("Accept Current Change", Command::AcceptCurrent),
    ("Accept Incoming Change", Command::AcceptIncoming),
    ("Accept Both Changes", Command::AcceptBoth),
    ("Compare Current and Incoming", Command::CompareConflict),
    ("Next Conflict", Command::NextConflict),
    ("Previous Conflict", Command::PreviousConflict),
    ("Abort Merge", Command::AbortMerge),
    ("Quit", Command::Quit),
];

/// Quick open and the command palette: a filtered list with a text box.
pub struct Palette {
    pub query: String,
    /// How much of what's typed stands after the caret, in bytes. It's counted from the end,
    /// so whatever sets or adds to the words leaves the caret after them.
    back: usize,
    /// What the empty field says.
    hint: &'static str,
    items: Vec<Item>,
    matches: Vec<usize>,
    /// Each match's score, beside it, while something is typed.
    scores: Vec<i32>,
    /// What the matches were found for: a longer query only needs to look among them.
    matched: String,
    pub selected: usize,
    /// The first row of the list that's shown: it scrolls to keep the picked row in it, and
    /// with the wheel.
    top: usize,
    /// What's still happening, as "8,310 files…", shown at the right of the field.
    pub status: Option<String>,
    /// Set when the palette finds a folder, which lists again as a path is typed. Kept
    /// apart, as the keys are: every other palette carries nothing for it.
    pub folders: Option<Box<crate::folders::Picker>>,
    /// What starts the query and isn't part of the search: Quick Open's `@` for this file's
    /// symbols and `#` for the workspace's.
    pub prefix: Option<char>,
    /// Whether rows are kept together by kind, each group labelled with its size.
    grouped: bool,
    /// Quick Open, where `@` or `#` at the start turns to symbols and taking it away turns
    /// back.
    pub quick: bool,
    /// The keys each command has now, the person's own among them, to show beside it in
    /// place of its usual ones. Kept apart, so a palette stays the size it was.
    keys: Option<std::rc::Rc<HashMap<Command, String>>>,
    /// What Quick Open looks for in place of what's typed, when that names a path its own
    /// way: with a line after it, or from the top of the disk.
    needle: Option<String>,
    /// The place what's typed ends with, to go to in the file that's picked.
    pub place: Option<Place>,
    /// The item kept for the file or line that what's typed names outright, and whether
    /// what's typed now names one. It's listed first, whether or not the walk found it.
    direct: Option<(usize, bool)>,
    /// Whether that item is all there is to list, as a line of the file in view is.
    alone: bool,
}

/// What Quick Open is asked for when what's typed is more than letters to match: a path
/// with a place after it, a path from somewhere else, or only a place.
#[derive(Default)]
pub struct Sought {
    /// The path as the list writes paths, from the workspace's folder and without its place.
    pub needle: String,
    pub place: Option<Place>,
    /// The file itself when the path names one that's there, to list first: the walk leaves
    /// out what `.gitignore` names and can't reach outside the folder.
    pub direct: Option<Item>,
    /// Nothing else is listed.
    pub alone: bool,
}

/// A number at the start of `text`, and what follows it.
fn leading_number(text: &str) -> Option<(usize, &str)> {
    let digits = text.len() - text.trim_start_matches(|c: char| c.is_ascii_digit()).len();
    let number = text[..digits].parse().ok()?;
    Some((number, &text[digits..]))
}

/// What's typed as the path to look for and the place it ends with, in the ways a place is
/// written after a path: `app.rs:120`, `app.rs:120:8`, `app.rs:120-140`, a compiler's
/// `app.rs:120:8:`, GitHub's `app.rs#L120` and MSBuild's `app.rs(120,8)`. A colon with
/// nothing after it yet is on its way to being one, and isn't part of the path.
pub fn path_and_place(typed: &str) -> (&str, Option<Place>) {
    let typed = typed.trim();
    if let Some((path, tail)) = typed.rsplit_once("#L")
        && let Some((line, rest)) = leading_number(tail)
        && (rest.is_empty() || rest.starts_with(['-', 'C']))
    {
        let column = rest.strip_prefix('C').and_then(leading_number);
        let column = column.map(|(column, _)| column);
        return (path, Some(Place { line, column }));
    }
    if let Some(inner) = typed.strip_suffix(')')
        && let Some((path, inner)) = inner.rsplit_once('(')
        && !path.is_empty()
        && let Some((line, rest)) = leading_number(inner)
    {
        let column = rest.strip_prefix(',').map(str::trim_start);
        let column = column.and_then(leading_number);
        if rest.is_empty() || column.is_some_and(|(_, after)| after.is_empty()) {
            let column = column.map(|(column, _)| column);
            return (path, Some(Place { line, column }));
        }
    }
    for (at, _) in typed.match_indices(':') {
        let tail = &typed[at + 1..];
        if tail.is_empty() {
            return (&typed[..at], None);
        }
        let numbers = tail
            .chars()
            .all(|c| c.is_ascii_digit() || c == ':' || c == '-');
        if let (true, Some((line, rest))) = (numbers, leading_number(tail)) {
            let column = rest.strip_prefix(':').and_then(leading_number);
            let column = column.map(|(column, _)| column);
            return (&typed[..at], Some(Place { line, column }));
        }
    }
    (typed, None)
}

/// What was pasted, as the path it names: its first line, without the quotes, brackets,
/// `file://`, `@` or full stop it was copied with, and without the message a compiler or a
/// search printed after it.
pub fn pasted_path(text: &str) -> String {
    let line = text.lines().map(str::trim).find(|line| !line.is_empty());
    let mut path = line.unwrap_or("");
    // rustc's arrow before a place, and Python's `File "app.py", line 12, in main`.
    path = path.strip_prefix("--> ").unwrap_or(path).trim_start();
    if let Some(rest) = path.strip_prefix("File \"")
        && let Some((file, after)) = rest.split_once('"')
    {
        let line = after.strip_prefix(", line ").and_then(leading_number);
        return match line {
            Some((line, _)) => format!("{file}:{line}"),
            None => file.to_string(),
        };
    }
    // A Markdown link is where it leads.
    if path.starts_with('[')
        && let Some(inner) = path.strip_suffix(')')
        && let Some((_, target)) = inner.split_once("](")
    {
        path = target;
    }
    // `app.rs:120:8: error…` and `app.rs:120:    let x` are the place before the space.
    if let Some((first, _)) = path.split_once(char::is_whitespace)
        && let Some(place) = first.strip_suffix(':')
        && path_and_place(place).1.is_some()
    {
        path = place;
    }
    const PAIRS: [(char, char); 6] = [
        ('"', '"'),
        ('\'', '\''),
        ('`', '`'),
        ('<', '>'),
        ('(', ')'),
        ('[', ']'),
    ];
    loop {
        let before = path;
        for (open, close) in PAIRS {
            if path.len() >= 2 && path.starts_with(open) && path.ends_with(close) {
                path = path[1..path.len() - 1].trim();
            }
            // Half a pair, as a path at the end of a sentence in brackets keeps.
            if path.ends_with(close) && !path[..path.len() - 1].contains(open) {
                path = &path[..path.len() - 1];
            }
            if path.starts_with(open) && !path[1..].contains(close) {
                path = &path[1..];
            }
        }
        // What a sentence leaves after a path.
        path = path.trim_end_matches(['.', ',', ';']);
        if path == before {
            break;
        }
    }
    if let Some(local) = path.strip_prefix("file://") {
        let local = local.strip_prefix("localhost").unwrap_or(local);
        return unescaped(local);
    }
    // An agent's mention of a file, which Quick Open would take for a symbol.
    match path.strip_prefix('@') {
        Some(file) if file.contains(['/', '\\', '.']) => file.to_string(),
        _ => path.to_string(),
    }
}

/// A URL's path with its `%20`s read.
fn unescaped(path: &str) -> String {
    let mut bytes = Vec::with_capacity(path.len());
    let mut rest = path.as_bytes();
    while let Some((&byte, after)) = rest.split_first() {
        let code = after.get(..2).and_then(|hex| std::str::from_utf8(hex).ok());
        let code = code.and_then(|hex| u8::from_str_radix(hex, 16).ok());
        match (byte, code) {
            (b'%', Some(code)) => {
                bytes.push(code);
                rest = &after[2..];
            }
            _ => {
                bytes.push(byte);
                rest = after;
            }
        }
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

/// Where the letters of `query` fall in `text`, as `fuzzy_score` finds them.
pub fn matched_chars(query: &str, text: &str) -> Vec<usize> {
    let mut found = Vec::new();
    let mut rest = text.chars().enumerate();
    for q in query.chars().flat_map(char::to_lowercase) {
        match rest.by_ref().find(|(_, c)| same_letter(*c, q)) {
            Some((at, _)) => found.push(at),
            None => return Vec::new(),
        }
    }
    found
}

/// Scores `text` against `query`, whose characters must appear in order. Runs of
/// consecutive matches, matches in the file name and matches that start a word score
/// higher, and most of all a word that begins with the whole query: "op" is Open Folder
/// before it's Pop Stash.
pub fn fuzzy_score(query: &str, text: &str) -> Option<i32> {
    let name_start = text.rfind('/').map_or(0, |i| i + 1);
    let mut score = 0;
    // Where the last match ended, to see a run.
    let mut last = None;
    // Nothing is allocated: Quick Open scores every file in a big tree on each key.
    let mut rest = text.char_indices();
    for q in query.chars().flat_map(char::to_lowercase) {
        let (found, c) = rest.by_ref().find(|(_, c)| same_letter(*c, q))?;
        score += 1;
        if last == Some(found) {
            score += 4;
        }
        if found >= name_start {
            score += 2;
        }
        if word_start(text, found) {
            score += 2;
        }
        last = Some(found + c.len_utf8());
    }
    if let Some(at) = word_beginning_with(query, text) {
        score += 6;
        // The first word of a command, or of a file's name, is what people type.
        if at == name_start {
            score += 3;
        }
    }
    Some(score - text.len() as i32 / 16)
}

/// Whether a word starts at byte `at` of `text`: at its beginning, after anything that
/// isn't a letter or digit, or at a capital after a small letter.
fn word_start(text: &str, at: usize) -> bool {
    let Some(before) = text[..at].chars().next_back() else {
        return true;
    };
    let capital = || text[at..].chars().next().is_some_and(char::is_uppercase);
    !before.is_alphanumeric() || before.is_lowercase() && capital()
}

/// Where the first word of `text` that begins with all of `query` starts.
fn word_beginning_with(query: &str, text: &str) -> Option<usize> {
    if query.is_empty() {
        return None;
    }
    let begins = |at: usize| {
        let mut letters = text[at..].chars();
        let mut typed = query.chars().flat_map(char::to_lowercase);
        typed.all(|q| letters.next().is_some_and(|c| same_letter(c, q)))
    };
    text.char_indices()
        .map(|(at, _)| at)
        .find(|at| word_start(text, *at) && begins(*at))
}

/// Whether `c` is `lower`, whatever its case.
fn same_letter(c: char, lower: char) -> bool {
    c == lower || (!c.is_ascii() || c.is_ascii_uppercase()) && c.to_lowercase().eq([lower])
}

impl Palette {
    pub fn new(items: Vec<Item>) -> Self {
        let mut palette = Self {
            query: String::new(),
            back: 0,
            hint: "Type to search",
            items,
            matches: Vec::new(),
            scores: Vec::new(),
            matched: String::new(),
            selected: 0,
            top: 0,
            status: None,
            folders: None,
            prefix: None,
            grouped: false,
            quick: false,
            keys: None,
            needle: None,
            place: None,
            direct: None,
            alone: false,
        };
        palette.filter();
        palette
    }

    /// Where the caret is in what's typed, in bytes.
    pub fn caret(&self) -> usize {
        let mut at = self.query.len().saturating_sub(self.back);
        while !self.query.is_char_boundary(at) {
            at -= 1;
        }
        at
    }

    /// Takes a key that edits what's typed or moves the caret in it, as any text box does:
    /// the arrows, Home and End, by word with Alt or Ctrl, Delete, and the deletes a Mac or
    /// a shell has taught. Returns whether the words changed.
    pub fn key(&mut self, key: KeyEvent) -> bool {
        let was = self.query.clone();
        let mut caret = self.caret();
        crate::field::key(&mut self.query, &mut caret, key);
        self.back = self.query.len().saturating_sub(caret);
        self.query != was
    }

    /// Puts text in at the caret, which ends up after it.
    pub fn insert(&mut self, text: &str) {
        let at = self.caret();
        self.query.insert_str(at, text);
    }

    /// Where what's typed starts in the field, how wide each of its letters is, and how
    /// many of them are out of sight before the field's start: a query longer than the
    /// field shows the part the caret is in.
    fn typed_window(&self, area: Rect, measure: &dyn Measure) -> (f32, Rc<Vec<f32>>, usize) {
        let font = Font {
            size: INTERFACE_SIZE_PERCENT,
            ..Font::default()
        };
        // Clear of the icon, where a row's words start after its own.
        let x = area.x as f32 + TEXT_INSET + 2.3;
        let room = f32::from(area.right().saturating_sub(2)) - x - 0.5;
        let advances = measure.advances(&self.query, font);
        let before = self.query[..self.caret()]
            .chars()
            .count()
            .min(advances.len());
        let mut hidden = 0;
        while advances[hidden..before].iter().sum::<f32>() > room {
            hidden += 1;
        }
        (x, advances, hidden)
    }

    /// Puts the caret where the field was pressed, `x` with its fraction of a column: at the
    /// nearer edge of the letter there.
    pub fn press_field(&mut self, screen: Rect, x: f32, measure: &dyn Measure) {
        let area = Self::area(screen);
        let (painted, advances, hidden) = self.typed_window(area, measure);
        // In cells the words start on a cell, a little before where they're painted.
        let start = if measure.in_cells() {
            f32::from(area.x + 4)
        } else {
            painted
        };
        let mut edge = start;
        let mut letters = hidden;
        for advance in advances.iter().skip(hidden) {
            if x < edge + advance / 2.0 {
                break;
            }
            edge += advance;
            letters += 1;
        }
        let at = self.query.char_indices().nth(letters);
        let at = at.map_or(self.query.len(), |(at, _)| at);
        self.back = self.query.len() - at;
    }

    /// Takes what Quick Open is asked for, to list by at the next `filter`.
    pub fn seek(&mut self, sought: Sought) {
        self.needle = Some(sought.needle);
        self.place = sought.place;
        self.alone = sought.alone;
        let slot = self.direct.map(|(slot, _)| slot);
        self.direct = match (sought.direct, slot) {
            (Some(item), Some(slot)) => {
                self.items[slot] = item;
                Some((slot, true))
            }
            (Some(item), None) => {
                self.items.push(item);
                Some((self.items.len() - 1, true))
            }
            (None, slot) => slot.map(|slot| (slot, false)),
        };
        // A different file may stand in the slot now, so what matched before isn't what to
        // look among.
        self.matched.clear();
    }

    /// What the rows are matched against: what Quick Open made of a path, or what's typed.
    fn sought(&self) -> &str {
        match &self.needle {
            Some(needle) => needle,
            None => self.typed(),
        }
    }

    /// The file the query names outright, where it names one.
    fn direct_path(&self) -> Option<&PathBuf> {
        match self.direct {
            Some((slot, true)) => match &self.items[slot] {
                Item::File(path, _) => Some(path),
                _ => None,
            },
            _ => None,
        }
    }

    /// Puts the file or line the query names first, in place of that file as the walk
    /// found it.
    fn pin(&mut self) {
        let Some((slot, true)) = self.direct else {
            return;
        };
        if let Some(path) = self.direct_path().cloned() {
            let same = |item: &Item| matches!(item, Item::File(found, _) if *found == path);
            let kept: Vec<bool> = self
                .matches
                .iter()
                .map(|&i| !same(&self.items[i]))
                .collect();
            let mut keep = kept.iter();
            self.matches.retain(|_| *keep.next().unwrap_or(&true));
            let mut keep = kept.iter();
            self.scores.retain(|_| *keep.next().unwrap_or(&true));
        }
        self.matches.insert(0, slot);
        self.scores.insert(0, i32::MAX);
        self.selected = 0;
    }

    /// Shows beside each command the keys it has now, as the Keyboard page lists them:
    /// a command given other keys shows those, one whose keys were taken away shows none,
    /// and every command with keys shows them, not only the best known.
    pub fn with_keys(mut self, shortcuts: &[crate::keys::Shortcut]) -> Self {
        let now = shortcuts.iter();
        let now = now.filter_map(|s| Some((s.command, s.keys.first()?.clone())));
        self.keys = Some(std::rc::Rc::new(now.collect()));
        self
    }

    /// What sits at the right of a row.
    fn keys_at(&self, index: usize) -> Option<&str> {
        match (&self.items[index], &self.keys) {
            (Item::Command(_, command), Some(keys)) => keys.get(command).map(String::as_str),
            (item, _) => item.keys(),
        }
    }

    /// Go to Symbol: the query starts with `prefix`, and the file's own symbols are grouped
    /// by kind.
    pub fn symbols(items: Vec<Item>, prefix: char, typed: &str, hint: &'static str) -> Self {
        let mut palette = Self::asking(Vec::new(), hint);
        palette.prefix = Some(prefix);
        palette.grouped = prefix == '@';
        palette.quick = true;
        palette.query = format!("{prefix}{typed}");
        palette.set_items(items);
        palette
    }

    /// Puts new items in place of the old, keeping what's typed and, where it's still there,
    /// the picked row.
    pub fn set_items(&mut self, items: Vec<Item>) {
        let picked = self.current().cloned();
        self.items = items;
        self.direct = None;
        self.matched.clear();
        self.filter();
        if let Some(picked) = picked {
            let index = self.items.iter().position(|item| *item == picked);
            if let Some(row) = index.and_then(|i| self.matches.iter().position(|&m| m == i)) {
                self.selected = row;
            }
        }
        self.show_picked();
    }

    /// What's typed, without the prefix that chose what's listed.
    pub fn typed(&self) -> &str {
        match self.prefix {
            Some(prefix) => self.query.strip_prefix(prefix).unwrap_or(&self.query),
            None => &self.query,
        }
    }

    /// Keeps each kind together, with the best matches first in each, and a gap between
    /// kinds. With nothing typed the kinds come in VS Code's order; with something typed, the
    /// kind with the best match comes first, so the best match is the one picked.
    fn group(&mut self) {
        if !self.grouped {
            return;
        }
        let gap = match self.items.iter().position(|item| *item == Item::Gap) {
            Some(gap) => gap,
            None => {
                self.items.push(Item::Gap);
                self.items.len() - 1
            }
        };
        let rank = |i: usize| match &self.items[i] {
            Item::Symbol(symbol) => kind_group(symbol.kind).0,
            _ => u8::MAX,
        };
        let mut rows: Vec<(usize, i32)> = self
            .matches
            .iter()
            .copied()
            .zip(self.scores.iter().copied().chain(std::iter::repeat(0)))
            .filter(|(i, _)| self.items[*i] != Item::Gap)
            .collect();
        let mut best: HashMap<u8, i32> = HashMap::new();
        for (i, score) in &rows {
            let top = best.entry(rank(*i)).or_insert(*score);
            *top = (*top).max(*score);
        }
        rows.sort_by_key(|(i, _)| (-best[&rank(*i)], rank(*i)));
        let keep_scores = !self.scores.is_empty();
        let (mut matches, mut scores) = (Vec::new(), Vec::new());
        for (n, (i, score)) in rows.iter().enumerate() {
            if n > 0 && rank(rows[n - 1].0) != rank(*i) {
                matches.push(gap);
                scores.push(0);
            }
            matches.push(*i);
            scores.push(*score);
        }
        (self.matches, self.scores) = (matches, scores);
        if !keep_scores {
            self.scores.clear();
        }
        // How many there are, and how many of them match what's typed.
        let total = self
            .items
            .iter()
            .filter(|i| matches!(i, Item::Symbol(_)))
            .count();
        let shown = self
            .matches
            .iter()
            .filter(|&&i| self.items[i] != Item::Gap)
            .count();
        if total > 0 {
            self.status = Some(match (shown, total) {
                (1, 1) => "1 symbol".to_string(),
                (shown, total) if shown == total => format!("{total} symbols"),
                (shown, total) => format!("{shown} of {total}"),
            });
        }
    }

    /// The group label at the right of row `row`, on the first row of each group.
    fn group_label(&self, row: usize) -> Option<String> {
        if !self.grouped {
            return None;
        }
        let kind = |row: usize| match &self.items[*self.matches.get(row)?] {
            Item::Symbol(symbol) => Some(kind_group(symbol.kind)),
            _ => None,
        };
        let (rank, name) = kind(row)?;
        if row > 0 && kind(row - 1).map(|k| k.0) == Some(rank) {
            return None;
        }
        let size = (row..self.matches.len())
            .take_while(|&r| kind(r).map(|k| k.0) == Some(rank))
            .count();
        Some(format!("{name} ({size})"))
    }

    /// Adds items that arrived after the palette opened, as files do from a walk of a big
    /// tree. What's typed applies to them too, and the picked row stays picked.
    pub fn extend(&mut self, items: Vec<Item>) {
        let picked = self.matches.get(self.selected).copied();
        // Where the picked row stands in the list as shown, which is where it stays: more
        // files arriving don't move what's being looked at.
        let stood = self.selected as isize - self.top as isize;
        let from = self.items.len();
        self.items.extend(items);
        if self.alone {
            return;
        }
        let typed = self.sought().to_string();
        if typed.is_empty() {
            self.matches.extend(from..self.items.len());
            return;
        }
        let mut scored: Vec<(i32, usize)> = self
            .scores
            .iter()
            .copied()
            .zip(self.matches.iter().copied())
            .collect();
        // The file the query names is listed already, first.
        let direct = self.direct_path();
        let named = |item: &Item| matches!(item, Item::File(path, _) if Some(path) == direct);
        for (i, item) in self.items.iter().enumerate().skip(from) {
            if let Some(score) = fuzzy_score(&typed, item.label()).filter(|_| !named(item)) {
                scored.push((score, i));
            }
        }
        scored.sort_by_key(|(score, i)| (-score, *i));
        (self.scores, self.matches) = scored.into_iter().unzip();
        if let Some(picked) = picked {
            self.selected = self.matches.iter().position(|&i| i == picked).unwrap_or(0);
        }
        let last = self.matches.len().saturating_sub(ROWS as usize) as isize;
        self.top = (self.selected as isize - stood).clamp(0, last.max(0)) as usize;
    }

    /// How many items it holds.
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.items.len() - usize::from(self.direct.is_some())
    }

    /// A palette whose empty field asks for something other than a search.
    pub fn asking(items: Vec<Item>, hint: &'static str) -> Self {
        Self {
            hint,
            ..Self::new(items)
        }
    }

    /// Lists folders again for what's typed, when it's a path into another folder, and
    /// filters by it either way.
    pub fn refresh(&mut self) {
        let fresh = self
            .folders
            .as_mut()
            .and_then(|picker| picker.items_for(&self.query));
        if let Some(items) = fresh {
            self.items = items;
            self.matched.clear();
        }
        self.filter();
    }

    /// Lists what fits what's typed, best first, from the top.
    pub fn filter(&mut self) {
        self.refilter();
        self.top = 0;
        self.show_picked();
    }

    fn refilter(&mut self) {
        let typed = self.sought().to_string();
        let slot = self.direct.map(|(slot, _)| slot);
        if self.alone {
            self.matches.clear();
            self.scores.clear();
            self.matched.clear();
            self.pin();
            self.selected = 0;
            return;
        }
        // Nothing typed means no ranking: the list keeps the order it came in.
        if typed.is_empty() {
            self.matches = (0..self.items.len()).filter(|i| Some(*i) != slot).collect();
            self.scores.clear();
            self.matched.clear();
            self.selected = 0;
            self.group();
            // A heading or a gap is never what's picked.
            if self.current().is_some_and(Item::divider) {
                self.selected = self
                    .matches
                    .iter()
                    .position(|&i| !self.items[i].divider())
                    .unwrap_or(0);
            }
            return;
        }
        // A letter added at the end can only narrow what matched, so only that is looked at
        // again: in a tree of half a million files, each key stays quick.
        let narrowing = !self.matched.is_empty() && typed.starts_with(&self.matched);
        let candidates: Vec<usize> = if narrowing {
            std::mem::take(&mut self.matches)
        } else {
            (0..self.items.len()).collect()
        };
        let mut scored: Vec<(i32, usize)> = candidates
            .into_iter()
            .filter(|&i| !self.items[i].divider() && Some(i) != slot)
            .filter_map(|i| Some((fuzzy_score(&typed, self.items[i].label())?, i)))
            .collect();
        scored.sort_by_key(|(score, i)| (-score, *i));
        (self.scores, self.matches) = scored.into_iter().unzip();
        self.matched = typed;
        self.selected = 0;
        self.group();
        self.pin();
    }

    /// Moves the pick by `delta` rows, over a rule or a group's name the way it's going, and
    /// back off one at either end.
    pub fn move_by(&mut self, delta: isize) {
        let last = self.matches.len().saturating_sub(1) as isize;
        let step = delta.signum();
        let divider = |at: isize| {
            let index = self.matches.get(at as usize);
            index.is_some_and(|index| self.items[*index].divider())
        };
        let mut at = (self.selected as isize + delta).clamp(0, last);
        while step != 0 && divider(at) && (0..=last).contains(&(at + step)) {
            at += step;
        }
        while step != 0 && divider(at) && (0..=last).contains(&(at - step)) {
            at -= step;
        }
        self.selected = at as usize;
        self.show_picked();
    }

    /// Moves the pick a screen of the list up or down, less a row, so the last one seen is
    /// the first.
    pub fn page(&mut self, down: bool) {
        let rows = ROWS as isize - 1;
        self.move_by(if down { rows } else { -rows });
    }

    /// Scrolls the list by notches of the wheel. The pick stays on its row, in view or not.
    pub fn wheel(&mut self, notches: isize) {
        let last = self.matches.len().saturating_sub(ROWS as usize) as isize;
        self.top = (self.top as isize + notches * NOTCH).clamp(0, last) as usize;
    }

    /// Scrolls the list so the picked row is in it, with the rule or the name over it when
    /// it's the first of its group.
    fn show_picked(&mut self) {
        let rows = ROWS as usize;
        if self.selected < self.top {
            self.top = self.selected;
        } else if self.selected >= self.top + rows {
            self.top = self.selected + 1 - rows;
        }
        let over = self
            .top
            .checked_sub(1)
            .and_then(|over| self.matches.get(over));
        if self.top == self.selected && over.is_some_and(|index| self.items[*index].divider()) {
            self.top -= 1;
        }
        self.top = self.top.min(self.matches.len().saturating_sub(rows));
    }

    pub fn current(&self) -> Option<&Item> {
        self.items.get(*self.matches.get(self.selected)?)
    }

    pub fn area(screen: Rect) -> Rect {
        let width = WIDTH.min(screen.w);
        Rect::new(
            screen.x + (screen.w - width) / 2,
            // Below the breadcrumbs, so its edge doesn't run through them.
            3,
            width,
            ROWS + HEADER_ROWS,
        )
    }

    /// How many rows of the list are drawn: at least one, which says so when there's
    /// nothing to list, so the field never moves as the list empties and fills.
    fn rows(&self) -> u16 {
        self.matches.len().clamp(1, ROWS as usize) as u16
    }

    /// What it covers now, with the rows it has.
    pub fn shown(&self, screen: Rect) -> Rect {
        let area = Self::area(screen);
        Rect::new(area.x, area.y, area.w, self.rows() + HEADER_ROWS)
    }

    /// The row of the list at `(x, y)`, `y` with its fraction of a row. The rows are painted
    /// a little above the cells that hold their words, and a press lands on what's painted.
    pub fn row_at(&self, screen: Rect, (x, y): (u16, f32), measure: &dyn Measure) -> Option<usize> {
        let area = Self::area(screen);
        if x < area.x || x >= area.right() {
            return None;
        }
        let lift = if measure.in_cells() { 0.0 } else { LIST_LIFT };
        let row = (y - (f32::from(area.y + HEADER_ROWS) - lift)).floor();
        let shown = self
            .matches
            .len()
            .saturating_sub(self.top)
            .min(ROWS as usize);
        (row >= 0.0 && (row as usize) < shown).then(|| self.top + row as usize)
    }

    /// `hover` is where the pointer is, its row with the fraction.
    pub fn draw(
        &self,
        grid: &mut Grid,
        hover: Option<(u16, f32)>,
        measure: &dyn Measure,
        theme: &Theme,
    ) {
        let screen = grid.area();
        let area = self.shown(screen);
        raised(grid, area, theme);
        let fine = !measure.in_cells();
        let from = grid.painted_so_far();
        let caret = self.draw_field(grid, area, measure, theme);
        let drop = if fine { FIELD_DROP } else { 0.0 };
        grid.lift_since(from, -drop);
        // The caret in the middle of the box, wherever that's painted.
        grid.cursor = Some((caret as u16, area.y));
        let top = f32::from(area.y) + drop + (1.0 - CARET) / 2.0;
        grid.caret = Some(Bounds::cells(caret, top, 0.0, CARET));
        let from = grid.painted_so_far();
        let pointed = hover.and_then(|at| self.row_at(screen, at, measure));
        if self.matches.is_empty() {
            let quiet = Style {
                fg: theme.popup_dim,
                bg: theme.popup,
                ..theme.bar
            };
            let inset = area.x as f32 + TEXT_INSET;
            let end = area.right().saturating_sub(2);
            let y = area.y + HEADER_ROWS;
            grid.label_at(inset, area.x + 2, y, self.nothing(), quiet, end, 100);
        }
        let rows = self.matches.iter().enumerate();
        for (row, index) in rows.skip(self.top).take(ROWS as usize) {
            let y = area.y + HEADER_ROWS + (row - self.top) as u16;
            let lit = (row == self.selected, pointed == Some(row));
            self.draw_row(grid, (area, y), (row, *index), lit, measure, theme);
        }
        self.draw_thumb(grid, area, theme);
        if fine {
            grid.lift_since(from, LIST_LIFT);
        }
    }

    /// A thumb at the list's edge when there's more than is in view, in the gutter between
    /// the picked row's band and the panel's edge: as much of the list's height as the rows
    /// in view are of all the rows, and as far down it as they are.
    fn draw_thumb(&self, grid: &mut Grid, area: Rect, theme: &Theme) {
        let (all, shown) = (self.matches.len() as f32, f32::from(ROWS));
        if all <= shown {
            return;
        }
        let tall = (shown * shown / all).max(THUMB);
        let down = (shown - tall) * self.top as f32 / (all - shown);
        let top = f32::from(area.y + HEADER_ROWS) + down;
        // In cells it's half blocks down the panel's last column, which a row's band leaves
        // clear.
        let style = Style {
            fg: theme.scrollbar,
            bg: theme.popup,
            painted: true,
            ..theme.bar
        };
        let x = area.right().saturating_sub(1);
        let cells = top.round() as u16..(top + tall).round().max(top.round() + 1.0) as u16;
        for y in cells {
            grid.put_str(x, y, "▐", style, area.right());
        }
        let thumb = Bounds::cells(f32::from(area.right()) - 0.7, top + 0.1, 0.35, tall - 0.2);
        grid.panel(thumb, 12, Some(theme.scrollbar), None);
    }

    /// What the list's one row says when there's nothing in it.
    fn nothing(&self) -> &'static str {
        let working = self.status.as_deref().is_some_and(|s| s.ends_with('…'));
        match (self.typed().is_empty(), working) {
            (false, _) if self.alone => "Type the number of a line to go to",
            (false, _) => "No matches",
            (true, true) => "Nothing yet",
            (true, false) => "Nothing to list",
        }
    }

    /// The field: its box a column in from the panel's edges, where the picked row's band
    /// stands too, the search icon and what's typed where a row's icon and words are, and
    /// what's still happening at its far end. Returns where the caret stands.
    fn draw_field(&self, grid: &mut Grid, area: Rect, measure: &dyn Measure, theme: &Theme) -> f32 {
        let base = Style {
            fg: theme.text.fg,
            bg: theme.input,
            ..theme.bar
        };
        let dim = Style {
            fg: theme.popup_dim,
            ..base
        };
        let y = area.y;
        let inside = Rect::new(area.x + 1, y, area.w.saturating_sub(2), 1);
        // In cells the box is a band of the field's colour.
        grid.fill(inside, base);
        let boxed = Bounds::inset(inside, 0.0, BOX_AIR);
        grid.panel(boxed, 22, Some(theme.input), Some(theme.focus));
        grid.glyph(
            area.x as f32 + TEXT_INSET,
            y,
            Icon::Search,
            theme.popup_dim,
            "⌕",
            base,
        );
        // The hint waits a little past the caret, so they don't touch.
        let (x, advances, hidden) = self.typed_window(area, measure);
        let cell = area.x + 4;
        // A column inside the box's far edge, as the icon is inside its near one.
        let right = area.right().saturating_sub(2);
        let from = self.query.char_indices().nth(hidden);
        let shown = &self.query[from.map_or(self.query.len(), |(from, _)| from)..];
        let before = self.query[..self.caret()]
            .chars()
            .count()
            .min(advances.len());
        let wide = |letters: &[f32]| letters.iter().sum::<f32>();
        let caret = x - 0.1 + wide(&advances[hidden..before]);
        let end_of_words = x + wide(&advances[hidden..]);
        // The status keeps clear of what's typed, and gives way to it when both don't fit:
        // the query is what the person is working on.
        let status = self.status.as_deref().map(|text| {
            let width = measure.width(
                text,
                Font {
                    size: 85,
                    ..Font::default()
                },
            );
            (text, right as f32 - width)
        });
        let status = status.filter(|(_, left)| end_of_words + STATUS_GAP <= *left);
        let end = status.map_or(right, |(_, left)| (left - STATUS_GAP / 2.0) as u16);
        if self.query.is_empty() {
            grid.label_at(x + HINT_GAP, cell, y, self.hint, dim, end, 100);
        } else {
            grid.label_at(x, cell, y, shown, base, end, 100);
        }
        if let Some((text, _)) = status {
            grid.label_right(right, y, text, dim, 85);
        }
        caret.min(right as f32)
    }

    /// One row of the list: a rule, a group's name, or something to pick.
    fn draw_row(
        &self,
        grid: &mut Grid,
        (area, y): (Rect, u16),
        (row, index): (usize, usize),
        (picked, pointed): (bool, bool),
        measure: &dyn Measure,
        theme: &Theme,
    ) {
        let base = Style {
            fg: theme.text.fg,
            bg: theme.popup,
            ..theme.bar
        };
        if self.items[index] == Item::Gap {
            let across = area.w as f32 - TEXT_INSET * 2.0;
            let rule = Bounds::cells(area.x as f32 + TEXT_INSET, y as f32 + 0.5, across, 0.0);
            grid.hairline(rule, theme.popup_border);
            return;
        }
        if let Item::Header(name) = &self.items[index] {
            let small = Style {
                fg: theme.popup_dim,
                bold: true,
                ..base
            };
            let inset = area.x as f32 + TEXT_INSET;
            grid.label_at(inset, area.x + 2, y, name, small, area.right(), 80);
            let font = Font {
                size: 80,
                ..Font::default()
            };
            let from = inset + measure.width(name, font) + 1.0;
            let across = (area.x as f32 + area.w as f32 - TEXT_INSET - from).max(0.0);
            let rule = Bounds::cells(from, y as f32 + 0.5, across, 0.0);
            grid.hairline(rule, theme.popup_border);
            return;
        }
        // The picked row's colour is in its cells too, which is all a terminal without
        // pictures has to show it by.
        let base = Style {
            bg: if picked { theme.pick } else { theme.popup },
            ..base
        };
        if picked || pointed {
            let fill = if picked {
                theme.pick
            } else {
                theme.popup_hover
            };
            if picked {
                let cells = Rect::new(area.x + 1, y, area.w.saturating_sub(2), 1);
                grid.fill(cells, base);
            }
            let band = Bounds::cells(
                area.x as f32 + BAND_INSET,
                y as f32 + BAND_AIR,
                area.w as f32 - BAND_INSET * 2.0,
                BAND_HEIGHT,
            );
            grid.panel(band, 18, Some(fill), None);
        }
        if let Item::Symbol(symbol) = &self.items[index] {
            let group = self.group_label(row);
            let right = group.as_deref().or(Some(symbol.place.as_str()));
            let right = right.filter(|r| !r.is_empty());
            self.draw_symbol(grid, (area, y), symbol, right, picked, measure, theme);
            return;
        }
        let text = self.items[index].label();
        // A file has nothing at its right, so its path has the row.
        let end = match &self.items[index] {
            Item::File(..) => area.right().saturating_sub(2),
            _ => area.right().saturating_sub(16),
        };
        let inset = area.x as f32 + TEXT_INSET;
        // A file or folder from Open Recent: its name, then where it is.
        let away = match &self.items[index] {
            // Either separator: a Windows path outside home keeps its own.
            Item::Recent(_, label, _) => label.rsplit_once(['/', '\\']),
            _ => None,
        };
        // A long path keeps its end, which names the folder or the file, and loses its
        // middle.
        let shown = match &self.items[index] {
            Item::Folder(..) | Item::File(..) => middle_out(text, f32::from(end) - inset, measure),
            _ => away.map_or(text, |(_, name)| name).to_string(),
        };
        grid.label_at(inset, area.x + 2, y, &shown, base, end, 100);
        // The picked row's band is lighter, so what's quiet on it needs to be lighter too.
        let quiet = Style {
            fg: if picked {
                theme.text.fg
            } else {
                theme.popup_dim
            },
            ..base
        };
        if let Some((folder, _)) = away {
            // A cell after the name as it's drawn, and on to the row's end: no keys here.
            let from = inset + measure.width(&shown, Font::default()) + 1.0;
            let right = area.right().saturating_sub(2);
            let folder = middle_out(folder, f32::from(right) - from, measure);
            grid.label_at(from, from as u16, y, &folder, quiet, right, 100);
        }
        // A task shows what it runs, quietly, when its name doesn't already say.
        if let Item::Task(task) = &self.items[index]
            && task.command != task.label
        {
            let from = inset + measure.width(&shown, Font::default()) + 1.0;
            grid.label_at(from, from as u16, y, &task.command, quiet, end, 100);
        }
        if let Some(keys) = self.keys_at(index) {
            grid.label_right(area.right().saturating_sub(2), y, keys, quiet, 85);
        }
    }
}

impl Palette {
    /// A symbol's row: its kind's icon, its name with the typed letters picked out, what it
    /// sits in, and its group or place at the right.
    #[allow(clippy::too_many_arguments)]
    fn draw_symbol(
        &self,
        grid: &mut Grid,
        (area, y): (Rect, u16),
        symbol: &SymbolRow,
        right: Option<&str>,
        picked: bool,
        measure: &dyn Measure,
        theme: &Theme,
    ) {
        let base = Style {
            fg: theme.text.fg,
            bg: if picked { theme.pick } else { theme.popup },
            ..theme.bar
        };
        // The picked row's wash is lighter, so what's quiet on it needs to be lighter too.
        let quiet = Style {
            fg: if picked {
                theme.text.fg
            } else {
                theme.popup_dim
            },
            ..base
        };
        let (icon, fallback, colour) = kind_icon(symbol.kind, theme);
        let inset = area.x as f32 + TEXT_INSET;
        let icon_style = Style { fg: colour, ..base };
        grid.glyph(inset, y, icon, colour, fallback, icon_style);
        let right_width = right.map_or(0, |r| r.chars().count() as u16 + 3);
        let end = area.right().saturating_sub(right_width.max(2));
        // Clear of the icon, as the field's text is of its search icon.
        let x = inset + 2.3;
        let cell = area.x + 4;
        let font = Font {
            size: INTERFACE_SIZE_PERCENT,
            ..Font::default()
        };
        let hits = matched_chars(self.typed(), &symbol.name);
        let chars: Vec<char> = symbol.name.chars().collect();
        let (mut at_px, mut at_cell, mut i) = (x, cell, 0);
        while i < chars.len() {
            let hit = hits.contains(&i);
            let run_end = (i..chars.len())
                .find(|j| hits.contains(j) != hit)
                .unwrap_or(chars.len());
            let run: String = chars[i..run_end].iter().collect();
            let style = if hit {
                Style {
                    fg: theme.accent,
                    bold: true,
                    ..base
                }
            } else {
                base
            };
            grid.label_at(at_px, at_cell, y, &run, style, end, INTERFACE_SIZE_PERCENT);
            at_px += measure.width(&run, Font { bold: hit, ..font });
            at_cell += (run_end - i) as u16;
            i = run_end;
        }
        if !symbol.detail.is_empty() {
            let gap = 1.0;
            grid.label_at(at_px + gap, at_cell + 1, y, &symbol.detail, quiet, end, 85);
        }
        if let Some(right) = right {
            grid.label_right(area.right().saturating_sub(2), y, right, quiet, 85);
        }
    }
}

/// `text` with characters taken from its middle until it fits `room` cells.
pub fn middle_out(text: &str, room: f32, measure: &dyn Measure) -> String {
    let font = Font {
        size: INTERFACE_SIZE_PERCENT,
        ..Font::default()
    };
    if measure.width(text, font) <= room {
        return text.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    let mut keep = chars.len();
    while keep > 2 {
        keep -= 1;
        let head = keep / 3;
        let tail = keep - head;
        let tried: String = chars[..head]
            .iter()
            .chain(std::iter::once(&'…'))
            .chain(&chars[chars.len() - tail..])
            .collect();
        if measure.width(&tried, font) <= room {
            return tried;
        }
    }
    "…".to_string()
}

/// What a text box shows.
pub struct Input<'a> {
    pub text: &'a str,
    pub hint: &'a str,
    pub focused: bool,
    /// The colour behind the box's rounded corners.
    pub behind: Rgb,
}

/// A one-line text box. Text, placeholder and caret share one origin, and the caret comes
/// from measuring the text, so it can't drift out of the box or onto the words.
pub fn input(grid: &mut Grid, line: Rect, field: &Input, measure: &dyn Measure, theme: &Theme) {
    /// Space between the box's edge and its text: a column.
    const PADDING: f32 = 1.0;
    let style = Style {
        fg: theme.text.fg,
        bg: theme.input,
        ..theme.sidebar
    };
    grid.fill(line, style);
    let border = if field.focused {
        theme.focus
    } else {
        theme.popup_border
    };
    grid.field(line, field.behind, theme.input, border);
    let (x, y) = (line.x as f32 + PADDING, line.y);
    let first = line.x + 1;
    if field.text.is_empty() {
        let dim = Style {
            fg: theme.gutter.fg,
            ..style
        };
        grid.label_at(x, first, y, field.hint, dim, line.right(), 95);
    } else {
        grid.label_at(x, first, y, field.text, style, line.right(), 95);
    }
    if field.focused {
        let font = Font {
            size: 95,
            ..Font::default()
        };
        // Just clear of the first letter, when there's only a placeholder.
        let end = x + measure.width(field.text, font) - 0.1;
        let last = line.right() as f32 - PADDING;
        grid.set_caret(end.min(last), y, 0.56);
    }
}

/// A floating panel with room above and below its rows. Paint reaches past the cells, so the
/// cell tier keeps its plain box.
pub fn raised(grid: &mut Grid, area: Rect, theme: &Theme) {
    let base = Style {
        fg: theme.text.fg,
        bg: theme.popup,
        ..theme.bar
    };
    grid.fill(area, base);
    let bounds = Bounds::inset(area, 0.0, -PANEL_PADDING);
    grid.popup(bounds, 28, theme.popup, theme.popup_border);
}

/// How far a bar that holds a text box reaches past its rows, in rows: a tenth, which
/// doubles the air around its box and stays clear of the letters in the rows beside it. A
/// quarter covered the tops of the code under the Find bar.
const BAR_AIR: f32 = 0.1;

/// A floating bar around a text box or two, as the Find bar and a prompt are: `raised`, with
/// a little air past its rows above and below, which only paint can show.
pub fn raised_bar(grid: &mut Grid, area: Rect, theme: &Theme) {
    let base = Style {
        fg: theme.text.fg,
        bg: theme.popup,
        ..theme.bar
    };
    grid.fill(area, base);
    let bounds = Bounds::inset(area, 0.0, -BAR_AIR);
    grid.popup(bounds, 28, theme.popup, theme.popup_border);
}

/// The size of a sidebar's title, a little under the text's.
const TITLE_SIZE: u8 = 85;

/// The font of a sidebar's title, for measuring what follows it.
pub fn title_font() -> Font {
    Font {
        bold: true,
        size: TITLE_SIZE,
        ..Font::default()
    }
}

/// A sidebar's title on the row `row`: its name in capitals, small, quiet and two columns
/// in, the same in every sidebar. Returns the cell after it.
pub fn sidebar_title(grid: &mut Grid, row: Rect, title: &str, theme: &Theme) -> u16 {
    let style = Style {
        fg: theme.tab.fg,
        bold: true,
        ..theme.sidebar
    };
    let x = row.x + 2;
    let shown = title.to_uppercase();
    grid.label_at(
        f32::from(x),
        x,
        row.y,
        &shown,
        style,
        row.right(),
        TITLE_SIZE,
    )
}

/// The size of a section's heading, smaller than a sidebar's title.
const SECTION_SIZE: u8 = 80;

/// A section's heading in a sidebar or on a page: its name in capitals, small, bold and
/// quiet, one column in from the panel's title, a chevron before it when it folds, and a
/// count at the right. Source Control, the Timeline and Agent Context share it, as the Task page's
/// headings look, so a section reads the same wherever it is, as VS Code's do.
pub fn section_heading(
    grid: &mut Grid,
    (left, right, y): (u16, u16, u16),
    (name, count): (&str, Option<(&str, Rgb)>),
    folded: Option<bool>,
    theme: &Theme,
) {
    let quiet = Style {
        fg: theme.gutter.fg,
        bold: true,
        ..theme.sidebar
    };
    if let Some(folded) = folded {
        let (icon, cells) = if folded {
            (Icon::ChevronRight, "›")
        } else {
            (Icon::ChevronDown, "⌄")
        };
        let plain = Style {
            bold: false,
            ..quiet
        };
        // A column and three quarters before the name, as Agent Context has it: more than
        // 8 px clear of the sidebar's edge line, its cell never past it.
        let x = f32::from(left) - 1.75;
        grid.glyph(x, y, icon, quiet.fg, cells, plain);
    }
    grid.label_at(f32::from(left), left, y, name, quiet, right, SECTION_SIZE);
    if let Some((count, fg)) = count {
        let number = Style {
            fg,
            bold: false,
            ..quiet
        };
        grid.label_right(right, y, count, number, 85);
    }
}

/// A floating panel. Its corner cells keep what's behind them, so rounded corners show the
/// page rather than a square of panel colour.
pub fn popup(grid: &mut Grid, area: Rect, theme: &Theme) {
    let base = Style {
        fg: theme.text.fg,
        bg: theme.popup,
        ..theme.bar
    };
    for y in area.y..area.bottom() {
        let edge = y == area.y || y + 1 == area.bottom();
        let (x, w) = if edge {
            (area.x + 1, area.w.saturating_sub(2))
        } else {
            (area.x, area.w)
        };
        grid.fill(Rect::new(x, y, w, 1), base);
    }
    let bounds = cellix_render::Bounds::inset(area, 0.0, 0.0);
    grid.popup(bounds, 28, theme.popup, theme.popup_border);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The status at the field's right end gives way to a long query instead of covering it.
    #[test]
    fn a_long_query_keeps_clear_of_the_status() {
        let field = |query: &str| {
            let mut palette = Palette::new(Vec::new());
            palette.query = query.to_string();
            palette.status = Some("8,310 files…".to_string());
            let mut grid = Grid::new(100, 20, Style::default());
            palette.draw(
                &mut grid,
                None,
                &cellix_render::CellMeasure,
                &Theme::default(),
            );
            grid.row_text(Palette::area(grid.area()).y)
        };
        let short = field("needle");
        assert!(
            short.contains("needle") && short.contains("8,310 files…"),
            "{short}"
        );
        // Fits the field, but reaches where the status sits.
        let long = "src/a/long/path/that/someone/pasted/in/it/needle.rs";
        let row = field(long);
        assert!(row.contains(long), "the query stays whole: {row}");
        assert!(!row.contains("files…"), "and the status gives way: {row}");
    }

    #[test]
    fn a_place_after_a_path_is_read_however_its_written() {
        let at = |line, column| Some(Place { line, column });
        assert_eq!(path_and_place("src/app.rs"), ("src/app.rs", None));
        assert_eq!(
            path_and_place("src/app.rs:120"),
            ("src/app.rs", at(120, None))
        );
        assert_eq!(
            path_and_place("src/app.rs:120:8"),
            ("src/app.rs", at(120, Some(8)))
        );
        // A compiler's, with the colon its message follows, and a range.
        assert_eq!(
            path_and_place("src/app.rs:120:8:"),
            ("src/app.rs", at(120, Some(8)))
        );
        assert_eq!(
            path_and_place("src/app.rs:120-140"),
            ("src/app.rs", at(120, None))
        );
        assert_eq!(path_and_place("app.rs#L120"), ("app.rs", at(120, None)));
        assert_eq!(
            path_and_place("app.rs#L120-L140"),
            ("app.rs", at(120, None))
        );
        assert_eq!(
            path_and_place("app.rs#L120C8"),
            ("app.rs", at(120, Some(8)))
        );
        assert_eq!(
            path_and_place("app.rs(120,8)"),
            ("app.rs", at(120, Some(8)))
        );
        assert_eq!(path_and_place("app.rs(120)"), ("app.rs", at(120, None)));
        // A colon on its way to a line isn't part of the path, and a line alone has none.
        assert_eq!(path_and_place("src/app.rs:"), ("src/app.rs", None));
        assert_eq!(path_and_place(":42"), ("", at(42, None)));
        assert_eq!(
            path_and_place(" src/app.rs:7 "),
            ("src/app.rs", at(7, None))
        );
        // What only looks like one is left in the path.
        assert_eq!(path_and_place("C:\\src\\app.rs"), ("C:\\src\\app.rs", None));
        assert_eq!(
            path_and_place("C:\\src\\app.rs:9"),
            ("C:\\src\\app.rs", at(9, None))
        );
        assert_eq!(path_and_place("notes (2).md"), ("notes (2).md", None));
        assert_eq!(path_and_place("fn(x)"), ("fn(x)", None));
        assert_eq!(path_and_place("a#Label"), ("a#Label", None));
    }

    #[test]
    fn a_pasted_path_loses_what_it_was_copied_with() {
        for (pasted, path) in [
            ("src/app.rs\n", "src/app.rs"),
            ("  src/app.rs:120  \n\n", "src/app.rs:120"),
            ("`src/app.rs`", "src/app.rs"),
            ("\"src/my app.rs\"", "src/my app.rs"),
            ("'src/app.rs',", "src/app.rs"),
            ("(src/app.rs:12).", "src/app.rs:12"),
            ("src/app.rs).", "src/app.rs"),
            ("<src/app.rs>", "src/app.rs"),
            (
                "[app.rs](crates/cellix/src/app.rs)",
                "crates/cellix/src/app.rs",
            ),
            ("@src/app.rs", "src/app.rs"),
            (
                "file:///Users/me/my%20notes/a.md",
                "/Users/me/my notes/a.md",
            ),
            ("--> src/app.rs:120:8", "src/app.rs:120:8"),
            (
                "src/app.rs:120:8: error[E0308]: mismatched types",
                "src/app.rs:120:8",
            ),
            ("src/app.rs:120:    let x = 1;", "src/app.rs:120"),
            (
                "  File \"tools/run.py\", line 12, in main",
                "tools/run.py:12",
            ),
            ("first.rs\nsecond.rs\n", "first.rs"),
            // A symbol's `@` stays, and so does a name with a space or a dot in it.
            ("@push", "@push"),
            ("my notes.md", "my notes.md"),
            ("", ""),
        ] {
            assert_eq!(pasted_path(pasted), path, "{pasted:?}");
        }
    }

    /// A file the query names outright is listed first, once, found by the walk or not.
    #[test]
    fn the_file_a_query_names_is_listed_first_and_once() {
        let file = |path: &str| Item::File(PathBuf::from("/w").join(path), path.to_string());
        let mut palette = Palette::new(vec![file("src/app.rs"), file("vendor/src/app.rs")]);
        palette.query = "src/app.rs:12".to_string();
        palette.seek(Sought {
            needle: "src/app.rs".to_string(),
            place: Some(Place {
                line: 12,
                column: None,
            }),
            direct: Some(file("src/app.rs")),
            alone: false,
        });
        palette.filter();
        let labels = |palette: &Palette| -> Vec<String> {
            let rows = palette.matches.iter();
            rows.map(|i| palette.items[*i].label().to_string())
                .collect()
        };
        assert_eq!(labels(&palette), ["src/app.rs", "vendor/src/app.rs"]);
        assert_eq!(palette.len(), 2, "the row kept for it isn't another file");
        // The walk finding it afterwards doesn't list it again.
        palette.extend(vec![file("src/app.rs"), file("src/app.rs.orig")]);
        assert_eq!(
            labels(&palette),
            ["src/app.rs", "src/app.rs.orig", "vendor/src/app.rs"]
        );
        // Something else typed, and the row kept for it is out of the list.
        palette.query = "vendor".to_string();
        palette.seek(Sought {
            needle: "vendor".to_string(),
            ..Sought::default()
        });
        palette.filter();
        assert_eq!(labels(&palette), ["vendor/src/app.rs"]);
        assert_eq!(palette.place, None);
    }

    #[test]
    fn subsequence_required() {
        assert!(fuzzy_score("xyz", "abc").is_none());
        assert!(fuzzy_score("", "abc").is_some());
        assert!(fuzzy_score("abc", "a-b-c").is_some());
    }

    #[test]
    fn typed_symbols_put_the_best_match_first_and_count_what_matches() {
        let symbol = |name: &str, kind: u64| {
            Item::Symbol(Box::new(SymbolRow {
                name: name.to_string(),
                kind,
                detail: String::new(),
                place: String::new(),
                path: PathBuf::from("queue.rs"),
                line: 0,
                column: 0,
            }))
        };
        let items = vec![
            symbol("Queue", 23),
            symbol("impl Queue", 19),
            symbol("push", 6),
        ];
        let mut palette = Palette::symbols(items, '@', "", "Go to symbol");
        // Kinds in VS Code's order, a gap between each.
        assert_eq!(palette.current().map(Item::label), Some("Queue"));
        assert_eq!(palette.matches.len(), 5);
        assert_eq!(palette.status.as_deref(), Some("3 symbols"));
        palette.move_by(1);
        assert_eq!(palette.current().map(Item::label), Some("impl Queue"));
        // "push" matches "pu" better than "impl Queue" does, so its kind comes first.
        palette.query = "@pu".to_string();
        palette.filter();
        assert_eq!(palette.current().map(Item::label), Some("push"));
        assert_eq!(palette.status.as_deref(), Some("2 of 3"));
    }

    #[test]
    fn the_keyboard_steps_over_a_gap() {
        let items = vec![
            Item::Branch("main".to_string(), "current".to_string()),
            Item::Gap,
            Item::NewBranch,
        ];
        let mut palette = Palette::asking(items, "Switch branch");
        palette.move_by(1);
        assert_eq!(palette.current(), Some(&Item::NewBranch));
        palette.query = "new".to_string();
        palette.filter();
        assert_eq!(
            palette.current(),
            Some(&Item::NewBranch),
            "gaps don't match"
        );
    }

    #[test]
    fn a_command_that_starts_with_what_was_typed_comes_before_one_that_holds_it() {
        let items = COMMANDS
            .iter()
            .map(|(label, command)| Item::Command(label, *command))
            .collect();
        let mut palette = Palette::new(items);
        palette.query = "op".to_string();
        palette.filter();
        let labels: Vec<&str> = palette
            .matches
            .iter()
            .map(|index| palette.items[*index].label())
            .collect();
        assert!(labels[0].starts_with("Open"), "{labels:?}");
        // Every Open… stands above the commands that only hold the letters, like Pop Stash.
        let last_open = labels.iter().rposition(|l| l.starts_with("Open")).unwrap();
        let pop = labels.iter().position(|l| l.starts_with("Pop Stash"));
        assert!(pop.is_some_and(|pop| last_open < pop), "{labels:?}");
        // A later word that starts with it counts too: "stash" finds Pop Stash before a
        // command with those letters scattered through it.
        assert!(fuzzy_score("stash", "Pop Stash…") > fuzzy_score("stash", "Stage All and Push"));
    }

    #[test]
    fn file_name_beats_directory() {
        let by_name = fuzzy_score("app", "src/app.rs").unwrap();
        let by_dir = fuzzy_score("app", "app/src/main.rs").unwrap();
        assert!(by_name > by_dir);
    }
}
