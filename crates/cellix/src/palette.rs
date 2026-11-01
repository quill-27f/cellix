use std::path::PathBuf;

use cellix_input::Command;
use cellix_render::{Grid, Rect, Style};

use crate::theme::Theme;

const WIDTH: u16 = 64;
const ROWS: u16 = 12;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Item {
    File(PathBuf, String),
    Command(&'static str, Command),
    Completion(String),
}

impl Item {
    fn label(&self) -> &str {
        match self {
            Self::File(_, label) => label,
            Self::Command(label, _) => label,
            Self::Completion(label) => label,
        }
    }
}

pub const COMMANDS: [(&str, Command); 21] = [
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

    pub fn draw(&self, grid: &mut Grid, theme: &Theme) {
        let area = Self::area(grid.area());
        let rows = self.matches.len().min(ROWS as usize) as u16;
        let area = Rect::new(area.x, area.y, area.w, rows + 1);
        popup(grid, area, theme);
        let base = Style {
            fg: theme.text.fg,
            bg: theme.popup,
            ..theme.bar
        };
        let input = Rect::new(area.x + 1, area.y, area.w.saturating_sub(2), 1);
        grid.field(input, theme.popup, theme.input, theme.focus);
        let prompt = format!(" > {}", self.query);
        let hint = if self.query.is_empty() {
            "> "
        } else {
            prompt.as_str()
        };
        grid.put_str(input.x, input.y, hint, base, input.right());
        let end = input.x + prompt.chars().count() as u16;
        grid.cursor = Some((end.min(input.right().saturating_sub(1)), area.y));
        for (row, index) in self.matches.iter().take(ROWS as usize).enumerate() {
            let y = area.y + 1 + row as u16;
            if row == self.selected {
                let line = Rect::new(area.x, y, area.w, 1);
                grid.panel(
                    cellix_render::Bounds::inset(line, 0.5, 0.04),
                    18,
                    Some(theme.pick),
                    None,
                );
            }
            let label = format!("   {}", self.items[*index].label());
            let text = label.trim_start();
            grid.put_str(area.x, y, "  ", base, area.right());
            grid.label_at(
                area.x as f32 + 2.0,
                area.x + 3,
                y,
                text,
                base,
                area.right().saturating_sub(1),
                100,
            );
        }
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
