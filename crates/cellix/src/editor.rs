use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

use cellix_core::{Buffer, Motion, Selection, Selections};
use cellix_input::{Command, Direction};
use cellix_lang::{Registry, Span};
use cellix_render::{Bounds, Font, Grid, INTERFACE_SIZE, Icon, Measure, Rect, Rgb, units};
use unicode_width::UnicodeWidthChar;

use crate::rendered::{self, Glyph, Intent, Row, Tone};
use crate::theme::Theme;

const TAB_WIDTH: usize = 4;
const GUTTER_MIN_DIGITS: usize = 4;
const NARROW_PANE: u16 = 60;
/// Left margin of a rendered page, which also carries comment marks.
const PAGE_MARGIN: u16 = 2;
/// How wide a page can be set. Prose is narrower: see `rendered`.
const MAX_PAGE_WIDTH: u16 = 200;

/// Per-line marks for the gutter, as 1-based lines.
#[derive(Default)]
pub struct Marks {
    pub comments: Vec<usize>,
    pub errors: Vec<usize>,
}

/// A laid-out page: the revision and width it was made for, and its rows.
type Page = (u64, usize, Rc<Vec<Row>>);

/// One open document and where the view is within it.
pub struct Editor {
    pub buffer: Buffer,
    pub selections: Selections,
    pub top: usize,
    pub left: usize,
    spans: RefCell<Option<(u64, Vec<Span>)>>,
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
            top: 0,
            left: 0,
            spans: RefCell::new(None),
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

    /// Where text goes in `area`: all of it, less the scrollbar.
    fn text_area(&self, area: Rect) -> Rect {
        Rect::new(area.x, area.y, area.w.saturating_sub(1), area.h)
    }

    fn page_width(area: Rect) -> usize {
        area.w
            .saturating_sub(PAGE_MARGIN + 1)
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
        let column = x - (area.x + PAGE_MARGIN) as f32;
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
        x == self.text_area(area).right()
    }

    /// Scrolls so that row `y` of the scrollbar's track matches the document.
    pub fn scroll_to_row(&mut self, area: Rect, y: u16) {
        let lines = if self.rendered {
            self.page(self.text_area(area)).len()
        } else {
            self.buffer.rope().len_lines()
        };
        let row = y.saturating_sub(area.y) as usize;
        self.top = (row * lines / area.h.max(1) as usize).min(lines.saturating_sub(1));
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
            let column = (fine_x - (area.x + PAGE_MARGIN) as f32).max(0.0);
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

    pub fn scroll_by(&mut self, delta: isize) {
        // A cached page is close enough to bound the scroll; draw corrects any slack.
        let rows = self.page.borrow().as_ref().map(|(_, _, rows)| rows.len());
        let lines = rows
            .filter(|_| self.rendered)
            .unwrap_or_else(|| self.buffer.rope().len_lines());
        let last = lines.saturating_sub(1) as isize;
        self.top = (self.top as isize + delta).clamp(0, last) as usize;
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
            Command::Newline => {
                let head = self.selections.primary().head;
                let rope = self.buffer.rope();
                let line = rope.line(rope.char_to_line(head));
                let indent: String = line
                    .chars()
                    .take_while(|c| matches!(c, ' ' | '\t'))
                    .collect();
                self.buffer
                    .type_text(&mut self.selections, &format!("\n{indent}"));
            }
            Command::Indent => self.buffer.type_text(&mut self.selections, "    "),
            Command::Outdent => self.outdent(),
            Command::DeleteBackward => self.buffer.delete_backward(&mut self.selections),
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
        if self.rendered {
            self.draw_page(grid, area, theme, focused, marks);
            let track = Rect::new(area.right(), area.y, 1, area.h);
            self.draw_scrollbar(grid, track, self.page(area).len(), theme);
            return;
        }
        let lines = self.buffer.rope().len_lines();
        let track = Rect::new(area.right(), area.y, 1, area.h);
        self.draw_scrollbar(grid, track, lines, theme);
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
        for s in self.selections.iter().filter(|s| !s.is_empty()) {
            self.draw_selection(grid, area, gutter, s, theme);
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
        let left = (area.x + PAGE_MARGIN) as f32;
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
        let left = area.x + PAGE_MARGIN;
        let primary = self.selections.primary();
        let (caret_row, caret_x) = rendered::locate(&rows, primary.head);
        if proportional {
            self.draw_page_blocks(grid, area, &rows, theme);
        }
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
            self.draw_page_row(grid, row, (left, y), area, bg, theme);
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
        // Cells carry the text for the cell tier, and for tests. Pixels come from paint.
        for g in &row.glyphs {
            let style = cellix_render::Style {
                fg: fg_of(g.tone),
                bg: if selected(g.at) && !proportional {
                    theme.selection
                } else {
                    bg
                },
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
                    && selected(o.at) == selected(g.at)
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
            if selected(g.at) {
                let bounds = Bounds::cells(x, text_y + 0.06, width, 0.88);
                grid.panel(bounds, 6, Some(theme.selection), None);
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
            *cache = Some((revision, registry.highlight(path, &self.buffer.text())));
        }
        let Some((_, spans)) = cache.as_ref() else {
            return;
        };
        let rope = self.buffer.rope();
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

    fn draw_selection(
        &self,
        grid: &mut Grid,
        area: Rect,
        gutter: u16,
        s: &Selection,
        theme: &Theme,
    ) {
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
            let bg = theme.selection;
            grid.set_style(rect, |style| cellix_render::Style { bg, ..style });
        }
    }
}
