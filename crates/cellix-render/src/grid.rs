use compact_str::CompactString;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::icon::{Icon, Piece};
use crate::measure::Font;
use crate::paint::{Bounds, Paint, UNIT, units};

/// Heights of a button, in rows.
const BUTTON_HEIGHT: f32 = 0.9;
const LARGE_BUTTON_HEIGHT: f32 = 1.22;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Rgb(pub u8, pub u8, pub u8);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Style {
    pub fg: Rgb,
    pub bg: Rgb,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    /// The pixel tiers leave this cell's glyph out, because paint draws its text.
    pub painted: bool,
}

/// A screen cell. An empty symbol is the right half of a wide character.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Cell {
    pub symbol: CompactString,
    pub style: Style,
}

/// Copies a run of cells within the grid's own cells. `Cell` isn't `Copy`, so this clones.
trait CopyCells {
    fn copy_within_cells(&mut self, from: std::ops::Range<usize>, to: usize);
}

impl CopyCells for Vec<Cell> {
    fn copy_within_cells(&mut self, from: std::ops::Range<usize>, to: usize) {
        for (n, at) in from.enumerate() {
            let cell = self[at].clone();
            self[to + n] = cell;
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rect {
    pub x: u16,
    pub y: u16,
    pub w: u16,
    pub h: u16,
}

impl Rect {
    pub fn new(x: u16, y: u16, w: u16, h: u16) -> Self {
        Self { x, y, w, h }
    }

    pub fn contains(&self, x: u16, y: u16) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }

    pub fn right(&self) -> u16 {
        self.x + self.w
    }

    pub fn bottom(&self) -> u16 {
        self.y + self.h
    }
}

/// A span of changed cells on one row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Run {
    pub x: u16,
    pub y: u16,
    pub len: u16,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Grid {
    width: u16,
    height: u16,
    cells: Vec<Cell>,
    pub cursor: Option<(u16, u16)>,
    /// The caret's exact place on the pixel tiers, when text isn't on whole cells.
    pub caret: Option<Bounds>,
    /// Drawn over the cells on the pixel tiers, in order.
    pub paint: Vec<Paint>,
    /// Columns where a new band starts, in order. Each band of a row is its own tile, so text
    /// that scrolls isn't tied to whatever the sidebar or minimap shows at that height.
    pub bands: Vec<u16>,
}

impl Grid {
    /// Moves rows `top..=bottom` up by `by` rows, or down if negative, across the whole width,
    /// as a terminal's scroll region does. The rows left behind hold nothing any real cell
    /// matches, so the next diff draws them.
    pub fn scroll_rows(&mut self, top: u16, bottom: u16, by: i32) {
        let width = self.width as usize;
        let (top, bottom) = (
            top as usize,
            (bottom as usize).min(self.height as usize - 1),
        );
        if top > bottom || by == 0 {
            return;
        }
        let rows = bottom + 1 - top;
        let n = (by.unsigned_abs() as usize).min(rows);
        let unknown = Cell {
            symbol: CompactString::const_new("\u{fffe}"),
            style: Style::default(),
        };
        let row = |y: usize| y * width..(y + 1) * width;
        if by > 0 {
            for y in top..bottom + 1 - n {
                self.cells.copy_within_cells(row(y + n), row(y).start);
            }
            for y in bottom + 1 - n..=bottom {
                self.cells[row(y)].fill(unknown.clone());
            }
        } else {
            for y in (top + n..=bottom).rev() {
                self.cells.copy_within_cells(row(y - n), row(y).start);
            }
            for y in top..top + n {
                self.cells[row(y)].fill(unknown.clone());
            }
        }
    }

    pub fn new(width: u16, height: u16, style: Style) -> Self {
        let blank = Cell {
            symbol: " ".into(),
            style,
        };
        Self {
            width,
            height,
            cells: vec![blank; width as usize * height as usize],
            cursor: None,
            caret: None,
            paint: Vec::new(),
            bands: Vec::new(),
        }
    }

    /// Text in the interface font on the pixel tiers, and ordinary cells on the cell tier.
    pub fn label(&mut self, x: u16, y: u16, text: &str, style: Style, max_x: u16) -> u16 {
        self.label_at(x as f32, x, y, text, style, max_x, 100)
    }

    /// A label starting between cells, at a size relative to the interface font.
    #[allow(clippy::too_many_arguments)]
    pub fn label_at(
        &mut self,
        x_px: f32,
        x: u16,
        y: u16,
        text: &str,
        style: Style,
        max_x: u16,
        size: u8,
    ) -> u16 {
        let end = self.put_str(
            x,
            y,
            text,
            Style {
                painted: true,
                ..style
            },
            max_x,
        );
        if y < self.height {
            self.paint.push(Paint::Text {
                x: units(x_px),
                y: y as i32 * UNIT,
                max_width: units(max_x.min(self.width) as f32 - x_px),
                text: text.to_string(),
                color: style.fg,
                font: Font {
                    bold: style.bold,
                    size,
                    ..Font::default()
                },
                align_right: false,
                centred: false,
                underline: false,
            });
        }
        end
    }

    /// Paints a run of text with no cells behind it, at a place in cells and rows.
    pub fn text_run(
        &mut self,
        (x, y): (f32, f32),
        max_x: f32,
        text: &str,
        (color, underline): (Rgb, bool),
        font: Font,
    ) {
        self.paint.push(Paint::Text {
            x: units(x),
            y: units(y),
            max_width: units(max_x - x),
            text: text.to_string(),
            color,
            font,
            align_right: false,
            centred: false,
            underline,
        });
    }

    /// Puts the caret at a place between cells, `height` rows tall and centred on `y`.
    pub fn set_caret(&mut self, x: f32, y: u16, height: f32) {
        self.cursor = Some((x as u16, y));
        let top = y as f32 + (1.0 - height) / 2.0;
        self.caret = Some(Bounds::cells(x, top, 0.0, height));
    }

    /// A label whose right edge sits at `right`, since proportional text is narrower than
    /// its cells and would otherwise leave a gap at the end.
    pub fn label_right(&mut self, right: u16, y: u16, text: &str, style: Style, size: u8) {
        let cells = text.chars().count() as u16;
        let x = right.saturating_sub(cells);
        self.put_str(
            x,
            y,
            text,
            Style {
                painted: true,
                ..style
            },
            right,
        );
        self.paint.push(Paint::Text {
            x: units(right as f32),
            y: y as i32 * UNIT,
            max_width: units(right as f32),
            text: text.to_string(),
            color: style.fg,
            font: Font {
                bold: style.bold,
                size,
                ..Font::default()
            },
            align_right: true,
            centred: false,
            underline: false,
        });
    }

    /// A rounded box inside `rect`, with square corners cleaned up behind it.
    pub fn field(&mut self, rect: Rect, behind: Rgb, fill: Rgb, border: Rgb) {
        self.panel(Bounds::inset(rect, 0.0, 0.0), 0, Some(behind), None);
        // Inset a little from its row, so fields stacked in rows next to each other keep
        // their air.
        self.panel(
            Bounds::inset(rect, 0.15, 0.11),
            22,
            Some(fill),
            Some(border),
        );
    }

    /// A rounded button in a row of its own. The cell tier shows `cells`, which may be
    /// wider than the label.
    pub fn button(&mut self, x: u16, y: u16, cells: &str, label: &str, fill: Rgb, style: Style) {
        self.button_of((BUTTON_HEIGHT, 0.0), (x, y), cells, label, fill, style);
    }

    /// A button that stands past its row, for where there's air around it.
    pub fn large_button(
        &mut self,
        x: u16,
        y: u16,
        cells: &str,
        label: &str,
        fill: Rgb,
        style: Style,
    ) {
        self.button_of(
            (LARGE_BUTTON_HEIGHT, 0.0),
            (x, y),
            cells,
            label,
            fill,
            style,
        );
    }

    /// A large button centred between `y` and the row below it.
    pub fn large_button_between(
        &mut self,
        x: u16,
        y: u16,
        cells: &str,
        label: &str,
        fill: Rgb,
        style: Style,
    ) {
        self.button_of(
            (LARGE_BUTTON_HEIGHT, 0.5),
            (x, y),
            cells,
            label,
            fill,
            style,
        );
    }

    fn button_of(
        &mut self,
        (height, drop): (f32, f32),
        (x, y): (u16, u16),
        cells: &str,
        label: &str,
        fill: Rgb,
        style: Style,
    ) {
        let width = cells.chars().count() as u16;
        self.put_str(
            x,
            y,
            cells,
            Style {
                painted: true,
                ..style
            },
            self.width,
        );
        let top = y as f32 + drop + (1.0 - height) / 2.0;
        let bounds = Bounds::cells(x as f32, top, width as f32, height);
        self.panel(bounds, 22, Some(fill), None);
        let middle = x as f32 + width as f32 / 2.0;
        self.centred_text(middle, y as f32 + drop, label, style.fg, 90);
    }

    /// Moves what was painted last down by part of a row.
    pub fn drop_last(&mut self, by: f32) {
        match self.paint.last_mut() {
            Some(Paint::Text { y, .. }) => *y += units(by),
            Some(Paint::Icon { bounds, .. }) => bounds.y += units(by),
            _ => {}
        }
    }

    /// Paints text centred on `x`, at a place in rows.
    pub fn centred_text(&mut self, x: f32, y: f32, text: &str, color: Rgb, size: u8) {
        self.paint.push(Paint::Text {
            x: units(x),
            y: units(y),
            max_width: units(self.width as f32),
            text: text.to_string(),
            color,
            font: Font {
                size,
                ..Font::default()
            },
            align_right: false,
            centred: true,
            underline: false,
        });
    }

    pub fn panel(&mut self, bounds: Bounds, radius: u8, fill: Option<Rgb>, border: Option<Rgb>) {
        self.paint.push(Paint::Panel {
            bounds,
            radius,
            fill,
            border,
            shadow: false,
            dashed: false,
        });
    }

    /// A panel with its border in dashes, for something offered rather than settled.
    pub fn dashed_panel(&mut self, bounds: Bounds, radius: u8, fill: Option<Rgb>, border: Rgb) {
        self.paint.push(Paint::Panel {
            bounds,
            radius,
            fill,
            border: Some(border),
            shadow: false,
            dashed: true,
        });
    }

    /// A panel floating above the page, with a soft shadow.
    pub fn popup(&mut self, bounds: Bounds, radius: u8, fill: Rgb, border: Rgb) {
        self.paint.push(Paint::Panel {
            bounds,
            radius,
            fill: Some(fill),
            border: Some(border),
            shadow: true,
            dashed: false,
        });
    }

    /// A see-through wash of `color` over `bounds`, `alpha` out of 255, with a border.
    pub fn wash(
        &mut self,
        bounds: Bounds,
        radius: u8,
        (color, alpha): (Rgb, u8),
        border: Option<Rgb>,
    ) {
        self.paint.push(Paint::Wash {
            bounds,
            radius,
            color,
            alpha,
            border,
        });
    }

    pub fn hairline(&mut self, bounds: Bounds, color: Rgb) {
        self.paint.push(Paint::Hairline { bounds, color });
    }

    /// A small icon, one row tall, with its left edge at `x`. `fallback` is what the cell
    /// tier shows there.
    pub fn glyph(&mut self, x: f32, y: u16, icon: Icon, color: Rgb, fallback: &str, style: Style) {
        let cell = x.floor() as u16;
        if cell < self.width && y < self.height {
            self.put_str(
                cell,
                y,
                fallback,
                Style {
                    painted: true,
                    ..style
                },
                self.width,
            );
        }
        self.paint.push(Paint::Icon {
            bounds: Bounds::cells(x, y as f32 + 0.14, 2.0, 0.72),
            icon,
            color,
        });
    }

    /// Paint that touches row `y`, in drawing order.
    pub fn corners(&mut self, bounds: Bounds, radius: u8, color: Rgb, which: u8) {
        self.paint.push(Paint::Corners {
            bounds,
            radius,
            color,
            which,
        });
    }

    pub fn paint_on_row(&self, y: u16) -> impl Iterator<Item = &Paint> {
        let (top, bottom) = (y as i32 * UNIT, (y as i32 + 1) * UNIT);
        self.paint.iter().filter(move |p| {
            let (from, to) = p.span();
            from < bottom && to > top
        })
    }

    pub fn width(&self) -> u16 {
        self.width
    }

    pub fn height(&self) -> u16 {
        self.height
    }

    pub fn area(&self) -> Rect {
        Rect::new(0, 0, self.width, self.height)
    }

    fn index(&self, x: u16, y: u16) -> usize {
        y as usize * self.width as usize + x as usize
    }

    pub fn cell(&self, x: u16, y: u16) -> &Cell {
        &self.cells[self.index(x, y)]
    }

    pub fn row(&self, y: u16) -> &[Cell] {
        let start = self.index(0, y);
        &self.cells[start..start + self.width as usize]
    }

    pub fn fill(&mut self, rect: Rect, style: Style) {
        for y in rect.y..rect.bottom().min(self.height) {
            for x in rect.x..rect.right().min(self.width) {
                self.put_cell(x, y, " ", style);
            }
        }
    }

    fn put_cell(&mut self, x: u16, y: u16, symbol: &str, style: Style) {
        // Overwriting half of a wide character blanks its other half.
        let i = self.index(x, y);
        if self.cells[i].symbol.is_empty() && x > 0 {
            self.cells[i - 1].symbol = " ".into();
        }
        if x + 1 < self.width && self.cells[i + 1].symbol.is_empty() {
            self.cells[i + 1].symbol = " ".into();
        }
        self.cells[i] = Cell {
            symbol: symbol.into(),
            style,
        };
    }

    /// Writes text stopping before `max_x`, and returns the column after it. A wide
    /// character that doesn't fit becomes a space.
    pub fn put_str(&mut self, x: u16, y: u16, text: &str, style: Style, max_x: u16) -> u16 {
        if y >= self.height {
            return x;
        }
        let max_x = max_x.min(self.width);
        let mut at = x;
        for g in text.graphemes(true) {
            if at >= max_x {
                break;
            }
            let width = g.width() as u16;
            if width == 0 {
                continue;
            }
            if at + width > max_x {
                self.put_cell(at, y, " ", style);
                return at + 1;
            }
            self.put_cell(at, y, g, style);
            if width == 2 {
                self.put_cell(at + 1, y, "", style);
            }
            at += width;
        }
        at
    }

    /// Places a multi-cell icon with its top-left corner at `x`, `y`.
    pub fn put_icon(&mut self, x: u16, y: u16, icon: Icon, size: (u8, u8), style: Style) {
        for row in 0..size.1 {
            for column in 0..size.0 {
                let (cx, cy) = (x + u16::from(column), y + u16::from(row));
                if cx < self.width && cy < self.height {
                    let piece = Piece {
                        icon,
                        width: size.0,
                        height: size.1,
                        column,
                        row,
                    };
                    self.put_cell(cx, cy, piece.symbol().encode_utf8(&mut [0; 4]), style);
                }
            }
        }
    }

    /// Overrides the style of existing cells, keeping their text.
    pub fn set_style(&mut self, rect: Rect, f: impl Fn(Style) -> Style) {
        for y in rect.y..rect.bottom().min(self.height) {
            for x in rect.x..rect.right().min(self.width) {
                let i = self.index(x, y);
                self.cells[i].style = f(self.cells[i].style);
            }
        }
    }

    /// Spans that differ from `prev`. A size change makes the whole grid differ.
    pub fn diff(&self, prev: &Grid) -> Vec<Run> {
        let same_size = self.width == prev.width && self.height == prev.height;
        let mut runs = Vec::new();
        for y in 0..self.height {
            if same_size && self.row(y) == prev.row(y) {
                continue;
            }
            let mut x = 0;
            while x < self.width {
                if same_size && self.cell(x, y) == prev.cell(x, y) {
                    x += 1;
                    continue;
                }
                let start = x;
                while x < self.width && !(same_size && self.cell(x, y) == prev.cell(x, y)) {
                    x += 1;
                }
                // A run must never begin on the right half of a wide character.
                let start = if self.cell(start, y).symbol.is_empty() && start > 0 {
                    start - 1
                } else {
                    start
                };
                runs.push(Run {
                    x: start,
                    y,
                    len: x - start,
                });
            }
        }
        runs
    }

    /// The text of a row, for tests and copy.
    pub fn row_text(&self, y: u16) -> String {
        self.row(y).iter().map(|c| c.symbol.as_str()).collect()
    }

    /// Everything on screen, one line per row with trailing spaces removed.
    pub fn text(&self) -> String {
        (0..self.height)
            .map(|y| self.row_text(y).trim_end().to_string())
            .collect::<Vec<_>>()
            .join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid() -> Grid {
        Grid::new(10, 3, Style::default())
    }

    #[test]
    fn diff_emits_only_changed_cells() {
        let before = grid();
        let mut after = grid();
        after.put_str(2, 1, "ab", Style::default(), 10);
        after.put_str(7, 1, "c", Style::default(), 10);
        assert_eq!(
            after.diff(&before),
            vec![Run { x: 2, y: 1, len: 2 }, Run { x: 7, y: 1, len: 1 }]
        );
        assert!(before.diff(&before.clone()).is_empty());
    }

    #[test]
    fn resize_redraws_everything() {
        let runs = grid().diff(&Grid::new(4, 4, Style::default()));
        assert_eq!(runs.len(), 3);
        assert!(runs.iter().all(|r| r.len == 10));
    }

    #[test]
    fn wide_characters_take_two_cells() {
        let mut g = grid();
        let end = g.put_str(0, 0, "日本", Style::default(), 10);
        assert_eq!(end, 4);
        assert_eq!(g.cell(0, 0).symbol, "日");
        assert_eq!(g.cell(1, 0).symbol, "");
        assert_eq!(g.row_text(0).trim_end(), "日本");
    }

    #[test]
    fn overwriting_half_a_wide_character_blanks_the_other_half() {
        let mut g = grid();
        g.put_str(0, 0, "日", Style::default(), 10);
        g.put_str(1, 0, "x", Style::default(), 10);
        assert_eq!(g.cell(0, 0).symbol, " ");
        assert_eq!(g.cell(1, 0).symbol, "x");
    }

    #[test]
    fn wide_character_at_the_edge_becomes_a_space() {
        let mut g = grid();
        g.put_str(9, 0, "日", Style::default(), 10);
        assert_eq!(g.cell(9, 0).symbol, " ");
    }

    #[test]
    fn combining_marks_stay_in_one_cell() {
        let mut g = grid();
        let end = g.put_str(0, 0, "e\u{301}x", Style::default(), 10);
        assert_eq!(end, 2);
        assert_eq!(g.cell(0, 0).symbol, "e\u{301}");
    }

    #[test]
    fn diff_never_starts_on_a_continuation_cell() {
        let mut before = grid();
        before.put_str(0, 0, "日", Style::default(), 10);
        let mut after = grid();
        after.put_str(
            0,
            0,
            "日",
            Style {
                bold: true,
                ..Style::default()
            },
            10,
        );
        let mut only_right = before.clone();
        only_right.cells[1].style.bold = true;
        assert_eq!(only_right.diff(&before)[0].x, 0);
        assert_eq!(after.diff(&before)[0].x, 0);
    }
}
