use std::collections::HashMap;
use std::rc::Rc;

use compact_str::CompactString;
use cosmic_text::{
    Attrs, Buffer, Color, Family, FontSystem, Metrics, Shaping, Style as FontStyle, SwashCache,
    Weight,
};

use crate::grid::{Cell, Rgb};

/// A tightly packed RGB image, matching the Kitty protocol's `f=24`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Image {
    pub width: usize,
    pub height: usize,
    pub rgb: Vec<u8>,
}

/// Turns one row of cells into an image. `cursor` marks the column to draw the cursor at.
pub trait Rasterizer {
    fn row(&mut self, cells: &[Cell], cursor: Option<u16>) -> Image;
}

#[derive(Clone, Hash, PartialEq, Eq)]
struct GlyphKey {
    symbol: CompactString,
    bold: bool,
    italic: bool,
}

/// Coverage of one glyph: pixel offsets from the cell's corner, with alpha.
type Mask = Rc<Vec<(u16, u16, u8)>>;

/// CPU text rendering. Each glyph is rasterized once and then blended into its cell, so
/// text stays locked to the grid and a row costs little more than a copy.
pub struct SoftwareRasterizer {
    fonts: FontSystem,
    swash: SwashCache,
    metrics: Metrics,
    cell: (usize, usize),
    masks: HashMap<GlyphKey, Mask>,
}

impl SoftwareRasterizer {
    /// Returns None when the host has no fonts, as many servers don't, or when the cell
    /// size is unknown. The caller then stays on the cell tier.
    pub fn new(cell_width: usize, cell_height: usize) -> Option<Self> {
        if cell_width == 0 || cell_height == 0 {
            return None;
        }
        let mut fonts = FontSystem::new();
        if fonts.db().is_empty() {
            return None;
        }
        let metrics = fit_font(&mut fonts, cell_width, cell_height)?;
        Some(Self {
            fonts,
            swash: SwashCache::new(),
            metrics,
            cell: (cell_width, cell_height),
            masks: HashMap::new(),
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
                        }
                    }
                }
            },
        );
        let mask = Rc::new(pixels);
        self.masks.insert(key.clone(), mask.clone());
        mask
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
    fn row(&mut self, cells: &[Cell], cursor: Option<u16>) -> Image {
        let (cw, ch) = self.cell;
        let width = cw * cells.len();
        let mut rgb = vec![0; width * ch * 3];
        for (i, cell) in cells.iter().enumerate() {
            let is_cursor = cursor == Some(i as u16);
            let (fg, bg) = if is_cursor {
                (cell.style.bg, cell.style.fg)
            } else {
                (cell.style.fg, cell.style.bg)
            };
            fill(&mut rgb, width, i * cw, 0, cw, ch, bg);
            if cell.symbol.is_empty() || cell.symbol == " " {
                continue;
            }
            let mask = self.mask(&GlyphKey {
                symbol: cell.symbol.clone(),
                bold: cell.style.bold,
                italic: cell.style.italic,
            });
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
            if cell.style.underline {
                fill(&mut rgb, width, i * cw, ch - 2, cw, 1, cell.style.fg);
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

    /// None on a host without fonts, where this test has nothing to check.
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
        let image = r.row(grid.row(0), None);
        assert_eq!((image.width, image.height), (80, 20));
        assert_eq!(image.rgb.len(), 80 * 20 * 3);
    }

    #[test]
    fn glyphs_land_in_their_own_cells() {
        let Some(mut r) = rasterizer() else { return };
        let mut grid = Grid::new(4, 1, style());
        grid.put_str(2, 0, "M", style(), 4);
        let image = r.row(grid.row(0), None);
        assert_eq!(lit(&image, 0, 10), 0);
        assert!(lit(&image, 2, 10) > 10, "M drew nothing");
        assert_eq!(lit(&image, 3, 10), 0, "M spilled into the next cell");
    }

    #[test]
    fn cursor_inverts_its_cell() {
        let Some(mut r) = rasterizer() else { return };
        let grid = Grid::new(3, 1, style());
        let image = r.row(grid.row(0), Some(1));
        let at = |x: usize| image.rgb[(10 * image.width + x) * 3];
        assert_eq!(at(5), 10);
        assert_eq!(at(15), 250);
        assert_eq!(at(25), 10);
    }

    #[test]
    fn unknown_cell_size_refuses_pixels() {
        assert!(SoftwareRasterizer::new(0, 20).is_none());
    }
}
