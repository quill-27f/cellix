//! What the pixel tiers draw that a cell can't hold: proportional text, rounded panels,
//! hairlines, shadows and small icons. The grid stays the source of truth for layout and
//! hit-testing, and the cell tier ignores all of this.

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
    },
    Icon {
        bounds: Bounds,
        icon: Icon,
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
            Self::Hairline { bounds, .. } | Self::Icon { bounds, .. } => {
                (bounds.y, bounds.y + bounds.h.max(1))
            }
            // Large text stands taller than its row.
            Self::Text { y, font, .. } if font.size > 100 => (*y - UNIT, *y + 2 * UNIT),
            Self::Text { y, .. } => (*y, *y + UNIT),
        }
    }
}
