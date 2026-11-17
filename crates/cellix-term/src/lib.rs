//! Terminal side of Cellix: choosing a rendering tier and presenting grids.

mod cells;
pub mod clipboard;
mod detect;
mod notify;
mod pixels;
mod probe;

pub use cells::CellBackend;
pub use detect::{background, cell_size, detect};
pub use notify::{Notice, sequence as notification, terminal_names};
pub use pixels::{Bleed, DEFAULT_QUOTA, OwnRows, PixelBackend, Placement, Smooth, row_key};
pub use probe::{Env, Multiplexer, Tier, select_tier};

use std::io::{self, Write};
use std::time::{Duration, Instant};

use cellix_render::{Grid, Rect};

/// What a backend did since it was last asked, for the counters `CELLIX_PROF` keeps.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Work {
    /// Tiles sent to the terminal.
    pub tiles: u64,
    /// Rows rasterized.
    pub rows: u64,
}

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

    /// How many lines the terminal has. Only a backend whose rows aren't the terminal's
    /// lines needs telling: it fills what's left under its last row.
    fn set_lines(&mut self, _lines: u16) {}

    /// How far below its own place each of the last `foot` rows of `rows` is drawn, in
    /// pixels, the last of them last. Only a backend that stands its foot's rows apart in
    /// what's under the sheets says anything: a press there is then on the row drawn where
    /// it lands, which isn't the row that begins there.
    fn foot_drops(&self, _rows: u16, _foot: u16) -> Vec<usize> {
        Vec::new()
    }

    /// What it has done since it was last asked: tiles sent and rows rasterized.
    fn take_work(&mut self) -> Work {
        Work::default()
    }
}
