use std::collections::HashMap;
use std::rc::Rc;

use compact_str::CompactString;
use cosmic_text::{
    Attrs, Buffer, Color, Fallback, Family, FontSystem, Metrics, PlatformFallback, Shaping,
    Style as FontStyle, SwashCache, Weight, fontdb,
};
use unicode_script::Script;
use unicode_width::UnicodeWidthStr;

use crate::grid::{Grid, Rgb};
use crate::icon::{Icon, Piece, inked};
use crate::measure::{Font, attrs, emoji_letter, runs};
use crate::paint::{
    BOTTOM_LEFT, BOTTOM_RIGHT, Bitmap, Bounds, Paint, Pixels, TOP_LEFT, TOP_RIGHT, UNIT,
};

/// A tightly packed RGB image, matching the Kitty protocol's `f=24`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Image {
    pub width: usize,
    pub height: usize,
    pub rgb: Vec<u8>,
}

/// Turns one row of a grid into an image.
pub trait Rasterizer {
    fn row(&mut self, grid: &Grid, y: u16) -> Image;
}

#[derive(Clone, Hash, PartialEq, Eq)]
struct GlyphKey {
    symbol: CompactString,
    bold: bool,
    italic: bool,
}

/// Coverage of one glyph: pixel offsets from the cell's corner, with alpha.
type Mask = Rc<Vec<(u16, u16, u8)>>;
/// A colour glyph: pixel offsets, colours and alpha.
type Painted = Rc<Vec<(u16, u16, Rgb, u8)>>;

#[derive(Clone, Hash, PartialEq, Eq)]
struct TextKey {
    text: String,
    font: Font,
}

/// A line of interface text, drawn once and blended wherever it's used.
struct TextMask {
    width: usize,
    height: usize,
    alpha: Vec<u8>,
    /// The colour of each pixel that has one of its own, as an emoji's do, where the line
    /// holds any. The rest take the text's.
    own: Option<Vec<Option<Rgb>>>,
}

/// Fonts that hold Chinese and Japanese. The usual choices can be on-demand downloads a
/// machine doesn't have, and a missing one leaves the text blank.
const CJK_FONTS: [&str; 9] = [
    "PingFang SC",
    "Hiragino Sans",
    "Hiragino Sans GB",
    "Songti SC",
    "Heiti SC",
    "Arial Unicode MS",
    "Noto Sans CJK SC",
    "Noto Sans CJK JP",
    "WenQuanYi Micro Hei",
];

/// The platform's fallback, with a longer list for the scripts it runs out of.
struct WiderFallback(PlatformFallback);

impl Fallback for WiderFallback {
    fn common_fallback(&self) -> &[&'static str] {
        self.0.common_fallback()
    }

    fn forbidden_fallback(&self) -> &[&'static str] {
        self.0.forbidden_fallback()
    }

    fn script_fallback(&self, script: Script, locale: &str) -> &[&'static str] {
        match script {
            Script::Han | Script::Hiragana | Script::Katakana => &CJK_FONTS,
            other => self.0.script_fallback(other, locale),
        }
    }
}

fn system_fonts() -> fontdb::Database {
    let mut db = fontdb::Database::new();
    db.load_system_fonts();
    db
}

/// The fonts text is drawn with: the ones in the binary, and behind them whatever the
/// machine has, for the scripts those don't hold and for emoji in the machine's own style.
fn drawing_fonts() -> FontSystem {
    let mut fonts = FontSystem::new_with_locale_and_db_and_fallback(
        "en-US".to_string(),
        system_fonts(),
        WiderFallback(PlatformFallback),
    );
    let db = fonts.db_mut();
    db.load_font_data(INTER.to_vec());
    db.load_font_data(INTER_ITALIC.to_vec());
    db.load_font_data(MONO.to_vec());
    db.load_font_data(MONO_ITALIC.to_vec());
    db.load_font_data(EMOJI.to_vec());
    db.set_monospace_family("JetBrains Mono");
    db.set_sans_serif_family("Inter Variable");
    if EMOJI_FONT.get().is_none() {
        let found = find_emoji_font(&mut fonts);
        EMOJI_FONT.get_or_init(|| found);
    }
    fonts
}

/// The font emoji are drawn in, as its family and where it's read from. Found once: finding
/// it means looking at every font there is, which the rasterizer does as it starts.
static EMOJI_FONT: std::sync::OnceLock<Option<(String, fontdb::Source)>> =
    std::sync::OnceLock::new();

/// The machine's own emoji fonts, in the order they're reached for, so emoji look here as
/// they do in its other programs.
const SYSTEM_EMOJI: [&str; 3] = ["Apple Color Emoji", "Noto Color Emoji", "Segoe UI Emoji"];
/// What the emoji font in the binary calls itself: Twemoji, as Mozilla builds it.
const BUILT_IN_EMOJI: &str = "Twemoji Mozilla";

/// A line of emoji set in one family, when every one of them came from it and not from a
/// font the search fell back to.
fn set_in(fonts: &mut FontSystem, family: &str, emoji: &str) -> Option<Buffer> {
    let mut buffer = Buffer::new(fonts, Metrics::new(32.0, 40.0));
    buffer.set_size(None, Some(40.0));
    let attrs = Attrs::new().family(Family::Name(family));
    let rich = std::iter::once((emoji, attrs.clone()));
    buffer.set_rich_text(rich, &attrs, Shaping::Advanced, None);
    buffer.shape_until_scroll(fonts, false);
    let from_it = |glyph: &cosmic_text::LayoutGlyph| {
        let face = fonts.db().face(glyph.font_id);
        glyph.glyph_id != 0
            && face.is_some_and(|face| face.families.iter().any(|(name, _)| name == family))
    };
    let glyphs: Vec<&cosmic_text::LayoutGlyph> = buffer
        .layout_runs()
        .flat_map(|run| run.glyphs.iter())
        .collect();
    let whole = !glyphs.is_empty() && glyphs.into_iter().all(from_it);
    whole.then_some(buffer)
}

/// Whether the emoji font in the binary holds an emoji, whole: one it only has the parts of
/// would be drawn as those, side by side. This is what a machine with no emoji font of its
/// own can show, so it's what's worth offering.
pub fn built_in_emoji_holds(emoji: &str) -> bool {
    thread_local! {
        static FONTS: std::cell::RefCell<FontSystem> = std::cell::RefCell::new({
            let mut db = fontdb::Database::new();
            db.load_font_data(EMOJI.to_vec());
            FontSystem::new_with_locale_and_db("en-US".to_string(), db)
        });
    }
    FONTS.with(|fonts| {
        let set = set_in(&mut fonts.borrow_mut(), BUILT_IN_EMOJI, emoji);
        set.is_some_and(|buffer| {
            let glyphs = buffer.layout_runs().flat_map(|run| run.glyphs.iter());
            glyphs.filter(|glyph| glyph.w > 0.0).count() == 1
        })
    })
}

/// Whether a family's emoji come out in colour here. A font in a format the rasterizer
/// can't read, as a newer Noto Color Emoji is, draws nothing or a plain shape.
fn draws_colour(fonts: &mut FontSystem, family: &str) -> bool {
    let Some(mut buffer) = set_in(fonts, family, "👍") else {
        return false;
    };
    let mut coloured = false;
    let mut swash = SwashCache::new();
    let white = Color::rgb(255, 255, 255);
    buffer.draw(fonts, &mut swash, white, |_, _, _, _, color| {
        coloured |= color.a() > 0 && (color.r(), color.g(), color.b()) != (255, 255, 255);
    });
    coloured
}

/// The machine's own emoji font where it has one this can draw, and the one in the binary
/// where it doesn't: a server reached over SSH rarely has any.
fn find_emoji_font(fonts: &mut FontSystem) -> Option<(String, fontdb::Source)> {
    // `CELLIX_EMOJI=built-in` keeps to the one in the binary, for the same emoji on every
    // machine.
    let built_in_only = std::env::var("CELLIX_EMOJI").is_ok_and(|choice| choice == "built-in");
    let own = SYSTEM_EMOJI.iter().copied().filter(|_| !built_in_only);
    for family in own.chain([BUILT_IN_EMOJI]) {
        let source = fonts
            .db()
            .faces()
            .find(|face| face.families.iter().any(|(name, _)| name == family))
            .map(|face| face.source.clone());
        if let Some(source) = source
            && draws_colour(fonts, family)
        {
            return Some((family.to_string(), source));
        }
    }
    None
}

/// The font emoji are drawn in. Whatever measures text sets its emoji in the same one, or a
/// caret after an emoji stands in the wrong place.
pub(crate) fn emoji_font() -> Option<&'static (String, fontdb::Source)> {
    if EMOJI_FONT.get().is_none() {
        drop(drawing_fonts());
    }
    EMOJI_FONT.get().and_then(Option::as_ref)
}

/// How dark a floating panel's shadow is at its edge, out of 255.
const DARK_SHADOW: f32 = 110.0;
const LIGHT_SHADOW: f32 = 38.0;
/// Interface text is drawn at this size relative to the row height: 13 pt in a terminal set
/// to 14 pt, whose row is 18.5 pt, which is the size VS Code sets its own words at.
pub const INTERFACE_SIZE: f32 = 0.70;
/// Cached text lines before the cache is dropped, so a long session can't grow it forever.
const MAX_TEXT_MASKS: usize = 4096;

pub(crate) const INTER: &[u8] = include_bytes!("../fonts/InterVariable.ttf");
pub(crate) const INTER_ITALIC: &[u8] = include_bytes!("../fonts/InterVariable-Italic.ttf");
pub(crate) const MONO: &[u8] = include_bytes!("../fonts/JetBrainsMonoVariable.ttf");
pub(crate) const MONO_ITALIC: &[u8] = include_bytes!("../fonts/JetBrainsMonoVariable-Italic.ttf");
/// Twemoji, for emoji on a machine with no emoji font of its own.
pub(crate) const EMOJI: &[u8] = include_bytes!("../fonts/TwemojiMozilla.ttf");

/// The pixel size painted text is drawn at, for a row height and font.
pub(crate) fn text_pixels(row_height: usize, font: Font) -> f32 {
    row_height as f32 * INTERFACE_SIZE * font.size as f32 / 100.0
}

/// Where something `h` paint units tall at `y` stands in rows `row` pixels tall whose words
/// are set for a line of `line` pixels, as a top and a height in pixels. Anything shorter
/// than a row keeps the height it has on that line, in the middle of where it stood: a
/// mark, a badge, a rule. What all but fills its row on that line is a control with a row to
/// itself, a button, a text box, the band behind a picked row, a tip. On a taller row
/// those are one height, `CONTROL` of the row, about 20 pt where a row is 26: the air goes
/// round them and inside them both. A chip, which holds words and is a little shorter than
/// a control, is `CHIP` of a taller row, so its words stand in its middle with room under
/// their tails: on the line it's too short for that, and its words are raised in it.
pub fn stood(y: i32, h: i32, row: f32, line: f32) -> (f32, f32) {
    let (top, tall) = (y as f32 * row / UNIT as f32, h as f32 * row / UNIT as f32);
    if h < UNIT && line < row {
        let kept = h as f32 * line / UNIT as f32;
        let share = h as f32 / UNIT as f32;
        let own = if share >= FILLS {
            kept.max(row * CONTROL).min(tall)
        } else if share >= HOLDS {
            kept.max(row * CHIP).min(tall)
        } else {
            kept
        };
        (top + (tall - own) / 2.0, own)
    } else {
        (top, tall)
    }
}

/// Where a button on a row's wash stands, as a top and a height in pixels: at the height it
/// has on the terminal's line, in the middle of where it stood. `stood` grows a button to a
/// control's height on a taller row, which is the height of the wash behind a pointed row,
/// so a button on one was exactly as tall as the wash. Kept at the line's height, a button
/// of 32 px on a wash of 39 has the wash 4 px over it and under it.
pub fn inset(y: i32, h: i32, row: f32, line: f32) -> (f32, f32) {
    let (top, tall) = (y as f32 * row / UNIT as f32, h as f32 * row / UNIT as f32);
    let kept = (h as f32 * line / UNIT as f32).min(tall);
    (top + (tall - kept) / 2.0, kept)
}

/// How much of the terminal's line something takes before it counts as a control with the
/// row to itself, and how much of a taller row such a control is.
const FILLS: f32 = 0.8;
const CONTROL: f32 = 0.78;
/// The same for a chip that holds words: the design's 20 px in a row of 28.
const HOLDS: f32 = 0.7;
const CHIP: f32 = 0.714;

/// CPU rendering. Glyphs are rasterized once and blended into their cells, so text stays
/// locked to the grid and a row costs little more than a copy.
pub struct SoftwareRasterizer {
    fonts: FontSystem,
    /// The font emoji are drawn in, where the machine has one.
    emoji: Option<String>,
    swash: SwashCache,
    metrics: Metrics,
    cell: (usize, usize),
    /// The row height that words, icons and controls take their size from. It's the row's
    /// own, unless Cellix lays out rows taller than the terminal's line: then it's that line.
    type_row: usize,
    masks: HashMap<GlyphKey, Mask>,
    /// Glyphs with colours of their own, such as emoji, as pixels with those colours.
    painted: HashMap<GlyphKey, Painted>,
    text: HashMap<TextKey, Rc<TextMask>>,
}

impl SoftwareRasterizer {
    /// The fonts ship in the binary, because pixels are drawn on the remote host over SSH
    /// and servers rarely have any. Returns None only when the cell size is unknown.
    pub fn new(cell_width: usize, cell_height: usize) -> Option<Self> {
        if cell_width == 0 || cell_height == 0 {
            return None;
        }
        let mut fonts = drawing_fonts();
        let metrics = fit_font(&mut fonts, cell_width, cell_height)?;
        let emoji = emoji_font().map(|(family, _)| family.clone());
        Some(Self {
            fonts,
            emoji,
            swash: SwashCache::new(),
            metrics,
            cell: (cell_width, cell_height),
            type_row: cell_height,
            masks: HashMap::new(),
            painted: HashMap::new(),
            text: HashMap::new(),
        })
    }

    /// Takes the size of words, icons and controls from a row of `rows` pixels instead of
    /// this one's own height. Where Cellix lays out rows taller than the terminal's line,
    /// what stands in a row stays the size it is and the row gives it air.
    pub fn with_type_row(mut self, rows: usize) -> Self {
        self.type_row = rows.clamp(1, self.cell.1);
        self
    }

    /// Where something `h` tall at `y` stands, as a top and a height in pixels. Anything
    /// shorter than a row is a control or a mark: it keeps the height it has on the
    /// terminal's own line, in the middle of where it stood.
    fn stand(&self, y: i32, h: i32) -> (f32, f32) {
        stood(y, h, self.cell.1 as f32, self.type_row as f32)
    }

    fn mask(&mut self, key: &GlyphKey) -> Mask {
        if let Some(mask) = self.masks.get(key) {
            return mask.clone();
        }
        let (width, height) = (self.cell.0 * 2, self.cell.1);
        let mut buffer = Buffer::new(&mut self.fonts, self.metrics);
        buffer.set_size(Some(width as f32 * 2.0), Some(height as f32));
        // Asked for a monospaced Han glyph, the search stops at a font that only has blanks.
        // A wide glyph fills its two cells whatever font it's from.
        // And an emoji is drawn in the emoji font: a text font may hold a plain form of it.
        let emoji = self.emoji.clone().filter(|_| emoji_letter(&key.symbol));
        let family = if let Some(emoji) = &emoji {
            Family::Name(emoji)
        } else if UnicodeWidthStr::width(key.symbol.as_str()) > 1 {
            Family::SansSerif
        } else {
            Family::Monospace
        };
        let mut attrs = Attrs::new().family(family);
        if key.bold {
            attrs = attrs.weight(Weight::BOLD);
        }
        if key.italic {
            attrs = attrs.style(FontStyle::Italic);
        }
        let rich = std::iter::once((key.symbol.as_str(), attrs.clone()));
        buffer.set_rich_text(rich, &attrs, Shaping::Advanced, None);
        buffer.shape_until_scroll(&mut self.fonts, false);
        let mut pixels = Vec::new();
        let mut coloured = Vec::new();
        let white = Color::rgb(255, 255, 255);
        buffer.draw(
            &mut self.fonts,
            &mut self.swash,
            white,
            |x, y, w, h, color| {
                if color.a() == 0 {
                    return;
                }
                for py in y..y + h as i32 {
                    for px in x..x + w as i32 {
                        if px >= 0 && py >= 0 && (px as usize) < width && (py as usize) < height {
                            pixels.push((px as u16, py as u16, color.a()));
                            let rgb = Rgb(color.r(), color.g(), color.b());
                            coloured.push((px as u16, py as u16, rgb, color.a()));
                        }
                    }
                }
            },
        );
        // A terminal's box-drawing is made to meet the next line's. On a taller row it's
        // drawn to the row's whole height, or the sides of a box would stop short of each other.
        if self.type_row < height && boxes(&key.symbol) {
            pixels = stretched(&pixels, self.type_row, height);
            coloured.clear();
        }
        // Text comes out white, ready to take any colour. Anything else is the glyph's own.
        if coloured
            .iter()
            .any(|(_, _, rgb, _)| *rgb != Rgb(255, 255, 255))
        {
            self.painted.insert(key.clone(), Rc::new(coloured));
        }
        let mask = Rc::new(pixels);
        self.masks.insert(key.clone(), mask.clone());
        mask
    }

    /// A line of painted text, tall enough for its size.
    fn text_mask(&mut self, text: &str, font: Font) -> Rc<TextMask> {
        let key = TextKey {
            text: text.to_string(),
            font,
        };
        if let Some(mask) = self.text.get(&key) {
            return mask.clone();
        }
        if self.text.len() >= MAX_TEXT_MASKS {
            self.text.clear();
        }
        let px = text_pixels(self.type_row, font);
        let height = self.cell.1.max((px * 1.45).ceil() as usize);
        let mut buffer = Buffer::new(&mut self.fonts, Metrics::new(px, height as f32));
        buffer.set_size(None, Some(height as f32));
        let emoji = self.emoji.clone();
        let rich = runs(text, font, emoji.as_deref());
        buffer.set_rich_text(rich, &attrs(font), Shaping::Advanced, None);
        buffer.shape_until_scroll(&mut self.fonts, false);
        let width = buffer
            .layout_runs()
            .next()
            .map_or(0.0, |run| run.line_w)
            .ceil() as usize
            + 2;
        let mut alpha = vec![0; width * height];
        // Text comes out in the colour it's asked for, and an emoji in its own. Drawn in
        // white, anything that isn't white is an emoji's.
        let white = Rgb(255, 255, 255);
        let mut drawn = vec![white; width * height];
        let mut tinted = false;
        buffer.draw(
            &mut self.fonts,
            &mut self.swash,
            Color::rgb(255, 255, 255),
            |x, y, w, h, color| {
                let rgb = Rgb(color.r(), color.g(), color.b());
                tinted |= color.a() > 0 && rgb != white;
                for at in spots((width, height), (x, y), (w, h)) {
                    if color.a() >= alpha[at] {
                        drawn[at] = rgb;
                    }
                    alpha[at] = alpha[at].max(color.a());
                }
            },
        );
        // An emoji has white of its own too, which drawn in white can't be told from text.
        // Drawn again in black, what turned black is text and what stayed is the emoji's.
        let own = tinted.then(|| {
            let black = Rgb(0, 0, 0);
            let mut own = vec![None; width * height];
            buffer.draw(
                &mut self.fonts,
                &mut self.swash,
                Color::rgb(0, 0, 0),
                |x, y, w, h, color| {
                    let rgb = Rgb(color.r(), color.g(), color.b());
                    for at in spots((width, height), (x, y), (w, h)) {
                        if color.a() > 0 && (rgb != black || drawn[at] != white) {
                            own[at] = Some(drawn[at]);
                        }
                    }
                },
            );
            own
        });
        let mask = Rc::new(TextMask {
            width,
            height,
            alpha,
            own,
        });
        self.text.insert(key, mask.clone());
        mask
    }

    fn draw_paint(&mut self, target: &mut Target, paint: &Paint) {
        let (cw, ch) = (self.cell.0 as f32, self.cell.1 as f32);
        // What sizes are taken from: the row's own height, or the terminal's line under a
        // taller row.
        let th = self.type_row as f32;
        let scale = |v: i32, cell: f32| v as f32 * cell / UNIT as f32;
        match paint {
            Paint::Panel {
                bounds,
                radius,
                fill,
                border,
                shadow,
                dashed,
                inset: on_a_wash,
            } => {
                let (y, h) = if *on_a_wash {
                    inset(bounds.y, bounds.h, ch, th)
                } else {
                    self.stand(bounds.y, bounds.h)
                };
                let mut shape = Shape {
                    x: scale(bounds.x, cw),
                    y,
                    w: scale(bounds.w, cw),
                    h,
                    radius: *radius as f32 * th / 100.0,
                };
                // A panel with a line round it stands on whole pixels, so the line is one
                // pixel of its own colour on every side. Between pixels it was two rows at
                // half strength on one side and one at full on the other.
                if border.is_some() {
                    let right = (shape.x + shape.w).round();
                    let bottom = (shape.y + shape.h).round();
                    shape.x = shape.x.round();
                    shape.y = shape.y.round();
                    shape.w = right - shape.x;
                    shape.h = bottom - shape.y;
                }
                target.panel(shape, (*fill, *border), *shadow, *dashed, th);
            }
            Paint::Wash {
                bounds,
                radius,
                color,
                alpha,
                border,
            } => {
                let (y, h) = self.stand(bounds.y, bounds.h);
                let shape = Shape {
                    x: scale(bounds.x, cw),
                    y,
                    w: scale(bounds.w, cw),
                    h,
                    radius: *radius as f32 * th / 100.0,
                };
                target.wash(shape, (*color, *alpha), *border, th);
            }
            Paint::Hairline { bounds, color } => {
                let thickness = (th / 24.0).ceil().max(1.0);
                // Across its length on whole pixels: between two, a menu's rule a fraction of a
                // row down was a pale band three pixels tall.
                let (x, y) = (scale(bounds.x, cw), scale(bounds.y, ch));
                let shape = Shape {
                    x: if bounds.w == 0 { x.round() } else { x },
                    y: if bounds.h == 0 { y.round() } else { y },
                    w: scale(bounds.w, cw).max(thickness),
                    h: scale(bounds.h, ch).max(thickness),
                    radius: 0.0,
                };
                target.panel(shape, (Some(*color), None), false, false, th);
            }
            Paint::Text {
                x,
                y,
                max_width,
                text,
                color,
                font,
                align_right,
                centred,
                underline,
            } => {
                let mask = self.text_mask(text, *font);
                let mut left = scale(*x, cw).round() as i32;
                if *align_right {
                    left -= mask.width as i32;
                } else if *centred {
                    left -= mask.width as i32 / 2;
                }
                // Tall text stays centred on its row.
                let lift = (mask.height as i32 - ch as i32) / 2;
                let top = scale(*y, ch).round() as i32 - lift;
                let limit = scale(*max_width, cw).max(0.0) as usize;
                target.blit(&mask, left, top, limit, *color);
                if *underline {
                    let thickness = (th / 24.0).ceil().max(1.0);
                    let shape = Shape {
                        x: left as f32,
                        // Under the words, which stand in the middle of the row.
                        y: scale(*y, ch) + (ch - th) / 2.0 + th * 0.78,
                        w: mask.width.saturating_sub(2).min(limit) as f32,
                        h: thickness,
                        radius: 0.0,
                    };
                    target.panel(shape, (Some(*color), None), false, false, th);
                }
            }
            Paint::Icon {
                bounds,
                icon,
                color,
            } => {
                // An icon is the size it is on the terminal's own line, in the middle of
                // where a taller row puts it.
                let side = scale(bounds.h, th);
                let left = scale(bounds.x, cw);
                let top = scale(bounds.y, ch) + (scale(bounds.h, ch) - side) / 2.0;
                target.icon(*icon, left, top, side, *color);
            }
            Paint::Spinner {
                bounds,
                color,
                step,
            } => {
                let side = scale(bounds.h, th);
                let left = scale(bounds.x, cw);
                let top = scale(bounds.y, ch) + (scale(bounds.h, ch) - side) / 2.0;
                target.spinner(*step, left, top, side, *color);
            }
            Paint::Corners {
                bounds,
                radius,
                color,
                which,
            } => {
                let shape = Shape {
                    x: scale(bounds.x, cw),
                    y: scale(bounds.y, ch),
                    w: scale(bounds.w, cw),
                    h: scale(bounds.h, ch),
                    radius: *radius as f32 * th / 100.0,
                };
                target.corners(&shape, *color, *which);
            }
            Paint::Image {
                bounds,
                clip,
                bitmap,
                color,
            } => {
                let shape = Shape {
                    x: scale(bounds.x, cw),
                    y: scale(bounds.y, ch),
                    w: scale(bounds.w, cw),
                    h: scale(bounds.h, ch),
                    radius: 0.0,
                };
                let across = (
                    scale(clip.x, cw) as i32,
                    scale(clip.x + clip.w, cw).ceil() as i32,
                );
                let down = (
                    scale(clip.y, ch) as i32,
                    scale(clip.y + clip.h, ch).ceil() as i32,
                );
                target.image(&shape, (across, down), bitmap, *color);
            }
            Paint::Strips { bounds, strips } => {
                let (left, top) = (scale(bounds.x, cw), scale(bounds.y, ch));
                let (width, height) = (scale(bounds.w, cw), scale(bounds.h, ch));
                let thickness = (th / 24.0).ceil().max(1.0);
                for strip in strips {
                    let shape = Shape {
                        x: left + strip.x as f32 * width / 256.0,
                        y: top + strip.y as f32 * height / 256.0,
                        w: (strip.w as f32 * width / 256.0).max(1.0),
                        h: thickness,
                        radius: 0.0,
                    };
                    target.panel(shape, (Some(strip.color), None), false, false, ch);
                }
            }
        }
    }
}

/// The places in a `size` image that a box at `at`, `by` across and down, covers.
fn spots(
    (width, height): (usize, usize),
    (x, y): (i32, i32),
    (w, h): (u32, u32),
) -> impl Iterator<Item = usize> {
    let across = x.max(0) as usize..((x + w as i32).max(0) as usize).min(width);
    let down = y.max(0) as usize..((y + h as i32).max(0) as usize).min(height);
    down.flat_map(move |py| across.clone().map(move |px| py * width + px))
}

/// Where the caret goes: its exact place, or the cell the cursor is in.
fn caret_bounds(grid: &Grid) -> Option<Bounds> {
    grid.caret.or_else(|| {
        let (x, y) = grid.cursor?;
        Some(Bounds::cells(x as f32, y as f32 + 0.08, 0.0, 0.84))
    })
}

fn caret_color(grid: &Grid) -> Rgb {
    let Some((x, y)) = grid.cursor else {
        return Rgb(255, 255, 255);
    };
    grid.cell(x.min(grid.width().saturating_sub(1)), y).style.fg
}

struct Shape {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    radius: f32,
}

/// Distance from a point to a rounded box's edge, negative inside.
fn rounded_distance(shape: &Shape, x: f32, y: f32) -> f32 {
    let (cx, cy) = (shape.x + shape.w / 2.0, shape.y + shape.h / 2.0);
    let qx = (x - cx).abs() - shape.w / 2.0 + shape.radius;
    let qy = (y - cy).abs() - shape.h / 2.0 + shape.radius;
    let outside = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt();
    outside + qx.max(qy).min(0.0) - shape.radius
}

/// The part of the screen one row's tile covers, in screen pixels.
struct Target<'a> {
    rgb: &'a mut [u8],
    width: usize,
    height: usize,
    top: i32,
}

impl Target<'_> {
    fn corners(&mut self, s: &Shape, color: Rgb, which: u8) {
        let r = s.radius;
        let spots = [
            (TOP_LEFT, s.x, s.y, s.x + r, s.y + r),
            (TOP_RIGHT, s.x + s.w - r, s.y, s.x + s.w - r, s.y + r),
            (
                BOTTOM_RIGHT,
                s.x + s.w - r,
                s.y + s.h - r,
                s.x + s.w - r,
                s.y + s.h - r,
            ),
            (BOTTOM_LEFT, s.x, s.y + s.h - r, s.x + r, s.y + s.h - r),
        ];
        for (flag, left, top, cx, cy) in spots {
            if which & flag == 0 {
                continue;
            }
            for y in top.floor() as i32..(top + r).ceil() as i32 {
                for x in left.floor() as i32..(left + r).ceil() as i32 {
                    let d = ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2)).sqrt();
                    let outside = (d - r + 0.5).clamp(0.0, 1.0);
                    self.blend(x, y, color, (outside * 255.0) as u8);
                }
            }
        }
    }

    fn blend(&mut self, x: i32, y: i32, color: Rgb, alpha: u8) {
        let row = y - self.top;
        if alpha == 0 || x < 0 || row < 0 || x as usize >= self.width || row as usize >= self.height
        {
            return;
        }
        let at = (row as usize * self.width + x as usize) * 3;
        self.rgb[at] = blend(color.0, self.rgb[at], alpha);
        self.rgb[at + 1] = blend(color.1, self.rgb[at + 1], alpha);
        self.rgb[at + 2] = blend(color.2, self.rgb[at + 2], alpha);
    }

    fn panel(
        &mut self,
        s: Shape,
        (fill, border): (Option<Rgb>, Option<Rgb>),
        shadow: bool,
        dashed: bool,
        row: f32,
    ) {
        // Dashes a quarter of a row long, with gaps as long, measured along the edge so every
        // row of a tall panel continues the same pattern.
        let dash = (row / 4.0).max(2.0);
        let on_dash = |x: f32, y: f32| {
            let along_top = x > s.x + s.radius && x < s.x + s.w - s.radius;
            let run = if along_top { x - s.x } else { y - s.y };
            (run / dash).floor() as i64 % 2 == 0
        };
        // A shadow falls down and to the sides, never up onto what opened the panel.
        let reach = if shadow { row * 0.35 } else { 1.0 };
        let (x0, x1) = (
            (s.x - reach).floor() as i32,
            (s.x + s.w + reach).ceil() as i32,
        );
        let above = if shadow { 1.0 } else { reach };
        let y0 = ((s.y - above).floor() as i32).max(self.top);
        let y1 = ((s.y + s.h + reach).ceil() as i32).min(self.top + self.height as i32);
        let stroke = (row / 30.0).max(1.0);
        for y in y0..y1 {
            for x in x0..x1 {
                let d = rounded_distance(&s, x as f32 + 0.5, y as f32 + 0.5);
                if shadow && d > 0.0 && y as f32 + 0.5 > s.y + s.radius {
                    let fade = (1.0 - d / reach).clamp(0.0, 1.0);
                    // A shadow heavy enough for dark surfaces is a grey halo on light ones.
                    let light = fill.is_some_and(|f| f.0 > 180);
                    let depth = if light { LIGHT_SHADOW } else { DARK_SHADOW };
                    self.blend(x, y, Rgb(0, 0, 0), (fade * fade * depth) as u8);
                }
                let inside = ((0.5 - d) * 255.0).clamp(0.0, 255.0) as u8;
                if let Some(color) = fill {
                    self.blend(x, y, color, inside);
                }
                if let Some(color) = border
                    && (!dashed || on_dash(x as f32 + 0.5, y as f32 + 0.5))
                {
                    let edge = ((0.5 - (d + stroke / 2.0).abs() + stroke / 2.0) * 255.0)
                        .clamp(0.0, 255.0) as u8;
                    self.blend(x, y, color, edge);
                }
            }
        }
    }

    /// A see-through fill with round corners, and a border twice a panel's weight.
    fn wash(&mut self, s: Shape, (color, alpha): (Rgb, u8), border: Option<Rgb>, row: f32) {
        let (x0, x1) = (s.x.floor() as i32 - 1, (s.x + s.w).ceil() as i32 + 1);
        let y0 = (s.y.floor() as i32 - 1).max(self.top);
        let y1 = ((s.y + s.h).ceil() as i32 + 1).min(self.top + self.height as i32);
        let stroke = (row / 15.0).max(2.0);
        for y in y0..y1 {
            for x in x0..x1 {
                let d = rounded_distance(&s, x as f32 + 0.5, y as f32 + 0.5);
                let inside = (0.5 - d).clamp(0.0, 1.0);
                self.blend(x, y, color, (inside * alpha as f32) as u8);
                if let Some(edge_color) = border {
                    // The border runs just inside the edge.
                    let along = (0.5 - (d + stroke / 2.0).abs() + stroke / 2.0).clamp(0.0, 1.0);
                    self.blend(x, y, edge_color, (along * 255.0) as u8);
                }
            }
        }
    }

    fn blit(&mut self, mask: &TextMask, left: i32, top: i32, limit: usize, color: Rgb) {
        let from = (self.top - top).max(0) as usize;
        let to = ((self.top + self.height as i32 - top).max(0) as usize).min(mask.height);
        for y in from..to {
            for x in 0..mask.width.min(limit) {
                let at = y * mask.width + x;
                let ink = mask.own.as_ref().and_then(|own| own[at]).unwrap_or(color);
                self.blend(left + x as i32, top + y as i32, ink, mask.alpha[at]);
            }
        }
    }

    /// Scales a bitmap into `s`, sampling between its pixels so nothing shimmers.
    fn image(&mut self, s: &Shape, clip: ((i32, i32), (i32, i32)), bitmap: &Bitmap, color: Rgb) {
        let (width, height) = (bitmap.width as usize, bitmap.height as usize);
        if width == 0 || height == 0 || s.w <= 0.0 || s.h <= 0.0 {
            return;
        }
        let channels = match bitmap.kind {
            Pixels::Coverage => 1,
            Pixels::Rgba => 4,
        };
        let at = |x: usize, y: usize, c: usize| -> f32 {
            let index = (y.min(height - 1) * width + x.min(width - 1)) * channels + c;
            bitmap.pixels.get(index).copied().unwrap_or(0) as f32
        };
        let sample = |u: f32, v: f32, c: usize| -> f32 {
            let (fx, fy) = (u.max(0.0), v.max(0.0));
            let (x, y) = (fx as usize, fy as usize);
            let (dx, dy) = (fx - x as f32, fy - y as f32);
            let top = at(x, y, c) * (1.0 - dx) + at(x + 1, y, c) * dx;
            let bottom = at(x, y + 1, c) * (1.0 - dx) + at(x + 1, y + 1, c) * dx;
            top * (1.0 - dy) + bottom * dy
        };
        let ((left, right), (top, bottom)) = clip;
        let y0 = (s.y.floor() as i32).max(self.top).max(top);
        let y1 = ((s.y + s.h).ceil() as i32)
            .min(self.top + self.height as i32)
            .min(bottom);
        let x0 = (s.x.floor() as i32).max(0).max(left);
        let x1 = ((s.x + s.w).ceil() as i32)
            .min(self.width as i32)
            .min(right);
        let (ax, ay) = (width as f32 / s.w, height as f32 / s.h);
        // A frame made for exactly this size is a copy, which keeps a live page cheap.
        if bitmap.kind == Pixels::Rgba && (ax - 1.0).abs() < 0.002 && (ay - 1.0).abs() < 0.002 {
            let (left, top) = (s.x.round() as i32, s.y.round() as i32);
            for y in y0..y1 {
                for x in x0..x1 {
                    let (u, v) = (x - left, y - top);
                    if u < 0 || v < 0 || u as usize >= width || v as usize >= height {
                        continue;
                    }
                    let at = (v as usize * width + u as usize) * 4;
                    if let Some(p) = bitmap.pixels.get(at..at + 4) {
                        self.blend(x, y, Rgb(p[0], p[1], p[2]), p[3]);
                    }
                }
            }
            return;
        }
        for y in y0..y1 {
            for x in x0..x1 {
                let u = (x as f32 + 0.5 - s.x) * ax - 0.5;
                let v = (y as f32 + 0.5 - s.y) * ay - 0.5;
                match bitmap.kind {
                    Pixels::Coverage => self.blend(x, y, color, sample(u, v, 0) as u8),
                    Pixels::Rgba => {
                        let ink = Rgb(
                            sample(u, v, 0) as u8,
                            sample(u, v, 1) as u8,
                            sample(u, v, 2) as u8,
                        );
                        self.blend(x, y, ink, sample(u, v, 3) as u8);
                    }
                }
            }
        }
    }

    /// A spinner in a square box: its ring at a quarter of the colour's strength and its arc
    /// over that at full, or with no step a still dot.
    fn spinner(&mut self, step: Option<u8>, left: f32, top: f32, side: f32, color: Rgb) {
        const SAMPLES: usize = 5;
        let y0 = (top.floor() as i32).max(self.top);
        let y1 = ((top + side).ceil() as i32).min(self.top + self.height as i32);
        for y in y0..y1 {
            for x in left.floor() as i32..(left + side).ceil() as i32 {
                let (mut ring, mut arc) = (0, 0);
                for sy in 0..SAMPLES {
                    for sx in 0..SAMPLES {
                        let u = (x as f32 + (sx as f32 + 0.5) / SAMPLES as f32 - left) / side;
                        let v = (y as f32 + (sy as f32 + 0.5) / SAMPLES as f32 - top) / side;
                        if (0.0..1.0).contains(&u) && (0.0..1.0).contains(&v) {
                            let (r, a) = crate::spinner::inked(step, u, v);
                            ring += usize::from(r);
                            arc += usize::from(a);
                        }
                    }
                }
                let all = SAMPLES * SAMPLES;
                self.blend(x, y, color, (ring * 64 / all) as u8);
                self.blend(x, y, color, (arc * 255 / all) as u8);
            }
        }
    }

    fn icon(&mut self, icon: Icon, left: f32, top: f32, side: f32, color: Rgb) {
        // Five by five keeps thin curved strokes smooth, and tiles only draw this once.
        const SAMPLES: usize = 5;
        let y0 = (top.floor() as i32).max(self.top);
        let y1 = ((top + side).ceil() as i32).min(self.top + self.height as i32);
        for y in y0..y1 {
            for x in left.floor() as i32..(left + side).ceil() as i32 {
                let mut hits = 0;
                for sy in 0..SAMPLES {
                    for sx in 0..SAMPLES {
                        let u = (x as f32 + (sx as f32 + 0.5) / SAMPLES as f32 - left) / side;
                        let v = (y as f32 + (sy as f32 + 0.5) / SAMPLES as f32 - top) / side;
                        if (0.0..1.0).contains(&u) && (0.0..1.0).contains(&v) && inked(icon, u, v) {
                            hits += 1;
                        }
                    }
                }
                self.blend(x, y, color, (hits * 255 / (SAMPLES * SAMPLES)) as u8);
            }
        }
    }
}

/// Whether a cell holds box-drawing or a block element, which join across rows.
fn boxes(symbol: &str) -> bool {
    let mut chars = symbol.chars();
    matches!(
        (chars.next(), chars.next()),
        (Some('\u{2500}'..='\u{259f}'), None)
    )
}

/// A glyph drawn in the middle `from` rows of `to`, pulled out to fill all of them.
fn stretched(pixels: &[(u16, u16, u8)], from: usize, to: usize) -> Vec<(u16, u16, u8)> {
    let first = (to - from) / 2;
    let mut out = Vec::with_capacity(pixels.len() * to / from.max(1) + 1);
    for &(x, y, alpha) in pixels {
        let line = (y as usize).checked_sub(first).filter(|line| *line < from);
        let Some(line) = line else { continue };
        for row in line * to / from..(line + 1) * to / from {
            out.push((x, row as u16, alpha));
        }
    }
    out
}

/// Picks the largest font whose advance fits the cell and whose height fits the row.
fn fit_font(fonts: &mut FontSystem, cell_width: usize, cell_height: usize) -> Option<Metrics> {
    const PROBE: f32 = 100.0;
    let mut buffer = Buffer::new(fonts, Metrics::new(PROBE, PROBE * 1.2));
    buffer.set_size(Some(PROBE * 4.0), Some(PROBE * 2.0));
    let attrs = Attrs::new().family(Family::Monospace);
    buffer.set_rich_text(
        std::iter::once(("M", attrs.clone())),
        &attrs,
        Shaping::Advanced,
        None,
    );
    buffer.shape_until_scroll(fonts, false);
    let advance = buffer.layout_runs().next()?.line_w;
    if advance <= 0.0 {
        return None;
    }
    let by_width = cell_width as f32 * PROBE / advance;
    let by_height = cell_height as f32 * 0.8;
    Some(Metrics::new(by_width.min(by_height), cell_height as f32))
}

/// Inks this cell's share of an icon, supersampled so curves stay smooth.
fn draw_icon(
    rgb: &mut [u8],
    stride: usize,
    left: usize,
    cell: (usize, usize),
    piece: Piece,
    fg: Rgb,
) {
    const SAMPLES: usize = 4;
    let (cw, ch) = cell;
    let (full_w, full_h) = (cw * piece.width as usize, ch * piece.height as usize);
    let side = full_w.min(full_h) as f32 * 0.94;
    let (off_x, off_y) = ((full_w as f32 - side) / 2.0, (full_h as f32 - side) / 2.0);
    for y in 0..ch {
        for x in 0..cw {
            let mut hits = 0;
            for sy in 0..SAMPLES {
                for sx in 0..SAMPLES {
                    let px = (piece.column as usize * cw + x) as f32
                        + (sx as f32 + 0.5) / SAMPLES as f32;
                    let py =
                        (piece.row as usize * ch + y) as f32 + (sy as f32 + 0.5) / SAMPLES as f32;
                    let (u, v) = ((px - off_x) / side, (py - off_y) / side);
                    if (0.0..1.0).contains(&u) && (0.0..1.0).contains(&v) && inked(piece.icon, u, v)
                    {
                        hits += 1;
                    }
                }
            }
            if hits > 0 {
                let alpha = (hits * 255 / (SAMPLES * SAMPLES)) as u8;
                let at = (y * stride + left + x) * 3;
                rgb[at] = blend(fg.0, rgb[at], alpha);
                rgb[at + 1] = blend(fg.1, rgb[at + 1], alpha);
                rgb[at + 2] = blend(fg.2, rgb[at + 2], alpha);
            }
        }
    }
}

fn blend(fg: u8, bg: u8, alpha: u8) -> u8 {
    let a = alpha as u32;
    ((fg as u32 * a + bg as u32 * (255 - a)) / 255) as u8
}

fn fill(rgb: &mut [u8], stride: usize, x: usize, y: usize, w: usize, h: usize, color: Rgb) {
    for row in y..y + h {
        for px in x..x + w {
            let i = (row * stride + px) * 3;
            if i + 2 < rgb.len() {
                rgb[i..i + 3].copy_from_slice(&[color.0, color.1, color.2]);
            }
        }
    }
}

impl Rasterizer for SoftwareRasterizer {
    fn row(&mut self, grid: &Grid, y: u16) -> Image {
        let cells = grid.row(y);
        let (cw, ch) = self.cell;
        let width = cw * cells.len();
        let mut rgb = vec![0; width * ch * 3];
        // Backgrounds first: a wide glyph runs into the next cell, whose colour would cut it.
        for (i, cell) in cells.iter().enumerate() {
            fill(&mut rgb, width, i * cw, 0, cw, ch, cell.style.bg);
        }
        for (i, cell) in cells.iter().enumerate() {
            let fg = cell.style.fg;
            if cell.symbol.is_empty() || cell.symbol == " " || cell.style.painted {
                continue;
            }
            // Drawn rather than taken from a font, so stacked rows join with no gap.
            if cell.symbol == "▌" {
                fill(&mut rgb, width, i * cw, 0, cw.div_ceil(5), ch, fg);
                continue;
            }
            if let Some(piece) = Piece::decode(&cell.symbol) {
                draw_icon(&mut rgb, width, i * cw, (cw, ch), piece, fg);
                continue;
            }
            let key = GlyphKey {
                symbol: cell.symbol.clone(),
                bold: cell.style.bold,
                italic: cell.style.italic,
            };
            let mask = self.mask(&key);
            if let Some(painted) = self.painted.get(&key) {
                for &(x, y, color, alpha) in painted.iter() {
                    let at = (y as usize * width + i * cw + x as usize) * 3;
                    if at + 2 < rgb.len() && (i * cw + x as usize) < width {
                        rgb[at] = blend(color.0, rgb[at], alpha);
                        rgb[at + 1] = blend(color.1, rgb[at + 1], alpha);
                        rgb[at + 2] = blend(color.2, rgb[at + 2], alpha);
                    }
                }
                continue;
            }
            for &(x, y, alpha) in mask.iter() {
                let at = (y as usize * width + i * cw + x as usize) * 3;
                if at + 2 < rgb.len() && (i * cw + x as usize) < width {
                    rgb[at] = blend(fg.0, rgb[at], alpha);
                    rgb[at + 1] = blend(fg.1, rgb[at + 1], alpha);
                    rgb[at + 2] = blend(fg.2, rgb[at + 2], alpha);
                }
            }
        }
        for (i, cell) in cells.iter().enumerate() {
            // Painted text underlines itself, at its true width.
            if cell.style.underline && !cell.style.painted {
                fill(&mut rgb, width, i * cw, ch - 2, cw, 1, cell.style.fg);
            }
        }
        let paint: Vec<Paint> = grid.paint_on_row(y).cloned().collect();
        let mut target = Target {
            rgb: &mut rgb,
            width,
            height: ch,
            top: y as i32 * ch as i32,
        };
        for item in &paint {
            self.draw_paint(&mut target, item);
        }
        // A thin caret, as in a real editor, not a terminal's block.
        if let Some(bounds) = caret_bounds(grid) {
            let row = (y as i32 * UNIT, (y as i32 + 1) * UNIT);
            if bounds.y < row.1 && bounds.y + bounds.h > row.0 {
                // As tall as the words it stands among, whatever the row's height.
                let tall = bounds.h as f32 * ch as f32 / UNIT as f32;
                let own = if bounds.h <= UNIT {
                    bounds.h as f32 * self.type_row as f32 / UNIT as f32
                } else {
                    tall
                };
                let shape = Shape {
                    x: bounds.x as f32 * cw as f32 / UNIT as f32,
                    y: bounds.y as f32 * ch as f32 / UNIT as f32 + (tall - own) / 2.0,
                    w: (cw as f32 / 9.0).max(2.0),
                    h: own,
                    radius: 0.0,
                };
                let mut target = Target {
                    rgb: &mut rgb,
                    width,
                    height: ch,
                    top: y as i32 * ch as i32,
                };
                target.panel(
                    shape,
                    (Some(caret_color(grid)), None),
                    false,
                    false,
                    self.type_row as f32,
                );
            }
        }
        Image {
            width,
            height: ch,
            rgb,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grid::{Grid, Rect, Style};

    fn rasterizer() -> Option<SoftwareRasterizer> {
        SoftwareRasterizer::new(10, 20)
    }

    fn style() -> Style {
        Style {
            fg: Rgb(250, 250, 250),
            bg: Rgb(10, 20, 30),
            ..Style::default()
        }
    }

    fn lit(image: &Image, cell: usize, width: usize) -> usize {
        let mut count = 0;
        for y in 0..image.height {
            for x in cell * width..(cell + 1) * width {
                let i = (y * image.width + x) * 3;
                count += usize::from(image.rgb[i] > 128);
            }
        }
        count
    }

    #[test]
    fn row_image_is_exactly_the_grid_size() {
        let Some(mut r) = rasterizer() else { return };
        let grid = Grid::new(8, 1, style());
        let image = r.row(&grid, 0);
        assert_eq!((image.width, image.height), (80, 20));
        assert_eq!(image.rgb.len(), 80 * 20 * 3);
    }

    #[test]
    fn han_characters_are_drawn_in_the_cells() {
        let Some(mut r) = rasterizer() else { return };
        let fonts = system_fonts();
        let has_han = fonts.faces().any(|face| {
            let name = &face.families[0].0;
            CJK_FONTS.iter().any(|font| font == name)
        });
        if !has_han {
            return;
        }
        let mut grid = Grid::new(4, 1, style());
        grid.put_str(0, 0, "中", style(), 4);
        let image = r.row(&grid, 0);
        assert!(lit(&image, 0, 10) > 10);
    }

    #[test]
    fn a_wide_glyph_keeps_its_right_half() {
        let Some(mut r) = rasterizer() else { return };
        let mut grid = Grid::new(4, 1, style());
        grid.put_str(1, 0, "MM", style(), 4);
        let image = r.row(&grid, 0);
        assert!(lit(&image, 1, 10) > 10);
        assert!(
            lit(&image, 2, 10) > 10,
            "the next cell's colour didn't cover it"
        );
    }

    #[test]
    fn glyphs_land_in_their_own_cells() {
        let Some(mut r) = rasterizer() else { return };
        let mut grid = Grid::new(4, 1, style());
        grid.put_str(2, 0, "M", style(), 4);
        let image = r.row(&grid, 0);
        assert_eq!(lit(&image, 0, 10), 0);
        assert!(lit(&image, 2, 10) > 10, "M drew nothing");
        assert_eq!(lit(&image, 3, 10), 0, "M spilled into the next cell");
    }

    #[test]
    fn the_caret_is_a_thin_line() {
        let Some(mut r) = rasterizer() else { return };
        let mut grid = Grid::new(3, 1, style());
        grid.cursor = Some((1, 0));
        let image = r.row(&grid, 0);
        let at = |x: usize| image.rgb[(10 * image.width + x) * 3];
        assert_eq!(at(10), 250);
        assert_eq!(at(15), 10);
        grid.caret = Some(Bounds::cells(1.5, 0.0, 0.0, 1.0));
        let image = r.row(&grid, 0);
        let at = |x: usize| image.rgb[(10 * image.width + x) * 3];
        assert_eq!(at(10), 10, "the exact caret replaces the cell one");
        assert_eq!(at(15), 250);
    }

    #[test]
    fn labels_draw_without_their_cell_glyphs() {
        let Some(mut r) = rasterizer() else { return };
        let mut grid = Grid::new(20, 1, style());
        grid.label(1, 0, "Illi", style(), 20);
        assert!(grid.cell(1, 0).style.painted);
        let with_paint = r.row(&grid, 0);
        // Inter is proportional: this text is far narrower than four cells.
        let lit_right = (0..20)
            .flat_map(|y| (40..200).map(move |x| (x, y)))
            .any(|(x, y)| with_paint.rgb[(y * with_paint.width + x) * 3] > 128);
        assert!(!lit_right, "drew past the narrow label");
    }

    /// Words are set at the size VS Code sets its own: 13 pt. A terminal set to 14 pt has a
    /// cell of 17 by 37 px at two pixels a point, and a capital of 13 pt Inter stands 19 px
    /// tall there, with as much of the row over it as under it. At three fifths of a row,
    /// as the words were, it stood 16.
    #[test]
    fn words_are_thirteen_points_in_a_terminal_set_to_fourteen() {
        use crate::measure::Measure as _;
        let Some(mut r) = SoftwareRasterizer::new(17, 37) else {
            return;
        };
        let mut grid = Grid::new(8, 1, style());
        grid.label(1, 0, "H", style(), 8);
        let image = r.row(&grid, 0);
        let inked: Vec<usize> = (0..image.height)
            .filter(|y| (0..image.width).any(|x| image.rgb[(y * image.width + x) * 3] > 128))
            .collect();
        let (top, bottom) = (inked[0], inked[inked.len() - 1]);
        let tall = bottom - top + 1;
        assert!((18..=20).contains(&tall), "a capital is {tall} px tall");
        let (over, under) = (top, image.height - 1 - bottom);
        assert!(
            over.abs_diff(under) <= 2,
            "{over} px over it, {under} under"
        );
        // And as wide as 13 pt is: the measure and the picture agree on it.
        let wide = crate::measure::FontMeasure::new(17, 37).width("H", Font::default()) * 17.0;
        assert!((17.0..21.0).contains(&wide), "an H is {wide} px wide");
    }

    /// Where Cellix's rows are taller than the terminal's line, words stay the size they are
    /// on that line and stand in the middle of the taller row, which is air round them. So
    /// does a chip, being shorter than a row. And the side of a terminal's box is drawn the
    /// whole height of the row, so it meets the one in the next.
    #[test]
    fn a_taller_row_is_air_round_what_stands_in_it() {
        use crate::measure::Measure as _;
        let Some(r) = SoftwareRasterizer::new(17, 52) else {
            return;
        };
        let mut r = r.with_type_row(37);
        let mut grid = Grid::new(8, 1, style());
        grid.label(1, 0, "H", style(), 8);
        let image = r.row(&grid, 0);
        assert_eq!(image.height, 52);
        let inked: Vec<usize> = (0..image.height)
            .filter(|y| (0..image.width).any(|x| image.rgb[(y * image.width + x) * 3] > 128))
            .collect();
        let (top, bottom) = (inked[0], inked[inked.len() - 1]);
        let tall = bottom - top + 1;
        assert!(
            (18..=20).contains(&tall),
            "a capital is still {tall} px tall"
        );
        let (over, under) = (top, image.height - 1 - bottom);
        assert!(
            over.abs_diff(under) <= 2,
            "{over} px over it, {under} under"
        );
        let measure = crate::measure::FontMeasure::new(17, 52).with_type_row(37);
        let wide = measure.width("H", Font::default()) * 17.0;
        assert!((17.0..21.0).contains(&wide), "and as wide: {wide} px");
        // A badge five eighths of a row tall is five eighths of the line's 37 pixels. A chip
        // that holds words is taller than that, and grows: see
        // `a_control_is_one_height_on_a_taller_row`.
        let mut grid = Grid::new(8, 1, style());
        let chip = crate::paint::Bounds::cells(1.0, 0.1875, 4.0, 0.625);
        grid.panel(chip, 20, Some(Rgb(255, 0, 0)), None);
        let image = r.row(&grid, 0);
        let red: Vec<usize> = (0..image.height)
            .filter(|y| {
                let at = (y * image.width + 51) * 3;
                image.rgb[at] > 200 && image.rgb[at + 1] < 60
            })
            .collect();
        assert!(
            (22..=25).contains(&red.len()),
            "the badge is {} px tall",
            red.len()
        );
        let (over, under) = (red[0], 51 - red[red.len() - 1]);
        assert!(
            over.abs_diff(under) <= 2,
            "{over} px over it, {under} under"
        );
        // The side of a box, from a terminal in the side pane.
        let mut grid = Grid::new(4, 1, style());
        grid.put_str(1, 0, "│", style(), 4);
        let image = r.row(&grid, 0);
        let inked = (0..image.height)
            .filter(|y| (17..34).any(|x| image.rgb[(y * image.width + x) * 3] > 100))
            .count();
        assert!(inked >= 50, "the side is {inked} px of the row's 52");
    }

    /// On a taller row a control was the 32 px it is on the terminal's line, in 50 px of
    /// row, with its words 3 px high in it. The design has one height for a button, a text
    /// box, the band behind a picked row and a tip: about 20 pt in a row of 26.
    #[test]
    fn a_control_is_one_height_on_a_taller_row() {
        let Some(r) = SoftwareRasterizer::new(17, 52) else {
            return;
        };
        let mut r = r.with_type_row(37);
        // A box that all but fills its row on the terminal's line is a control: on the
        // taller row it's 0.78 of it, whichever of them it is, and stands in the middle.
        for share in [0.89, 0.92, 0.98] {
            let mut grid = Grid::new(8, 1, style());
            let inset = (1.0 - share) / 2.0;
            let field = crate::paint::Bounds::cells(1.0, inset, 4.0, share);
            grid.panel(field, 20, Some(Rgb(255, 0, 0)), None);
            let image = r.row(&grid, 0);
            let red: Vec<usize> = (0..image.height)
                .filter(|y| {
                    let at = (y * image.width + 51) * 3;
                    image.rgb[at] > 200 && image.rgb[at + 1] < 60
                })
                .collect();
            assert!(
                (39..=42).contains(&red.len()),
                "a box of {share} is {} px tall",
                red.len()
            );
            let (over, under) = (red[0], 51 - red[red.len() - 1]);
            assert!(over.abs_diff(under) <= 2, "{over} over, {under} under");
        }
        // On the terminal's own line it's what it was.
        assert_eq!(stood(4, 57, 36.0, 36.0), (2.25, 57.0 * 36.0 / 64.0));
        // A chip that holds words, three quarters of the line, is 0.714 of the taller row,
        // which leaves room under a tail with its words in its middle.
        let (top, chip) = stood(8, 48, 52.0, 37.0);
        assert!((chip - 52.0 * 0.714).abs() < 0.01, "{chip}");
        assert!(
            (top + chip / 2.0 - 26.0).abs() < 0.01,
            "in the row's middle: {top}"
        );
        // A mark or a badge, shorter than that, keeps the height it has on the line.
        let (_, mark) = stood(12, 40, 52.0, 37.0);
        assert!((mark - 40.0 * 37.0 / 64.0).abs() < 0.01, "{mark}");
        // And a control never outgrows the room it was given.
        let (top, tall) = stood(0, 60, 40.0, 37.0);
        assert!(top >= 0.0 && tall <= 60.0 * 40.0 / 64.0, "{top} {tall}");
    }

    /// A button on a pointed row's wash stands inside it where a row has room. Both were a
    /// control's height on a taller row, 39 px in one of 50, so the button's edges lay on the
    /// wash's. It keeps the 32 px it has on the terminal's line, in the middle of the wash.
    #[test]
    fn a_spinner_is_a_faint_ring_with_a_quarter_of_it_in_full_colour() {
        let Some(r) = SoftwareRasterizer::new(17, 50) else {
            return;
        };
        let mut r = r.with_type_row(37);
        let black = Style {
            bg: Rgb(0, 0, 0),
            ..style()
        };
        let side = 0.86 * 37.0;
        let (left, top) = (17.0, (50.0 - side) / 2.0);
        let ring = 3.0 / 24.0;
        // 0.86 of the line, 32 px across, in the middle of a row of 50: how blue it is at a
        // place in its box.
        let mut drawn = |turn: Option<u8>| {
            let mut grid = Grid::new(6, 1, black);
            grid.turn = turn;
            grid.spinner(1.0, 0, 0.86, Rgb(0, 0, 200), black);
            let image = r.row(&grid, 0);
            assert!(image.rgb.chunks(3).all(|p| p[0] == 0), "only its colour");
            move |u: f32, v: f32| {
                let (x, y) = ((left + u * side) as usize, (top + v * side) as usize);
                image.rgb[(y * image.width + x) * 3 + 2]
            }
        };
        // On the ring at its middle height: at the first step the arc is at the right, and the
        // left and the foot are only the faint ring, a quarter of the colour's strength.
        let at = drawn(Some(0));
        assert!(
            at(1.0 - ring, 0.5) > 150,
            "the arc: {}",
            at(1.0 - ring, 0.5)
        );
        assert!(
            (25..=70).contains(&at(ring, 0.5)),
            "faint: {}",
            at(ring, 0.5)
        );
        assert!((25..=70).contains(&at(0.5, 1.0 - ring)), "and at the foot");
        assert_eq!(at(0.5, 0.5), 0, "nothing in its middle");
        // Half a turn on, the arc is at the left.
        let at = drawn(Some(5));
        assert!(at(ring, 0.5) > 150);
        assert!(at(1.0 - ring, 0.5) < 70);
        // Still, it's a dot in its middle and no ring.
        let at = drawn(None);
        assert!(at(0.5, 0.5) > 150);
        assert_eq!(at(ring, 0.5), 0);
    }

    #[test]
    fn a_button_on_a_rows_wash_stands_inside_it() {
        let Some(r) = SoftwareRasterizer::new(17, 50) else {
            return;
        };
        let mut r = r.with_type_row(36);
        let mut grid = Grid::new(12, 1, style());
        grid.roomy = true;
        // The wash behind the row, and the button at its end.
        let wash = crate::paint::Bounds::cells(0.4, 0.06, 11.2, 0.88);
        grid.panel(wash, 12, Some(Rgb(0, 0, 255)), None);
        grid.row_button((6, 0), "    ", "", Rgb(255, 0, 0), style());
        let image = r.row(&grid, 0);
        let rows = |x: usize, red: bool| -> Vec<usize> {
            (0..image.height)
                .filter(|y| {
                    let at = (y * image.width + x) * 3;
                    let (r, b) = (image.rgb[at], image.rgb[at + 2]);
                    if red {
                        r > 200 && b < 60
                    } else {
                        b > 200 && r < 60
                    }
                })
                .collect()
        };
        // Down the button's middle, eight cells along: wash, button, wash.
        let (button, round) = (rows(136, true), rows(136, false));
        // 31.7 px, the height it has on the terminal's line, thirty rows of it whole and a
        // blended one at each edge, so the wash has 4 px over it and under it. At 27 px, three
        // quarters of the line, its words' tails touched its foot.
        assert!(
            (30..=33).contains(&button.len()),
            "{} px of button",
            button.len()
        );
        let over = round.iter().filter(|y| **y < button[0]).count();
        let under = round.len() - over;
        assert!(
            (3..=5).contains(&over) && over.abs_diff(under) <= 1,
            "{over} px of wash over the button, {under} under"
        );
        // Its corners are tighter than a button's, with its height.
        let tight = grid.paint.iter().any(|p| {
            matches!(
                p,
                Paint::Panel {
                    inset: true,
                    radius: 17,
                    ..
                }
            )
        });
        assert!(tight, "17 per cent of the line, where a button's are 22");
        // Beside it the wash is a control, all of 39 px.
        assert!((38..=40).contains(&rows(40, false).len()));
        // Where a row is the terminal's line, it's a button like any other: as it stood.
        assert_eq!(inset(4, 56, 36.0, 36.0), stood(4, 56, 36.0, 36.0));
        grid.paint.clear();
        grid.roomy = false;
        grid.row_button((6, 0), "    ", "", Rgb(255, 0, 0), style());
        let plain = grid
            .paint
            .iter()
            .any(|p| matches!(p, Paint::Panel { inset: true, .. }));
        assert!(!plain, "not inset where a row has no room");
    }

    /// A button in a row between lines (`Grid::row_button`) is 32 px on a row of 50, 9 px
    /// clear of the row, and its words' ink, a capital's top to a "p"'s tail, stands in its
    /// middle with 4 px or more under the tail: at 27 px the tail of Keep's "p" ended on its
    /// foot.
    #[test]
    fn a_row_buttons_words_stand_in_its_middle_tails_and_all() {
        let Some(r) = SoftwareRasterizer::new(17, 50) else {
            return;
        };
        let mut r = r.with_type_row(36);
        let mut grid = Grid::new(16, 1, style());
        grid.roomy = true;
        grid.row_button((2, 0), "            ", "Keep", Rgb(255, 0, 0), style());
        let image = r.row(&grid, 0);
        let at = |x: usize, y: usize| {
            let i = (y * image.width + x) * 3;
            (image.rgb[i], image.rgb[i + 1])
        };
        // The button, down a column inside it and clear of its words.
        let button: Vec<usize> = (0..image.height).filter(|y| at(45, *y).0 > 200).collect();
        assert!(
            (30..=33).contains(&button.len()),
            "{} px of button",
            button.len()
        );
        let (top, foot) = (button[0], *button.last().unwrap());
        assert!(
            top >= 8 && image.height - 1 - foot >= 8,
            "{top} px over it, {} under",
            image.height - 1 - foot
        );
        // Its words, white on its red: from the capital's top to the tail's foot.
        let ink: Vec<usize> = (0..image.height)
            .filter(|y| (100..170).any(|x| at(x, *y).1 > 90))
            .collect();
        let (caps, tail) = (ink[0], *ink.last().unwrap());
        let (over, under) = (caps - top, foot - tail);
        assert!(
            under >= 4 && over.abs_diff(under) <= 1,
            "{over} px over its capitals, {under} under its tails"
        );
    }

    /// An emoji in a line of painted text keeps its own colours, where the machine has a
    /// font that holds them: the letters beside it take the text's.
    #[test]
    fn an_emoji_in_painted_text_keeps_its_own_colours() {
        let Some(mut r) = rasterizer() else { return };
        let mask = r.text_mask("ok 🚀", Font::default());
        // Without a colour emoji font it's a blank, in the text's colour like the rest.
        let Some(own) = &mask.own else { return };
        let coloured = own.iter().flatten().filter(|rgb| rgb.0 != rgb.2).count();
        assert!(coloured > 10, "a rocket has red and blue in it: {coloured}");
        let plain = r.text_mask("ok", Font::default());
        assert!(plain.own.is_none(), "letters have no colour of their own");
        // Drawn in dark text on light, the letters are dark and the emoji is still itself.
        let mut grid = Grid::new(12, 1, style());
        let dark = Style {
            fg: Rgb(10, 10, 10),
            bg: Rgb(240, 240, 240),
            ..Style::default()
        };
        grid.fill(Rect::new(0, 0, 12, 1), dark);
        grid.label(0, 0, "ok 🚀", dark, 12);
        let image = r.row(&grid, 0);
        let pixels: Vec<Rgb> = image.rgb.chunks(3).map(|p| Rgb(p[0], p[1], p[2])).collect();
        let tinted = |p: &&Rgb| p.0.abs_diff(p.2) > 60;
        assert!(pixels.iter().filter(tinted).count() > 10);
    }

    #[test]
    fn panels_have_rounded_corners() {
        let Some(mut r) = rasterizer() else { return };
        let mut grid = Grid::new(10, 1, style());
        grid.panel(
            crate::paint::Bounds::cells(1.0, 0.0, 8.0, 1.0),
            40,
            Some(Rgb(255, 0, 0)),
            None,
        );
        let image = r.row(&grid, 0);
        let red = |x: usize, y: usize| image.rgb[(y * image.width + x) * 3];
        assert_eq!(red(50, 10), 255, "the middle is filled");
        assert_eq!(red(10, 0), 10, "the corner is cut");
    }

    /// A hairline a fraction of a row down stands on whole pixels, in its full colour: between
    /// two it was a pale band a pixel taller, as a menu's rules were where they shared out a
    /// half row.
    #[test]
    fn a_hairline_stands_on_whole_pixels() {
        let Some(mut r) = rasterizer() else { return };
        let mut grid = Grid::new(10, 1, style());
        // A third of a row down a row of 20 pixels: 6.67 pixels.
        let line = crate::paint::Bounds::cells(1.0, 1.0 / 3.0, 8.0, 0.0);
        grid.hairline(line, Rgb(255, 0, 0));
        let image = r.row(&grid, 0);
        let red = |y: usize| image.rgb[(y * image.width + 50) * 3];
        let touched: Vec<u8> = (0..image.height).map(red).filter(|r| *r > 10).collect();
        assert!(!touched.is_empty(), "the line is drawn");
        assert!(
            touched.iter().all(|r| *r == 255),
            "every row it touches in full: {touched:?}"
        );
    }

    #[test]
    fn a_wash_lets_what_is_under_it_show() {
        let Some(mut r) = rasterizer() else { return };
        let mut grid = Grid::new(20, 2, style());
        let bounds = crate::paint::Bounds::cells(1.0, 0.0, 18.0, 2.0);
        grid.wash(bounds, 10, (Rgb(255, 0, 0), 64), Some(Rgb(0, 0, 255)));
        let image = r.row(&grid, 0);
        let at = |x: usize, y: usize| {
            let i = (y * image.width + x) * 3;
            (image.rgb[i], image.rgb[i + 1], image.rgb[i + 2])
        };
        let middle = at(image.width / 2, image.height - 1);
        assert!(middle.0 > 0 && middle.0 < 200, "a little red: {middle:?}");
        let edge = at(image.width / 2, 0);
        assert!(edge.2 > 200, "the border: {edge:?}");
    }

    #[test]
    fn a_dashed_border_has_gaps() {
        let Some(mut r) = rasterizer() else { return };
        let mut grid = Grid::new(20, 1, style());
        grid.dashed_panel(
            crate::paint::Bounds::cells(1.0, 0.0, 18.0, 1.0),
            10,
            None,
            Rgb(255, 0, 0),
        );
        let image = r.row(&grid, 0);
        // Along the top edge, past the corner: some pixels inked, some left.
        let y = 0;
        let reds: Vec<u8> = (30..170)
            .map(|x| image.rgb[(y * image.width + x) * 3])
            .collect();
        assert!(reds.iter().any(|r| *r > 200), "dashes are drawn");
        assert!(reds.iter().any(|r| *r < 60), "with gaps between them");
    }

    /// The emoji font in the binary draws in colour with none of the machine's fonts, which
    /// is what a server reached over SSH has, and holds the eight emoji that stand under a
    /// comment being written.
    #[test]
    fn the_emoji_font_in_the_binary_draws_in_colour_by_itself() {
        let mut db = fontdb::Database::new();
        for data in [INTER, INTER_ITALIC, MONO, MONO_ITALIC, EMOJI] {
            db.load_font_data(data.to_vec());
        }
        db.set_monospace_family("JetBrains Mono");
        db.set_sans_serif_family("Inter Variable");
        let mut fonts = FontSystem::new_with_locale_and_db("en-US".to_string(), db);
        assert!(draws_colour(&mut fonts, BUILT_IN_EMOJI));
        let found = find_emoji_font(&mut fonts).map(|(family, _)| family);
        assert_eq!(found.as_deref(), Some(BUILT_IN_EMOJI));
        for emoji in ["👍", "💯", "❤\u{fe0f}", "🎉", "😄", "👀", "🚀", "✅"] {
            assert!(
                set_in(&mut fonts, BUILT_IN_EMOJI, emoji).is_some(),
                "{emoji}"
            );
        }
        // A family the machine doesn't have draws nothing in colour.
        assert!(!draws_colour(&mut fonts, "No Such Emoji"));
        // It holds a flag and a family whole, and not an emoji newer than it is.
        for emoji in ["🇹🇷", "👩\u{200d}💻", "❤\u{fe0f}", "1\u{fe0f}\u{20e3}"] {
            assert!(built_in_emoji_holds(emoji), "{emoji}");
        }
        assert!(
            !built_in_emoji_holds("🫨"),
            "a face from a year after the font"
        );
        assert!(!built_in_emoji_holds("ok"));
        // And through the rasterizer itself: a line of words with two in it.
        let metrics = fit_font(&mut fonts, 10, 20).unwrap();
        let mut r = SoftwareRasterizer {
            fonts,
            emoji: Some(BUILT_IN_EMOJI.to_string()),
            swash: SwashCache::new(),
            metrics,
            cell: (10, 20),
            type_row: 20,
            masks: HashMap::new(),
            painted: HashMap::new(),
            text: HashMap::new(),
        };
        let mask = r.text_mask("ok 🚀 ❤\u{fe0f}", Font::default());
        let own = mask
            .own
            .as_ref()
            .expect("the emoji have colours of their own");
        let red = own
            .iter()
            .flatten()
            .filter(|rgb| rgb.0 > 150 && rgb.2 < 120);
        assert!(red.count() > 10, "a heart and a rocket's flame are red");
    }

    #[test]
    fn fonts_ship_in_the_binary() {
        let mut r = SoftwareRasterizer::new(10, 20).unwrap();
        assert!(
            r.fonts
                .db()
                .faces()
                .any(|f| f.families.iter().any(|(n, _)| n.contains("Inter")))
        );
        assert!(
            r.text_mask("x", Font::default())
                .alpha
                .iter()
                .any(|a| *a > 0)
        );
    }

    #[test]
    fn unknown_cell_size_refuses_pixels() {
        assert!(SoftwareRasterizer::new(0, 20).is_none());
    }
}
