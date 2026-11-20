//! The cell grid every Cellix view draws to. Backends only present a grid, so every
//! feature works on the text-cell tier and the pixel tiers alike.

mod grid;
mod icon;
mod measure;
mod paint;
mod raster;
pub mod spinner;

pub use grid::{
    Cell, CursorShape, FIELD_AIR, GLYPH, GLYPH_LEAD, Grid, Rect, Rgb, Run, SMALL_GLYPH, Style,
    TermCursor, WORDS_BASELINE, air_under_button, button_in_row, on_large_button,
    row_button_in_row,
};
pub use icon::{Icon, Piece, inked};
pub use measure::{CellMeasure, EMOJI_AIR, Font, FontMeasure, Measure};
pub use paint::{
    ALL_CORNERS, BOTTOM_LEFT, BOTTOM_RIGHT, Bitmap, Bounds, Paint, Pixels, Strip, TOP_LEFT,
    TOP_RIGHT, UNIT, units,
};
pub use raster::{
    INTERFACE_SIZE, Image, Rasterizer, SoftwareRasterizer, built_in_emoji_holds, inset, stood,
};
