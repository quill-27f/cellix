//! Terminal side of Cellix: choosing a rendering tier and presenting grids.

mod cells;
mod detect;
mod pixels;
mod probe;

pub use cells::CellBackend;
pub use detect::detect;
pub use pixels::{Bleed, DEFAULT_QUOTA, PixelBackend, Placement, Smooth, row_key};
pub use probe::{Env, Multiplexer, Tier, select_tier};

use std::io::{self, Write};

use cellix_render::{Grid, Rect};

/// Presents a grid. Implementations send only what changed since the last call.
pub trait Backend {
    fn present(&mut self, grid: &Grid, out: &mut dyn Write) -> io::Result<()>;

    /// Animates `area` from one grid to the other, `rows` lines further down the content,
    /// or up if negative. The present that follows settles it. Tiers that can't move
    /// anything by less than a cell do nothing, and the page just jumps.
    fn scroll(
        &mut self,
        _from: &Grid,
        _to: &Grid,
        _area: Rect,
        _rows: i32,
        _out: &mut dyn Write,
    ) -> io::Result<()> {
        Ok(())
    }

    /// Forgets what the terminal shows, so the next present redraws everything.
    fn invalidate(&mut self);
}
