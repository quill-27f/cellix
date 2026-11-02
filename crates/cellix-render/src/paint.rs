//! What the pixel tiers draw that a cell can't hold: proportional text, rounded panels,
//! hairlines, shadows and small icons. The grid stays the source of truth for layout and
//! hit-testing, and the cell tier ignores all of this.

use std::hash::{Hash, Hasher};
use std::sync::Arc;

use crate::grid::{Rect, Rgb};
use crate::icon::Icon;
use crate::measure::Font;

/// Paint positions are in 64ths of a cell, so they can sit between cells and still hash.
pub const UNIT: i32 = 64;

/// Converts cells to paint units.
pub fn units(cells: f32) -> i32 {
    (cells * UNIT as f32).round() as i32
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Bounds {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Bounds {
    pub fn cells(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self {
            x: units(x),
            y: units(y),
            w: units(w),
            h: units(h),
        }
    }

    /// A rect shrunk by `x` columns and `y` rows on each side.
    pub fn inset(rect: Rect, x: f32, y: f32) -> Self {
        Self::cells(
            rect.x as f32 + x,
            rect.y as f32 + y,
            rect.w as f32 - x * 2.0,
            rect.h as f32 - y * 2.0,
        )
    }
}

/// One bar in a minimap, in 256ths of its box.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Strip {
    pub x: u8,
    pub y: u8,
    pub w: u8,
    pub color: Rgb,
}

pub const TOP_LEFT: u8 = 1;
pub const TOP_RIGHT: u8 = 2;
pub const BOTTOM_RIGHT: u8 = 4;
pub const BOTTOM_LEFT: u8 = 8;
pub const ALL_CORNERS: u8 = 15;

/// What a bitmap's bytes are.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Pixels {
    /// One byte a pixel of ink, drawn in a colour: a formula takes the text's.
    Coverage,
    /// Four bytes a pixel: a picture or a page of a PDF.
    Rgba,
}

/// Pixels made elsewhere. `id` stands for the content, so a tile hashes it and not every byte.
#[derive(Clone, Debug)]
pub struct Bitmap {
    pub id: u64,
    pub width: u32,
    pub height: u32,
    pub kind: Pixels,
    pub pixels: Arc<Vec<u8>>,
}

impl PartialEq for Bitmap {
    fn eq(&self, other: &Self) -> bool {
        (self.id, self.width, self.height) == (other.id, other.width, other.height)
    }
}

impl Eq for Bitmap {}

impl Hash for Bitmap {
    fn hash<H: Hasher>(&self, state: &mut H) {
        (self.id, self.width, self.height).hash(state);
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Paint {
    Panel {
        bounds: Bounds,
        /// As a percentage of a row's height.
        radius: u8,
        fill: Option<Rgb>,
        border: Option<Rgb>,
        shadow: bool,
    },
    /// A line one device pixel thick, along the top or left of `bounds`.
    Hairline { bounds: Bounds, color: Rgb },
    Text {
        x: i32,
        /// The row it's centred on, in paint units.
        y: i32,
        max_width: i32,
        text: String,
        color: Rgb,
        font: Font,
        /// `x` is the right edge rather than the left.
        align_right: bool,
        /// `x` is the middle of the text. Only the rasterizer knows a text's true width.
        centred: bool,
        /// Drawn under the text at its true width.
        underline: bool,
    },
    Icon {
        bounds: Bounds,
        icon: Icon,
        color: Rgb,
    },
    /// Fills the listed corners of `bounds` back in. Cells can't be rounded and paint
    /// can't go under text, so a rounded surface is cells with its corners painted out.
    Corners {
        bounds: Bounds,
        /// As a percentage of a row's height.
        radius: u8,
        color: Rgb,
        which: u8,
    },
    /// A row's share of a minimap: many small bars, which as panels would be slow.
    Strips { bounds: Bounds, strips: Vec<Strip> },
    /// A bitmap scaled to `bounds`. Coverage is inked in `color`.
    Image {
        bounds: Bounds,
        /// What it stays inside, since a page scrolls past its pictures.
        clip: Bounds,
        bitmap: Bitmap,
        color: Rgb,
    },
}

impl Paint {
    /// Paint units of the rows this touches, as (top, bottom).
    pub(crate) fn span(&self) -> (i32, i32) {
        match self {
            Self::Panel { bounds, shadow, .. } => {
                let reach = if *shadow { UNIT } else { 0 };
                (bounds.y - reach, bounds.y + bounds.h + reach)
            }
            Self::Hairline { bounds, .. }
            | Self::Icon { bounds, .. }
            | Self::Strips { bounds, .. }
            | Self::Corners { bounds, .. } => (bounds.y, bounds.y + bounds.h.max(1)),
            Self::Image { bounds, clip, .. } => (
                bounds.y.max(clip.y),
                (bounds.y + bounds.h.max(1)).min(clip.y + clip.h),
            ),
            // Large text stands taller than its row.
            Self::Text { y, font, .. } if font.size > 100 => (*y - UNIT, *y + 2 * UNIT),
            Self::Text { y, .. } => (*y, *y + UNIT),
        }
    }
}
