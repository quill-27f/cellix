//! The cell grid every Cellix view draws to. Backends only present a grid, so every
//! feature works on the text-cell tier and the pixel tiers alike.

mod grid;
mod raster;

pub use grid::{Cell, Grid, Rect, Rgb, Run, Style};
pub use raster::{Image, Rasterizer, SoftwareRasterizer};
