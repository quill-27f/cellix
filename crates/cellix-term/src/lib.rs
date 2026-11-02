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
use std::time::{Duration, Instant};

use cellix_render::{Grid, Rect};

/// Presents a grid. Implementations send only what changed since the last call.
pub trait Backend {
    fn present(&mut self, grid: &Grid, out: &mut dyn Write) -> io::Result<()>;

    /// Starts moving `area` from one grid to the other, `rows` lines further down the content,
    /// or up if negative, and carries on from where it is if it's already moving. The moving
    /// happens in `glide`. Tiers that can't move
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

    /// How often a scroll under way wants to move, if there is one.
    fn gliding(&self) -> Option<Duration> {
        None
    }

    /// Moves a scroll under way one step on.
    fn glide(&mut self, _now: Instant, _out: &mut dyn Write) -> io::Result<()> {
        Ok(())
    }

    /// Ends a scroll under way, for the next present to show the page as it is.
    fn settle(&mut self) {}

    /// Forgets what the terminal shows, so the next present redraws everything.
    fn invalidate(&mut self);
}
