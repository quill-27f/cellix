use std::collections::HashMap;
use std::path::PathBuf;

use cellix_input::{Command, Format};
use cellix_render::{Bounds, Font, Grid, Icon, Measure, Rect, Rgb, Style};

use crate::theme::Theme;

const WIDTH: u16 = 64;
/// Text in the search field, as a percentage of the interface font.
const INTERFACE_SIZE_PERCENT: u8 = 100;
const ROWS: u16 = 12;
/// Space from the panel's edge to the icon and the text of the list, which line up.
const TEXT_INSET: f32 = 1.4;
/// How far a floating panel reaches past its rows, so nothing sits on its edge.
const PANEL_PADDING: f32 = 0.3;
/// Rows above the list: the search field, and a row holding the line under it.
const HEADER_ROWS: u16 = 2;
/// Where that line falls, as far below the field's text as the panel's edge is above it.
const HEADER_LINE: f32 = 1.3;
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
            | Self::Folder(_, label, _)
            | Self::Recent(_, label, _) => label,
            Self::Command(label, _) => label,
            Self::Task(task) => &task.label,
            Self::Header(name) => name,
            Self::Branch(name, _) | Self::Citation(name, _) | Self::Git(name, _, _) => name,
            Self::Symbol(symbol) => &symbol.name,
            Self::NewBranch => "New branch…",
            Self::Gap => "",
        }
    }

    /// What sits at the right of the row.
    fn keys(&self) -> Option<&str> {
        match self {
            Self::File(..) | Self::Image(..) | Self::NewBranch | Self::Gap | Self::Recent(..) => {
                None
            }
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
        Command::SplitPane => "Ctrl+\\",
        Command::Quit => "Ctrl+Q",
        _ => return None,
    })
}

pub const COMMANDS: [(&str, Command); 117] = [
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
    ("Toggle Terminal", Command::ToggleTerminal),
    ("New Terminal", Command::NewTerminal),
    ("Show Comments", Command::ShowComments),
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
    /// What the empty field says.
    hint: &'static str,
    items: Vec<Item>,
    matches: Vec<usize>,
    /// Each match's score, beside it, while something is typed.
    scores: Vec<i32>,
    /// What the matches were found for: a longer query only needs to look among them.
    matched: String,
    pub selected: usize,
    /// What's still happening, as "8,310 files…", shown at the right of the field.
    pub status: Option<String>,
    /// Set when the palette finds a folder, which lists again as a path is typed.
    pub folders: Option<crate::folders::Picker>,
    /// What starts the query and isn't part of the search: Quick Open's `@` for this file's
    /// symbols and `#` for the workspace's.
    pub prefix: Option<char>,
    /// Whether rows are kept together by kind, each group labelled with its size.
    grouped: bool,
    /// Quick Open, where `@` or `#` at the start turns to symbols and taking it away turns
    /// back.
    pub quick: bool,
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
/// consecutive matches and matches in the file name score higher.
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
        last = Some(found + c.len_utf8());
    }
    Some(score - text.len() as i32 / 16)
}

/// Whether `c` is `lower`, whatever its case.
fn same_letter(c: char, lower: char) -> bool {
    c == lower || (!c.is_ascii() || c.is_ascii_uppercase()) && c.to_lowercase().eq([lower])
}

impl Palette {
    pub fn new(items: Vec<Item>) -> Self {
        let mut palette = Self {
            query: String::new(),
            hint: "Type to search",
            items,
            matches: Vec::new(),
            scores: Vec::new(),
            matched: String::new(),
            selected: 0,
            status: None,
            folders: None,
            prefix: None,
            grouped: false,
            quick: false,
        };
        palette.filter();
        palette
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
        self.matched.clear();
        self.filter();
        if let Some(picked) = picked {
            let index = self.items.iter().position(|item| *item == picked);
            if let Some(row) = index.and_then(|i| self.matches.iter().position(|&m| m == i)) {
                self.selected = row;
            }
        }
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
        let from = self.items.len();
        self.items.extend(items);
        let typed = self.typed().to_string();
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
        for i in from..self.items.len() {
            if let Some(score) = fuzzy_score(&typed, self.items[i].label()) {
                scored.push((score, i));
            }
        }
        scored.sort_by_key(|(score, i)| (-score, *i));
        (self.scores, self.matches) = scored.into_iter().unzip();
        if let Some(picked) = picked {
            self.selected = self.matches.iter().position(|&i| i == picked).unwrap_or(0);
        }
    }

    /// How many items it holds.
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.items.len()
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

    pub fn filter(&mut self) {
        let typed = self.typed().to_string();
        // Nothing typed means no ranking: the list keeps the order it came in.
        if typed.is_empty() {
            self.matches = (0..self.items.len()).collect();
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
            .filter(|&i| !self.items[i].divider())
            .filter_map(|i| Some((fuzzy_score(&typed, self.items[i].label())?, i)))
            .collect();
        scored.sort_by_key(|(score, i)| (-score, *i));
        (self.scores, self.matches) = scored.into_iter().unzip();
        self.matched = typed;
        self.selected = 0;
        self.group();
    }

    pub fn move_by(&mut self, delta: isize) {
        let last = self.matches.len().saturating_sub(1) as isize;
        self.selected = (self.selected as isize + delta).clamp(0, last) as usize;
        if self.current().is_some_and(Item::divider) {
            self.selected = (self.selected as isize + delta).clamp(0, last) as usize;
        }
        // Two dividers together, or one at the end: back the other way.
        if self.current().is_some_and(Item::divider) {
            self.selected = (self.selected as isize - delta).clamp(0, last) as usize;
        }
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

    pub fn row_at(&self, screen: Rect, y: u16) -> Option<usize> {
        let row = y.checked_sub(Self::area(screen).y + HEADER_ROWS)? as usize;
        (row < self.matches.len().min(ROWS as usize)).then_some(row)
    }

    pub fn draw(
        &self,
        grid: &mut Grid,
        hover: Option<(u16, u16)>,
        measure: &dyn Measure,
        theme: &Theme,
    ) {
        let area = Self::area(grid.area());
        let rows = self.matches.len().min(ROWS as usize) as u16;
        let area = Rect::new(area.x, area.y, area.w, rows + HEADER_ROWS);
        raised(grid, area, theme);
        let base = Style {
            fg: theme.text.fg,
            bg: theme.popup,
            ..theme.bar
        };
        let dim = Style {
            fg: theme.popup_dim,
            ..base
        };
        grid.glyph(
            area.x as f32 + TEXT_INSET,
            area.y,
            Icon::Search,
            theme.popup_dim,
            "⌕",
            base,
        );
        // Clear of the icon. The hint waits a little past the caret, so they don't touch.
        let x = area.x as f32 + TEXT_INSET + 2.3;
        let cell = area.x + 4;
        let font = Font {
            size: INTERFACE_SIZE_PERCENT,
            ..Font::default()
        };
        let caret = x - 0.1 + measure.width(&self.query, font);
        // The status keeps clear of what's typed, and gives way to it when both don't fit:
        // the query is what the person is working on.
        let status_right = area.right().saturating_sub(2);
        let status = self.status.as_deref().map(|text| {
            let width = measure.width(
                text,
                Font {
                    size: 85,
                    ..Font::default()
                },
            );
            (text, status_right as f32 - width)
        });
        let status = status.filter(|(_, left)| caret + STATUS_GAP <= *left);
        let end = status.map_or(area.right(), |(_, left)| (left - STATUS_GAP / 2.0) as u16);
        if self.query.is_empty() {
            grid.label_at(x + HINT_GAP, cell, area.y, self.hint, dim, end, 100);
        } else {
            grid.label_at(x, cell, area.y, &self.query, base, end, 100);
        }
        if let Some((text, _)) = status {
            grid.label_right(status_right, area.y, text, dim, 85);
        }
        grid.set_caret(caret, area.y, 0.56);
        let line_y = area.y as f32 + HEADER_LINE;
        let edge = Bounds::cells(area.x as f32, line_y, area.w as f32, 0.0);
        grid.hairline(edge, theme.popup_border);
        for (row, index) in self.matches.iter().take(ROWS as usize).enumerate() {
            let y = area.y + HEADER_ROWS + row as u16;
            let line = Rect::new(area.x, y, area.w, 1);
            if self.items[*index] == Item::Gap {
                let across = area.w as f32 - TEXT_INSET * 2.0;
                let rule = Bounds::cells(area.x as f32 + TEXT_INSET, y as f32 + 0.5, across, 0.0);
                grid.hairline(rule, theme.popup_border);
                continue;
            }
            if let Item::Header(name) = &self.items[*index] {
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
                continue;
            }
            let hovered = hover.is_some_and(|h| line.contains(h.0, h.1));
            if row == self.selected || hovered {
                let fill = if row == self.selected {
                    theme.pick
                } else {
                    theme.popup_hover
                };
                grid.panel(Bounds::inset(line, 0.5, 0.055), 18, Some(fill), None);
            }
            if let Item::Symbol(symbol) = &self.items[*index] {
                let group = self.group_label(row);
                let right = group.as_deref().or(Some(symbol.place.as_str()));
                let right = right.filter(|r| !r.is_empty());
                let picked = row == self.selected;
                self.draw_symbol(grid, (area, y), symbol, right, picked, measure, theme);
                continue;
            }
            let text = self.items[*index].label();
            let end = area.right().saturating_sub(16);
            let inset = area.x as f32 + TEXT_INSET;
            // A file or folder from Open Recent: its name, then where it is.
            let away = match &self.items[*index] {
                Item::Recent(_, label, _) => label.rsplit_once('/'),
                _ => None,
            };
            // A long path keeps its end, which names the folder, and loses its middle.
            let shown = match &self.items[*index] {
                Item::Folder(..) => middle_out(text, f32::from(end) - inset, measure),
                _ => away.map_or(text, |(_, name)| name).to_string(),
            };
            grid.label_at(inset, area.x + 2, y, &shown, base, end, 100);
            if let Some((folder, _)) = away {
                let quiet = Style {
                    fg: theme.popup_dim,
                    ..base
                };
                // A cell after the name as it's drawn, and on to the row's end: no keys here.
                let from = inset + measure.width(&shown, Font::default()) + 1.0;
                let right = area.right().saturating_sub(2);
                let folder = middle_out(folder, f32::from(right) - from, measure);
                grid.label_at(from, from as u16, y, &folder, quiet, right, 100);
            }
            // A task shows what it runs, quietly, when its name doesn't already say.
            if let Item::Task(task) = &self.items[*index]
                && task.command != task.label
            {
                let quiet = Style {
                    fg: theme.popup_dim,
                    ..base
                };
                let from = inset + measure.width(&shown, Font::default()) + 1.0;
                grid.label_at(from, from as u16, y, &task.command, quiet, end, 100);
            }
            if let Some(keys) = self.items[*index].keys() {
                // The picked row's wash is lighter, so its keys need to be too.
                let fg = if row == self.selected {
                    theme.text.fg
                } else {
                    theme.popup_dim
                };
                let quiet = Style { fg, ..base };
                grid.label_right(area.right().saturating_sub(2), y, keys, quiet, 85);
            }
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
            bg: theme.popup,
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
    /// Space between the box's edge and its text.
    const PADDING: f32 = 0.8;
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
    fn subsequence_required() {
        assert!(fuzzy_score("xyz", "abc").is_none());
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
    fn file_name_beats_directory() {
        let by_name = fuzzy_score("app", "src/app.rs").unwrap();
        let by_dir = fuzzy_score("app", "app/src/main.rs").unwrap();
        assert!(by_name > by_dir);
    }
}
