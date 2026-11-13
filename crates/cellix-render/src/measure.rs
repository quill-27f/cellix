//! How wide text really is. Carets, wrapping and clicks all need this, and it differs by
//! tier: a text cell is one column per character, and a font isn't.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use cosmic_text::{
    Attrs, Buffer, Family, FontSystem, Metrics, Shaping, Style as FontStyle, Weight, fontdb,
};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::raster::{INTER, INTER_ITALIC, MONO, MONO_ITALIC, emoji_font, text_pixels};

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

/// Whether a letter is drawn as an emoji: it carries the mark that asks for it, or it's one
/// of those that are emoji unless told otherwise. A text font may hold a plain heart or a
/// smiling face, and would draw the emoji as that.
pub(crate) fn emoji_letter(letter: &str) -> bool {
    letter.contains('\u{fe0f}')
        || letter.chars().next().is_some_and(|c| {
            let code = c as u32;
            (0x1f000..=0x1faff).contains(&code)
                || ((0x2300..=0x2bff).contains(&code) && c.width() == Some(2))
        })
}

/// A line of text as runs to set: its emoji in the emoji font, and the rest in its own.
pub(crate) fn runs<'a>(
    text: &'a str,
    font: Font,
    emoji: Option<&'a str>,
) -> Vec<(&'a str, Attrs<'a>)> {
    let plain = attrs(font);
    let Some(family) = emoji.filter(|_| !text.is_ascii()) else {
        return vec![(text, plain)];
    };
    let coloured = plain.clone().family(Family::Name(family));
    let pick = |emoji: bool| {
        if emoji {
            coloured.clone()
        } else {
            plain.clone()
        }
    };
    let mut out = Vec::new();
    let (mut start, mut kind) = (0, None);
    for (at, letter) in text.grapheme_indices(true) {
        let is = emoji_letter(letter);
        if let Some(was) = kind.filter(|was| *was != is) {
            out.push((&text[start..at], pick(was)));
            start = at;
        }
        kind = Some(is);
    }
    out.push((&text[start..], pick(kind == Some(true))));
    out
}

/// Measures text, in cells, one advance per char.
pub trait Measure {
    fn advances(&self, text: &str, font: Font) -> Rc<Vec<f32>>;

    /// True when text sits in whole cells, so cell positions are exact.
    fn in_cells(&self) -> bool;

    /// A cell's size in pixels, where there are pixels.
    fn cell_pixels(&self) -> Option<(u32, u32)> {
        None
    }

    /// Whether emoji come out as themselves here. A terminal draws its own; painted text
    /// needs a font for them on the machine that paints it, which a server may not have.
    fn shows_emoji(&self) -> bool {
        true
    }

    fn width(&self, text: &str, font: Font) -> f32 {
        self.advances(text, font).iter().sum()
    }
}

/// The text-cell tier, and tests.
pub struct CellMeasure;

impl Measure for CellMeasure {
    fn advances(&self, text: &str, _font: Font) -> Rc<Vec<f32>> {
        // Plain text, which is nearly all of it, is a column a letter.
        if text.is_ascii() {
            let column = |byte: u8| f32::from(u8::from(!byte.is_ascii_control()));
            return Rc::new(text.bytes().map(column).collect());
        }
        // By what reads as one letter, as the grid places them: an emoji made of several
        // code points takes its two columns once, on its first.
        let mut out = Vec::with_capacity(text.len());
        for letter in text.graphemes(true) {
            let count = letter.chars().count();
            // A line's end is two controls in one letter, and takes no room.
            if count == 1 || letter.chars().any(char::is_control) {
                out.extend(letter.chars().map(|c| c.width().unwrap_or(0) as f32));
            } else {
                out.push(letter.width() as f32);
                out.extend(std::iter::repeat_n(0.0, count - 1));
            }
        }
        Rc::new(out)
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
    /// The built-in fonts with the machine's emoji font beside them, and that font's name,
    /// for text that holds emoji: the rasterizer draws those in it, so they're measured
    /// in it. Made when such text first turns up.
    with_emoji: RefCell<Option<(FontSystem, Option<String>)>>,
    cell: (usize, usize),
    cache: RefCell<HashMap<(String, Font), Advances>>,
}

fn built_in() -> fontdb::Database {
    let mut db = fontdb::Database::new();
    for data in [INTER, INTER_ITALIC, MONO, MONO_ITALIC] {
        db.load_font_data(data.to_vec());
    }
    db.set_monospace_family("JetBrains Mono");
    db.set_sans_serif_family("Inter Variable");
    db
}

impl FontMeasure {
    pub fn new(cell_width: usize, cell_height: usize) -> Self {
        Self {
            fonts: RefCell::new(FontSystem::new_with_locale_and_db(
                "en-US".into(),
                built_in(),
            )),
            with_emoji: RefCell::new(None),
            cell: (cell_width, cell_height),
            cache: RefCell::new(HashMap::new()),
        }
    }

    /// Each byte's advance in pixels, set in `fonts` with its emoji in the `emoji` font.
    fn shape(
        &self,
        fonts: &mut FontSystem,
        emoji: Option<&str>,
        text: &str,
        font: Font,
    ) -> Vec<f32> {
        let px = text_pixels(self.cell.1, font);
        let mut buffer = Buffer::new(fonts, Metrics::new(px, px * 1.4));
        buffer.set_size(None, None);
        buffer.set_rich_text(
            runs(text, font, emoji),
            &attrs(font),
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
        by_byte
    }

    /// The same for text that holds emoji, with them set in the machine's emoji font.
    fn shape_with_emoji(&self, text: &str, font: Font) -> Vec<f32> {
        let mut with_emoji = self.with_emoji.borrow_mut();
        let (fonts, emoji) = with_emoji.get_or_insert_with(|| {
            let mut db = built_in();
            let family = emoji_font().map(|(family, source)| {
                db.load_font_source(source.clone());
                family.clone()
            });
            let fonts = FontSystem::new_with_locale_and_db("en-US".into(), db);
            (fonts, family)
        });
        self.shape(fonts, emoji.as_deref(), text, font)
    }
}

impl Measure for FontMeasure {
    fn advances(&self, text: &str, font: Font) -> Rc<Vec<f32>> {
        let key = (text.to_string(), font);
        if let Some(found) = self.cache.borrow().get(&key) {
            return found.clone();
        }
        // An emoji is drawn in the machine's emoji font, so that's what it's measured in.
        let emoji = !text.is_ascii() && text.graphemes(true).any(emoji_letter);
        let by_byte = if emoji {
            self.shape_with_emoji(text, font)
        } else {
            self.shape(&mut self.fonts.borrow_mut(), None, text, font)
        };
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

    fn cell_pixels(&self) -> Option<(u32, u32)> {
        Some((self.cell.0 as u32, self.cell.1 as u32))
    }

    fn shows_emoji(&self) -> bool {
        emoji_font().is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cells_count_columns() {
        assert_eq!(CellMeasure.width("abc", Font::default()), 3.0);
    }

    /// An emoji takes the two columns the grid gives it, however many code points it's
    /// made of, and an advance for each of them all the same.
    #[test]
    fn an_emoji_of_several_code_points_is_two_columns() {
        let font = Font::default();
        for emoji in ["👍", "❤\u{fe0f}", "👍🏽", "👩\u{200d}💻", "🇹🇷"] {
            assert_eq!(CellMeasure.width(emoji, font), 2.0, "{emoji}");
            let advances = CellMeasure.advances(emoji, font);
            assert_eq!(advances.len(), emoji.chars().count(), "{emoji}");
        }
        assert_eq!(CellMeasure.width("a❤\u{fe0f}b", font), 4.0);
        assert_eq!(CellMeasure.width("e\u{301}\r\n", font), 1.0);
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

    /// An emoji is measured in the font that draws it, where the machine has one: the
    /// built-in fonts have none, and their blank is narrower than any emoji.
    #[test]
    fn an_emoji_is_measured_in_the_font_that_draws_it() {
        let m = FontMeasure::new(17, 37);
        let font = Font::default();
        if !m.shows_emoji() {
            return;
        }
        let advances = m.advances("a👍b", font);
        assert_eq!(advances.len(), 3);
        let letter = m.width("a", font);
        assert!((advances[0] - letter).abs() < 0.01, "{advances:?}");
        // About as wide as it is tall, which is more than a letter and less than three.
        assert!(advances[1] > letter && advances[1] < 3.0, "{advances:?}");
        // A heart the built-in font has a plain form of is still measured as the emoji.
        let heart = m.width("❤\u{fe0f}", font);
        assert!(
            (heart - advances[1]).abs() < 0.2,
            "{heart} beside {advances:?}"
        );
    }

    #[test]
    fn a_line_is_set_in_runs_with_its_emoji_apart() {
        let font = Font::default();
        let all = runs("ok ❤\u{fe0f}👍 then ✅", font, Some("Some Emoji"));
        let words: Vec<&str> = all.iter().map(|(text, _)| *text).collect();
        assert_eq!(words, ["ok ", "❤\u{fe0f}👍", " then ", "✅"]);
        assert_eq!(runs("plain", font, Some("Some Emoji")).len(), 1);
        assert_eq!(runs("ok 👍", font, None).len(), 1);
        // A letter from another script, an arrow and a check mark aren't emoji.
        for letter in ["中", "→", "✓", "é"] {
            assert!(!emoji_letter(letter), "{letter}");
        }
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
