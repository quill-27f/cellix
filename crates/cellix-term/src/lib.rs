//! Terminal side of Cellix: choosing a rendering tier and presenting grids.

mod cells;
mod detect;
mod pixels;
mod probe;

pub use cells::CellBackend;
pub use detect::detect;
pub use pixels::{DEFAULT_QUOTA, PixelBackend, Placement};
pub use probe::{Env, Multiplexer, Tier, select_tier};

use std::io::{self, Write};

use cellix_render::Grid;

/// Presents a grid. Implementations send only what changed since the last call.
pub trait Backend {
    fn present(&mut self, grid: &Grid, out: &mut dyn Write) -> io::Result<()>;

    /// Forgets what the terminal shows, so the next present redraws everything.
    fn invalidate(&mut self);
}
