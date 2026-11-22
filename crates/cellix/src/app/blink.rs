//! The caret blinks, VS Code's way: shown half a second and away half a second, solid again
//! and starting over at any key, click, paste or move of the caret, and still while the
//! window is behind another. Files, pages, every text box and a terminal pane's cursor share
//! one phase; a program that asks for a steady cursor keeps one, and Settings turns it off.
//!
//! On the cell tier the terminal blinks its own cursor, told once (DECSCUSR), so Cellix wakes
//! for nothing. On the pixel tiers the loop takes the frame shown to the other half
//! (`Grid::blink_to`), whose row's two tiles the terminal holds: a blink is a placement. Inside
//! Claude Code's pane nothing blinks, as nothing turns there, since each half would be its
//! row's picture crossing again. See "The caret blinks" in docs/DESIGN.md.

use std::cell::Cell;
use std::hash::{Hash, Hasher};
use std::time::{Duration, Instant};

use cellix_input::{Event, MouseEvent, MouseKind};
use cellix_render::{Grid, TermCursor};

use super::App;

/// Each half of a blink: VS Code's editor's (`ViewCursors.BLINK_INTERVAL`).
pub const BLINK: Duration = Duration::from_millis(500);

/// Where the caret's blink stands.
pub(crate) struct Blink {
    /// When it last started over, in its shown half.
    since: Cell<Instant>,
    /// What the caret and any terminal's cursor were drawn at, so a move starts it over.
    place: Cell<u64>,
    /// Whether the terminal says Cellix's window is in front.
    focused: bool,
    /// Whether what shows this app keeps the blink's time: the terminal's loop does, unless
    /// `CELLIX_CURSOR_BLINK=0` holds it still for the harness. A snapshot is one picture.
    live: bool,
    /// The clock, for a test to set; unset, a test's caret never blinks.
    #[cfg(test)]
    pub(crate) now: Option<Instant>,
}

impl Default for Blink {
    fn default() -> Self {
        Self {
            since: Cell::new(Instant::now()),
            place: Cell::new(0),
            focused: true,
            live: false,
            #[cfg(test)]
            now: None,
        }
    }
}

/// Whether an event is the person moving on, which shows the caret and starts its blink over:
/// a key, a paste, a press or a drag, but not the pointer passing or the wheel.
fn moves_on(event: &Event) -> bool {
    match event {
        Event::Key(_) | Event::Paste(_) => true,
        Event::Mouse(MouseEvent { kind, .. }) => matches!(
            kind,
            MouseKind::Left | MouseKind::Middle | MouseKind::Right | MouseKind::Drag
        ),
        Event::Resize(..) | Event::Tick => false,
    }
}

impl App {
    /// Whatever shows this app keeps the blink's time from now on: the terminal's loop.
    pub fn keep_blink_time(&mut self) {
        self.blink.live = true;
    }

    /// The terminal said whether Cellix's window is in front. Behind another, the caret stays
    /// shown and nothing wakes for it; back in front, it's shown and starts over. A program in
    /// the side pane with the keys is told it lost them, and has them again, as VS Code's
    /// terminal tells one.
    pub fn set_window_focused(&mut self, focused: bool) {
        self.blink.focused = focused;
        if focused {
            self.restart_blink();
        }
        self.tell_keys();
    }

    /// Whether the terminal says Cellix's window is in front; without word from it, it is.
    pub(super) fn window_focused(&self) -> bool {
        self.blink.focused
    }

    /// The time the blink goes by, where it's kept.
    fn blink_clock(&self) -> Option<Instant> {
        #[cfg(test)]
        if let Some(now) = self.blink.now {
            return Some(now);
        }
        self.blink.live.then(Instant::now)
    }

    /// Whether the caret blinks now: Settings' Cursor blinking, the window in front, and not
    /// inside Claude Code's pane.
    fn caret_blinks(&self) -> bool {
        self.settings.cursor_blink != Some(false)
            && self.blink.focused
            && !self.in_pane
            && self.blink_clock().is_some()
    }

    /// Shows the caret and starts its blink over.
    pub(crate) fn restart_blink(&self) {
        if let Some(now) = self.blink_clock() {
            self.blink.since.set(now);
        }
    }

    /// What an event does to the blink, before it's handled.
    pub(super) fn blink_on(&self, event: &Event) {
        if moves_on(event) {
            self.restart_blink();
        }
    }

    /// Whether the blink is in its shown half now.
    pub fn blink_shown(&self) -> bool {
        let Some(now) = self.blink_clock() else {
            return true;
        };
        let into = now.saturating_duration_since(self.blink.since.get());
        (into.as_millis() / BLINK.as_millis()).is_multiple_of(2)
    }

    /// When the frame `grid` shows next comes to the other half of its blink: none where
    /// nothing on it blinks, on the cell tier, whose terminal blinks its own cursor, and
    /// while the caret doesn't blink.
    pub fn next_blink(&self, grid: &Grid) -> Option<Instant> {
        if !grid.blinks() || self.measure.in_cells() || !self.caret_blinks() {
            return None;
        }
        let now = self.blink_clock()?;
        let since = self.blink.since.get();
        let halves = now.saturating_duration_since(since).as_millis() / BLINK.as_millis();
        let next = u32::try_from(halves + 1).unwrap_or(u32::MAX);
        Some(since + BLINK * next)
    }

    /// After everything is drawn: a terminal pane's cursor placed as the tier shows it, the
    /// blink started over if the caret moved, and the frame in the half of its blink it's in.
    /// On cells the terminal's own cursor stands for a terminal pane's where no caret is
    /// shown, in its program's shape, so the terminal blinks it; on the pixel tiers its cell
    /// shows its own look in the half the cursor is away. A cursor something was drawn over
    /// since, as a tip or a menu, is left as that drew it.
    pub(super) fn finish_caret(&self, grid: &mut Grid) {
        let on = self.caret_blinks();
        let drawn = |c: &TermCursor, grid: &Grid| {
            c.at.0 < grid.width() && c.at.1 < grid.height() && *grid.cell(c.at.0, c.at.1) == c.on
        };
        let cursors: Vec<TermCursor> = std::mem::take(&mut grid.term_cursors)
            .into_iter()
            .filter(|c| drawn(c, grid))
            .collect();
        let caret = grid.cursor.is_some() || grid.caret.is_some();
        grid.cursor_blinks = on;
        if self.measure.in_cells() {
            if let Some(cursor) = cursors.last().filter(|_| !caret) {
                let (x, y) = cursor.at;
                grid.put_str(x, y, &cursor.off.symbol, cursor.off.style, x + 1);
                grid.cursor = Some(cursor.at);
                grid.cursor_shape = cursor.shape;
                grid.cursor_blinks = on && cursor.blinks;
            }
        } else {
            for cursor in cursors.iter().filter(|c| c.blinks) {
                grid.blink_cell(cursor.at.0, cursor.at.1, cursor.off.clone());
            }
        }
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        (grid.cursor, grid.caret.map(|b| (b.x, b.y, b.h))).hash(&mut hasher);
        for cursor in &cursors {
            cursor.at.hash(&mut hasher);
        }
        let place = hasher.finish();
        if place != self.blink.place.get() {
            self.blink.place.set(place);
            self.restart_blink();
        }
        if on && !self.measure.in_cells() && !self.blink_shown() {
            grid.blink_to(false);
        }
    }
}
