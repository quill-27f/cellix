use std::cell::RefCell;
use std::path::Path;

use cellix_core::{Buffer, Motion, Selection, Selections};
use cellix_input::{Command, Direction};
use cellix_lang::{Registry, Span};
use cellix_render::{Grid, Rect};
use unicode_width::UnicodeWidthChar;

use crate::theme::Theme;

const TAB_WIDTH: usize = 4;
const GUTTER_MIN_DIGITS: usize = 4;
const NARROW_PANE: u16 = 60;
/// Below this a source and preview side by side are too narrow to read.
const MIN_SPLIT_WIDTH: u16 = 60;

/// Per-line marks for the gutter, as 1-based lines.
#[derive(Default)]
pub struct Marks {
    pub comments: Vec<usize>,
    pub errors: Vec<usize>,
}

/// One open document and where the view is within it.
pub struct Editor {
    pub buffer: Buffer,
    pub selections: Selections,
    pub top: usize,
    pub left: usize,
    spans: RefCell<Option<(u64, Vec<Span>)>>,
    /// Show Markdown as formatted text.
    pub preview: bool,
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
    pub fn new(buffer: Buffer) -> Self {
        let extension = buffer.path().and_then(Path::extension);
        let is_markdown = extension.is_some_and(|e| e == "md" || e == "markdown");
        Self {
            buffer,
            selections: Selections::default(),
            top: 0,
            left: 0,
            spans: RefCell::new(None),
            preview: is_markdown,
            synced: 0,
        }
    }

    pub fn title(&self) -> String {
        self.buffer
            .path()
            .and_then(Path::file_name)
            .map_or_else(|| "untitled".into(), |n| n.to_string_lossy().into_owned())
    }

    /// The source's share of `area`: the left half beside a preview, and otherwise all
    /// of it, less the scrollbar.
    fn text_area(&self, area: Rect) -> Rect {
        let split = self.preview && area.w >= MIN_SPLIT_WIDTH;
        let width = if split { area.w / 2 } else { area.w };
        Rect::new(area.x, area.y, width.saturating_sub(1), area.h)
    }

    /// True when a click at `x` lands on the scrollbar.
    pub fn on_scrollbar(&self, area: Rect, x: u16) -> bool {
        x == self.text_area(area).right()
    }

    /// Scrolls so that row `y` of the scrollbar's track matches the document.
    pub fn scroll_to_row(&mut self, area: Rect, y: u16) {
        let lines = self.buffer.rope().len_lines();
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
    pub fn position_at(&self, area: Rect, x: u16, y: u16) -> usize {
        let area = self.text_area(area);
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
        let last = self.buffer.rope().len_lines().saturating_sub(1) as isize;
        self.top = (self.top as isize + delta).clamp(0, last) as usize;
    }

    /// Scrolls just enough to show the primary cursor.
    pub fn reveal(&mut self, area: Rect) {
        let area = self.text_area(area);
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
        if area.right() < full.right() {
            self.draw_scrollbar(grid, Rect::new(area.right(), area.y, 1, area.h), theme);
        }
        if self.preview && full.w >= MIN_SPLIT_WIDTH {
            let x = area.right() + 1;
            let right = Rect::new(x, area.y, full.right() - x, area.h);
            let lines = self.buffer.rope().len_lines().saturating_sub(1).max(1);
            let scroll = self.top as f32 / lines as f32;
            crate::markdown::draw(grid, right, &self.buffer.text(), scroll, theme);
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

    fn draw_scrollbar(&self, grid: &mut Grid, track: Rect, theme: &Theme) {
        grid.fill(track, theme.text);
        let lines = self.buffer.rope().len_lines().max(1);
        let rows = track.h as usize;
        if lines <= rows {
            return;
        }
        let height = (rows * rows / lines).max(1);
        let top = self.top * rows / lines;
        let thumb = cellix_render::Style {
            bg: theme.scrollbar,
            ..theme.text
        };
        grid.fill(
            Rect::new(track.x, track.y + top as u16, 1, height as u16),
            thumb,
        );
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
