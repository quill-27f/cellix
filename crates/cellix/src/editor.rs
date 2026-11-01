use std::cell::{Cell, OnceCell, RefCell};
use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;

use cellix_core::{Buffer, Motion, Selection, Selections};
use cellix_input::{Command, Direction, Format};
use cellix_lang::{Registry, Span, Worker};
use cellix_render::{Bounds, Font, Grid, INTERFACE_SIZE, Icon, Measure, Rect, Rgb, Strip, units};
use cellix_review::LineChange;
use unicode_width::UnicodeWidthChar;

use crate::rendered::{self, Glyph, Intent, Row, Tone};
use crate::theme::Theme;

const TAB_WIDTH: usize = 4;
const GUTTER_MIN_DIGITS: usize = 4;
const NARROW_PANE: u16 = 60;
/// Left margin of a rendered page, which also carries comment marks.
/// Heading levels the toolbar steps through before going back to plain text.
const MAX_HEADING_CYCLE: usize = 3;
const INDENT: &str = "    ";
const MINIMAP_WIDTH: u16 = 13;
/// Lines of code a minimap fits in one row of the grid.
const MINIMAP_LINES_PER_ROW: usize = 6;
/// Columns of code a minimap shows, past which lines are cut off.
const MINIMAP_COLUMNS: usize = 96;
/// Narrower panes spend every column on the code, as beside an agent.
const MIN_MINIMAP_PANE: u16 = 90;
/// Files up to this size highlight as they're drawn. Past it, a background thread does it,
/// because a full reparse on each key starts to show.
const MAX_INLINE_HIGHLIGHT: usize = 32 * 1024;
const PAGE_MARGIN: u16 = 2;
const WIDE_PAGE_MARGIN: u16 = 4;
/// How wide a page can be set.
const MAX_PAGE_WIDTH: u16 = 512;

/// Per-line marks for the gutter, as 1-based lines.
#[derive(Default)]
pub struct Marks {
    pub comments: Vec<usize>,
    pub errors: Vec<usize>,
}

/// A laid-out page: the revision and width it was made for, and its rows.
type Page = (u64, usize, Rc<Vec<Row>>);

/// One open document and where the view is within it.
/// A colour pulled towards the background, as text is at a minimap's size.
fn dimmed(color: Rgb, bg: Rgb) -> Rgb {
    let mix = |a: u8, b: u8| ((a as u16 * 3 + b as u16 * 2) / 5) as u8;
    Rgb(mix(color.0, bg.0), mix(color.1, bg.1), mix(color.2, bg.2))
}

/// Bars per row, with the revision, height and width they fit.
type MinimapRows = ((u64, u16, u16), Vec<Vec<Strip>>);

pub struct Editor {
    pub buffer: Buffer,
    pub selections: Selections,
    /// Char ranges to mark, while find is open.
    pub found: Vec<(usize, usize)>,
    /// The last commit's copy of this file, to mark changed lines against.
    pub committed: Option<String>,
    /// Changed lines by number, with the revision they were worked out at.
    changes: RefCell<(u64, HashMap<usize, LineChange>)>,
    /// Bars for each row of the minimap, with what they were built for.
    minimap: RefCell<Option<MinimapRows>>,
    pub top: usize,
    pub left: usize,
    spans: RefCell<Option<(u64, Vec<Span>)>>,
    /// How many rows the pane showed last, which is what bounds scrolling.
    view_rows: Cell<u16>,
    /// Started for the first large file, and only then.
    worker: OnceCell<Worker>,
    /// The revision the worker was last asked about.
    asked: Cell<u64>,
    /// What the cached spans were read from, to carry them across an edit.
    spans_bytes: Cell<usize>,
    /// Show Markdown as an editable page rather than its source, as Xcode does.
    pub rendered: bool,
    /// The laid-out page, for one revision and width.
    page: RefCell<Option<Page>>,
    measure: Rc<dyn Measure>,
    registry: Rc<Registry>,
    /// The revision last sent to a language server.
    pub synced: u64,
}

fn display_width(c: char, column: usize) -> usize {
    if c == '\t' {
        TAB_WIDTH - column % TAB_WIDTH
    } else {
        c.width().unwrap_or(0)
    }
}

impl Editor {
    pub fn new(buffer: Buffer, measure: Rc<dyn Measure>, registry: Rc<Registry>) -> Self {
        let extension = buffer.path().and_then(Path::extension);
        let is_markdown = extension.is_some_and(|e| e == "md" || e == "markdown");
        Self {
            buffer,
            selections: Selections::default(),
            found: Vec::new(),
            committed: None,
            changes: RefCell::new((u64::MAX, HashMap::new())),
            minimap: RefCell::new(None),
            top: 0,
            left: 0,
            spans: RefCell::new(None),
            view_rows: Cell::new(0),
            worker: OnceCell::new(),
            asked: Cell::new(u64::MAX),
            spans_bytes: Cell::new(0),
            rendered: is_markdown,
            page: RefCell::new(None),
            measure,
            registry,
            synced: 0,
        }
    }

    pub fn title(&self) -> String {
        self.buffer
            .path()
            .and_then(Path::file_name)
            .map_or_else(|| "untitled".into(), |n| n.to_string_lossy().into_owned())
    }

    /// Whether a pane has room for a minimap, which only pixels can draw.
    fn shows_minimap(&self, area: Rect) -> bool {
        !self.measure.in_cells() && area.w >= MIN_MINIMAP_PANE
    }

    /// Where text goes in `area`: all of it, less the minimap or the scrollbar.
    fn text_area(&self, area: Rect) -> Rect {
        let side = if self.shows_minimap(area) {
            MINIMAP_WIDTH
        } else {
            1
        };
        Rect::new(area.x, area.y, area.w.saturating_sub(side), area.h)
    }

    /// Forgets the changed lines, as when the file's committed copy changes.
    pub fn refresh_changes(&self) {
        self.changes.borrow_mut().0 = u64::MAX;
    }

    fn line_changes(&self) -> std::cell::Ref<'_, (u64, HashMap<usize, LineChange>)> {
        let revision = self.buffer.revision();
        if self.changes.borrow().0 != revision {
            let found = self.committed.as_ref().map_or_else(HashMap::new, |old| {
                cellix_review::line_changes(old, &self.buffer.text())
                    .into_iter()
                    .collect()
            });
            *self.changes.borrow_mut() = (revision, found);
        }
        self.changes.borrow()
    }

    /// The bar beside a changed line, in the margin of `x`.
    fn draw_change(&self, grid: &mut Grid, x: f32, y: u16, change: LineChange, theme: &Theme) {
        let color = match change {
            LineChange::Added => theme.git_added,
            LineChange::Modified => theme.git_modified,
            LineChange::DeletedAbove => theme.error,
        };
        let bounds = match change {
            LineChange::DeletedAbove => Bounds::cells(x, y as f32 - 0.06, 0.55, 0.12),
            _ => Bounds::cells(x, y as f32, 0.28, 1.0),
        };
        grid.panel(bounds, 0, Some(color), None);
        if self.measure.in_cells() {
            let style = cellix_render::Style {
                fg: color,
                ..theme.gutter
            };
            grid.put_str(x as u16, y, "▎", style, x as u16 + 1);
        }
    }

    /// The words of a line of code, as (column, length, colour).
    fn code_words(&self, line: usize, spans: &[Span], theme: &Theme) -> Vec<(usize, usize, Rgb)> {
        let start = self.buffer.rope().line_to_byte(line);
        let text = self.line_text(line);
        let mut words = Vec::new();
        let (mut column, mut byte) = (0, 0);
        for word in text.split_inclusive(char::is_whitespace) {
            let size = word.trim_end().chars().count();
            if size > 0 {
                let at = start + byte;
                let found = spans.partition_point(|s| s.end <= at);
                let span = spans.get(found).filter(|s| s.start <= at);
                let color = span.map_or(theme.text.fg, |s| theme.token(s.token));
                words.push((column, size, color));
            }
            column += word.chars().count();
            byte += word.len();
        }
        words
    }

    /// The words of a page row, in the same shape.
    fn page_words(row: &Row, theme: &Theme) -> Vec<(usize, usize, Rgb)> {
        let color = |tone| match tone {
            Tone::Heading => theme.heading,
            Tone::Code | Tone::Block(_) => theme.string,
            Tone::Link | Tone::Check(_) => theme.accent,
            _ => theme.text.fg,
        };
        let mut words = Vec::new();
        let mut run: Option<(usize, usize, Rgb)> = None;
        for g in &row.glyphs {
            if g.ch.is_whitespace() {
                words.extend(run.take());
            } else {
                let column = g.x as usize;
                let word = run.get_or_insert((column, 0, color(g.tone)));
                word.1 = (column + 1).saturating_sub(word.0);
            }
        }
        words.extend(run);
        words
    }

    /// Bars for each row of a minimap `rows` tall, in the colours of the code. The whole
    /// file always fits, so the map's fill shows how long the file is.
    fn minimap_rows(&self, rows: u16, text: Rect, theme: &Theme) -> Vec<Vec<Strip>> {
        let page = self.rendered.then(|| self.page(text));
        let lines = page
            .as_ref()
            .map_or_else(|| self.buffer.rope().len_lines(), |p| p.len());
        let capacity = rows as usize * MINIMAP_LINES_PER_ROW;
        let shown = lines.min(capacity);
        let mut out = vec![Vec::new(); rows as usize];
        let spans = self.spans.borrow();
        let no_spans = Vec::new();
        let spans = spans.as_ref().map_or(&no_spans, |(_, spans)| spans);
        for slot in 0..shown {
            let line = slot * lines / shown.max(1);
            let words = match &page {
                Some(page) => Self::page_words(&page[line], theme),
                None => self.code_words(line, spans, theme),
            };
            let row = slot / MINIMAP_LINES_PER_ROW;
            let y = ((slot % MINIMAP_LINES_PER_ROW) * 256 / MINIMAP_LINES_PER_ROW) as u8;
            for (column, size, color) in words {
                if column < MINIMAP_COLUMNS {
                    let x = (column * 256 / MINIMAP_COLUMNS) as u8;
                    let w = (size.min(MINIMAP_COLUMNS - column) * 256 / MINIMAP_COLUMNS) as u8;
                    let color = dimmed(color, theme.text.bg);
                    out[row].push(Strip { x, y, w, color });
                }
            }
        }
        out
    }

    fn draw_minimap(&self, grid: &mut Grid, full: Rect, theme: &Theme) {
        let text = self.text_area(full);
        let track = Rect::new(text.right(), full.y, full.w - text.w, full.h);
        grid.fill(track, theme.text);
        let total = if self.rendered {
            self.page(text).len()
        } else {
            self.buffer.rope().len_lines()
        }
        .max(1);
        let revision = self.buffer.revision();
        let built = (revision, track.h, text.w);
        let stale = !self
            .minimap
            .borrow()
            .as_ref()
            .is_some_and(|(key, _)| *key == built);
        if stale {
            let rows = self.minimap_rows(track.h, text, theme);
            *self.minimap.borrow_mut() = Some((built, rows));
        }
        let height = track.h as f32;
        let filled = (total as f32 / MINIMAP_LINES_PER_ROW as f32).min(height);
        let span = (text.h as f32 * filled / total as f32).clamp(1.0, height);
        // Past the fold the marker's top stops short, so the end of the file meets the end
        // of the map rather than sliding off it.
        let range = total.saturating_sub(text.h as usize).max(1) as f32;
        let travel = (filled - span).max(0.0);
        let top = track.y as f32 + (self.top as f32 / range).min(1.0) * travel;
        let margin = 0.6;
        let width = track.w as f32 - margin * 1.5;
        let view = Bounds::cells(track.x as f32 + margin - 0.3, top, width + 0.6, span);
        grid.panel(view, 12, Some(theme.minimap_view), Some(theme.popup_border));
        if let Some((_, rows)) = self.minimap.borrow().as_ref() {
            for (row, strips) in rows.iter().enumerate().filter(|(_, s)| !s.is_empty()) {
                let bounds = Bounds::cells(
                    track.x as f32 + margin,
                    track.y as f32 + row as f32,
                    width,
                    1.0,
                );
                grid.paint.push(cellix_render::Paint::Strips {
                    bounds,
                    strips: strips.clone(),
                });
            }
        }
        // Changes down the edge, so they're findable in a long file.
        let lines = self.buffer.rope().len_lines().max(1);
        let fill_lines = lines as f32;
        for (line, change) in &self.line_changes().1 {
            let y = track.y as f32 + *line as f32 * filled / fill_lines;
            let color = match change {
                LineChange::Added => theme.git_added,
                LineChange::Modified => theme.git_modified,
                LineChange::DeletedAbove => theme.error,
            };
            let height = (filled / fill_lines).max(0.1);
            let mark = Bounds::cells(track.right() as f32 - 0.45, y, 0.3, height);
            grid.panel(mark, 0, Some(color), None);
        }
    }

    /// Scrolls to a point on the minimap or scrollbar.
    fn map_filled(&self, area: Rect, lines: usize) -> usize {
        if self.shows_minimap(area) {
            (lines.div_ceil(MINIMAP_LINES_PER_ROW)).clamp(1, area.h as usize)
        } else {
            area.h.max(1) as usize
        }
    }

    /// Pages use the whole pane, with room in the margin for marks.
    fn page_margin(area: Rect) -> u16 {
        if area.w < NARROW_PANE {
            PAGE_MARGIN
        } else {
            WIDE_PAGE_MARGIN
        }
    }

    fn page_width(area: Rect) -> usize {
        area.w
            .saturating_sub(Self::page_margin(area) * 2)
            .clamp(1, MAX_PAGE_WIDTH) as usize
    }

    /// The rendered page for this width, laid out again only after an edit.
    fn page(&self, area: Rect) -> Rc<Vec<Row>> {
        let width = Self::page_width(area);
        let revision = self.buffer.revision();
        let mut cache = self.page.borrow_mut();
        if let Some((r, w, rows)) = cache.as_ref()
            && *r == revision
            && *w == width
        {
            return rows.clone();
        }
        let text = self.buffer.text();
        let rows = rendered::layout(&text, width, &*self.measure, &self.registry);
        let rows = Rc::new(rows);
        *cache = Some((revision, width, rows.clone()));
        rows
    }

    /// Text is measured differently from here on, so the page has to be set again.
    pub fn set_measure(&mut self, measure: Rc<dyn Measure>) {
        self.measure = measure;
        *self.page.borrow_mut() = None;
    }

    /// Moves a caret out of hidden source, so typing never lands inside a marker.
    pub fn prepare_edit(&mut self, area: Rect) {
        if !self.rendered {
            return;
        }
        let area = self.text_area(area);
        let primary = self.selections.primary();
        if primary.is_empty() {
            let at = rendered::snap(&self.page(area), primary.head);
            self.selections.set_single(Selection::cursor(at));
        }
    }

    /// Runs a move on the page, where rows and hidden markers change what it means.
    fn move_on_page(&mut self, area: Rect, direction: Direction, extend: bool) -> bool {
        let rows = self.page(area);
        // Start from a visible place, so a selection never takes in a hidden marker.
        let mut primary = self.selections.primary();
        if primary.is_empty() {
            let at = rendered::snap(&rows, primary.head);
            primary.anchor = at;
            primary.head = at;
        }
        let head = primary.head;
        let page = area.h.max(2) as isize - 1;
        let (_, column) = rendered::locate(&rows, head);
        // Selection keeps its goal as a whole number, so this one is in paint units.
        let goal = primary
            .goal
            .map_or(column, |g| g as f32 / units(1.0) as f32);
        let vertical = |delta| rendered::vertical(&rows, head, delta, goal);
        let (next, keep_goal) = match direction {
            Direction::Left => (rendered::step(&rows, head, false), false),
            Direction::Right => (rendered::step(&rows, head, true), false),
            Direction::Up => (vertical(-1), true),
            Direction::Down => (vertical(1), true),
            Direction::PageUp => (vertical(-page), true),
            Direction::PageDown => (vertical(page), true),
            Direction::LineStart | Direction::LineColumnZero => {
                (rendered::row_edge(&rows, head, false), false)
            }
            Direction::LineEnd => (rendered::row_edge(&rows, head, true), false),
            _ => return false,
        };
        self.selections.set_single(Selection {
            anchor: if extend { primary.anchor } else { next },
            head: next,
            goal: keep_goal.then_some(units(goal) as usize),
        });
        true
    }

    fn line_of_head(&self) -> (String, usize) {
        let rope = self.buffer.rope();
        let line = rope.char_to_line(self.selections.primary().head);
        (self.line_text(line), rope.line_to_char(line))
    }

    /// Enter on a page continues lists and quotes, and ends them on an empty item.
    fn enter_on_page(&mut self) {
        let (line, start) = self.line_of_head();
        let head = self.selections.primary().head;
        let at_end = head == start + line.chars().count();
        match rendered::enter_intent(&line) {
            Intent::Continue(marker) if at_end => {
                self.buffer
                    .replace(&mut self.selections, &format!("\n{marker}"));
            }
            Intent::EndList(hidden) => {
                self.buffer
                    .replace_range(&mut self.selections, start, start + hidden, "");
                self.selections.set_single(Selection::cursor(start));
            }
            _ => self.buffer.replace(&mut self.selections, "\n"),
        }
    }

    /// Backspace at the start of an item or heading removes its marker in one step.
    fn backspace_on_page(&mut self) {
        let (line, start) = self.line_of_head();
        let hidden = rendered::hidden_prefix(&line);
        let primary = self.selections.primary();
        if primary.is_empty() && hidden > 0 && primary.head == start + hidden {
            self.buffer
                .replace_range(&mut self.selections, start, start + hidden, "");
            self.selections.set_single(Selection::cursor(start));
        } else {
            self.buffer.delete_backward(&mut self.selections);
        }
    }

    /// Tab moves between table cells, and otherwise indents.
    fn tab_on_page(&mut self, area: Rect) {
        let rows = self.page(area);
        let head = self.selections.primary().head;
        if let Some(cell) = rendered::next_cell(&rows, head) {
            self.selections.set_single(Selection::cursor(cell));
            return;
        }
        let (line, start) = self.line_of_head();
        if rendered::hidden_prefix(&line) > 0 {
            self.buffer
                .replace_range(&mut self.selections, start, start, "  ");
            self.selections.set_single(Selection::cursor(head + 2));
        } else {
            self.buffer.type_text(&mut self.selections, "    ");
        }
    }

    /// Toggles a task checkbox at a click. Returns true if there was one.
    pub fn click_checkbox(&mut self, area: Rect, x: f32, y: u16) -> bool {
        if !self.rendered {
            return false;
        }
        let area = self.text_area(area);
        let rows = self.page(area);
        let row = self.top + y.saturating_sub(area.y) as usize;
        let column = x - (area.x + Self::page_margin(area)) as f32;
        let Some((at, mark)) = rows.get(row).and_then(|r| r.checkbox) else {
            return false;
        };
        if column < at || column >= at + 1.3 {
            return false;
        }
        let checked = self.buffer.rope().char(mark) != ' ';
        let text = if checked { " " } else { "x" };
        self.buffer
            .replace_range(&mut self.selections, mark, mark + 1, text);
        true
    }

    /// True when a click at `x` lands on the scrollbar.
    pub fn on_scrollbar(&self, area: Rect, x: u16) -> bool {
        x >= self.text_area(area).right()
    }

    /// Scrolls so that row `y` of the scrollbar's track matches the document.
    pub fn scroll_to_row(&mut self, area: Rect, y: u16) {
        let lines = if self.rendered {
            self.page(self.text_area(area)).len()
        } else {
            self.buffer.rope().len_lines()
        };
        let row = y.saturating_sub(area.y) as usize;
        let filled = self.map_filled(area, lines);
        // The point clicked comes to the middle of the view.
        let middle = (row * lines / filled).saturating_sub(area.h as usize / 2);
        self.view_rows.set(area.h);
        self.top = middle.min(self.last_top());
    }

    /// Narrow panes keep only the digits they need: every column counts there.
    fn gutter_width(&self, pane_width: u16) -> u16 {
        let digits = self.buffer.rope().len_lines().to_string().len();
        let minimum = if pane_width < NARROW_PANE {
            2
        } else {
            GUTTER_MIN_DIGITS
        };
        (digits.max(minimum) + 2) as u16
    }

    fn line_text(&self, line: usize) -> String {
        let text = self.buffer.rope().line(line).to_string();
        text.trim_end_matches(['\n', '\r']).to_string()
    }

    /// Display column of a char position.
    fn column_of(&self, at: usize) -> usize {
        let rope = self.buffer.rope();
        let line = rope.char_to_line(at);
        let start = rope.line_to_char(line);
        let mut column = 0;
        for c in rope.slice(start..at).chars() {
            column += display_width(c, column);
        }
        column
    }

    /// Char position at a screen cell inside `area`, clamped to the text.
    pub fn position_at(&self, area: Rect, fine_x: f32, y: u16) -> usize {
        let x = fine_x as u16;
        let area = self.text_area(area);
        if self.rendered {
            let rows = self.page(area);
            let row = self.top + y.saturating_sub(area.y) as usize;
            let column = (fine_x - (area.x + Self::page_margin(area)) as f32).max(0.0);
            let row = row.min(rows.len().saturating_sub(1));
            // A tall heading's lower half is the spacer under it, and still the heading.
            let above = row
                .checked_sub(1)
                .and_then(|r| rows.get(r))
                .filter(|r| r.tall);
            let stop = above.or_else(|| rows[row..].iter().find(|r| r.stop));
            return stop.map_or(self.buffer.rope().len_chars(), |r| r.position_at(column));
        }
        let rope = self.buffer.rope();
        let line = (self.top + (y.saturating_sub(area.y)) as usize).min(rope.len_lines() - 1);
        let target = self.left + x.saturating_sub(area.x + self.gutter_width(area.w)) as usize;
        let start = rope.line_to_char(line);
        let mut column = 0;
        for (i, c) in self.line_text(line).chars().enumerate() {
            let width = display_width(c, column);
            if column + width > target {
                return start + i;
            }
            column += width;
        }
        start + self.line_text(line).chars().count()
    }

    pub fn top(&self) -> usize {
        self.top
    }

    /// The lines or page rows there are to scroll through.
    fn scroll_length(&self) -> usize {
        // A cached page is close enough to bound the scroll; draw corrects any slack.
        let rows = self.page.borrow().as_ref().map(|(_, _, rows)| rows.len());
        rows.filter(|_| self.rendered)
            .unwrap_or_else(|| self.buffer.rope().len_lines())
    }

    /// The furthest down the view goes: the end of the file at the bottom of the pane.
    fn last_top(&self) -> usize {
        self.scroll_length()
            .saturating_sub(self.view_rows.get().max(1) as usize)
    }

    pub fn scroll_by(&mut self, delta: isize) {
        let last = self.last_top() as isize;
        self.top = (self.top as isize + delta).clamp(0, last) as usize;
    }

    /// What moves when the wheel does: the text, without the minimap beside it or the left
    /// column, where the sheet's rounded corners are.
    pub fn scroll_region(&self, area: Rect) -> Rect {
        let text = self.text_area(area);
        Rect::new(text.x + 1, text.y, text.w.saturating_sub(1), text.h)
    }

    /// Scrolls just enough to show the primary cursor.
    pub fn reveal(&mut self, area: Rect) {
        let area = self.text_area(area);
        if self.rendered {
            let (row, _) = rendered::locate(&self.page(area), self.selections.primary().head);
            let height = area.h.max(1) as usize;
            if row < self.top {
                self.top = row;
            } else if row >= self.top + height {
                self.top = row + 1 - height;
            }
            self.left = 0;
            return;
        }
        let head = self.selections.primary().head;
        let line = self.buffer.rope().char_to_line(head);
        let rows = area.h.max(1) as usize;
        if line < self.top {
            self.top = line;
        } else if line >= self.top + rows {
            self.top = line + 1 - rows;
        }
        let column = self.column_of(head);
        let cols = area.w.saturating_sub(self.gutter_width(area.w)).max(1) as usize;
        if column < self.left {
            self.left = column;
        } else if column >= self.left + cols {
            self.left = column + 1 - cols;
        }
    }

    pub fn run(&mut self, command: Command, area: Rect) {
        let page = area.h.max(2) as usize - 1;
        if self.rendered {
            let text_area = self.text_area(area);
            self.prepare_for_command(command, text_area);
            let handled = match command {
                Command::Move(direction, extend) => self.move_on_page(text_area, direction, extend),
                Command::Newline => {
                    self.enter_on_page();
                    true
                }
                Command::DeleteBackward => {
                    self.backspace_on_page();
                    true
                }
                Command::Indent => {
                    self.tab_on_page(text_area);
                    true
                }
                _ => false,
            };
            if handled {
                self.reveal(area);
                return;
            }
        }
        match command {
            Command::Move(direction, extend) => {
                let motion = match direction {
                    Direction::Left => Motion::Left,
                    Direction::Right => Motion::Right,
                    Direction::Up => Motion::Up,
                    Direction::Down => Motion::Down,
                    Direction::WordLeft => Motion::WordLeft,
                    Direction::WordRight => Motion::WordRight,
                    Direction::LineStart => Motion::LineStart,
                    Direction::LineColumnZero => Motion::LineColumnZero,
                    Direction::LineEnd => Motion::LineEnd,
                    Direction::Top => Motion::Top,
                    Direction::Bottom => Motion::Bottom,
                    Direction::PageUp => Motion::PageUp(page),
                    Direction::PageDown => Motion::PageDown(page),
                };
                self.selections
                    .apply_motion(self.buffer.rope(), motion, extend);
            }
            Command::Newline => self.newline(),
            Command::ToggleComment => self.toggle_comment(),
            Command::MoveLineUp => self.move_line(-1),
            Command::MoveLineDown => self.move_line(1),
            Command::DuplicateLine => self.duplicate_line(),
            Command::Indent => self.buffer.type_text(&mut self.selections, "    "),
            Command::Outdent => self.outdent(),
            Command::DeleteWordLeft => self.delete_to(Motion::WordLeft),
            Command::DeleteWordRight => self.delete_to(Motion::WordRight),
            Command::DeleteToLineStart => self.delete_to(Motion::LineColumnZero),
            Command::DeleteToLineEnd => self.delete_to(Motion::LineEnd),
            Command::DeleteBackward => self.backspace(),
            Command::DeleteForward => self.buffer.delete_forward(&mut self.selections),
            Command::Undo => {
                self.buffer.undo(&mut self.selections);
            }
            Command::Redo => {
                self.buffer.redo(&mut self.selections);
            }
            Command::SelectAll => {
                let len = self.buffer.rope().len_chars();
                self.selections.set_single(Selection {
                    anchor: 0,
                    head: len,
                    goal: None,
                });
            }
            Command::SelectLines(count) => {
                let rope = self.buffer.rope();
                let line = rope.char_to_line(self.selections.primary().head);
                let last = rope.len_lines();
                self.selections.set_single(Selection {
                    anchor: rope.line_to_char(line),
                    head: rope.line_to_char((line + count).min(last)),
                    goal: None,
                });
            }
            Command::SelectNextMatch => self.buffer.select_next_match(&mut self.selections),
            _ => return,
        }
        self.reveal(area);
    }

    /// Edits start from a visible place. Moves start wherever the caret is.
    fn prepare_for_command(&mut self, command: Command, area: Rect) {
        if matches!(
            command,
            Command::Newline | Command::DeleteBackward | Command::DeleteForward | Command::Indent
        ) {
            let primary = self.selections.primary();
            if primary.is_empty() {
                let at = rendered::snap(&self.page(area), primary.head);
                self.selections.set_single(Selection::cursor(at));
            }
        }
    }

    fn outdent(&mut self) {
        let head = self.selections.primary().head;
        let rope = self.buffer.rope();
        let start = rope.line_to_char(rope.char_to_line(head));
        let spaces = rope
            .line(rope.char_to_line(head))
            .chars()
            .take(TAB_WIDTH)
            .take_while(|c| *c == ' ')
            .count();
        if spaces > 0 {
            self.buffer
                .replace_range(&mut self.selections, start, start + spaces, "");
        }
    }

    fn char_at(&self, at: usize) -> Option<char> {
        let rope = self.buffer.rope();
        (at < rope.len_chars()).then(|| rope.char(at))
    }

    /// Keeps the line's indent, adds a level after an opening bracket, and puts a closing
    /// bracket on a line of its own.
    fn newline(&mut self) {
        let head = self.selections.primary().head;
        let rope = self.buffer.rope();
        let line = rope.line(rope.char_to_line(head));
        let indent: String = line
            .chars()
            .take_while(|c| matches!(c, ' ' | '\t'))
            .collect();
        let before = head.checked_sub(1).and_then(|at| self.char_at(at));
        let after = self.char_at(head);
        let opens = matches!(before, Some('{' | '(' | '['));
        let closes = matches!(
            (before, after),
            (Some('{'), Some('}')) | (Some('('), Some(')')) | (Some('['), Some(']'))
        );
        let inner = format!("{indent}{}", if opens { INDENT } else { "" });
        let text = if closes {
            format!("\n{inner}\n{indent}")
        } else {
            format!("\n{inner}")
        };
        self.buffer.type_text(&mut self.selections, &text);
        if closes {
            let at = head + 1 + inner.chars().count();
            self.selections.set_single(Selection::cursor(at));
        }
    }

    /// Types a character, closing brackets and quotes, wrapping a selection in them, and
    /// stepping over a closing one that's already there.
    pub fn type_char(&mut self, c: char) {
        let primary = self.selections.primary();
        let close = match c {
            '(' => Some(')'),
            '[' => Some(']'),
            '{' => Some('}'),
            '"' | '`' | '\'' => Some(c),
            _ => None,
        };
        let pairs = !self.rendered && self.selections.len() == 1;
        let after = self.char_at(primary.head);
        let before = primary.head.checked_sub(1).and_then(|at| self.char_at(at));
        if pairs
            && primary.is_empty()
            && after == Some(c)
            && matches!(c, ')' | ']' | '}' | '"' | '`' | '\'')
        {
            self.selections
                .set_single(Selection::cursor(primary.head + 1));
            return;
        }
        let text = c.to_string();
        let Some(close) = close.filter(|_| pairs) else {
            self.buffer.type_text(&mut self.selections, &text);
            return;
        };
        if !primary.is_empty() {
            let (start, end) = (primary.start(), primary.end());
            let inner = self.buffer.rope().slice(start..end).to_string();
            let wrapped = format!("{c}{inner}{close}");
            self.buffer
                .replace_range(&mut self.selections, start, end, &wrapped);
            self.selections.set_single(Selection {
                anchor: start + 1,
                head: end + 1,
                goal: None,
            });
            return;
        }
        // Beside a word, a quote is an apostrophe or a lifetime, and a bracket wraps it later.
        let by_word = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric() || c == '_');
        if by_word(after) || (c == close && by_word(before)) {
            self.buffer.type_text(&mut self.selections, &text);
            return;
        }
        self.buffer
            .type_text(&mut self.selections, &format!("{c}{close}"));
        self.selections
            .set_single(Selection::cursor(primary.head + 1));
    }

    /// Deletes both halves of an empty pair.
    fn backspace(&mut self) {
        let primary = self.selections.primary();
        let pair = primary.head.checked_sub(1).and_then(|at| {
            let open = self.char_at(at)?;
            let close = self.char_at(primary.head)?;
            matches!(
                (open, close),
                ('(', ')') | ('[', ']') | ('{', '}') | ('"', '"') | ('`', '`') | ('\'', '\'')
            )
            .then_some(at)
        });
        match pair.filter(|_| primary.is_empty() && self.selections.len() == 1) {
            Some(at) => {
                self.buffer
                    .replace_range(&mut self.selections, at, at + 2, "");
                self.selections.set_single(Selection::cursor(at));
            }
            None => self.buffer.delete_backward(&mut self.selections),
        }
    }

    /// The whole lines the selection touches, as (first, last, start, end in chars).
    fn selected_lines(&self) -> (usize, usize, usize, usize) {
        let primary = self.selections.primary();
        let rope = self.buffer.rope();
        let first = rope.char_to_line(primary.start());
        let last = rope.char_to_line(primary.end());
        let start = rope.line_to_char(first);
        let end = rope.line_to_char(last) + self.line_text(last).chars().count();
        (first, last, start, end)
    }

    fn toggle_comment(&mut self) {
        let Some(mark) = self.buffer.path().and_then(Registry::line_comment) else {
            return;
        };
        let (first, last, start, end) = self.selected_lines();
        let lines: Vec<String> = (first..=last).map(|line| self.line_text(line)).collect();
        let written = |line: &&String| !line.trim().is_empty();
        let commented = lines
            .iter()
            .filter(written)
            .all(|line| line.trim_start().starts_with(mark));
        let margin = lines
            .iter()
            .filter(written)
            .map(|line| line.len() - line.trim_start().len())
            .min()
            .unwrap_or(0);
        let changed: Vec<String> = lines
            .iter()
            .map(|line| {
                if line.trim().is_empty() {
                    line.clone()
                } else if commented {
                    let at = line.len() - line.trim_start().len();
                    let rest = line[at + mark.len()..]
                        .strip_prefix(' ')
                        .unwrap_or(&line[at + mark.len()..]);
                    format!("{}{rest}", &line[..at])
                } else {
                    format!("{}{mark} {}", &line[..margin], &line[margin..])
                }
            })
            .collect();
        let text = changed.join("\n");
        self.buffer
            .replace_range(&mut self.selections, start, end, &text);
        self.selections
            .set_single(Selection::cursor(start + text.chars().count()));
    }

    fn move_line(&mut self, by: isize) {
        let (first, last, start, end) = self.selected_lines();
        let rope = self.buffer.rope();
        let count = rope.len_lines();
        let trailing = rope.line(count - 1).len_chars() == 0;
        let lines = count - usize::from(trailing);
        let other = if by < 0 {
            first.checked_sub(1)
        } else {
            (last + 1 < lines).then_some(last + 1)
        };
        let Some(other) = other else {
            return;
        };
        let head = self.selections.primary().head;
        let block = rope.slice(start..end).to_string();
        let neighbour = self.line_text(other);
        let (from, to, text, moved) = if by < 0 {
            let from = rope.line_to_char(other);
            (
                from,
                end,
                format!("{block}\n{neighbour}"),
                from + head - start,
            )
        } else {
            let to = rope.line_to_char(other) + neighbour.chars().count();
            let shift = neighbour.chars().count() + 1;
            (start, to, format!("{neighbour}\n{block}"), head + shift)
        };
        self.buffer
            .replace_range(&mut self.selections, from, to, &text);
        self.selections.set_single(Selection::cursor(moved));
    }

    fn duplicate_line(&mut self) {
        let (_, _, start, end) = self.selected_lines();
        let block = self.buffer.rope().slice(start..end).to_string();
        let head = self.selections.primary().head;
        let copy = format!("\n{block}");
        self.buffer
            .replace_range(&mut self.selections, end, end, &copy);
        let at = head + block.chars().count() + 1;
        self.selections.set_single(Selection::cursor(at));
    }

    pub fn go_to_line(&mut self, line: usize, area: Rect) {
        let at = self.buffer.position(line.saturating_sub(1), 0);
        self.selections.set_single(Selection::cursor(at));
        self.reveal(area);
    }

    /// Where the caret is on screen, if it's in view.
    pub fn caret_cell(&self, area: Rect) -> Option<(f32, u16)> {
        let area = self.text_area(area);
        let head = self.selections.primary().head;
        let (row, x) = if self.rendered {
            let rows = self.page(area);
            let (row, x) = rendered::locate(&rows, head);
            (row, (area.x + Self::page_margin(area)) as f32 + x)
        } else {
            let row = self.buffer.rope().char_to_line(head);
            let column = self.column_of(head).checked_sub(self.left)?;
            let gutter = self.gutter_width(area.w);
            (row, (area.x + gutter) as f32 + column as f32)
        };
        let y = row.checked_sub(self.top)?;
        (y < area.h as usize).then_some((x, area.y + y as u16))
    }

    /// Takes a finished highlight. Returns whether there's something new to draw.
    pub fn collect_highlights(&self) -> bool {
        let Some((revision, spans)) = self.worker.get().and_then(Worker::finished) else {
            return false;
        };
        // It was asked for at this revision's size, or the next answer will correct it.
        if revision == self.buffer.revision() {
            self.spans_bytes.set(self.buffer.rope().len_bytes());
            *self.spans.borrow_mut() = Some((revision, spans));
            // Its bars were drawn without colours.
            *self.minimap.borrow_mut() = None;
            true
        } else {
            false
        }
    }

    /// Deletes from the cursor to where a motion lands, or just the selection if any.
    fn delete_to(&mut self, motion: Motion) {
        let primary = self.selections.primary();
        let (start, end) = if primary.is_empty() {
            let mut moved = self.selections.clone();
            moved.apply_motion(self.buffer.rope(), motion, false);
            let target = moved.primary().head;
            (target.min(primary.head), target.max(primary.head))
        } else {
            (primary.start(), primary.end())
        };
        self.buffer
            .replace_range(&mut self.selections, start, end, "");
        self.selections.set_single(Selection::cursor(start));
    }

    fn toggle_inline(&mut self, marker: &str) {
        let primary = self.selections.primary();
        let (start, end) = if primary.is_empty() {
            self.buffer.word_at(primary.head)
        } else {
            (primary.start(), primary.end())
        };
        let size = marker.chars().count();
        let rope = self.buffer.rope();
        let before = start
            .checked_sub(size)
            .filter(|from| rope.slice(*from..start) == marker);
        let after_end = end + size;
        let after = after_end <= rope.len_chars() && rope.slice(end..after_end) == marker;
        let inner = rope.slice(start..end).to_string();
        let (from, to, text, selected) = match before.filter(|_| after) {
            Some(from) => (from, after_end, inner, from),
            None => (start, end, format!("{marker}{inner}{marker}"), start + size),
        };
        let length = end - start;
        self.buffer
            .replace_range(&mut self.selections, from, to, &text);
        self.selections.set_single(Selection {
            anchor: selected,
            head: selected + length,
            goal: None,
        });
    }

    /// The prefix a selected line should get, given the one it has.
    fn next_prefix(format: Format, line: &str) -> (usize, String) {
        let hashes = line.chars().take_while(|c| *c == '#').count();
        let heading = hashes > 0 && line[hashes..].starts_with(' ');
        let simple = |prefix: &str| {
            if line.starts_with(prefix) {
                (prefix.len(), String::new())
            } else {
                (0, prefix.to_string())
            }
        };
        match format {
            Format::Heading if heading && hashes < MAX_HEADING_CYCLE => {
                (hashes + 1, format!("{} ", "#".repeat(hashes + 1)))
            }
            Format::Heading if heading => (hashes + 1, String::new()),
            Format::Heading => (0, "# ".to_string()),
            Format::Bullet => simple("- "),
            Format::Task => simple("- [ ] "),
            _ => simple("> "),
        }
    }

    fn toggle_lines(&mut self, format: Format) {
        let primary = self.selections.primary();
        let rope = self.buffer.rope();
        let first = rope.char_to_line(primary.start());
        let last = rope.char_to_line(primary.end());
        let start = rope.line_to_char(first);
        let end = rope.line_to_char(last) + self.line_text(last).chars().count();
        let mut text = Vec::new();
        for line in first..=last {
            let line = self.line_text(line);
            let (drop, add) = Self::next_prefix(format, &line);
            text.push(format!("{add}{}", &line[drop..]));
        }
        let text = text.join("\n");
        self.buffer
            .replace_range(&mut self.selections, start, end, &text);
        let at = start + text.chars().count();
        self.selections.set_single(Selection::cursor(at));
    }

    pub fn format(&mut self, format: Format, area: Rect) {
        self.prepare_edit(area);
        match format {
            Format::Bold => self.toggle_inline("**"),
            Format::Italic => self.toggle_inline("*"),
            Format::Code => self.toggle_inline("`"),
            _ => self.toggle_lines(format),
        }
    }

    pub fn selected_text(&self) -> Option<String> {
        let rope = self.buffer.rope();
        let parts: Vec<String> = self
            .selections
            .iter()
            .filter(|s| !s.is_empty())
            .map(|s| rope.slice(s.start()..s.end()).to_string())
            .collect();
        (!parts.is_empty()).then(|| parts.join("\n"))
    }

    pub fn draw(
        &self,
        grid: &mut Grid,
        area: Rect,
        theme: &Theme,
        focused: bool,
        registry: &Registry,
        marks: &Marks,
    ) {
        let full = area;
        let area = self.text_area(full);
        self.view_rows.set(area.h);
        if self.rendered {
            self.draw_page(grid, area, theme, focused, marks);
            if self.shows_minimap(full) {
                self.draw_minimap(grid, full, theme);
            } else {
                let track = Rect::new(area.right(), area.y, 1, area.h);
                self.draw_scrollbar(grid, track, self.page(area).len(), theme);
            }
            return;
        }
        let lines = self.buffer.rope().len_lines();
        let track = Rect::new(area.right(), area.y, 1, area.h);
        if !self.shows_minimap(full) {
            self.draw_scrollbar(grid, track, lines, theme);
        }
        grid.fill(area, theme.text);
        let gutter = self.gutter_width(area.w);
        let rope = self.buffer.rope();
        let current = rope.char_to_line(self.selections.primary().head);
        for row in 0..area.h {
            let line = self.top + row as usize;
            if line >= rope.len_lines() {
                break;
            }
            let y = area.y + row;
            let active = focused && line == current;
            let bg = if active { theme.line } else { theme.text.bg };
            let text_style = cellix_render::Style { bg, ..theme.text };
            grid.fill(Rect::new(area.x, y, area.w, 1), text_style);
            let number_fg = if marks.errors.contains(&(line + 1)) {
                theme.error
            } else if active {
                theme.number_active
            } else {
                theme.gutter.fg
            };
            let gutter_style = cellix_render::Style {
                fg: number_fg,
                bg,
                ..theme.gutter
            };
            let number = format!("{:>w$} ", line + 1, w = gutter as usize - 1);
            grid.put_str(area.x, y, &number, gutter_style, area.x + gutter);
            if marks.comments.contains(&(line + 1)) {
                let mark = cellix_render::Style {
                    fg: theme.accent,
                    bg,
                    ..theme.gutter
                };
                grid.put_str(area.x, y, "◆", mark, area.x + gutter);
            }

            let mut column = 0;
            let mut text = String::new();
            for c in self.line_text(line).chars() {
                let width = display_width(c, column);
                if column + width > self.left {
                    if c == '\t' {
                        text.extend(std::iter::repeat_n(' ', width));
                    } else {
                        text.push(c);
                    }
                }
                column += width;
            }
            grid.put_str(area.x + gutter, y, &text, text_style, area.right());
            let guide = cellix_render::Style {
                fg: theme.guide,
                bg,
                ..theme.text
            };
            let indent = text.chars().take_while(|c| *c == ' ').count();
            for at in (0..indent).step_by(TAB_WIDTH) {
                let x = area.x + gutter + at as u16;
                if x < area.right() {
                    grid.put_str(x, y, "│", guide, area.right());
                }
            }
        }
        self.draw_highlights(grid, area, gutter, theme, registry);
        if self.shows_minimap(full) {
            self.draw_minimap(grid, full, theme);
        }
        for (line, change) in &self.line_changes().1 {
            if *line >= self.top && *line < self.top + area.h as usize {
                let y = area.y + (*line - self.top) as u16;
                self.draw_change(grid, area.x as f32 + gutter as f32 - 1.0, y, *change, theme);
            }
        }
        for (start, end) in self.found.iter().copied() {
            let s = Selection {
                anchor: start,
                head: end,
                goal: None,
            };
            self.draw_range(grid, area, gutter, &s, theme.found);
        }
        for s in self.selections.iter().filter(|s| !s.is_empty()) {
            self.draw_range(grid, area, gutter, s, theme.selection);
        }
        if focused {
            let head = self.selections.primary().head;
            let line = rope.char_to_line(head);
            let column = self.column_of(head);
            if line >= self.top && column >= self.left {
                let x = area.x as usize + gutter as usize + column - self.left;
                let y = area.y as usize + line - self.top;
                if x < area.right() as usize && y < area.bottom() as usize {
                    grid.cursor = Some((x as u16, y as u16));
                }
            }
        }
    }

    fn draw_scrollbar(&self, grid: &mut Grid, track: Rect, lines: usize, theme: &Theme) {
        grid.fill(track, theme.text);
        let rows = track.h as usize;
        if lines <= rows {
            return;
        }
        let height = (rows * rows / lines).max(1);
        let top = self.top * rows / lines;
        // A slim rounded thumb on the pixel tiers, and a block of colour on cells.
        let cells = Rect::new(track.x, track.y + top as u16, 1, height as u16);
        let thumb = cellix_render::Style {
            fg: theme.scrollbar,
            painted: true,
            ..theme.text
        };
        for y in cells.y..cells.bottom() {
            grid.put_str(cells.x, y, "▐", thumb, cells.right());
        }
        let bounds =
            cellix_render::Bounds::cells(cells.x as f32 + 0.3, cells.y as f32, 0.5, cells.h as f32);
        grid.panel(bounds, 20, Some(theme.scrollbar), None);
    }

    /// Backgrounds of code blocks and tables, drawn before the text on top of them.
    fn draw_page_blocks(&self, grid: &mut Grid, area: Rect, rows: &[Row], theme: &Theme) {
        let left = (area.x + Self::page_margin(area)) as f32;
        let visible = self.top..(self.top + area.h as usize).min(rows.len());
        let y_of = |index: usize| area.y as f32 + (index - self.top) as f32;
        let mut index = visible.start;
        while index < visible.end {
            let row = &rows[index];
            let group =
                |r: &Row| (row.shaded && r.shaded) || (row.table.is_some() && r.table.is_some());
            if !group(row) {
                index += 1;
                continue;
            }
            let end = (index..visible.end)
                .find(|i| !group(&rows[*i]))
                .unwrap_or(visible.end);
            let (top, height) = (y_of(index), (end - index) as f32);
            if row.shaded {
                let width = area.right() as f32 - left - 1.0;
                let bounds = Bounds::cells(left, top - 0.08, width, height + 0.2);
                grid.panel(bounds, 22, Some(theme.code_bg), Some(theme.border));
            } else if let Some(table) = &row.table {
                let width = table.columns.last().copied().unwrap_or(0.0);
                let bounds = Bounds::cells(left, top + 0.02, width, height - 0.04);
                if table.header {
                    let head = Bounds::cells(left, top + 0.02, width, 1.0);
                    grid.panel(head, 14, Some(theme.code_bg), None);
                }
                for at in index + 1..end {
                    let line = Bounds::cells(left, y_of(at), width, 0.0);
                    grid.hairline(line, theme.popup_border);
                }
                let inner = table.columns.len().saturating_sub(2);
                for x in table.columns.iter().skip(1).take(inner) {
                    let line = Bounds::cells(left + x, top + 0.02, 0.0, height - 0.04);
                    grid.hairline(line, theme.popup_border);
                }
                grid.panel(bounds, 14, None, Some(theme.popup_border));
            }
            index = end;
        }
    }

    fn draw_page(&self, grid: &mut Grid, area: Rect, theme: &Theme, focused: bool, marks: &Marks) {
        grid.fill(area, theme.text);
        let rows = self.page(area);
        let proportional = !self.measure.in_cells();
        let left = area.x + Self::page_margin(area);
        let primary = self.selections.primary();
        let (caret_row, caret_x) = rendered::locate(&rows, primary.head);
        if proportional {
            self.draw_page_blocks(grid, area, &rows, theme);
        }
        let changes = self.line_changes();
        for (index, row) in rows.iter().enumerate().skip(self.top) {
            let y = area.y + (index - self.top) as u16;
            if y >= area.bottom() {
                break;
            }
            let bg = if row.shaded && !proportional {
                theme.code_bg
            } else {
                theme.text.bg
            };
            if !proportional {
                grid.fill(
                    Rect::new(area.x, y, area.w, 1),
                    cellix_render::Style { bg, ..theme.text },
                );
            }
            // A tall heading's spacer shares its line, and takes no mark.
            let mut earlier = rows[..index]
                .iter()
                .rev()
                .take_while(|r| r.line == row.line);
            let first_row = row.stop && !earlier.any(|r| r.stop);
            if first_row && let Some(change) = changes.1.get(&row.line) {
                self.draw_change(grid, left as f32 - 1.4, y, *change, theme);
            }
            if marks.comments.contains(&(row.line + 1)) {
                let mark = cellix_render::Style {
                    fg: theme.accent,
                    bg,
                    ..theme.text
                };
                grid.put_str(area.x, y, "◆", mark, left);
            }
            // Xcode shows a heading's level in the margin, beside the caret.
            let commented = marks.comments.contains(&(row.line + 1));
            if proportional && focused && index == caret_row && row.heading > 0 && !commented {
                let dim = cellix_render::Style {
                    fg: theme.gutter.fg,
                    ..theme.text
                };
                self.draw_heading_mark(grid, row, (area.x, left, y), dim.fg);
            }
            if row.rule && proportional {
                let line = Bounds::cells(left as f32, y as f32 + 0.5, 60.0, 0.0);
                grid.hairline(line, theme.popup_border);
            }
            if row.quote {
                let bar = Bounds::cells(left as f32 + 0.1, y as f32 + 0.08, 0.3, 0.84);
                grid.panel(bar, 12, Some(theme.popup_border), None);
            }
            // A tall heading on the last row would reach into the status bar, which paint
            // can't be clipped by, so it waits until it fits.
            if !(proportional && row.tall && y + 1 >= area.bottom()) {
                self.draw_page_row(grid, row, (left, y), area, bg, theme);
            }
        }
        if focused && primary.is_empty() && caret_row >= self.top {
            let y = area.y as usize + caret_row - self.top;
            let x = left as f32 + caret_x;
            if y < area.bottom() as usize && x < area.right() as f32 {
                let row = &rows[caret_row];
                let size = row.glyphs.first().map_or(100, |g| g.font.size) as f32 / 100.0;
                grid.set_caret(x, y as u16, (0.62 * size).min(1.6));
                if row.tall
                    && let Some(caret) = &mut grid.caret
                {
                    caret.y += units(0.5);
                }
            }
        }
    }

    /// The small `#` beside a heading, sitting on the heading's baseline.
    fn draw_heading_mark(
        &self,
        grid: &mut Grid,
        row: &Row,
        (x, left, y): (u16, u16, u16),
        color: Rgb,
    ) {
        /// How far a font's baseline sits below the middle of its line, in font sizes.
        const BASELINE: f32 = 0.35;
        const MARK_SIZE: u8 = 85;
        let size = row.glyphs.first().map_or(100, |g| g.font.size);
        let scale = (size as f32 - MARK_SIZE as f32) / 100.0;
        let drop = BASELINE * INTERFACE_SIZE * scale;
        let top = y as f32 + if row.tall { 0.5 } else { 0.0 } + drop;
        let font = Font {
            size: MARK_SIZE,
            ..Font::default()
        };
        grid.text_run(
            (x as f32 + 0.7, top),
            left as f32,
            "#",
            (color, false),
            font,
        );
    }

    fn draw_page_row(
        &self,
        grid: &mut Grid,
        row: &Row,
        (left, y): (u16, u16),
        area: Rect,
        bg: Rgb,
        theme: &Theme,
    ) {
        let proportional = !self.measure.in_cells();
        let origin = left as f32;
        let fg_of = |tone: Tone| match tone {
            Tone::Text | Tone::Block(None) => theme.text.fg,
            Tone::Heading => theme.heading,
            Tone::Code => theme.string,
            Tone::Block(Some(token)) => theme.token(token),
            Tone::Link | Tone::Check(_) => theme.accent,
            Tone::Dim | Tone::TableLine => theme.gutter.fg,
        };
        let selected = |at: Option<usize>| {
            at.is_some_and(|at| {
                self.selections
                    .iter()
                    .any(|s| at >= s.start() && at < s.end())
            })
        };
        let found = |at: Option<usize>| {
            at.is_some_and(|at| self.found.iter().any(|f| at >= f.0 && at < f.1))
        };
        let mark = |at: Option<usize>| {
            if selected(at) {
                Some(theme.selection)
            } else {
                found(at).then_some(theme.found)
            }
        };
        // Cells carry the text for the cell tier, and for tests. Pixels come from paint.
        for g in &row.glyphs {
            let style = cellix_render::Style {
                fg: fg_of(g.tone),
                bg: mark(g.at).filter(|_| !proportional).unwrap_or(bg),
                bold: g.font.bold,
                italic: g.font.italic,
                underline: g.underline,
                painted: proportional,
            };
            let x = left + g.x as u16;
            grid.put_str(x, y, g.ch.encode_utf8(&mut [0; 4]), style, area.right());
        }
        if !proportional {
            return;
        }
        let text_y = y as f32 + if row.tall { 0.5 } else { 0.0 };
        let right = area.right() as f32;
        let mut i = 0;
        while i < row.glyphs.len() {
            let g = row.glyphs[i];
            let same = |o: &Glyph| {
                o.font == g.font
                    && o.tone == g.tone
                    && o.underline == g.underline
                    && mark(o.at) == mark(g.at)
                    && (g.tone != Tone::TableLine || o.x == g.x)
            };
            // A table's cells are apart, so a run stops at a jump in position.
            let mut end = i + 1;
            while end < row.glyphs.len() && same(&row.glyphs[end]) {
                let prior = row.glyphs[end - 1];
                if (prior.x + prior.advance - row.glyphs[end].x).abs() > 0.01 {
                    break;
                }
                end += 1;
            }
            let run = &row.glyphs[i..end];
            let text: String = run.iter().map(|g| g.ch).collect();
            let x = origin + g.x;
            let width: f32 = run.iter().map(|g| g.advance).sum();
            if let Some(color) = mark(g.at) {
                let bounds = Bounds::cells(x, text_y + 0.06, width, 0.88);
                grid.panel(bounds, 6, Some(color), None);
            }
            match g.tone {
                Tone::TableLine => {}
                Tone::Check(checked) => {
                    let bounds = Bounds::cells(x, y as f32 + 0.2, 1.1, 0.6);
                    let fill = checked.then_some(theme.accent);
                    grid.panel(bounds, 16, fill, Some(theme.gutter.fg));
                    if checked {
                        grid.paint.push(cellix_render::Paint::Icon {
                            bounds: Bounds::cells(x + 0.15, y as f32 + 0.22, 1.0, 0.56),
                            icon: Icon::Check,
                            color: theme.button_text,
                        });
                    }
                }
                _ => {
                    if g.tone == Tone::Code {
                        let pill = Bounds::cells(x - 0.2, text_y + 0.14, width + 0.4, 0.72);
                        grid.panel(pill, 14, Some(theme.code_bg), None);
                    }
                    let color = (fg_of(g.tone), g.underline);
                    grid.text_run((x, text_y), right, &text, color, g.font);
                }
            }
            i = end;
        }
    }

    fn draw_highlights(
        &self,
        grid: &mut Grid,
        area: Rect,
        gutter: u16,
        theme: &Theme,
        registry: &Registry,
    ) {
        let Some(path) = self.buffer.path() else {
            return;
        };
        let revision = self.buffer.revision();
        let mut cache = self.spans.borrow_mut();
        if !cache.as_ref().is_some_and(|(r, _)| *r == revision) {
            let bytes = self.buffer.rope().len_bytes();
            if bytes <= MAX_INLINE_HIGHLIGHT {
                *cache = Some((revision, registry.highlight(path, &self.buffer.text())));
                self.spans_bytes.set(bytes);
            } else if self.asked.get() != revision {
                self.asked.set(revision);
                let worker = self.worker.get_or_init(Worker::default);
                worker.request(path, self.buffer.text(), revision);
            }
        }
        let Some((cached, spans)) = cache.as_ref() else {
            return;
        };
        let rope = self.buffer.rope();
        // Until the worker answers, text after the edit keeps its colours by moving with it.
        let moved = rope.len_bytes() as isize - self.spans_bytes.get() as isize;
        let edit = rope.char_to_byte(self.selections.primary().head.min(rope.len_chars()));
        let shifted: Vec<Span>;
        let spans = if *cached == revision || moved == 0 {
            spans
        } else {
            let shift = |at: usize| {
                if at >= edit.saturating_sub(moved.max(0) as usize) {
                    at.saturating_add_signed(moved).min(rope.len_bytes())
                } else {
                    at
                }
            };
            shifted = spans
                .iter()
                .map(|s| Span {
                    start: shift(s.start),
                    end: shift(s.end),
                    token: s.token,
                })
                .collect();
            &shifted
        };
        let last_line = (self.top + area.h as usize).min(rope.len_lines());
        let from = rope.line_to_byte(self.top);
        let to = rope.line_to_byte(last_line);
        let first = spans.partition_point(|s| s.end <= from);
        for span in spans[first..].iter().take_while(|s| s.start < to) {
            let fg = theme.token(span.token);
            let (mut at, end) = (rope.byte_to_char(span.start), rope.byte_to_char(span.end));
            while at < end {
                let line = rope.char_to_line(at);
                let line_end =
                    end.min(rope.line_to_char(line) + self.line_text(line).chars().count());
                if line >= self.top && line < last_line && at < line_end {
                    let (a, b) = (self.column_of(at), self.column_of(line_end));
                    let x_from = a.max(self.left) - self.left;
                    let x_to = b.max(self.left) - self.left;
                    let rect = Rect::new(
                        (area.x + gutter).saturating_add(x_from as u16),
                        area.y + (line - self.top) as u16,
                        (x_to - x_from) as u16,
                        1,
                    );
                    let clipped = Rect::new(
                        rect.x,
                        rect.y,
                        rect.w.min(area.right().saturating_sub(rect.x)),
                        1,
                    );
                    grid.set_style(clipped, |style| cellix_render::Style { fg, ..style });
                }
                let next = rope.len_lines().min(line + 1);
                if next == line + 1 && next < rope.len_lines() {
                    at = rope.line_to_char(next);
                } else {
                    break;
                }
            }
        }
    }

    fn draw_range(&self, grid: &mut Grid, area: Rect, gutter: u16, s: &Selection, bg: Rgb) {
        let rope = self.buffer.rope();
        let (first, last) = (rope.char_to_line(s.start()), rope.char_to_line(s.end()));
        for line in first.max(self.top)..=last {
            if line >= self.top + area.h as usize {
                break;
            }
            let start = rope.line_to_char(line);
            let from = self.column_of(s.start().max(start));
            let line_end = start + self.line_text(line).chars().count();
            let to = self.column_of(s.end().min(line_end));
            let x_from = (area.x + gutter) as usize + from.saturating_sub(self.left);
            let x_to = (area.x + gutter) as usize + to.saturating_sub(self.left);
            let y = area.y + (line - self.top) as u16;
            let rect = Rect::new(
                x_from as u16,
                y,
                (x_to - x_from.min(x_to)).max(usize::from(line < last)) as u16,
                1,
            );
            grid.set_style(rect, |style| cellix_render::Style { bg, ..style });
        }
    }
}
