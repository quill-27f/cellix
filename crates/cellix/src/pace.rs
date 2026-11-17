//! When the main loop draws on its own, without a key or an answer asking: the half-second
//! tick's decisions, kept apart from the terminal so a test can run them on a clock of its
//! own.

use std::time::{Duration, Instant};

/// Background file checks run about twice a second.
pub const TICK: Duration = Duration::from_millis(500);
/// Ticks that may pass with nothing seen to change before a frame is laid out all the same:
/// what tells the time ("2 min ago") moves on with no event, and the marks a tick compares
/// can't name everything, such as what a look at the disk or git leaves for the next frame.
/// Every two seconds, as often as the disk is looked at; a frame laid out with nothing
/// different sends nothing.
pub const HEARTBEAT: u32 = 4;

/// The tick, and what it last saw on screen.
pub struct Pace {
    last_tick: Instant,
    /// Ticks since a frame was last laid out.
    quiet: u32,
    /// What the screen's marks were when it was last drawn.
    drawn: Option<u64>,
}

impl Pace {
    pub fn new(now: Instant) -> Self {
        Self {
            last_tick: now,
            quiet: 0,
            drawn: None,
        }
    }

    /// Whether the tick is due.
    pub fn tick_due(&self, now: Instant) -> bool {
        now.duration_since(self.last_tick) >= TICK
    }

    /// How long until it's due.
    pub fn until_tick(&self, now: Instant) -> Duration {
        TICK.saturating_sub(now.duration_since(self.last_tick))
    }

    /// The tick ran, and the screen's marks are now `mark`. Says whether to draw: only if
    /// they differ from what was drawn, or the heartbeat is due.
    pub fn ticked(&mut self, now: Instant, mark: u64) -> bool {
        self.last_tick = now;
        self.quiet += 1;
        self.drawn != Some(mark) || self.quiet >= HEARTBEAT
    }

    /// A frame was drawn, showing what has these marks.
    pub fn drew(&mut self, mark: u64) {
        self.quiet = 0;
        self.drawn = Some(mark);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Left alone, the tick draws nothing but the heartbeat, every two seconds; a change in
    /// what it marks draws at the next tick.
    #[test]
    fn an_idle_tick_draws_only_its_heartbeat() {
        let start = Instant::now();
        let mut pace = Pace::new(start);
        pace.drew(7);
        let mut drawn = Vec::new();
        for step in 1..=20u32 {
            let now = start + TICK * step;
            assert!(pace.tick_due(now));
            if pace.ticked(now, 7) {
                drawn.push(step);
                pace.drew(7);
            }
        }
        assert_eq!(drawn, [4, 8, 12, 16, 20], "ten seconds: five layouts");
        let now = start + TICK * 21;
        assert!(pace.ticked(now, 8), "a changed mark draws");
        pace.drew(8);
        assert!(!pace.ticked(start + TICK * 22, 8));
        assert!(!pace.tick_due(start + TICK * 22 + TICK / 2 - Duration::from_millis(1)));
    }
}
