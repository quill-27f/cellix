//! The cell grid every Cellix view draws to. Backends only present a grid, so every
//! feature works on the text-cell tier and the pixel tiers alike.

mod grid;
mod icon;
mod measure;
mod paint;
mod raster;

pub use grid::{Cell, Grid, Rect, Rgb, Run, Style};
pub use icon::{Icon, Piece};
pub use measure::{CellMeasure, Font, FontMeasure, Measure};
pub use paint::{Bounds, Paint, units};
pub use raster::{Image, Rasterizer, SoftwareRasterizer};
