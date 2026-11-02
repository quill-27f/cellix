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
}

impl Item {
    fn label(&self) -> &str {
        match self {
            Self::File(_, label) | Self::Image(_, label, _) => label,
            Self::Command(label, _) => label,
            Self::Branch(name, _) | Self::Citation(name, _) => name,
            Self::NewBranch => "New branch…",
            Self::Gap => "",
        }
    }

    /// What sits at the right of the row.
    fn keys(&self) -> Option<&str> {
        match self {
            Self::File(..) | Self::Image(..) | Self::NewBranch | Self::Gap => None,
            Self::Command(_, command) => keys_of(*command),
            Self::Branch(_, when) | Self::Citation(_, when) => Some(when),
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
        Command::FindInProject => "Ctrl+Shift+F",
        Command::OpenDiff => "Ctrl+G",
        Command::ToggleSidebar => "Ctrl+B",
        Command::ToggleComment => "Ctrl+/",
        Command::GoToLine => "Ctrl+L",
        Command::GoToDefinition => "F12",
        Command::GoToReferences => "Shift+F12",
        Command::FormatDocument => "Alt+Shift+F",
        Command::RenameSymbol => "F2",
        Command::AddComment => "Alt+C",
        Command::SplitPane => "Ctrl+\\",
        Command::Quit => "Ctrl+Q",
        _ => return None,
    })
}

pub const COMMANDS: [(&str, Command); 69] = [
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
    ("Show Line in PDF", Command::ShowInPdf),
    ("Find in File", Command::FindInFile),
    ("Toggle Comment", Command::ToggleComment),
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
    ("Show Outline", Command::ShowOutline),
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
    ("Open Settings", Command::OpenSettings),
    ("Switch Branch", Command::SwitchBranch),
    ("Open Gemini CLI", Command::OpenGemini),
    ("Go to Definition", Command::GoToDefinition),
    ("Go to References", Command::GoToReferences),
    ("Format Document", Command::FormatDocument),
    ("Complete", Command::Complete),
    ("Rename Symbol", Command::RenameSymbol),
    ("Quit", Command::Quit),
];

/// Quick open and the command palette: a filtered list with a text box.
pub struct Palette {
    pub query: String,
    /// What the empty field says.
    hint: &'static str,
    items: Vec<Item>,
    matches: Vec<usize>,
    pub selected: usize,
}

/// Scores `text` against `query`, whose characters must appear in order. Runs of
/// consecutive matches and matches in the file name score higher.
pub fn fuzzy_score(query: &str, text: &str) -> Option<i32> {
    let name_start = text.rfind('/').map_or(0, |i| i + 1);
    let mut score = 0;
    let mut last = None;
    let lower = text.to_lowercase();
    let mut from = 0;
    for q in query.to_lowercase().chars() {
        let found = lower[from..].find(q)? + from;
        score += 1;
        if last.is_some_and(|l| l + 1 == found) {
            score += 4;
        }
        if found >= name_start {
            score += 2;
        }
        last = Some(found);
        from = found + q.len_utf8();
    }
    Some(score - text.len() as i32 / 16)
}

impl Palette {
    pub fn new(items: Vec<Item>) -> Self {
        let mut palette = Self {
            query: String::new(),
            hint: "Type to search",
            items,
            matches: Vec::new(),
            selected: 0,
        };
        palette.filter();
        palette
    }

    /// A palette whose empty field asks for something other than a search.
    pub fn asking(items: Vec<Item>, hint: &'static str) -> Self {
        Self {
            hint,
            ..Self::new(items)
        }
    }

    pub fn filter(&mut self) {
        // Nothing typed means no ranking: the list keeps the order it came in.
        if self.query.is_empty() {
            self.matches = (0..self.items.len()).collect();
            self.selected = 0;
            return;
        }
        let mut scored: Vec<(i32, usize)> = self
            .items
            .iter()
            .enumerate()
            .filter(|(_, item)| **item != Item::Gap)
            .filter_map(|(i, item)| Some((fuzzy_score(&self.query, item.label())?, i)))
            .collect();
        scored.sort_by_key(|(score, i)| (-score, *i));
        self.matches = scored.into_iter().map(|(_, i)| i).collect();
        self.selected = 0;
    }

    pub fn move_by(&mut self, delta: isize) {
        let last = self.matches.len().saturating_sub(1) as isize;
        self.selected = (self.selected as isize + delta).clamp(0, last) as usize;
        if self.current() == Some(&Item::Gap) {
            self.selected = (self.selected as isize + delta).clamp(0, last) as usize;
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
        if self.query.is_empty() {
            grid.label_at(
                x + HINT_GAP,
                cell,
                area.y,
                self.hint,
                dim,
                area.right(),
                100,
            );
        } else {
            grid.label_at(x, cell, area.y, &self.query, base, area.right(), 100);
        }
        let font = Font {
            size: INTERFACE_SIZE_PERCENT,
            ..Font::default()
        };
        let caret = x - 0.1 + measure.width(&self.query, font);
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
            let hovered = hover.is_some_and(|h| line.contains(h.0, h.1));
            if row == self.selected || hovered {
                let fill = if row == self.selected {
                    theme.pick
                } else {
                    theme.popup_hover
                };
                grid.panel(Bounds::inset(line, 0.5, 0.055), 18, Some(fill), None);
            }
            let text = self.items[*index].label();
            let end = area.right().saturating_sub(16);
            let inset = area.x as f32 + TEXT_INSET;
            grid.label_at(inset, area.x + 2, y, text, base, end, 100);
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

    #[test]
    fn subsequence_required() {
        assert!(fuzzy_score("xyz", "abc").is_none());
        assert!(fuzzy_score("abc", "a-b-c").is_some());
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
