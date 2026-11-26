//! The To-dos view: what is left in the workspace, in progress first, then to do, then what's
//! done. It reads the same `.cellix/todos.jsonl` the hooks and `cellix todo` write, so the
//! list is shared with every agent in the folder.
//!
//! Marks are one column of a 16 px box where there are pictures, and a glyph on cells: an
//! empty box, a half box for a parent some of whose children are done, a tick, and the
//! shared spinner for what's under way. A parent's box groups its children; a child stands
//! two columns in under an elbow, as the Agents list draws one.

use std::collections::HashSet;
use std::path::Path;
use std::time::SystemTime;

use cellix_render::{Bounds, Grid, Icon, Measure, Paint, Rect, Rgb, Style};
use cellix_review::todos as store;
use cellix_review::todos::{Item, State};

use crate::theme::Theme;

/// The title's count, set as the title is but not bold.
const COUNT_SIZE: u8 = 85;
/// The words a parent shows of how far it has got.
const PROGRESS_SIZE: u8 = 80;
/// What an item under way is doing, under its words.
const ACTIVE_SIZE: u8 = 80;
/// The owner at a row's end: quiet, and smaller than the words.
const OWNER_SIZE: u8 = 73;
const WORD_SIZE: u8 = 100;
/// A mark's box: 16 px on the canvas's 37 px row.
const MARK: f32 = 16.0 / 37.0;

/// The canvas's cell: 17 px across, so positions measured in pixels are columns when divided
/// by it. The words start 76 px from the sidebar's edge, a mark at 51 px and 16 px wide with
/// 9 px after it; a child's mark stands two columns further in.
const CELL: f32 = 17.0;
const MARK_LEFT: f32 = 3.0;
const WORDS_LEFT: f32 = 76.0 / CELL;
const CHILD_WORDS_LEFT: f32 = 110.0 / CELL;
const INDENT: u16 = 2;
const MARK_CELL: u16 = 3;
const WORDS_CELL: u16 = 4;

/// One column clear at the sidebar's far edge, and three for the owner at a row's end.
const RIGHT: u16 = 1;
const OWNER: u16 = 3;

/// How far the elbow over a child reaches into the row above, and how strong its line is
/// toward the words' colour, as the Agents list draws one.
const ELBOW_TOP: f32 = 0.04;
const ELBOW_SHARE: f32 = 0.21;

/// One line of the list, in the order it is drawn. An item under way with an active form
/// takes a second line under it, the form quiet.
enum Line {
    Section(&'static str, usize),
    Item(ItemRow),
    Active { at: usize },
}

/// A row of the list: an item, its depth, and what its row needs to know.
struct ItemRow {
    /// Its place in the items read from the log.
    at: usize,
    depth: u8,
    /// The last of its parent's children, whose elbow stops at its row instead of going on.
    last: bool,
    /// A parent's children done of its children all told.
    progress: Option<(usize, usize)>,
    /// Done, so its words are quiet.
    muted: bool,
    /// Whether the active form follows on the next line.
    active: bool,
}

/// The To-dos panel in the sidebar, and what it last read.
#[derive(Default)]
pub struct Todos {
    pub items: Vec<Item>,
    /// The first line drawn, for scrolling a list that does not fit.
    top: usize,
    /// The children of a parent folded away, by the parent's id.
    folded: HashSet<String>,
    /// When and how big the log was when it was last read, to know when to read it again.
    seen: Option<(SystemTime, u64)>,
}

impl Todos {
    /// Reads the log afresh, as opening the view does.
    pub fn show(&mut self, root: &Path) {
        self.top = 0;
        self.seen = store::changed(root);
        self.items = store::list(root);
    }

    /// Reads the log again if it changed since it was last read, and says whether it did. The
    /// workspace's log is stat'd each tick even while the view is closed, since the views'
    /// row carries how many items are under way.
    pub fn poll(&mut self, root: &Path) -> bool {
        let seen = store::changed(root);
        if seen == self.seen {
            return false;
        }
        self.items = store::list(root);
        self.seen = seen;
        true
    }

    /// How many items are under way, for the views' badge.
    pub fn in_progress(&self) -> usize {
        self.items
            .iter()
            .filter(|item| item.state == State::Doing)
            .count()
    }

    pub fn scroll_by(&mut self, delta: isize) {
        let last = self.lines().len().saturating_sub(1);
        self.top = self.top.saturating_add_signed(delta).min(last);
    }

    /// The top-level items done, and there are, for the title's count.
    fn counts(&self) -> Option<(usize, usize)> {
        let tops: Vec<&Item> = store::top_level(&self.items).collect();
        if tops.is_empty() {
            return None;
        }
        let done = tops
            .iter()
            .filter(|item| store::effective(item, &self.items) == State::Done)
            .count();
        Some((done, tops.len()))
    }

    /// The list, section by section, as lines to draw: in progress, to do, and done.
    fn lines(&self) -> Vec<Line> {
        let mut doing = Vec::new();
        let mut todo = Vec::new();
        let mut done = Vec::new();
        for (at, item) in self.items.iter().enumerate() {
            if item.parent.is_some() {
                continue;
            }
            match store::effective(item, &self.items) {
                State::Doing => doing.push(at),
                State::Todo => todo.push(at),
                State::Done => done.push(at),
            }
        }
        let mut lines = Vec::new();
        for (name, bucket) in [("IN PROGRESS", &doing), ("TO DO", &todo), ("DONE", &done)] {
            if bucket.is_empty() {
                continue;
            }
            lines.push(Line::Section(name, bucket.len()));
            for &at in bucket {
                self.item_lines(&mut lines, at, 0, false);
            }
        }
        lines
    }

    /// One item's lines: its own, then its children's under it unless it is folded.
    fn item_lines(&self, lines: &mut Vec<Line>, at: usize, depth: u8, last: bool) {
        let item = &self.items[at];
        let children: Vec<usize> = self
            .items
            .iter()
            .enumerate()
            .filter(|(_, child)| child.parent.as_deref() == Some(item.id.as_str()))
            .map(|(index, _)| index)
            .collect();
        let progress = (!children.is_empty()).then(|| {
            let done = children
                .iter()
                .filter(|index| self.items[**index].state == State::Done)
                .count();
            (done, children.len())
        });
        let active = item.state == State::Doing && !item.active.trim().is_empty();
        let muted = if depth == 0 {
            store::effective(item, &self.items) == State::Done
        } else {
            item.state == State::Done
        };
        lines.push(Line::Item(ItemRow {
            at,
            depth,
            last,
            progress,
            muted,
            active,
        }));
        if active {
            lines.push(Line::Active { at });
        }
        if progress.is_some() && self.folded.contains(&item.id) {
            return;
        }
        for (index, &child) in children.iter().enumerate() {
            let last = index + 1 == children.len();
            self.item_lines(lines, child, depth + 1, last);
        }
    }

    /// Draws the panel: its title, its sections, and a row for each item and child.
    pub fn draw(
        &self,
        grid: &mut Grid,
        area: Rect,
        hover: Option<(u16, u16)>,
        (theme, measure): (&Theme, &dyn Measure),
    ) {
        grid.fill(area, theme.sidebar);
        crate::palette::sidebar_title(grid, area, "To-dos", theme);
        let right = area.right().saturating_sub(RIGHT);
        if let Some((done, total)) = self.counts() {
            let count = format!("{done} of {total}");
            let style = Style {
                fg: theme.tab.fg,
                ..theme.sidebar
            };
            grid.label_right_at(right as f32, right, area.y, &count, style, COUNT_SIZE);
        }
        if self.counts().is_none() {
            self.draw_empty(grid, area, (theme, measure));
            return;
        }
        let lines = self.lines();
        let visible = area.h.saturating_sub(1) as usize;
        if visible == 0 {
            return;
        }
        let top = self.top.min(lines.len().saturating_sub(1));
        for (row, line) in lines.iter().skip(top).take(visible).enumerate() {
            let y = area.y + 1 + row as u16;
            match line {
                Line::Section(name, count) => {
                    let count = count.to_string();
                    let look = (*name, Some((count.as_str(), theme.gutter.fg)));
                    crate::palette::section_heading(
                        grid,
                        (area.x + MARK_CELL, right, y),
                        look,
                        None,
                        theme,
                    );
                }
                Line::Item(row) => {
                    self.draw_item(grid, area, y, row, hover, (theme, measure));
                }
                Line::Active { at } => {
                    self.draw_active(grid, area, y, *at, (theme, measure));
                }
            }
        }
    }

    fn draw_empty(&self, grid: &mut Grid, area: Rect, (theme, measure): (&Theme, &dyn Measure)) {
        let right = area.right().saturating_sub(RIGHT);
        let x = area.x + WORDS_CELL;
        let x_px = area.x as f32 + WORDS_LEFT;
        let space = words_room(measure, right, x, area.x as f32 + WORDS_LEFT);
        let mut y = area.y + 1;
        let note = Style {
            fg: theme.sidebar.fg,
            ..theme.sidebar
        };
        grid.label_at(x_px, x, y, "No to-dos yet.", note, right, WORD_SIZE);
        y += 1;
        let quiet = Style {
            fg: theme.tab.fg,
            ..theme.sidebar
        };
        for line in crate::threads::wrap("An agent's plan shows here as it works.", space, measure)
        {
            if y >= area.bottom() {
                break;
            }
            grid.label_at(x_px, x, y, &line, quiet, right, ACTIVE_SIZE);
            y += 1;
        }
    }

    fn draw_item(
        &self,
        grid: &mut Grid,
        area: Rect,
        y: u16,
        row: &ItemRow,
        hover: Option<(u16, u16)>,
        (theme, measure): (&Theme, &dyn Measure),
    ) {
        let (at, depth, last, progress, muted, active) = (
            row.at,
            row.depth,
            row.last,
            row.progress,
            row.muted,
            row.active,
        );
        let item = &self.items[at];
        let tall = 1 + u16::from(active);
        let row = Rect::new(area.x, y, area.w, tall);
        if hover.is_some_and(|h| row.contains(h.0, h.1)) {
            grid.panel(
                Bounds::inset(row, 0.5, 0.06),
                12,
                Some(theme.row_hover),
                None,
            );
        }
        let (mark_x, mark_cell) = mark_left(area, depth);
        if depth > 0 {
            draw_elbow(grid, area, y, (mark_x, last), (theme, measure));
        }
        self.draw_mark(grid, (mark_x, mark_cell), y, at, progress, (theme, measure));
        let words_x = if depth == 0 {
            WORDS_LEFT
        } else {
            CHILD_WORDS_LEFT
        };
        let words_cell = if depth == 0 {
            WORDS_CELL
        } else {
            WORDS_CELL + INDENT
        };
        let right = area.right().saturating_sub(RIGHT);
        let max_x = words_right(area, measure, progress);
        let space = words_room(measure, max_x, area.x + words_cell, area.x as f32 + words_x);
        let style = Style {
            fg: if muted {
                theme.tab.fg
            } else {
                theme.sidebar.fg
            },
            ..theme.sidebar
        };
        let shown = crate::context::elide(&item.text, space, measure);
        grid.label_at(
            area.x as f32 + words_x,
            area.x + words_cell,
            y,
            &shown,
            style,
            max_x,
            WORD_SIZE,
        );
        if let Some((done, total)) = progress {
            self.draw_progress(grid, (right, y), (done, total), (theme, measure));
        }
        self.draw_owner(grid, (right, y), at, progress, (theme, measure));
    }

    fn draw_active(
        &self,
        grid: &mut Grid,
        area: Rect,
        y: u16,
        at: usize,
        (theme, measure): (&Theme, &dyn Measure),
    ) {
        // The wash belongs to the item's first line, which is the row above; the words here
        // are quiet under it.
        let item = &self.items[at];
        let depth = if item.parent.is_some() { 1 } else { 0 };
        let words_x = if depth == 0 {
            WORDS_LEFT
        } else {
            CHILD_WORDS_LEFT
        };
        let words_cell = if depth == 0 {
            WORDS_CELL
        } else {
            WORDS_CELL + INDENT
        };
        let max_x = words_right(area, measure, None);
        let space = words_room(measure, max_x, area.x + words_cell, area.x as f32 + words_x);
        let style = Style {
            fg: theme.tab.fg,
            ..theme.sidebar
        };
        let mut active = item.active.trim().to_string();
        if !active.is_empty() && !active.ends_with(|c: char| ".!?…".contains(c)) {
            active.push('…');
        }
        let shown = crate::context::elide(&active, space, measure);
        grid.label_at(
            area.x as f32 + words_x,
            area.x + words_cell,
            y,
            &shown,
            style,
            max_x,
            ACTIVE_SIZE,
        );
    }

    fn draw_mark(
        &self,
        grid: &mut Grid,
        (x, cell): (f32, u16),
        y: u16,
        at: usize,
        progress: Option<(usize, usize)>,
        (theme, measure): (&Theme, &dyn Measure),
    ) {
        let item = &self.items[at];
        match progress {
            Some((done, total)) if done == total => tick(grid, (x, cell), y, theme, measure),
            Some((done, _)) if done > 0 => {
                checkbox(grid, (x, cell), y, theme.tab.fg, "◐", theme, measure);
                if !measure.in_cells() {
                    // The line across the box that says part of the group is done.
                    let dash = Bounds::cells(x + 0.26, y as f32 + 0.5 - 0.027, 0.42, 0.054);
                    grid.panel(dash, 6, Some(theme.tab.fg), None);
                }
            }
            Some(_) => checkbox(grid, (x, cell), y, theme.gutter.fg, "☐", theme, measure),
            None => match item.state {
                State::Done => tick(grid, (x, cell), y, theme, measure),
                State::Doing => {
                    grid.spinner(x, y, MARK, theme.accent, theme.sidebar);
                }
                State::Todo => {
                    checkbox(grid, (x, cell), y, theme.gutter.fg, "☐", theme, measure);
                }
            },
        }
    }

    /// How far a parent has got: its children done of them all, at the right of its row.
    fn draw_progress(
        &self,
        grid: &mut Grid,
        (right, y): (u16, u16),
        (done, total): (usize, usize),
        (theme, measure): (&Theme, &dyn Measure),
    ) {
        let text = format!("{done}/{total}");
        let style = Style {
            fg: theme.gutter.fg,
            ..theme.sidebar
        };
        // Where there are pictures it stands three columns before the owner, its ink ending
        // there; on cells it takes the row's end, since the owner goes to the tip there.
        let at = if measure.in_cells() {
            right
        } else {
            right.saturating_sub(OWNER)
        };
        grid.label_right_at(at as f32, at, y, &text, style, PROGRESS_SIZE);
    }

    /// The owner at a row's end: who it is, marked rather than named where room is short.
    /// On cells a parent's progress takes that place, and the tip names the owner.
    fn draw_owner(
        &self,
        grid: &mut Grid,
        (right, y): (u16, u16),
        at: usize,
        progress: Option<(usize, usize)>,
        (theme, measure): (&Theme, &dyn Measure),
    ) {
        if measure.in_cells() && progress.is_some() {
            return;
        }
        let item = &self.items[at];
        if item.owner == store::YOU {
            let style = Style {
                fg: theme.gutter.fg,
                ..theme.sidebar
            };
            grid.label_right_at(right as f32, right, y, store::YOU, style, OWNER_SIZE);
            return;
        }
        let icon = crate::dock::program_icon(&item.owner, false, false);
        let colour = crate::dock::icon_colour(icon, theme);
        let initial = crate::dock::initial(icon, &item.owner);
        let mut buffer = [0; 4];
        let fallback = match initial {
            Some(letter) => &*letter.encode_utf8(&mut buffer),
            None => crate::toolbar::fallback(icon),
        };
        let shown = Style {
            fg: colour,
            ..theme.sidebar
        };
        if measure.in_cells() {
            grid.put_str(right.saturating_sub(1), y, fallback, shown, right);
        } else {
            // Its ink ends where the owner's does, one column clear of the sidebar's edge.
            let x = f32::from(right) - 16.0 / CELL;
            grid.paint.push(Paint::Icon {
                bounds: Bounds::cells(x, y as f32 + (1.0 - MARK) / 2.0, 1.0, MARK),
                icon,
                color: colour,
            });
            if let Some(letter) = initial {
                crate::dock::initial_over(
                    grid,
                    (x + 8.0 / CELL, y),
                    (MARK, letter),
                    theme.sidebar.bg,
                );
            }
        }
    }
}

/// A mark's left edge, in columns and in cells, for an item at `depth`.
fn mark_left(area: Rect, depth: u8) -> (f32, u16) {
    let columns = f32::from(depth) * f32::from(INDENT);
    (
        area.x as f32 + MARK_LEFT + columns,
        area.x + MARK_CELL + u16::from(depth) * INDENT,
    )
}

/// The furthest column an item's words may reach: before the owner, or before the owner and
/// the progress on a parent's row where there are pictures, with a column's air.
fn words_right(area: Rect, measure: &dyn Measure, progress: Option<(usize, usize)>) -> u16 {
    let right = area.right().saturating_sub(RIGHT + OWNER);
    if measure.in_cells() {
        return right.saturating_sub(1);
    }
    if progress.is_some() {
        right.saturating_sub(3)
    } else {
        right
    }
}

/// How many columns there are for words from `x_px` (or cell `x`) to `max_x`.
fn words_room(measure: &dyn Measure, max_x: u16, x: u16, x_px: f32) -> f32 {
    let from = if measure.in_cells() {
        f32::from(x)
    } else {
        x_px
    };
    (f32::from(max_x) - from).max(1.0)
}

/// The box to tick, one column wide: a rounded outline on pictures and `fallback` on cells.
fn checkbox(
    grid: &mut Grid,
    (x, cell): (f32, u16),
    y: u16,
    colour: Rgb,
    fallback: &str,
    theme: &Theme,
    measure: &dyn Measure,
) {
    let shown = Style {
        fg: colour,
        painted: true,
        ..theme.sidebar
    };
    grid.put_str(cell, y, fallback, shown, cell + 1);
    if !measure.in_cells() {
        // The canvas's box: 16 px square in a 17 px column, its corners round as the icons'.
        let box_ = Bounds::cells(x, y as f32 + (1.0 - MARK) / 2.0, 16.0 / CELL, MARK);
        grid.panel(box_, 8, None, Some(colour));
    }
}

/// The tick that says a to-do is done: a check, with no box round it.
fn tick(grid: &mut Grid, (x, cell): (f32, u16), y: u16, theme: &Theme, measure: &dyn Measure) {
    let colour = theme.git_added;
    let shown = Style {
        fg: colour,
        painted: true,
        ..theme.sidebar
    };
    grid.put_str(cell, y, "☑", shown, cell + 1);
    if !measure.in_cells() {
        grid.paint.push(Paint::Icon {
            bounds: Bounds::cells(x, y as f32 + (1.0 - MARK) / 2.0, 1.0, MARK),
            icon: Icon::Check,
            color: colour,
        });
    }
}

/// The line from a parent's marks down and across to a child's: a corner on cells, and two
/// hairlines with a rounded ell where there are pictures.
fn draw_elbow(
    grid: &mut Grid,
    area: Rect,
    y: u16,
    (mark_x, last): (f32, bool),
    (theme, measure): (&Theme, &dyn Measure),
) {
    let line = crate::theme::mix(theme.sidebar.fg, theme.sidebar.bg, ELBOW_SHARE);
    if measure.in_cells() {
        let corner = if last { "╰" } else { "├" };
        let style = Style {
            fg: line,
            ..theme.sidebar
        };
        grid.put_str(area.x + MARK_CELL, y, corner, style, area.right());
        return;
    }
    let x = area.x as f32 + MARK_LEFT;
    let down = if last {
        0.5 + ELBOW_TOP
    } else {
        1.0 + ELBOW_TOP
    };
    grid.hairline(Bounds::cells(x, y as f32 - ELBOW_TOP, 0.0, down), line);
    let across = mark_x - 0.02 - x;
    if across > 0.0 {
        grid.hairline(Bounds::cells(x, y as f32 + 0.5, across, 0.0), line);
    }
}
