use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, channel};
use std::thread;

use cellix_core::{Match, Workspace};
use cellix_render::{Grid, Icon, Rect, Style};

use crate::theme::Theme;

/// Rows above the results: title, query, replacement, summary.
const HEADER_ROWS: u16 = 4;
const REPLACE_ALL: &str = "All";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    Query,
    Replace,
}

/// What a click in the panel lands on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Hit {
    Field(Field),
    ReplaceAll,
    File(PathBuf),
    Match(Match),
}

enum Row<'a> {
    File(&'a Path, usize),
    Match(&'a Match),
}

/// The Search panel, which takes the sidebar's place as it does in VS Code.
pub struct Search {
    root: PathBuf,
    pub query: String,
    pub replace: String,
    /// Which box has the keyboard, if any.
    pub focus: Option<Field>,
    results: Vec<Match>,
    collapsed: HashSet<PathBuf>,
    top: usize,
    /// Answers from older queries are dropped, so results never run backwards.
    generation: u64,
    incoming: Option<Receiver<(u64, Vec<Match>)>>,
}

impl Search {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            query: String::new(),
            replace: String::new(),
            focus: None,
            results: Vec::new(),
            collapsed: HashSet::new(),
            top: 0,
            generation: 0,
            incoming: None,
        }
    }

    pub fn field_mut(&mut self) -> Option<&mut String> {
        match self.focus? {
            Field::Query => Some(&mut self.query),
            Field::Replace => Some(&mut self.replace),
        }
    }

    /// Searches on a thread, so a large project can't stall typing.
    pub fn start(&mut self) {
        self.generation += 1;
        self.top = 0;
        if self.query.is_empty() {
            self.results.clear();
            self.incoming = None;
            return;
        }
        let (sender, receiver) = channel();
        let (generation, query) = (self.generation, self.query.clone());
        let workspace = Workspace::new(self.root.clone());
        thread::spawn(move || {
            let _ = sender.send((generation, workspace.search(&query)));
        });
        self.incoming = Some(receiver);
    }

    /// Takes finished results. Returns true if the panel changed.
    pub fn poll(&mut self) -> bool {
        let Some(receiver) = &self.incoming else {
            return false;
        };
        let Ok((generation, results)) = receiver.try_recv() else {
            return false;
        };
        self.incoming = None;
        if generation == self.generation {
            self.results = results;
        }
        true
    }

    fn rows(&self) -> Vec<Row<'_>> {
        let mut rows = Vec::new();
        let mut at = 0;
        while at < self.results.len() {
            let path = &self.results[at].path;
            let count = self.results[at..]
                .iter()
                .take_while(|m| m.path == *path)
                .count();
            rows.push(Row::File(path, count));
            if !self.collapsed.contains(path) {
                rows.extend(self.results[at..at + count].iter().map(Row::Match));
            }
            at += count;
        }
        rows
    }

    pub fn toggle_file(&mut self, path: PathBuf) {
        if !self.collapsed.remove(&path) {
            self.collapsed.insert(path);
        }
    }

    pub fn scroll_by(&mut self, delta: isize) {
        let last = self.rows().len().saturating_sub(1) as isize;
        self.top = (self.top as isize + delta).clamp(0, last.max(0)) as usize;
    }

    pub fn hit(&self, area: Rect, x: u16, y: u16) -> Option<Hit> {
        match y.checked_sub(area.y)? {
            1 => Some(Hit::Field(Field::Query)),
            2 if x + REPLACE_ALL.len() as u16 + 2 >= area.right() => Some(Hit::ReplaceAll),
            2 => Some(Hit::Field(Field::Replace)),
            row if row >= HEADER_ROWS => {
                let index = self.top + (row - HEADER_ROWS) as usize;
                match self.rows().get(index)? {
                    Row::File(path, _) => Some(Hit::File(path.to_path_buf())),
                    Row::Match(found) => Some(Hit::Match((*found).clone())),
                }
            }
            _ => None,
        }
    }

    fn draw_box(&self, grid: &mut Grid, area: Rect, y: u16, field: Field, theme: &Theme) {
        let (text, hint) = match field {
            Field::Query => (&self.query, "Search"),
            Field::Replace => (&self.replace, "Replace"),
        };
        let end = area.right().saturating_sub(1);
        let width = if field == Field::Replace {
            end.saturating_sub(area.x + 5)
        } else {
            end.saturating_sub(area.x + 1)
        };
        let input = Style {
            bg: theme.input,
            ..theme.sidebar
        };
        let line = Rect::new(area.x + 1, y, width, 1);
        grid.fill(line, input);
        let border = if self.focus == Some(field) {
            theme.focus
        } else {
            theme.popup_border
        };
        grid.field(line, theme.sidebar.bg, theme.input, border);
        if text.is_empty() {
            let dim = Style {
                fg: theme.gutter.fg,
                ..input
            };
            grid.label_at(
                line.x as f32 + 0.8,
                line.x + 1,
                y,
                hint,
                dim,
                line.right(),
                95,
            );
        } else {
            grid.put_str(line.x + 1, y, text, input, line.right());
        }
        if self.focus == Some(field) {
            let cursor = line.x + 1 + text.chars().count() as u16;
            grid.cursor = Some((cursor.min(line.right().saturating_sub(1)), y));
        }
    }

    pub fn draw(&self, grid: &mut Grid, area: Rect, theme: &Theme) {
        grid.fill(area, theme.sidebar);
        let title = Style {
            fg: theme.gutter.fg,
            ..theme.sidebar
        };
        grid.label_at(
            area.x as f32 + 2.0,
            area.x + 2,
            area.y,
            "SEARCH",
            title,
            area.right(),
            80,
        );
        self.draw_box(grid, area, area.y + 1, Field::Query, theme);
        self.draw_box(grid, area, area.y + 2, Field::Replace, theme);
        let button = area.right().saturating_sub(REPLACE_ALL.len() as u16 + 1);
        grid.glyph(
            button as f32 - 0.2,
            area.y + 2,
            Icon::ReplaceAll,
            theme.sidebar.fg,
            " ",
            theme.sidebar,
        );
        grid.put_str(
            button,
            area.y + 2,
            REPLACE_ALL,
            Style {
                painted: true,
                ..theme.sidebar
            },
            area.right(),
        );

        let files = self
            .rows()
            .iter()
            .filter(|r| matches!(r, Row::File(..)))
            .count();
        let found = self.results.len();
        let summary = if self.incoming.is_some() {
            "Searching…".to_string()
        } else if self.query.is_empty() {
            String::new()
        } else if found == 0 {
            "No results".to_string()
        } else {
            let results = if found == 1 { "result" } else { "results" };
            let in_files = if files == 1 { "file" } else { "files" };
            format!("{found} {results} in {files} {in_files}")
        };
        let dim = Style {
            fg: theme.gutter.fg,
            ..theme.sidebar
        };
        grid.label_at(
            area.x as f32 + 2.0,
            area.x + 2,
            area.y + 3,
            &summary,
            dim,
            area.right(),
            85,
        );

        let rows = self.rows();
        for (index, row) in rows.iter().skip(self.top).enumerate() {
            let y = area.y + HEADER_ROWS + index as u16;
            if y >= area.bottom() {
                break;
            }
            match row {
                Row::File(path, count) => self.draw_file(grid, area, y, path, *count, theme),
                Row::Match(found) => self.draw_match(grid, area, y, found, theme),
            }
        }
    }

    fn draw_file(
        &self,
        grid: &mut Grid,
        area: Rect,
        y: u16,
        path: &Path,
        count: usize,
        theme: &Theme,
    ) {
        let arrow = if self.collapsed.contains(path) {
            "▸"
        } else {
            "▾"
        };
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy())
            .unwrap_or_default();
        let bold = Style {
            bold: true,
            ..theme.sidebar
        };
        let tally = count.to_string();
        let end = area.right().saturating_sub(tally.len() as u16 + 2);
        let chevron = if self.collapsed.contains(path) {
            Icon::ChevronRight
        } else {
            Icon::ChevronDown
        };
        grid.glyph(
            area.x as f32 + 0.4,
            y,
            chevron,
            theme.sidebar.fg,
            arrow,
            theme.sidebar,
        );
        let x = grid.label_at(area.x as f32 + 2.2, area.x + 3, y, &name, bold, end, 100);
        let folder = path
            .parent()
            .and_then(|p| p.strip_prefix(&self.root).ok())
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        let dim = Style {
            fg: theme.gutter.fg,
            ..theme.sidebar
        };
        grid.label(x + 1, y, &folder, dim, end);
        grid.label_right(area.right().saturating_sub(1), y, &tally, theme.sidebar, 85);
    }

    fn draw_match(&self, grid: &mut Grid, area: Rect, y: u16, found: &Match, theme: &Theme) {
        let start = area.x + 4;
        let width = area.right().saturating_sub(start + 1) as usize;
        let chars: Vec<char> = found.text.chars().collect();
        let column = found.column.min(chars.len());
        // Leading space is noise here, and a far-right match still has to show.
        let indent = chars.iter().take_while(|c| c.is_whitespace()).count();
        let from = indent.max(column.saturating_sub(width / 3)).min(column);
        let length = self.query.chars().count();
        let end = (column + length).min(chars.len());
        let before: String = chars[from..column].iter().collect();
        let hit: String = chars[column..end].iter().collect();
        let after: String = chars[end..].iter().collect();
        let right = area.right().saturating_sub(1);
        let x = grid.put_str(start, y, &before, theme.sidebar, right);
        let highlight = Style {
            bg: theme.find_match,
            ..theme.sidebar
        };
        let x = grid.put_str(x, y, &hit, highlight, right);
        grid.put_str(x, y, &after, theme.sidebar, right);
    }
}
