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

/// One open document and where the view is within it.
pub struct Editor {
    pub buffer: Buffer,
    pub selections: Selections,
    pub top: usize,
    pub left: usize,
    spans: RefCell<Option<(u64, Vec<Span>)>>,
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
        Self {
            buffer,
            selections: Selections::default(),
            top: 0,
            left: 0,
            spans: RefCell::new(None),
        }
    }

    pub fn title(&self) -> String {
        self.buffer
            .path()
            .and_then(Path::file_name)
            .map_or_else(|| "untitled".into(), |n| n.to_string_lossy().into_owned())
    }

    fn gutter_width(&self) -> u16 {
        let digits = self.buffer.rope().len_lines().to_string().len();
        (digits.max(GUTTER_MIN_DIGITS) + 2) as u16
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
        let rope = self.buffer.rope();
        let line = (self.top + (y.saturating_sub(area.y)) as usize).min(rope.len_lines() - 1);
        let target = self.left + x.saturating_sub(area.x + self.gutter_width()) as usize;
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
        let head = self.selections.primary().head;
        let line = self.buffer.rope().char_to_line(head);
        let rows = area.h.max(1) as usize;
        if line < self.top {
            self.top = line;
        } else if line >= self.top + rows {
            self.top = line + 1 - rows;
        }
        let column = self.column_of(head);
        let cols = area.w.saturating_sub(self.gutter_width()).max(1) as usize;
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
    ) {
        grid.fill(area, theme.text);
        let gutter = self.gutter_width();
        let rope = self.buffer.rope();
        for row in 0..area.h {
            let line = self.top + row as usize;
            if line >= rope.len_lines() {
                break;
            }
            let y = area.y + row;
            let number = format!("{:>w$} ", line + 1, w = gutter as usize - 1);
            grid.put_str(area.x, y, &number, theme.gutter, area.x + gutter);

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
            grid.put_str(area.x + gutter, y, &text, theme.text, area.right());
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
