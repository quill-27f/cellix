use compact_str::CompactString;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::icon::{Icon, Piece};
use crate::measure::Font;
use crate::paint::{Bounds, Paint, UNIT, units};

/// Heights of a button, in rows.
/// Every button is one chip centred inside its row, so none stands past it and what's
/// painted is what takes a click: about 32 px of a 36 px row. At four fifths of a row its
/// words stood 7 px under its top and their tails 2 px over its foot, which the review of
/// the applied design measured as the last of "buttons and text too close".
const BUTTON_HEIGHT: f32 = 0.88;
const LARGE_BUTTON_HEIGHT: f32 = BUTTON_HEIGHT;
/// The corners of a button that stands inside a row's wash, as a percentage of the line:
/// 6 px where a button's are 8, in keeping with its 32 px against a control's 39.
const ROW_BUTTON_RADIUS: u8 = 17;
/// The one picture `Grid::exact_fill` scales: a single pixel, all ink.
const EXACT_FILL: u64 = u64::MAX - 1;
/// How far a button's words, a capital's height in their middle (`Paint::Text`'s
/// `span_centred`), stand over the baseline of words of 13 pt in the middle of the same row, in
/// the terminal's lines: their 90 per cent puts it a pixel higher, 0.9 px of a line of 36. A
/// button in a row of words goes down this far with its words (`Grid::beside_words`), so they
/// stand on the row's baseline: "Reject" rode 3 px over the hunk's heading beside it.
pub const WORDS_BASELINE: f32 = 0.026;
/// Rows tall an icon beside a name is: 16 pt where a row is 18.5 pt, VS Code's size.
pub const GLYPH: f32 = 0.86;
/// Rows tall an icon inside a text box or a chip is, where one of `GLYPH` would stand against
/// the box's edges: 13 pt.
pub const SMALL_GLYPH: f32 = 0.70;
/// How far a text box stands inside its row, above and below: the box is seven eighths of
/// its row, which holds words of 13 pt raised in it with their tails off its lower edge, and
/// leaves 4 px between two boxes in rows one under the other.
pub const FIELD_AIR: f32 = 0.0625;
/// Columns such an icon starts before where it's asked for, which keeps its middle where an
/// icon of 0.72 of a row had it, so it grew evenly to both sides.
pub const GLYPH_LEAD: f32 = 0.15;

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

/// `text` with nothing a terminal would act on: a control character, as a file, an agent's
/// output or a transcript can hold, is shown by its picture (`␛` for escape), the way VS Code
/// shows one, so it can't move the cursor, set the title or write the clipboard. A tab is a
/// space.
fn visible(text: &str) -> std::borrow::Cow<'_, str> {
    let control = |c: char| c.is_control();
    if !text.contains(control) {
        return std::borrow::Cow::Borrowed(text);
    }
    let shown = text
        .chars()
        .map(|c| match c {
            '\t' => ' ',
            '\0'..='\u{1f}' => char::from_u32(0x2400 + c as u32).unwrap_or('\u{fffd}'),
            '\u{7f}' => '\u{2421}',
            c if c.is_control() => '\u{fffd}',
            c => c,
        })
        .collect();
    std::borrow::Cow::Owned(shown)
}

/// A screen cell. An empty symbol is the right half of a wide character.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Cell {
    pub symbol: CompactString,
    pub style: Style,
}

/// The shape a terminal's own cursor takes on the cell tier: a bar between letters for the
/// caret in a file or a box, as an editor's, and for a program in a terminal pane what it
/// asked for (DECSCUSR), a block unless it asked.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum CursorShape {
    #[default]
    Bar,
    Block,
    Underline,
}

/// A terminal pane's cursor as its session drew it: its cell inverted (`on`), and that cell's
/// own look (`off`), which is what shows in the half of a blink the cursor is away, and on the
/// cell tier, where the terminal's own cursor stands there instead. `blinks` is whether its
/// program left it blinking: DECSCUSR 2, 4 or 6, or `CSI ? 12 l`, asks for a steady one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TermCursor {
    pub at: (u16, u16),
    pub on: Cell,
    pub off: Cell,
    pub shape: CursorShape,
    pub blinks: bool,
}

/// What a blink takes away while it's in its hidden half, to put back: the caret and the
/// cursor's cell, and the cells that show another look, each with the look they had.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Blinked {
    caret: Option<Bounds>,
    cursor: Option<(u16, u16)>,
    cells: Vec<(u16, u16, Cell)>,
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
    /// The colour past the grid's right edge, where a terminal lets a row's picture run on
    /// over the window's padding. Unset, it's the colour of the row's last cell.
    pub edge: Option<Rgb>,
    /// Whether a row here is tall enough for a control to stand in it with its words in its
    /// middle: rows of Cellix's own, well taller than the terminal's line. What draws into
    /// the grid says so. Where it is, a button on a row's wash stands inside the wash.
    pub roomy: bool,
    /// How tall one of these rows is in hundredths of the terminal's line, which words are
    /// sized for: 100 unless Cellix lays out rows taller than that line. What draws into the
    /// grid says so.
    pub row_air: u16,
    /// How many rows at the grid's foot are bars under the sheets: one, unless what draws
    /// says there are two, as where the workspaces have a row of their own under the status
    /// bar. A backend that fills the window under its last row stands them evenly in what's
    /// there.
    pub foot: u16,
    /// A row cut into bands of its own, in place of `bands`, as (the row, where each band
    /// after the first starts): the status bar, where a spinner is a tile by itself, so its
    /// steps don't send the rest of the bar again, nor the bar's other changes the spinner.
    pub row_bands: Option<(u16, Vec<u16>)>,
    /// The step of its turn every spinner drawn here stands at, from the clock in
    /// `spinner`, or none where motion is off: a spinner is then a still dot.
    pub turn: Option<u8>,
    /// Whether anything drawn here is turning, so whatever shows the grid draws it again at
    /// the next step. Nothing turning, nothing asks for a frame.
    pub spinning: bool,
    /// The cells turning spinners stand in, so a step can be taken on the grid as drawn.
    turning: Vec<(u16, u16)>,
    /// Runs of `paint` kept to a band of rows: as (from, to) in `paint` and (top, bottom) in
    /// rows. A list scrolled in a pane paints its cards whole, and the part of one that's
    /// past the pane's edge mustn't land on the bar beside it.
    kept: Vec<((usize, usize), (u16, u16))>,
    /// The shape of the terminal's own cursor where the cell tier shows it at `cursor`.
    pub cursor_shape: CursorShape,
    /// Whether the caret or cursor shown here blinks: on the cell tier the terminal is asked
    /// to blink its own cursor (DECSCUSR), and on the pixel tiers whatever shows the grid
    /// takes it through its two halves (`blink_to`). What draws says so.
    pub cursor_blinks: bool,
    /// The terminal panes' cursors drawn, for what draws to place as its tier shows them.
    pub term_cursors: Vec<TermCursor>,
    /// Cells with another look while a blink is in its hidden half, as (column, row, that
    /// look, the look drawn): a terminal pane's cursor, which is its cell inverted.
    blinking: Vec<(u16, u16, Cell, Cell)>,
    /// What the blink took away, while it's in its hidden half.
    blinked: Option<Blinked>,
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
            edge: None,
            roomy: false,
            row_air: 100,
            foot: 1,
            row_bands: None,
            turn: Some(0),
            spinning: false,
            turning: Vec::new(),
            kept: Vec::new(),
            cursor_shape: CursorShape::Bar,
            cursor_blinks: false,
            term_cursors: Vec::new(),
            blinking: Vec::new(),
            blinked: None,
        }
    }

    /// Has a cell show `off` in the half of a blink the caret is away, while it still shows
    /// what it does now: a terminal pane's cursor, its cell inverted, on the pixel tiers.
    pub fn blink_cell(&mut self, x: u16, y: u16, off: Cell) {
        if x < self.width && y < self.height {
            let on = self.cell(x, y).clone();
            self.blinking.push((x, y, off, on));
        }
    }

    /// Whether anything shown here comes and goes with the blink: a caret, a cursor's cell,
    /// or a cell with another look for it.
    pub fn blinks(&self) -> bool {
        self.cursor_blinks
            && (self.caret.is_some()
                || self.cursor.is_some()
                || self.blinked.is_some()
                || !self.blinking.is_empty())
    }

    /// Whether the blink is in the half that shows the caret.
    pub fn caret_shown(&self) -> bool {
        self.blinked.is_none()
    }

    /// Takes the caret to one half of its blink, as drawing it again in that half would:
    /// away, there's no caret and no cursor's cell, and a terminal's cursor shows its cell's
    /// own look; shown, they're back. A cell something drawn later stands in keeps that. A
    /// tile is a hash of its row, so once both halves of a row are in the terminal, a blink
    /// sends nothing but its placement. Nothing that doesn't blink changes.
    pub fn blink_to(&mut self, shown: bool) {
        if shown == self.caret_shown() || (!shown && !self.blinks()) {
            return;
        }
        if shown {
            let blinked = self.blinked.take().unwrap_or_default();
            self.caret = blinked.caret;
            self.cursor = blinked.cursor;
            for (x, y, on) in blinked.cells {
                let at = self.index(x, y);
                self.cells[at] = on;
            }
            return;
        }
        let mut cells = Vec::new();
        for (x, y, off, on) in &self.blinking {
            let at = self.index(*x, *y);
            if self.cells[at] == *on {
                self.cells[at] = off.clone();
                cells.push((*x, *y, on.clone()));
            }
        }
        self.blinked = Some(Blinked {
            caret: self.caret.take(),
            cursor: self.cursor.take(),
            cells,
        });
    }

    /// Something under way, in the colour of what it means: a faint ring with a quarter of it
    /// turning where there are pictures, a turning braille letter on cells, and with motion
    /// off a still dot. Its box is `size` rows of the terminal's line square, as an icon's
    /// is, starting at `x` and standing in the middle of row `y`; on cells it's the one
    /// letter at `x`.
    pub fn spinner(&mut self, x: f32, y: u16, size: f32, color: Rgb, style: Style) {
        self.spinner_in((x, x.floor() as u16), y, size, color, style);
    }

    /// The same, its letter on the cell tier in column `cell` rather than the one its paint
    /// starts in: where painted words are narrower than their letters, the cells keep what
    /// they can of the words before it.
    pub fn spinner_in(
        &mut self,
        (x, cell): (f32, u16),
        y: u16,
        size: f32,
        color: Rgb,
        style: Style,
    ) {
        let step = self.turn;
        self.spinning |= step.is_some();
        let letter = step.map_or(crate::spinner::STILL, |s| {
            crate::spinner::FRAMES[s as usize % crate::spinner::FRAMES.len()]
        });
        self.mark((x, cell), y, size, color, style, (step, letter));
    }

    /// Takes every spinner on the grid to `step` of its turn, as drawing it again at that step
    /// would: when nothing else has changed, a step is this and not the whole interface laid
    /// out again. Still ones stay still.
    pub fn turn_to(&mut self, step: u8) {
        if self.turn.is_none() {
            return;
        }
        self.turn = Some(step);
        for paint in &mut self.paint {
            if let Paint::Spinner { step: Some(at), .. } = paint {
                *at = step;
            }
        }
        let letter = crate::spinner::FRAMES[step as usize % crate::spinner::FRAMES.len()];
        for &(x, y) in &self.turning {
            let at = y as usize * self.width as usize + x as usize;
            // Unless something drawn after it stands there now.
            let cell = self.cells.get_mut(at).filter(|_| x < self.width);
            if let Some(cell) = cell.filter(|c| crate::spinner::FRAMES.contains(&c.symbol.as_str()))
            {
                cell.symbol = letter.into();
            }
        }
    }

    /// A still dot in the colour of what it means, sized and placed as a spinner is: a
    /// state that wants seeing, as an agent that needs you or one that's done.
    pub fn dot(&mut self, x: f32, y: u16, size: f32, color: Rgb, style: Style) {
        self.dot_in((x, x.floor() as u16), y, size, color, style);
    }

    /// The same, its letter on the cell tier in column `cell`, as `spinner_in`'s.
    pub fn dot_in(&mut self, (x, cell): (f32, u16), y: u16, size: f32, color: Rgb, style: Style) {
        let still = (None, crate::spinner::STILL);
        self.mark((x, cell), y, size, color, style, still);
    }

    fn mark(
        &mut self,
        (x, cell): (f32, u16),
        y: u16,
        size: f32,
        color: Rgb,
        style: Style,
        (step, letter): (Option<u8>, &str),
    ) {
        if step.is_some() {
            self.turning.push((cell, y));
        }
        if cell < self.width && y < self.height {
            let shown = Style {
                fg: color,
                painted: true,
                ..style
            };
            self.put_str(cell, y, letter, shown, cell + 1);
        }
        self.paint.push(Paint::Spinner {
            bounds: Bounds::cells(x, y as f32 + (1.0 - size) / 2.0, 2.0, size),
            color,
            step,
        });
    }

    /// Text in the interface font on the pixel tiers, and ordinary cells on the cell tier.
    pub fn label(&mut self, x: u16, y: u16, text: &str, style: Style, max_x: u16) -> u16 {
        self.label_at(x as f32, x, y, text, style, max_x, 100)
    }

    /// Takes away what was painted that starts inside `area`, for a surface of cells laid over
    /// it: paint can't go under text, so cells can't cover it, and it would show through.
    pub fn clear_paint_in(&mut self, area: Rect) {
        let (left, right) = (i32::from(area.x) * UNIT, i32::from(area.right()) * UNIT);
        let (top, bottom) = (i32::from(area.y) * UNIT, i32::from(area.bottom()) * UNIT);
        self.paint.retain(|paint| {
            let (from, _) = paint.reach();
            let start = paint.top();
            !(start >= top && start < bottom && from >= left && from < right)
        });
    }

    /// Paints again, clipped to `area`, the pictures painted before `before` that reach into
    /// it, for a surface that hides its edge rows' cells whole, so no words show half covered,
    /// though its panel covers only part of their pixels: painted over that ground, a picture
    /// shows whole up to the surface's line, and the panel painted next hides what it covers.
    /// The Ask card's last row hid 29 px of an equation under the card's foot (the release
    /// review of 0.12.0, q4-ask-follow-up). Fills (`exact_fill`) aren't pictures and stay hidden
    /// with the words, and a picture kept to some rows (`keep_to_rows`) stays on them.
    pub fn pictures_again_in(&mut self, before: usize, area: Rect) {
        let clip_to = |clip: Bounds, (left, top, right, bottom): (i32, i32, i32, i32)| {
            let (x, y) = (clip.x.max(left), clip.y.max(top));
            let (end_x, end_y) = ((clip.x + clip.w).min(right), (clip.y + clip.h).min(bottom));
            Bounds {
                x,
                y,
                w: end_x - x,
                h: end_y - y,
            }
        };
        let (left, right) = (i32::from(area.x) * UNIT, i32::from(area.right()) * UNIT);
        let (top, bottom) = (i32::from(area.y) * UNIT, i32::from(area.bottom()) * UNIT);
        let mut again = Vec::new();
        for (index, paint) in self.paint.iter().enumerate().take(before) {
            let Paint::Image {
                bounds,
                clip,
                bitmap,
                color,
            } = paint
            else {
                continue;
            };
            if bitmap.id == EXACT_FILL {
                continue;
            }
            let mut clip = clip_to(*clip, (left, top, right, bottom));
            for ((from, to), (first, end)) in &self.kept {
                if (*from..*to).contains(&index) {
                    let rows = (i32::from(*first) * UNIT, i32::from(*end) * UNIT);
                    clip = clip_to(clip, (left, rows.0, right, rows.1));
                }
            }
            let shown = clip_to(
                clip,
                (bounds.x, bounds.y, bounds.x + bounds.w, bounds.y + bounds.h),
            );
            if shown.w > 0 && shown.h > 0 {
                again.push(Paint::Image {
                    bounds: *bounds,
                    clip,
                    bitmap: bitmap.clone(),
                    color: *color,
                });
            }
        }
        self.paint.extend(again);
    }

    /// How much has been painted so far, to `lift_since` later.
    pub fn painted_so_far(&self) -> usize {
        self.paint.len()
    }

    /// Keeps what was painted since `from` to the rows from `top` up to `bottom`: on any other
    /// row none of it is drawn, however far it reaches.
    pub fn keep_to_rows(&mut self, from: usize, top: u16, bottom: u16) {
        if from < self.paint.len() {
            self.kept.push(((from, self.paint.len()), (top, bottom)));
        }
    }

    /// Puts paint in under what's already painted from `index` on, as a mark under words is.
    pub fn insert_paint(&mut self, index: usize, paint: Paint) {
        let index = index.min(self.paint.len());
        self.paint.insert(index, paint);
        // What was kept to its rows still is, and what's put inside a kept run is kept too.
        for ((from, to), _) in &mut self.kept {
            if index < *from {
                *from += 1;
            }
            if index < *to {
                *to += 1;
            }
        }
    }

    /// Moves what was painted since `from` up by `rows`, which only the pixel tiers can show:
    /// the cells under it stay in their rows, and so does where a click lands.
    pub fn lift_since(&mut self, from: usize, rows: f32) {
        let by = units(rows);
        for paint in self.paint.iter_mut().skip(from) {
            paint.lift(by);
        }
    }

    /// Moves what was painted since `from` right by `columns`, which only the pixel tiers can
    /// show: the cells under it stay in their columns, and so does where a click lands.
    pub fn nudge_since(&mut self, from: usize, columns: f32) {
        let by = units(columns);
        for paint in self.paint.iter_mut().skip(from) {
            match paint {
                Paint::Text { x, .. } => *x += by,
                Paint::Panel { bounds, .. }
                | Paint::Wash { bounds, .. }
                | Paint::Hairline { bounds, .. }
                | Paint::Icon { bounds, .. }
                | Paint::Spinner { bounds, .. }
                | Paint::Corners { bounds, .. }
                | Paint::Strips { bounds, .. } => bounds.x += by,
                Paint::Image { bounds, clip, .. } => {
                    bounds.x += by;
                    clip.x += by;
                }
            }
        }
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
                span_centred: false,
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
            span_centred: false,
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
        self.label_right_at(right as f32, right, y, text, style, size);
    }

    /// The same, its painted words ending at `right`, between cells, and its letters in the
    /// cells ending at `cell`.
    pub fn label_right_at(
        &mut self,
        right: f32,
        cell: u16,
        y: u16,
        text: &str,
        style: Style,
        size: u8,
    ) {
        let cells = text.chars().count() as u16;
        let x = cell.saturating_sub(cells);
        self.put_str(
            x,
            y,
            text,
            Style {
                painted: true,
                ..style
            },
            cell,
        );
        self.paint.push(Paint::Text {
            x: units(right),
            y: y as i32 * UNIT,
            max_width: units(right),
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
            span_centred: false,
        });
    }

    /// A rounded box inside `rect`, with square corners cleaned up behind it.
    pub fn field(&mut self, rect: Rect, behind: Rgb, fill: Rgb, border: Rgb) {
        self.panel(Bounds::inset(rect, 0.0, 0.0), 0, Some(behind), None);
        // Inset a little from its row, so fields stacked in rows next to each other keep
        // their air.
        self.panel(
            Bounds::inset(rect, 0.15, FIELD_AIR),
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

    /// A button at the end of a row of words, which may be washed under the pointer. Where a
    /// row has room, it's the height it has on the terminal's line, 32 px in a row of 50 and
    /// inside a pointed row's wash: as tall as the wash, the two read as one shape with a step
    /// in it, and taller, as a button too big for its row. Its words' span, a capital's top to
    /// a tail's foot, stands in its middle as every button's does (at 27 px a "p"'s tail ended
    /// on its foot), and it goes down with its words onto the row's baseline (`beside_words`).
    /// On the terminal's line a button and the wash have the row between them and nothing to
    /// spare, and it's a button like any other.
    pub fn row_button(
        &mut self,
        (x, y): (u16, u16),
        cells: &str,
        label: &str,
        fill: Rgb,
        style: Style,
    ) {
        let from = self.paint.len();
        self.button(x, y, cells, label, fill, style);
        if !self.roomy {
            return;
        }
        // Its corners a little tighter with its height: a button's 8 px beside the wash's 4
        // read as two kinds of shape once there was wash to see round it.
        for paint in &mut self.paint[from..] {
            if let Paint::Panel { inset, radius, .. } = paint {
                *inset = true;
                *radius = ROW_BUTTON_RADIUS;
            }
        }
        self.beside_words(from);
    }

    /// The chip of a button at a row's end, from `left` to `right` between cells, behind words
    /// its caller sets where they stand with the row at rest: as tall as `row_button`'s and in
    /// the middle of the row, so in the middle of a pointed row's wash, where a row has room.
    /// Gone down onto the words' baseline, as a `row_button` goes, it stood 10 px under the
    /// wash's top and 8 over its foot.
    pub fn row_chip(&mut self, (left, right): (f32, f32), y: u16, fill: Rgb) {
        let (_, top, _, bottom) = button_span((BUTTON_HEIGHT, 0.0), (0, y), 1);
        let (radius, inset) = if self.roomy {
            (ROW_BUTTON_RADIUS, true)
        } else {
            (22, false)
        };
        self.paint.push(Paint::Panel {
            bounds: Bounds::cells(left, top, right - left, bottom - top),
            radius,
            fill: Some(fill),
            border: None,
            shadow: false,
            dashed: false,
            inset,
        });
    }

    /// Moves a button painted since `from` down onto the baseline of the words beside it in
    /// its row, chip and words together (`WORDS_BASELINE`), where a row has room. On the
    /// terminal's line a button has the row to itself and nothing to spare.
    pub fn beside_words(&mut self, from: usize) {
        if self.roomy {
            let line = 100.0 / f32::from(self.row_air.max(1));
            self.lift_since(from, -WORDS_BASELINE * line);
        }
    }

    /// A fill of `color` exactly where `bounds` is. A panel shorter than a row is stood as a
    /// control is on a taller row (`raster::stood`), grown or shrunk about its middle, which
    /// a surface reaching part way into a row mustn't be: peek's card reaching into the
    /// pane's last row stood 4 px clear of its own cells.
    pub fn exact_fill(&mut self, bounds: Bounds, color: Rgb) {
        let bitmap = crate::paint::Bitmap {
            id: EXACT_FILL,
            width: 1,
            height: 1,
            kind: crate::paint::Pixels::Coverage,
            pixels: std::sync::Arc::new(vec![255]),
        };
        self.paint.push(Paint::Image {
            bounds,
            clip: bounds,
            bitmap,
            color,
        });
    }

    /// `exact_fill`, put in under what's been painted from `index` on.
    pub fn exact_fill_under(&mut self, index: usize, bounds: Bounds, color: Rgb) {
        self.exact_fill(bounds, color);
        if let Some(fill) = self.paint.pop() {
            self.insert_paint(index, fill);
        }
    }

    /// A button's chip by itself, as wide as asked and not a whole number of cells, for one
    /// whose words and marks are placed by what draws it.
    pub fn chip(&mut self, x: f32, y: u16, wide: f32, fill: Rgb) {
        let top = y as f32 + (1.0 - BUTTON_HEIGHT) / 2.0;
        let bounds = Bounds::cells(x, top, wide, BUTTON_HEIGHT);
        self.panel(bounds, 22, Some(fill), None);
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
        let (left, top, right, bottom) = button_span((height, drop), (x, y), width);
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
        let bounds = Bounds::cells(left, top, right - left, bottom - top);
        self.panel(bounds, 22, Some(fill), None);
        let middle = x as f32 + width as f32 / 2.0;
        self.button_label(middle, y as f32 + drop, label, style.fg, 90);
    }

    /// A button's words by themselves, centred on `middle` in row `y` and standing in it as a
    /// button's do, for a control that paints its own chip.
    pub fn button_words(&mut self, middle: f32, y: u16, label: &str, color: Rgb) {
        self.button_label(middle, y as f32, label, color, 90);
    }

    /// Words centred on `x` that stand in the middle of the row starting at `y` as a button's
    /// do: their font's span, a capital's top to a tail's foot, in its middle, whatever their
    /// letters, so a row of buttons has one baseline. The font's line in the row's middle put
    /// "Show Source" 5 px under its button's top and 10 over its foot, and the tails of "Accept
    /// Rewrite" 6 px over its foot with its capitals 9 under its top.
    pub fn button_label(&mut self, x: f32, y: f32, text: &str, color: Rgb, size: u8) {
        self.centred_text(x, y, text, color, size);
        self.centre_span_since(self.paint.len() - 1);
    }

    /// Has the words painted since `from` stand as a button's words do (`button_label`), for a
    /// button that places its own words.
    pub fn centre_span_since(&mut self, from: usize) {
        for paint in self.paint.iter_mut().skip(from) {
            if let Paint::Text { span_centred, .. } = paint {
                *span_centred = true;
            }
        }
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
            span_centred: false,
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
            inset: false,
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
            inset: false,
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
            inset: false,
        });
    }

    /// A panel floating above the page drawn exactly over `bounds`, however much shorter than
    /// its row: `popup`'s is stood as a control is on a taller row (`raster::stood`), grown or
    /// shrunk about its middle, and a tip's box sized for its words' ink mustn't be.
    pub fn exact_popup(&mut self, bounds: Bounds, radius: u8, fill: Rgb, border: Rgb) {
        self.paint.push(Paint::Panel {
            bounds,
            radius,
            fill: Some(fill),
            border: Some(border),
            shadow: true,
            dashed: false,
            inset: true,
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

    /// A small icon in the row, 16 pt in a terminal set to 14 pt, as VS Code's are beside a
    /// name. It stands about `x`: its middle is where it was when it was a sixth smaller, so
    /// what's placed after it needn't move. `fallback` is what the cell tier shows there.
    pub fn glyph(&mut self, x: f32, y: u16, icon: Icon, color: Rgb, fallback: &str, style: Style) {
        self.glyph_in((x, x.floor() as u16), y, (icon, color), fallback, style);
    }

    /// The same, `fallback` on the cell tier in column `cell`, as `spinner_in`'s letter.
    pub fn glyph_in(
        &mut self,
        (x, cell): (f32, u16),
        y: u16,
        (icon, color): (Icon, Rgb),
        fallback: &str,
        style: Style,
    ) {
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
            bounds: Bounds::cells(x - GLYPH_LEAD, y as f32 + (1.0 - GLYPH) / 2.0, 2.0, GLYPH),
            icon,
            color,
        });
    }

    /// The same icon at `SMALL_GLYPH`, for inside a text box or a chip, its left edge at `x`.
    #[allow(clippy::too_many_arguments)]
    pub fn small_glyph(
        &mut self,
        x: f32,
        y: u16,
        icon: Icon,
        color: Rgb,
        fallback: &str,
        style: Style,
    ) {
        self.glyph(x, y, icon, color, fallback, style);
        if let Some(Paint::Icon { bounds, .. }) = self.paint.last_mut() {
            let top = y as f32 + (1.0 - SMALL_GLYPH) / 2.0;
            *bounds = Bounds::cells(x, top, 2.0, SMALL_GLYPH);
        }
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
        let kept_off = move |index: usize| {
            self.kept.iter().any(|((from, to), (first, end))| {
                (*from..*to).contains(&index) && !(*first..*end).contains(&y)
            })
        };
        self.paint
            .iter()
            .enumerate()
            .filter(move |(index, p)| {
                let (from, to) = p.span();
                from < bottom && to > top && !kept_off(*index)
            })
            .map(|(_, paint)| paint)
    }

    /// The bands of row `y`, as (first column, column past the last), each a tile of its own:
    /// `bands`, or `row_bands` on its own row. The last row is the status bar, which nothing
    /// scrolls in, and is one.
    pub fn spans(&self, y: u16) -> Vec<(u16, u16)> {
        let width = self.width;
        let own = self.row_bands.as_ref().filter(|(row, _)| *row == y);
        let bands = match own {
            Some((_, bands)) => bands,
            None if y + 1 == self.height => return vec![(0, width)],
            None => &self.bands,
        };
        let inside = bands.iter().copied().filter(|x| *x > 0 && *x < width);
        let edges: Vec<u16> = [0].into_iter().chain(inside).chain([width]).collect();
        edges.windows(2).map(|edge| (edge[0], edge[1])).collect()
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
        let text = visible(text);
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

/// Where a button is painted, as (left, top, right, bottom) in columns and rows.
fn button_span((height, drop): (f32, f32), (x, y): (u16, u16), width: u16) -> (f32, f32, f32, f32) {
    let top = y as f32 + drop + (1.0 - height) / 2.0;
    (x as f32, top, (x + width) as f32, top + height)
}

/// How far a button's row reaches under its foot, in pixels, on rows `row` px tall with words
/// sized for a line `line` px tall: what a menu dropping from the button already has clear.
pub fn air_under_button(row: f32, line: f32) -> f32 {
    row - button_in_row(row, line).1
}

/// Where a button inside a row's wash (`Grid::row_button`) stands in its row where a row has
/// room, its top and its foot in pixels from the row's top, on rows `row` px tall with words
/// sized for a line `line` px tall. It's under the row's middle by `WORDS_BASELINE`, as paint
/// moves, in whole units.
pub fn row_button_in_row(row: f32, line: f32) -> (f32, f32) {
    let (_, top, _, bottom) = button_span((BUTTON_HEIGHT, 0.0), (0, 0), 1);
    let drop = units(WORDS_BASELINE * line / row) as f32 * row / UNIT as f32;
    let (top, tall) = crate::raster::inset(units(top), units(bottom - top), row, line);
    (top + drop, top + drop + tall)
}

/// Where a button's chip stands in its row, its top and its foot in pixels from the row's
/// top, on rows `row` px tall with words sized for a line `line` px tall.
pub fn button_in_row(row: f32, line: f32) -> (f32, f32) {
    let (_, top, _, bottom) = button_span((BUTTON_HEIGHT, 0.0), (0, 0), 1);
    let bounds = Bounds::cells(0.0, top, 1.0, bottom - top);
    let (y, h) = crate::raster::stood(bounds.y, bounds.h, row, line);
    (y, y + h)
}

/// Whether a click at `at` (columns and rows, with their fractions) lands on the large
/// button `Grid::large_button` paints at `(x, y)`, `width` cells wide. It takes the whole
/// of what's painted, which stands a little past its row above and below. A terminal
/// without pixel reports gives the middle of a cell, so there it's the button's row.
pub fn on_large_button((x, y): (u16, u16), width: u16, at: (f32, f32)) -> bool {
    let (left, top, right, bottom) = button_span((LARGE_BUTTON_HEIGHT, 0.0), (x, y), width);
    at.0 >= left && at.0 < right && at.1 >= top && at.1 < bottom
}

#[cfg(test)]
mod tests {
    /// Paint kept to a band of rows shows on those rows only, and paint put in under it
    /// later doesn't move which paint that is.
    #[test]
    fn paint_kept_to_its_rows_is_on_no_other() {
        let style = Style::default();
        let mut grid = Grid::new(10, 6, style);
        let panel = |grid: &mut Grid, y: f32, h: f32| {
            grid.panel(Bounds::cells(0.0, y, 10.0, h), 0, Some(Rgb(1, 2, 3)), None);
        };
        panel(&mut grid, 0.0, 1.0);
        let from = grid.painted_so_far();
        // A card from the second row to past the fifth, in a list that ends before it.
        panel(&mut grid, 1.0, 4.5);
        grid.keep_to_rows(from, 2, 4);
        panel(&mut grid, 5.0, 1.0);
        let on = |grid: &Grid, y: u16| grid.paint_on_row(y).count();
        let rows: Vec<usize> = (0..6).map(|y| on(&grid, y)).collect();
        assert_eq!(rows, [1, 0, 1, 1, 0, 1]);
        // A mark put in before the card: the card is still the one that's kept.
        grid.insert_paint(
            0,
            Paint::Hairline {
                bounds: Bounds::cells(0.0, 1.0, 10.0, 0.0),
                color: Rgb(9, 9, 9),
            },
        );
        let rows: Vec<usize> = (0..6).map(|y| on(&grid, y)).collect();
        assert_eq!(rows, [1, 1, 1, 1, 0, 1]);
    }

    #[test]
    fn control_characters_never_reach_a_cell() {
        let style = Style::default();
        let mut grid = Grid::new(40, 2, style);
        let text = "a\x1b]52;c;aGk=\x07b\r\n\x7f\u{9b}2Jc\td";
        let end = grid.put_str(0, 0, text, style, 40);
        let row: String = (0..end)
            .map(|x| grid.cell(x, 0).symbol.to_string())
            .collect();
        assert_eq!(row, "a␛]52;c;aGk=␇b␍␊␡\u{fffd}2Jc d");
        assert!(row.chars().all(|c| !c.is_control()), "{row:?}");
    }

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

    /// A step taken on the grid as drawn is the grid drawn at that step: the frame the loop
    /// shows when nothing but a spinner changed is the one a whole draw would have made.
    #[test]
    fn a_step_on_the_drawn_grid_is_the_grid_drawn_at_that_step() {
        let draw = |step: u8| {
            let mut grid = Grid::new(30, 4, Style::default());
            grid.turn = Some(step);
            grid.put_str(0, 1, "working", Style::default(), 30);
            grid.spinner(9.0, 1, 0.65, Rgb(1, 2, 3), Style::default());
            grid.dot(12.0, 1, 0.65, Rgb(4, 5, 6), Style::default());
            grid.spinner(3.0, 3, 0.65, Rgb(1, 2, 3), Style::default());
            grid
        };
        let mut stepped = draw(0);
        for step in [1, 7, 0, 9] {
            stepped.turn_to(step);
            assert_eq!(stepped, draw(step), "step {step}");
        }
        assert_eq!(
            stepped.cell(12, 1).symbol,
            crate::spinner::STILL,
            "a dot stays"
        );
        // Over a spinner's cell, what's drawn after it is left as it is.
        let mut covered = draw(0);
        covered.put_str(8, 1, "menu", Style::default(), 30);
        covered.turn_to(4);
        assert_eq!(covered.row_text(1).trim_end(), "working menu●");
        // Motion off, nothing turns.
        let mut still = Grid::new(30, 4, Style::default());
        still.turn = None;
        still.spinner(9.0, 1, 0.65, Rgb(1, 2, 3), Style::default());
        let before = still.clone();
        still.turn_to(3);
        assert_eq!(still, before);
    }
}
