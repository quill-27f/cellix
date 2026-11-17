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
        .saturating_sub(3 + SWITCH * SWITCHES.len() as u16)
}

/// The three switches from `at`, each lit while it's on. The Find bar draws them too.
pub fn draw_switches(
    grid: &mut Grid,
    at: (u16, u16),
    options: Options,
    hover: Option<(u16, u16)>,
    base: Style,
    theme: &Theme,
    measure: &dyn Measure,
) {
    let (x, y) = at;
    for (index, label) in SWITCHES.iter().enumerate() {
        let left = x + index as u16 * SWITCH;
        let cell = Rect::new(left, y, SWITCH, 1);
        let on = options.is_on(index);
        let hovered = hover.is_some_and(|h| cell.contains(h.0, h.1));
        // Off has no chip, hover a quiet one, and on the selection's colour with a focus edge,
        // so the three states never read alike.
        // Inside the field's box, 3 px from its edges. Its letters are in its middle from
        // side to side, where two letters of 13 pt stood against its right edge, and on the
        // line of the words typed in the box, which are raised.
        let lift = crate::measures::field_lift(measure);
        // Under the 0.7 of a row at which a chip grows with the row: grown, on a taller row,
        // it stood on the box's own line top and bottom.
        let chip = Bounds::inset(cell, 0.15, 0.155);
        let chip = Bounds {
            y: chip.y - cellix_render::units(lift / 2.0),
            ..chip
        };
        if on {
            grid.panel(chip, 14, Some(theme.pick), Some(theme.focus));
        } else if hovered {
            grid.panel(chip, 14, Some(theme.popup_hover), None);
        }
        let style = Style {
            fg: if on { theme.text.fg } else { theme.gutter.fg },
            bg: if on { theme.pick } else { base.bg },
            painted: true,
            ..base
        };
        grid.put_str(left + 1, y, label, style, left + SWITCH);
        let middle = f32::from(left) + f32::from(SWITCH) / 2.0;
        grid.centred_text(middle, f32::from(y) - lift, label, style.fg, 100);
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

    /// Whether a search is under way.
    pub fn searching(&self) -> bool {
        self.incoming.is_some()
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
            1 if x >= switches_start(area) && x < area.right().saturating_sub(2) => Some(
                Hit::Switch(((x - switches_start(area)) / SWITCH).min(2) as usize),
            ),
            1 => Some(Hit::Field(Field::Query)),
            REPLACE_ROW if x + REPLACE_ALL.len() as u16 + 3 >= area.right() => {
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
        // On the panel's text edge, two columns in from either side, as its title is.
        let end = area.right().saturating_sub(2);
        let width = if field == Field::Replace {
            end.saturating_sub(area.x + 6)
        } else {
            end.saturating_sub(area.x + 2)
        };
        let line = Rect::new(area.x + 2, y, width, 1);
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
        crate::palette::sidebar_title(grid, area, "Search", theme);
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
            measure,
        );
        self.draw_box(
            grid,
            area,
            area.y + REPLACE_ROW,
            Field::Replace,
            measure,
            theme,
        );
        let button = area.right().saturating_sub(REPLACE_ALL.len() as u16 + 2);
        grid.small_glyph(
            button as f32,
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
        let searching = self.incoming.is_some();
        let summary = if searching {
            "Searching".to_string()
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
        // While it looks, a spinner before the words, where VS Code's view shows its bar.
        let y = area.y + REPLACE_ROW + 2;
        let words = if searching {
            let spin = crate::measures::WAIT_SPIN;
            grid.spinner(area.x as f32 + 2.0, y, spin, theme.accent, theme.sidebar);
            2
        } else {
            0
        };
        let at = area.x + 2 + words;
        grid.label_at(at as f32, at, y, &summary, dim, area.right(), 85);

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

#[cfg(test)]
mod switch_tests {
    use super::*;

    /// A switch inside a text box is well inside it: under the 0.7 of a row at which a chip
    /// that holds words grows with a taller row. Grown, it stood on the box's own line.
    #[test]
    fn a_switch_in_a_text_box_stays_smaller_than_the_box() {
        use cellix_render::{CellMeasure, Paint, units};
        let theme = Theme::default();
        let mut grid = Grid::new(30, 1, theme.sidebar);
        let on = Options {
            case: true,
            whole: true,
            regex: true,
        };
        draw_switches(
            &mut grid,
            (2, 0),
            on,
            None,
            theme.sidebar,
            &theme,
            &CellMeasure,
        );
        let chips: Vec<i32> = grid
            .paint
            .iter()
            .filter_map(|p| match p {
                Paint::Panel { bounds, .. } => Some(bounds.h),
                _ => None,
            })
            .collect();
        assert_eq!(chips.len(), SWITCHES.len(), "one chip a switch that's on");
        for tall in chips {
            assert!(tall < units(0.7) && tall > units(0.6), "{tall}");
        }
    }

    /// While a search runs, its view says so with a spinner before the words, under its
    /// boxes, where VS Code's view shows its bar; done, the count of what it found.
    #[test]
    fn a_search_under_way_turns_in_its_view() {
        let theme = Theme::default();
        let mut search = Search::new(PathBuf::from("/nowhere"));
        let (_send, incoming) = channel();
        search.incoming = Some(incoming);
        let mut grid = Grid::new(40, 12, theme.sidebar);
        grid.turn = Some(6);
        let area = Rect::new(0, 0, 40, 12);
        search.draw(&mut grid, area, None, &cellix_render::CellMeasure, &theme);
        let y = area.y + REPLACE_ROW + 2;
        let frame = cellix_render::spinner::FRAMES[6];
        assert!(
            grid.row_text(y).contains(&format!("{frame} Searching")),
            "{}",
            grid.text()
        );
        assert!(grid.spinning);
        assert!(search.searching());
        search.incoming = None;
        let mut grid = Grid::new(40, 12, theme.sidebar);
        search.draw(&mut grid, area, None, &cellix_render::CellMeasure, &theme);
        assert!(!grid.spinning && !grid.text().contains("Searching"));
    }
}
