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

pub const COMMANDS: [(&str, Command); 20] = [
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
        grid.fill(area, theme.bar);
        let prompt = format!(" > {}", self.query);
        let end = grid.put_str(area.x, area.y, &prompt, theme.bar, area.right());
        grid.cursor = Some((end, area.y));
        for (row, index) in self.matches.iter().take(ROWS as usize).enumerate() {
            let y = area.y + 1 + row as u16;
            let style = if row == self.selected {
                Style {
                    bg: theme.selection,
                    ..theme.bar
                }
            } else {
                theme.bar
            };
            grid.fill(Rect::new(area.x, y, area.w, 1), style);
            let label = format!("   {}", self.items[*index].label());
            grid.put_str(area.x, y, &label, style, area.right());
        }
    }
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
