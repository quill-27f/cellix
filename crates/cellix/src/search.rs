use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, channel};
use std::thread;

use cellix_core::pattern::{Options, SWITCHES};
use cellix_core::{Match, Workspace};
use cellix_render::{Bounds, Font, Grid, Icon, Measure, Rect, Style};

use crate::palette::{Input, input};
use crate::theme::Theme;

/// Rows above the results: title, query, replacement, summary.
const HEADER_ROWS: u16 = 5;
const REPLACE_ALL: &str = "All";
/// The replace field, straight under the search field.
const REPLACE_ROW: u16 = 2;
/// Columns for each of the switches at the search field's end.
const SWITCH: u16 = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    Query,
    Replace,
}

/// What a click in the panel lands on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Hit {
    Field(Field),
    /// One of the switches: case, whole word, regular expression.
    Switch(usize),
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
    pub options: Options,
    /// Which box has the keyboard, if any.
    pub focus: Option<Field>,
    results: Vec<Match>,
    collapsed: HashSet<PathBuf>,
    top: usize,
    /// Answers from older queries are dropped, so results never run backwards.
    generation: u64,
    incoming: Option<Receiver<(u64, Vec<Match>)>>,
}

/// Where the switches start, at the search field's end.
/// They sit inside the search field's end, a column clear of its edge.
fn switches_start(area: Rect) -> u16 {
    area.right()
        .saturating_sub(2 + SWITCH * SWITCHES.len() as u16)
}

/// The three switches from `at`, each lit while it's on. The Find bar draws them too.
pub fn draw_switches(
    grid: &mut Grid,
    at: (u16, u16),
    options: Options,
    hover: Option<(u16, u16)>,
    base: Style,
    theme: &Theme,
) {
    let (x, y) = at;
    for (index, label) in SWITCHES.iter().enumerate() {
        let left = x + index as u16 * SWITCH;
        let cell = Rect::new(left, y, SWITCH, 1);
        let on = options.is_on(index);
        let hovered = hover.is_some_and(|h| cell.contains(h.0, h.1));
        // Off has no chip, hover a quiet one, and on the selection's colour with a focus edge,
        // so the three states never read alike.
        let chip = Bounds::inset(cell, 0.15, 0.17);
        if on {
            grid.panel(chip, 14, Some(theme.pick), Some(theme.focus));
        } else if hovered {
            grid.panel(chip, 14, Some(theme.popup_hover), None);
        }
        let style = Style {
            fg: if on { theme.text.fg } else { theme.gutter.fg },
            bg: if on { theme.pick } else { base.bg },
            ..base
        };
        grid.label(left + 1, y, label, style, left + SWITCH);
    }
}

impl Search {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            query: String::new(),
            replace: String::new(),
            options: Options::default(),
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
        let (generation, query, options) = (self.generation, self.query.clone(), self.options);
        let workspace = Workspace::new(self.root.clone());
        thread::spawn(move || {
            let _ = sender.send((generation, workspace.search(&query, options)));
        });
        self.incoming = Some(receiver);
    }

    /// Shows places found some other way, such as a symbol's references.
    pub fn show(&mut self, query: String, results: Vec<Match>) {
        self.generation += 1;
        self.incoming = None;
        self.top = 0;
        self.collapsed.clear();
        self.query = query;
        self.results = results;
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
            1 if x >= switches_start(area) && x < area.right().saturating_sub(1) => Some(
                Hit::Switch(((x - switches_start(area)) / SWITCH).min(2) as usize),
            ),
            1 => Some(Hit::Field(Field::Query)),
            REPLACE_ROW if x + REPLACE_ALL.len() as u16 + 2 >= area.right() => {
                Some(Hit::ReplaceAll)
            }
            REPLACE_ROW => Some(Hit::Field(Field::Replace)),
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

    fn draw_box(
        &self,
        grid: &mut Grid,
        area: Rect,
        y: u16,
        field: Field,
        measure: &dyn Measure,
        theme: &Theme,
    ) {
        let (text, hint) = match field {
            Field::Query => (self.query.as_str(), "Search"),
            Field::Replace => (self.replace.as_str(), "Replace"),
        };
        let end = area.right().saturating_sub(1);
        let width = if field == Field::Replace {
            end.saturating_sub(area.x + 5)
        } else {
            end.saturating_sub(area.x + 1)
        };
        let line = Rect::new(area.x + 1, y, width, 1);
        let box_state = Input {
            text,
            hint,
            focused: self.focus == Some(field),
            behind: theme.sidebar.bg,
        };
        input(grid, line, &box_state, measure, theme);
    }

    pub fn draw(
        &self,
        grid: &mut Grid,
        area: Rect,
        hover: Option<(u16, u16)>,
        measure: &dyn Measure,
        theme: &Theme,
    ) {
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
        self.draw_box(grid, area, area.y + 1, Field::Query, measure, theme);
        draw_switches(
            grid,
            (switches_start(area), area.y + 1),
            self.options,
            hover,
            Style {
                bg: theme.input,
                ..theme.sidebar
            },
            theme,
        );
        self.draw_box(
            grid,
            area,
            area.y + REPLACE_ROW,
            Field::Replace,
            measure,
            theme,
        );
        let button = area.right().saturating_sub(REPLACE_ALL.len() as u16 + 1);
        grid.glyph(
            button as f32 - 0.2,
            area.y + REPLACE_ROW,
            Icon::ReplaceAll,
            theme.sidebar.fg,
            " ",
            theme.sidebar,
        );
        grid.put_str(
            button,
            area.y + REPLACE_ROW,
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
        } else if let Err(why) = self.options.matcher(&self.query)
            && self.options.regex
            && !self.query.is_empty()
        {
            why
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
            area.y + REPLACE_ROW + 2,
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
            let line = Rect::new(area.x, y, area.w, 1);
            if hover.is_some_and(|h| line.contains(h.0, h.1)) {
                let wash = Bounds::inset(line, 0.4, 0.055);
                grid.panel(wash, 18, Some(theme.row_hover), None);
            }
            match row {
                Row::File(path, count) => self.draw_file(grid, area, y, path, *count, theme),
                Row::Match(found) => self.draw_match(grid, area, y, found, measure, theme),
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

    /// A result line in the interface font. The line starts at a word, so a match far along
    /// it still shows with some sense of what's before it.
    fn draw_match(
        &self,
        grid: &mut Grid,
        area: Rect,
        y: u16,
        found: &Match,
        measure: &dyn Measure,
        theme: &Theme,
    ) {
        let font = Font {
            size: 92,
            ..Font::default()
        };
        let chars: Vec<char> = found.text.chars().collect();
        let column = found.column.min(chars.len());
        let indent = chars.iter().take_while(|c| c.is_whitespace()).count();
        let width = area.right().saturating_sub(area.x + 5) as usize;
        let mut from = indent.max(column.saturating_sub(width / 2)).min(column);
        while from > indent && !chars[from - 1].is_whitespace() {
            from += 1;
            if from >= column {
                from = column;
                break;
            }
        }
        let length = if found.len > 0 {
            found.len
        } else {
            self.query.chars().count()
        };
        let end = (column + length).min(chars.len());
        let before: String = chars[from..column].iter().collect();
        let hit: String = chars[column..end].iter().collect();
        let after: String = chars[end..].iter().collect();
        let right = area.right().saturating_sub(1);
        let quiet = Style {
            painted: !measure.in_cells(),
            ..theme.sidebar
        };
        // Cells carry the text for the cell tier and tests.
        let x = grid.put_str(area.x + 4, y, &before, quiet, right);
        let highlight = Style {
            bg: if measure.in_cells() {
                theme.find_match
            } else {
                theme.sidebar.bg
            },
            ..quiet
        };
        let x = grid.put_str(x, y, &hit, highlight, right);
        grid.put_str(x, y, &after, quiet, right);
        if measure.in_cells() {
            return;
        }
        let limit = right as f32;
        let mut at = area.x as f32 + 3.0;
        let plain = (theme.sidebar.fg, false);
        grid.text_run((at, y as f32), limit, &before, plain, font);
        at += measure.width(&before, font);
        let span = measure.width(&hit, font);
        let bounds = Bounds::cells(at, y as f32 + 0.16, span, 0.68);
        grid.panel(bounds, 12, Some(theme.find_match), None);
        grid.text_run((at, y as f32), limit, &hit, plain, font);
        at += span;
        grid.text_run((at, y as f32), limit, &after, plain, font);
    }
}
