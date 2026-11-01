//! How wide text really is. Carets, wrapping and clicks all need this, and it differs by
//! tier: a text cell is one column per character, and a font isn't.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use cosmic_text::{
    Attrs, Buffer, Family, FontSystem, Metrics, Shaping, Style as FontStyle, Weight, fontdb,
};
use unicode_width::UnicodeWidthChar;

use crate::raster::{INTER, INTER_ITALIC, MONO, MONO_ITALIC, text_pixels};

/// How a run of painted text is set.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Font {
    pub mono: bool,
    pub bold: bool,
    pub italic: bool,
    /// Percent of the normal interface size.
    pub size: u8,
}

impl Default for Font {
    fn default() -> Self {
        Self {
            mono: false,
            bold: false,
            italic: false,
            size: 100,
        }
    }
}

pub(crate) fn family(font: Font) -> Family<'static> {
    if font.mono {
        Family::Monospace
    } else {
        Family::SansSerif
    }
}

pub(crate) fn attrs(font: Font) -> Attrs<'static> {
    let mut attrs = Attrs::new().family(family(font));
    if font.bold {
        attrs = attrs.weight(Weight::SEMIBOLD);
    }
    if font.italic {
        attrs = attrs.style(FontStyle::Italic);
    }
    attrs
}

/// Measures text, in cells, one advance per char.
pub trait Measure {
    fn advances(&self, text: &str, font: Font) -> Rc<Vec<f32>>;

    /// True when text sits in whole cells, so cell positions are exact.
    fn in_cells(&self) -> bool;

    fn width(&self, text: &str, font: Font) -> f32 {
        self.advances(text, font).iter().sum()
    }
}

/// The text-cell tier, and tests.
pub struct CellMeasure;

impl Measure for CellMeasure {
    fn advances(&self, text: &str, _font: Font) -> Rc<Vec<f32>> {
        Rc::new(
            text.chars()
                .map(|c| c.width().unwrap_or(0) as f32)
                .collect(),
        )
    }

    fn in_cells(&self) -> bool {
        true
    }
}

/// One advance per char, in cells.
type Advances = Rc<Vec<f32>>;

/// The pixel tiers: the same fonts and sizes the rasterizer draws with.
pub struct FontMeasure {
    fonts: RefCell<FontSystem>,
    cell: (usize, usize),
    cache: RefCell<HashMap<(String, Font), Advances>>,
}

impl FontMeasure {
    pub fn new(cell_width: usize, cell_height: usize) -> Self {
        let mut db = fontdb::Database::new();
        for data in [INTER, INTER_ITALIC, MONO, MONO_ITALIC] {
            db.load_font_data(data.to_vec());
        }
        db.set_monospace_family("JetBrains Mono");
        db.set_sans_serif_family("Inter Variable");
        Self {
            fonts: RefCell::new(FontSystem::new_with_locale_and_db("en-US".into(), db)),
            cell: (cell_width, cell_height),
            cache: RefCell::new(HashMap::new()),
        }
    }
}

impl Measure for FontMeasure {
    fn advances(&self, text: &str, font: Font) -> Rc<Vec<f32>> {
        let key = (text.to_string(), font);
        if let Some(found) = self.cache.borrow().get(&key) {
            return found.clone();
        }
        let px = text_pixels(self.cell.1, font);
        let fonts = &mut *self.fonts.borrow_mut();
        let mut buffer = Buffer::new(fonts, Metrics::new(px, px * 1.4));
        buffer.set_size(None, None);
        let style = attrs(font);
        buffer.set_rich_text(
            std::iter::once((text, style.clone())),
            &style,
            Shaping::Advanced,
            None,
        );
        buffer.shape_until_scroll(fonts, false);
        let mut by_byte = vec![0.0; text.len() + 1];
        for run in buffer.layout_runs() {
            for glyph in run.glyphs {
                by_byte[glyph.start] += glyph.w;
            }
        }
        let cell = self.cell.0 as f32;
        let advances: Vec<f32> = text
            .char_indices()
            .map(|(at, _)| by_byte[at] / cell)
            .collect();
        let advances = Rc::new(advances);
        let mut cache = self.cache.borrow_mut();
        if cache.len() > 20_000 {
            cache.clear();
        }
        cache.insert(key, advances.clone());
        advances
    }

    fn in_cells(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cells_count_columns() {
        assert_eq!(CellMeasure.width("abc", Font::default()), 3.0);
    }

    #[test]
    fn proportional_text_has_real_widths() {
        let m = FontMeasure::new(17, 37);
        let font = Font::default();
        assert!(m.width("iiii", font) < m.width("WWWW", font));
        assert_eq!(m.advances("héllo", font).len(), 5);
    }

    #[test]
    fn mono_text_keeps_even_advances() {
        let m = FontMeasure::new(17, 37);
        let font = Font {
            mono: true,
            ..Font::default()
        };
        let advances = m.advances("iW", font);
        assert!((advances[0] - advances[1]).abs() < 0.01);
    }

    #[test]
    fn larger_text_is_wider() {
        let m = FontMeasure::new(17, 37);
        let big = Font {
            size: 200,
            ..Font::default()
        };
        let width = m.width("Title", big);
        assert!((width - 2.0 * m.width("Title", Font::default())).abs() < 0.3);
    }
}
