use std::path::PathBuf;

use cellix_input::{Command, Format};
use cellix_render::{Bounds, Font, Grid, Icon, Measure, Rect, Rgb, Style};

use crate::theme::Theme;

const WIDTH: u16 = 64;
/// Text in the search field, as a percentage of the interface font.
const INTERFACE_SIZE_PERCENT: u8 = 100;
const ROWS: u16 = 12;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Item {
    File(PathBuf, String),
    Command(&'static str, Command),
}

impl Item {
    fn label(&self) -> &str {
        match self {
            Self::File(_, label) => label,
            Self::Command(label, _) => label,
        }
    }

    fn keys(&self) -> Option<&'static str> {
        match self {
            Self::File(..) => None,
            Self::Command(_, command) => keys_of(*command),
        }
    }
}

/// A command's usual keys, shown beside it.
fn keys_of(command: Command) -> Option<&'static str> {
    Some(match command {
        Command::Save => "Ctrl+S",
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
        Command::RenameSymbol => "F2",
        Command::AddComment => "Alt+C",
        Command::SplitPane => "Ctrl+\\",
        Command::Quit => "Ctrl+Q",
        _ => return None,
    })
}

pub const COMMANDS: [(&str, Command); 45] = [
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
    ("Review Previous Agent Turn", Command::ReviewPreviousTurn),
    ("Undo Agent Turn", Command::UndoTurn),
    ("Commit Staged Changes", Command::Commit),
    ("Open Claude Code", Command::OpenClaude),
    ("Open Codex", Command::OpenCodex),
    ("Go to Definition", Command::GoToDefinition),
    ("Complete", Command::Complete),
    ("Rename Symbol", Command::RenameSymbol),
    ("Quit", Command::Quit),
];

/// Quick open and the command palette: a filtered list with a text box.
pub struct Palette {
    pub query: String,
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
            items,
            matches: Vec::new(),
            selected: 0,
        };
        palette.filter();
        palette
    }

    pub fn filter(&mut self) {
        let mut scored: Vec<(i32, usize)> = self
            .items
            .iter()
            .enumerate()
            .filter_map(|(i, item)| Some((fuzzy_score(&self.query, item.label())?, i)))
            .collect();
        scored.sort_by_key(|(score, i)| (-score, *i));
        self.matches = scored.into_iter().map(|(_, i)| i).collect();
        self.selected = 0;
    }

    pub fn move_by(&mut self, delta: isize) {
        let last = self.matches.len().saturating_sub(1) as isize;
        self.selected = (self.selected as isize + delta).clamp(0, last) as usize;
    }

    pub fn current(&self) -> Option<&Item> {
        self.items.get(*self.matches.get(self.selected)?)
    }

    pub fn area(screen: Rect) -> Rect {
        let width = WIDTH.min(screen.w);
        Rect::new(screen.x + (screen.w - width) / 2, 2, width, ROWS + 1)
    }

    pub fn row_at(&self, screen: Rect, y: u16) -> Option<usize> {
        let row = y.checked_sub(Self::area(screen).y + 1)? as usize;
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
        let area = Rect::new(area.x, area.y, area.w, rows + 1);
        popup(grid, area, theme);
        let base = Style {
            fg: theme.text.fg,
            bg: theme.popup,
            ..theme.bar
        };
        let dim = Style {
            fg: theme.status.fg,
            ..base
        };
        grid.glyph(
            area.x as f32 + 1.2,
            area.y,
            Icon::Search,
            theme.status.fg,
            "⌕",
            base,
        );
        let x = area.x + 3;
        if self.query.is_empty() {
            grid.label(x, area.y, "Type to search", dim, area.right());
        } else {
            grid.label(x, area.y, &self.query, base, area.right());
        }
        let font = Font {
            size: INTERFACE_SIZE_PERCENT,
            ..Font::default()
        };
        grid.set_caret(x as f32 + measure.width(&self.query, font), area.y, 0.56);
        let edge = Bounds::cells(area.x as f32, area.y as f32 + 1.0, area.w as f32, 0.0);
        grid.hairline(edge, theme.popup_border);
        for (row, index) in self.matches.iter().take(ROWS as usize).enumerate() {
            let y = area.y + 1 + row as u16;
            let line = Rect::new(area.x, y, area.w, 1);
            let hovered = hover.is_some_and(|h| line.contains(h.0, h.1));
            if row == self.selected || hovered {
                let fill = if row == self.selected {
                    theme.pick
                } else {
                    theme.row_hover
                };
                grid.panel(Bounds::inset(line, 0.5, 0.04), 18, Some(fill), None);
            }
            let text = self.items[*index].label();
            let end = area.right().saturating_sub(16);
            grid.label_at(area.x as f32 + 1.5, area.x + 3, y, text, base, end, 100);
            if let Some(keys) = self.items[*index].keys() {
                let quiet = Style {
                    fg: theme.status.fg,
                    ..base
                };
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
    fn file_name_beats_directory() {
        let by_name = fuzzy_score("app", "src/app.rs").unwrap();
        let by_dir = fuzzy_score("app", "app/src/main.rs").unwrap();
        assert!(by_name > by_dir);
    }
}
