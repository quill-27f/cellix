//! The cell grid every Cellix view draws to. Backends only present a grid, so every
//! feature works on the text-cell tier and the pixel tiers alike.

mod grid;
mod icon;
mod measure;
mod paint;
mod raster;

pub use grid::{Cell, Grid, Rect, Rgb, Run, Style, on_large_button};
pub use icon::{Icon, Piece};
pub use measure::{CellMeasure, Font, FontMeasure, Measure};
pub use paint::{
    ALL_CORNERS, BOTTOM_LEFT, BOTTOM_RIGHT, Bitmap, Bounds, Paint, Pixels, Strip, TOP_LEFT,
    TOP_RIGHT, UNIT, units,
};
pub use raster::{INTERFACE_SIZE, Image, Rasterizer, SoftwareRasterizer};
