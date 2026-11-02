use std::collections::HashMap;
use std::rc::Rc;

use compact_str::CompactString;
use cosmic_text::{
    Attrs, Buffer, Color, Fallback, Family, FontSystem, Metrics, PlatformFallback, Shaping,
    Style as FontStyle, SwashCache, Weight, fontdb,
};
use unicode_script::Script;

use crate::grid::{Grid, Rgb};
use crate::icon::{Icon, Piece, inked};
use crate::measure::{Font, attrs};
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

/// How dark a floating panel's shadow is at its edge, out of 255.
const DARK_SHADOW: f32 = 110.0;
const LIGHT_SHADOW: f32 = 38.0;
/// Interface text is drawn at this size relative to the row height.
pub const INTERFACE_SIZE: f32 = 0.60;
/// Cached text lines before the cache is dropped, so a long session can't grow it forever.
const MAX_TEXT_MASKS: usize = 4096;

pub(crate) const INTER: &[u8] = include_bytes!("../fonts/InterVariable.ttf");
pub(crate) const INTER_ITALIC: &[u8] = include_bytes!("../fonts/InterVariable-Italic.ttf");
pub(crate) const MONO: &[u8] = include_bytes!("../fonts/JetBrainsMonoVariable.ttf");
pub(crate) const MONO_ITALIC: &[u8] = include_bytes!("../fonts/JetBrainsMonoVariable-Italic.ttf");

/// The pixel size painted text is drawn at, for a row height and font.
pub(crate) fn text_pixels(row_height: usize, font: Font) -> f32 {
    row_height as f32 * INTERFACE_SIZE * font.size as f32 / 100.0
}

/// CPU rendering. Glyphs are rasterized once and blended into their cells, so text stays
/// locked to the grid and a row costs little more than a copy.
pub struct SoftwareRasterizer {
    fonts: FontSystem,
    swash: SwashCache,
    metrics: Metrics,
    cell: (usize, usize),
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
        db.set_monospace_family("JetBrains Mono");
        db.set_sans_serif_family("Inter Variable");
        let metrics = fit_font(&mut fonts, cell_width, cell_height)?;
        Some(Self {
            fonts,
            swash: SwashCache::new(),
            metrics,
            cell: (cell_width, cell_height),
            masks: HashMap::new(),
            painted: HashMap::new(),
            text: HashMap::new(),
        })
    }

    fn mask(&mut self, key: &GlyphKey) -> Mask {
        if let Some(mask) = self.masks.get(key) {
            return mask.clone();
        }
        let (width, height) = (self.cell.0 * 2, self.cell.1);
        let mut buffer = Buffer::new(&mut self.fonts, self.metrics);
        buffer.set_size(Some(width as f32 * 2.0), Some(height as f32));
        let mut attrs = Attrs::new().family(Family::Monospace);
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
        let px = text_pixels(self.cell.1, font);
        let height = self.cell.1.max((px * 1.45).ceil() as usize);
        let mut buffer = Buffer::new(&mut self.fonts, Metrics::new(px, height as f32));
        buffer.set_size(None, Some(height as f32));
        let style = attrs(font);
        let rich = std::iter::once((text, style.clone()));
        buffer.set_rich_text(rich, &style, Shaping::Advanced, None);
        buffer.shape_until_scroll(&mut self.fonts, false);
        let width = buffer
            .layout_runs()
            .next()
            .map_or(0.0, |run| run.line_w)
            .ceil() as usize
            + 2;
        let mut alpha = vec![0; width * height];
        let white = Color::rgb(255, 255, 255);
        buffer.draw(
            &mut self.fonts,
            &mut self.swash,
            white,
            |x, y, w, h, color| {
                for py in y..y + h as i32 {
                    for px in x..x + w as i32 {
                        if px >= 0 && py >= 0 && (px as usize) < width && (py as usize) < height {
                            let at = py as usize * width + px as usize;
                            alpha[at] = alpha[at].max(color.a());
                        }
                    }
                }
            },
        );
        let mask = Rc::new(TextMask {
            width,
            height,
            alpha,
        });
        self.text.insert(key, mask.clone());
        mask
    }

    fn draw_paint(&mut self, target: &mut Target, paint: &Paint) {
        let (cw, ch) = (self.cell.0 as f32, self.cell.1 as f32);
        let scale = |v: i32, cell: f32| v as f32 * cell / UNIT as f32;
        match paint {
            Paint::Panel {
                bounds,
                radius,
                fill,
                border,
                shadow,
            } => {
                let shape = Shape {
                    x: scale(bounds.x, cw),
                    y: scale(bounds.y, ch),
                    w: scale(bounds.w, cw),
                    h: scale(bounds.h, ch),
                    radius: *radius as f32 * ch / 100.0,
                };
                target.panel(shape, *fill, *border, *shadow, ch);
            }
            Paint::Hairline { bounds, color } => {
                let thickness = (ch / 24.0).ceil().max(1.0);
                let shape = Shape {
                    x: scale(bounds.x, cw),
                    y: scale(bounds.y, ch),
                    w: scale(bounds.w, cw).max(thickness),
                    h: scale(bounds.h, ch).max(thickness),
                    radius: 0.0,
                };
                target.panel(shape, Some(*color), None, false, ch);
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
                    let thickness = (ch / 24.0).ceil().max(1.0);
                    let shape = Shape {
                        x: left as f32,
                        y: scale(*y, ch) + ch * 0.78,
                        w: mask.width.saturating_sub(2).min(limit) as f32,
                        h: thickness,
                        radius: 0.0,
                    };
                    target.panel(shape, Some(*color), None, false, ch);
                }
            }
            Paint::Icon {
                bounds,
                icon,
                color,
            } => {
                let side = scale(bounds.h, ch);
                let left = scale(bounds.x, cw);
                let top = scale(bounds.y, ch);
                target.icon(*icon, left, top, side, *color);
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
                    radius: *radius as f32 * ch / 100.0,
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
                let thickness = (ch / 24.0).ceil().max(1.0);
                for strip in strips {
                    let shape = Shape {
                        x: left + strip.x as f32 * width / 256.0,
                        y: top + strip.y as f32 * height / 256.0,
                        w: (strip.w as f32 * width / 256.0).max(1.0),
                        h: thickness,
                        radius: 0.0,
                    };
                    target.panel(shape, Some(strip.color), None, false, ch);
                }
            }
        }
    }
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

    fn panel(&mut self, s: Shape, fill: Option<Rgb>, border: Option<Rgb>, shadow: bool, row: f32) {
        let reach = if shadow { row * 0.6 } else { 1.0 };
        let (x0, x1) = (
            (s.x - reach).floor() as i32,
            (s.x + s.w + reach).ceil() as i32,
        );
        let y0 = ((s.y - reach).floor() as i32).max(self.top);
        let y1 = ((s.y + s.h + reach).ceil() as i32).min(self.top + self.height as i32);
        let stroke = (row / 30.0).max(1.0);
        for y in y0..y1 {
            for x in x0..x1 {
                let d = rounded_distance(&s, x as f32 + 0.5, y as f32 + 0.5);
                if shadow && d > 0.0 {
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
                if let Some(color) = border {
                    let edge = ((0.5 - (d + stroke / 2.0).abs() + stroke / 2.0) * 255.0)
                        .clamp(0.0, 255.0) as u8;
                    self.blend(x, y, color, edge);
                }
            }
        }
    }

    fn blit(&mut self, mask: &TextMask, left: i32, top: i32, limit: usize, color: Rgb) {
        let from = (self.top - top).max(0) as usize;
        let to = ((self.top + self.height as i32 - top).max(0) as usize).min(mask.height);
        for y in from..to {
            for x in 0..mask.width.min(limit) {
                let alpha = mask.alpha[y * mask.width + x];
                self.blend(left + x as i32, top + y as i32, color, alpha);
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
                let shape = Shape {
                    x: bounds.x as f32 * cw as f32 / UNIT as f32,
                    y: bounds.y as f32 * ch as f32 / UNIT as f32,
                    w: (cw as f32 / 9.0).max(2.0),
                    h: bounds.h as f32 * ch as f32 / UNIT as f32,
                    radius: 0.0,
                };
                let mut target = Target {
                    rgb: &mut rgb,
                    width,
                    height: ch,
                    top: y as i32 * ch as i32,
                };
                target.panel(shape, Some(caret_color(grid)), None, false, ch as f32);
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
    use crate::grid::{Grid, Style};

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
