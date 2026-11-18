//! The small floating panels a document's toolbar opens: a size picker for new tables, and a
//! palette of symbols and formula templates. Both work with the mouse or the keyboard.

use cellix_input::Key;
use cellix_render::{Bitmap, Bounds, Grid, INTERFACE_SIZE, Measure, Paint, Pixels, Rect, Style};

use crate::palette::raised;
use crate::theme::Theme;

const PICK_COLUMNS: usize = 10;
const PICK_ROWS: usize = 8;
/// A picker square is this many cells wide: about square on screen.
const PICK_CELL: u16 = 3;
/// Columns from a panel's edge to what it holds: a column, and the fifth of one a square or
/// a button is inset in its own.
const SIDE_INSET: f32 = 1.2;

/// How far a panel's grid stands below its top row's top, in rows, so it's as far from the
/// panel's top as from its sides: on its first row it was 7 px from the top and 20 from the
/// sides. `inset` is the rows a square or button already keeps from its row's top.
fn grid_drop(inset: f32, measure: &dyn Measure) -> f32 {
    let Some((wide, row)) = measure.cell_pixels() else {
        return 0.0;
    };
    let side = SIDE_INSET * wide as f32;
    ((side / row.max(1) as f32) - inset).clamp(0.0, 0.5)
}

/// Hover over a grid to choose a table's size, as in a word processor.
pub struct TablePicker {
    origin: (u16, u16),
    /// The size chosen so far, as (rows, columns).
    pub size: (usize, usize),
    /// Rows its squares stand below its top row's top (`grid_drop`).
    drop: f32,
}

/// Rows a picker square keeps from its row's top and foot.
const SQUARE_INSET: f32 = 0.1;

impl TablePicker {
    pub fn new(origin: (u16, u16), measure: &dyn Measure) -> Self {
        Self {
            origin,
            size: (3, 3),
            drop: grid_drop(SQUARE_INSET, measure),
        }
    }

    pub fn area(&self) -> Rect {
        Rect::new(
            self.origin.0,
            self.origin.1,
            PICK_CELL * PICK_COLUMNS as u16 + 2,
            PICK_ROWS as u16 + 2,
        )
    }

    /// The size a place on screen stands for, `y` with its fraction of a row.
    pub fn size_at(&self, x: u16, y: f32) -> Option<(usize, usize)> {
        let area = self.area();
        let (left, top) = (area.x + 1, f32::from(area.y) + self.drop);
        let column = x.checked_sub(left)? / PICK_CELL;
        let row = y - top;
        (row >= 0.0 && (column as usize) < PICK_COLUMNS && (row as usize) < PICK_ROWS)
            .then_some((row as usize + 1, column as usize + 1))
    }

    pub fn key(&mut self, key: Key) {
        let (rows, columns) = self.size;
        self.size = match key {
            Key::Up => (rows.saturating_sub(1).max(1), columns),
            Key::Down => ((rows + 1).min(PICK_ROWS), columns),
            Key::Left => (rows, columns.saturating_sub(1).max(1)),
            Key::Right => (rows, (columns + 1).min(PICK_COLUMNS)),
            _ => self.size,
        };
    }

    pub fn draw(&self, grid: &mut Grid, hover: Option<(u16, f32)>, theme: &Theme) {
        let area = self.area();
        raised(grid, area, theme);
        let (rows, columns) = hover
            .and_then(|h| self.size_at(h.0, h.1))
            .unwrap_or(self.size);
        for row in 0..PICK_ROWS {
            for column in 0..PICK_COLUMNS {
                let square = Rect::new(
                    area.x + 1 + PICK_CELL * column as u16,
                    area.y + row as u16,
                    PICK_CELL,
                    1,
                );
                let on = row < rows && column < columns;
                let fill = if on {
                    theme.selection
                } else {
                    theme.popup_hover
                };
                let border = if on { theme.accent } else { theme.popup_border };
                let mut bounds = Bounds::inset(square, 0.2, SQUARE_INSET);
                bounds.y += cellix_render::units(self.drop);
                grid.panel(bounds, 8, Some(fill), Some(border));
            }
        }
        let label = format!("{rows} rows, {columns} columns");
        let style = Style {
            fg: theme.popup_dim,
            bg: theme.popup,
            ..theme.bar
        };
        let y = area.y + PICK_ROWS as u16 + 1;
        grid.label(area.x + 2, y, &label, style, area.right().saturating_sub(1));
    }
}

/// One button in the math panel.
pub struct Symbol {
    pub name: &'static str,
    /// What the button shows.
    preview: &'static str,
    /// What it inserts.
    pub text: &'static str,
    /// Where the caret goes in `text`, in chars.
    pub caret: usize,
    /// Goes on lines of its own, rather than in the sentence.
    pub block: bool,
}

const fn symbol(name: &'static str, text: &'static str) -> Symbol {
    Symbol {
        name,
        preview: text,
        text,
        caret: usize::MAX,
        block: false,
    }
}

const fn template(
    name: &'static str,
    preview: &'static str,
    text: &'static str,
    caret: usize,
) -> Symbol {
    Symbol {
        name,
        preview,
        text,
        caret,
        block: false,
    }
}

const fn block(
    name: &'static str,
    preview: &'static str,
    text: &'static str,
    caret: usize,
) -> Symbol {
    Symbol {
        name,
        preview,
        text,
        caret,
        block: true,
    }
}

pub const SYMBOLS: [Symbol; 70] = [
    template("Fraction", r"\frac{a}{b}", r"\frac{}{}", 6),
    template("Square root", r"\sqrt{x}", r"\sqrt{}", 6),
    template("Power", "x^{n}", "^{}", 2),
    template("Subscript", "x_{i}", "_{}", 2),
    template("Sum", r"\sum_{i}^{n}", r"\sum_{i=1}^{n} ", 14),
    template("Product", r"\prod_{i}^{n}", r"\prod_{i=1}^{n} ", 15),
    template("Integral", r"\int_{a}^{b}", r"\int_{a}^{b} ", 12),
    template("Limit", r"\lim_{x}", r"\lim_{x \to \infty} ", 19),
    template(
        "Partial derivative",
        r"\frac{\partial f}{\partial x}",
        r"\frac{\partial }{\partial }",
        15,
    ),
    template("Vector", r"\vec{v}", r"\vec{}", 5),
    block(
        "Matrix",
        r"\begin{pmatrix} a & b \\ c & d \end{pmatrix}",
        "\\begin{pmatrix}\n  a & b \\\\\n  c & d\n\\end{pmatrix}",
        18,
    ),
    block(
        "Cases",
        r"\begin{cases} a \\ b \end{cases}",
        "f(x) = \\begin{cases}\n  a & x < 0 \\\\\n  b & x \\ge 0\n\\end{cases}",
        23,
    ),
    block(
        "Aligned equations",
        r"\begin{aligned} a &= b \end{aligned}",
        "\\begin{aligned}\n  a &= b \\\\\n  c &= d\n\\end{aligned}",
        18,
    ),
    template("Binomial", r"\binom{n}{k}", r"\binom{}{}", 7),
    template("Hat", r"\hat{x}", r"\hat{}", 5),
    template("Bar", r"\bar{x}", r"\bar{}", 5),
    template("Bold", r"\mathbf{x}", r"\mathbf{}", 8),
    template("Blackboard", r"\mathbb{R}", r"\mathbb{}", 8),
    template("Calligraphic", r"\mathcal{L}", r"\mathcal{}", 9),
    template("Text", r"\text{if}", r"\text{}", 6),
    symbol("alpha", r"\alpha"),
    symbol("beta", r"\beta"),
    symbol("gamma", r"\gamma"),
    symbol("delta", r"\delta"),
    symbol("epsilon", r"\varepsilon"),
    symbol("theta", r"\theta"),
    symbol("lambda", r"\lambda"),
    symbol("mu", r"\mu"),
    symbol("pi", r"\pi"),
    symbol("rho", r"\rho"),
    symbol("sigma", r"\sigma"),
    symbol("tau", r"\tau"),
    symbol("phi", r"\phi"),
    symbol("chi", r"\chi"),
    symbol("psi", r"\psi"),
    symbol("omega", r"\omega"),
    symbol("Gamma", r"\Gamma"),
    symbol("Delta", r"\Delta"),
    symbol("Sigma", r"\Sigma"),
    symbol("Omega", r"\Omega"),
    symbol("Less or equal", r"\le"),
    symbol("Greater or equal", r"\ge"),
    symbol("Not equal", r"\ne"),
    symbol("Approximately", r"\approx"),
    symbol("Equivalent", r"\equiv"),
    symbol("Proportional", r"\propto"),
    symbol("Element of", r"\in"),
    symbol("Subset", r"\subset"),
    symbol("Union", r"\cup"),
    symbol("Intersection", r"\cap"),
    symbol("Times", r"\times"),
    symbol("Dot", r"\cdot"),
    symbol("Plus or minus", r"\pm"),
    symbol("Infinity", r"\infty"),
    symbol("Partial", r"\partial"),
    symbol("Nabla", r"\nabla"),
    symbol("Arrow", r"\to"),
    symbol("Implies", r"\Rightarrow"),
    symbol("For all", r"\forall"),
    symbol("Exists", r"\exists"),
    symbol("Tensor", r"\otimes"),
    symbol("Direct sum", r"\oplus"),
    symbol("Subset or equal", r"\subseteq"),
    symbol("Maps to", r"\mapsto"),
    symbol("Left and right arrow", r"\leftrightarrow"),
    symbol("If and only if", r"\Leftrightarrow"),
    symbol("Empty set", r"\emptyset"),
    symbol("Much less", r"\ll"),
    symbol("Tilde", r"\sim"),
    symbol("Dots", r"\ldots"),
];

const COLUMNS: u16 = 10;
/// A button's size in cells: about square on screen, with room for a tall formula.
const BUTTON: (u16, u16) = (5, 2);

/// The symbols, each shown typeset.
pub struct MathPanel {
    origin: (u16, u16),
    /// What each button shows, on the tiers that have pictures.
    previews: Vec<Option<(Bitmap, f32, f32)>>,
    pub selected: usize,
    /// Rows its buttons stand below its top row's top (`grid_drop`).
    drop: f32,
    /// Rows the line at its foot, the symbol's name and source, is raised in its row, so its
    /// tails are as far from the panel's foot as the buttons are from its top and sides: it
    /// stood 13 px from the foot against 20 there.
    foot_lift: f32,
}

/// How far a panel's last line of words is raised in its row so its tails stand `SIDE_INSET`
/// from the panel's foot, in rows; nothing where there are no pixels.
fn foot_lift(measure: &dyn Measure) -> f32 {
    let (Some((_, tails)), Some((wide, row))) = (
        crate::measures::words_in_row(measure),
        measure.cell_pixels(),
    ) else {
        return 0.0;
    };
    let row = row as f32;
    let side = SIDE_INSET * wide as f32;
    ((side - (row - tails)) / row).clamp(0.0, 0.5)
}

/// Rows the chip of the button picked keeps from the button's top and foot.
const CHIP_INSET: f32 = 0.08;

impl MathPanel {
    pub fn new(origin: (u16, u16), measure: &dyn Measure) -> Self {
        let previews = SYMBOLS
            .iter()
            .map(|symbol| {
                let cell = measure.cell_pixels()?;
                let em = (cell.1 as f32 * INTERFACE_SIZE * 1.15) as u32;
                let formula = cellix_doc::math::render(symbol.preview, false, em).ok()?;
                let bitmap = Bitmap {
                    id: formula.id,
                    width: formula.width,
                    height: formula.height,
                    kind: Pixels::Coverage,
                    pixels: formula.alpha.clone(),
                };
                let (width, height) = (
                    formula.width as f32 / cell.0 as f32,
                    formula.height as f32 / cell.1 as f32,
                );
                // Tall templates shrink to fit their button.
                let fit = (BUTTON.1 as f32 * 0.82 / height)
                    .min((BUTTON.0 as f32 - 0.8) / width)
                    .min(1.0);
                Some((bitmap, width * fit, height * fit))
            })
            .collect();
        Self {
            origin,
            previews,
            selected: 0,
            drop: grid_drop(CHIP_INSET, measure),
            foot_lift: foot_lift(measure),
        }
    }

    fn rows(&self) -> u16 {
        (SYMBOLS.len() as u16).div_ceil(COLUMNS)
    }

    pub fn area(&self) -> Rect {
        Rect::new(
            self.origin.0,
            self.origin.1,
            COLUMNS * BUTTON.0 + 2,
            self.rows() * BUTTON.1 + 2,
        )
    }

    /// The symbol at a place on screen, `y` with its fraction of a row.
    pub fn symbol_at(&self, x: u16, y: f32) -> Option<usize> {
        let area = self.area();
        let column = x.checked_sub(area.x + 1)? / BUTTON.0;
        let down = y - f32::from(area.y) - self.drop;
        if down < 0.0 {
            return None;
        }
        let row = down as u16 / BUTTON.1;
        let index = row as usize * COLUMNS as usize + column as usize;
        (column < COLUMNS && index < SYMBOLS.len()).then_some(index)
    }

    pub fn key(&mut self, key: Key) {
        let last = SYMBOLS.len() - 1;
        let across = COLUMNS as usize;
        self.selected = match key {
            Key::Left => self.selected.saturating_sub(1),
            Key::Right => (self.selected + 1).min(last),
            Key::Up => self.selected.saturating_sub(across),
            Key::Down => (self.selected + across).min(last),
            _ => self.selected,
        };
    }

    pub fn draw(&self, grid: &mut Grid, hover: Option<(u16, f32)>, theme: &Theme) {
        let area = self.area();
        raised(grid, area, theme);
        let hovered = hover.and_then(|h| self.symbol_at(h.0, h.1));
        let shown = hovered.unwrap_or(self.selected);
        let base = Style {
            fg: theme.text.fg,
            bg: theme.popup,
            ..theme.bar
        };
        let clip = Bounds::cells(area.x as f32, area.y as f32, area.w as f32, area.h as f32);
        for (index, symbol) in SYMBOLS.iter().enumerate() {
            let (row, column) = (index as u16 / COLUMNS, index as u16 % COLUMNS);
            let button = Rect::new(
                area.x + 1 + column * BUTTON.0,
                area.y + row * BUTTON.1,
                BUTTON.0,
                BUTTON.1,
            );
            if index == shown {
                let mut bounds = Bounds::inset(button, 0.2, CHIP_INSET);
                bounds.y += cellix_render::units(self.drop);
                grid.panel(bounds, 14, Some(theme.popup_hover), Some(theme.accent));
            }
            match &self.previews[index] {
                Some((bitmap, width, height)) => {
                    let left = button.x as f32 + (BUTTON.0 as f32 - width) / 2.0;
                    let top = button.y as f32 + self.drop + (BUTTON.1 as f32 - height) / 2.0;
                    grid.paint.push(Paint::Image {
                        bounds: Bounds::cells(left, top, *width, *height),
                        clip,
                        bitmap: bitmap.clone(),
                        color: theme.text.fg,
                    });
                }
                None => {
                    let short: String = symbol.name.chars().take(4).collect();
                    grid.put_str(button.x, button.y, &short, base, button.right());
                }
            }
        }
        let symbol = &SYMBOLS[shown];
        let dim = Style {
            fg: theme.popup_dim,
            ..base
        };
        let y = area.bottom().saturating_sub(1);
        let right = area.right().saturating_sub(1);
        let from = grid.painted_so_far();
        let end = grid.label(area.x + 2, y, symbol.name, base, right);
        grid.label(end + 2, y, symbol.preview, dim, right);
        grid.lift_since(from, self.foot_lift);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cellix_render::CellMeasure;

    #[test]
    fn every_preview_typesets() {
        for symbol in &SYMBOLS {
            cellix_doc::math::render(symbol.preview, false, 20)
                .unwrap_or_else(|e| panic!("{}: {e}", symbol.name));
        }
    }

    #[test]
    fn every_template_is_valid_once_filled() {
        for symbol in SYMBOLS.iter().filter(|s| s.caret != usize::MAX) {
            let filled = format!("x{}", symbol.text).replace("{}", "{x}");
            cellix_doc::math::render(&filled, true, 20)
                .unwrap_or_else(|e| panic!("{}: {e}", symbol.name));
            assert!(
                symbol.caret <= symbol.text.chars().count(),
                "{}",
                symbol.name
            );
        }
    }

    #[test]
    fn the_picker_reads_a_size_from_the_grid() {
        let picker = TablePicker::new((10, 2), &CellMeasure);
        assert_eq!(picker.size_at(11, 2.0), Some((1, 1)));
        assert_eq!(picker.size_at(11 + 3 * 4, 4.5), Some((3, 5)));
        assert_eq!(picker.size_at(5, 2.0), None);
    }

    #[test]
    fn the_keyboard_walks_the_panel() {
        let mut panel = MathPanel::new((0, 2), &CellMeasure);
        panel.key(Key::Down);
        assert_eq!(panel.selected, COLUMNS as usize);
        panel.key(Key::Left);
        panel.key(Key::Up);
        assert_eq!(panel.selected, 0);
        assert_eq!(panel.symbol_at(1, 2.0), Some(0));
        assert_eq!(panel.symbol_at(6, 4.0), Some(COLUMNS as usize + 1));
    }
}
