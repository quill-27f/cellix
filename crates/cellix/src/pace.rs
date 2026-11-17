//! When the main loop draws on its own, without a key or an answer asking: the half-second
//! tick's decisions, kept apart from the terminal so a test can run them on a clock of its
//! own.

use std::time::{Duration, Instant};

/// Background file checks run about twice a second.
pub const TICK: Duration = Duration::from_millis(500);
/// Output on screen is drawn at most this often: a program printing without pause is shown
/// sixty times a second, not as often as it writes.
pub const OUTPUT_FRAME: Duration = Duration::from_millis(16);
/// For this long after a key, what a program prints is taken for its echo, and drawn at once
/// however soon after the last frame.
pub const ECHO: Duration = Duration::from_millis(32);
/// The most frames drawn at once in that time, so a key that sets a flood going doesn't
/// draw the flood frame by frame.
pub const ECHO_FRAMES: u32 = 16;

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
    /// Something out of sight changed since: a session behind another printed, or another
    /// workspace changed. What's drawn from it, a tab's mark or a workspace's tile, waits
    /// for the tick.
    behind: bool,
    /// When a frame was last drawn.
    last_frame: Option<Instant>,
    /// When a key last came, and how many frames its echo has drawn.
    typed: Option<Instant>,
    echoes: u32,
    /// Output on screen waits for its frame.
    waiting: bool,
}

impl Pace {
    pub fn new(now: Instant) -> Self {
        Self {
            last_tick: now,
            quiet: 0,
            drawn: None,
            behind: false,
            last_frame: None,
            typed: None,
            echoes: 0,
            waiting: false,
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
    /// they differ from what was drawn, something out of sight changed, or the heartbeat is
    /// due.
    pub fn ticked(&mut self, now: Instant, mark: u64) -> bool {
        self.last_tick = now;
        self.quiet += 1;
        self.drawn != Some(mark) || self.behind || self.quiet >= HEARTBEAT
    }

    /// Something out of sight changed, for the tick to draw.
    pub fn out_of_sight(&mut self) {
        self.behind = true;
    }

    /// Whether something out of sight waits for the tick.
    pub fn behind(&self) -> bool {
        self.behind
    }

    /// A frame was drawn, showing what has these marks.
    pub fn drew(&mut self, mark: u64) {
        self.quiet = 0;
        self.drawn = Some(mark);
        self.behind = false;
    }

    /// A frame was drawn now: output waiting is in it.
    pub fn framed(&mut self, now: Instant) {
        self.last_frame = Some(now);
        self.waiting = false;
    }

    /// A key came, or a paste or a click: what's printed next is its echo.
    pub fn typed(&mut self, now: Instant) {
        self.typed = Some(now);
        self.echoes = 0;
    }

    /// A session on screen printed. Says whether to draw it now: as an echo, or because
    /// the last frame is a frame's time ago. Otherwise it waits for `output_due`.
    pub fn printed(&mut self, now: Instant) -> bool {
        let echo = self.typed.is_some_and(|at| now.duration_since(at) <= ECHO)
            && self.echoes < ECHO_FRAMES;
        if echo {
            self.echoes += 1;
            return true;
        }
        if self.spacing(now).is_zero() {
            return true;
        }
        self.waiting = true;
        false
    }

    /// How long until a frame may be drawn for output: nothing when one may be now.
    fn spacing(&self, now: Instant) -> Duration {
        self.last_frame.map_or(Duration::ZERO, |at| {
            OUTPUT_FRAME.saturating_sub(now.duration_since(at))
        })
    }

    /// How long until output waiting for its frame is drawn, if some is waiting.
    pub fn until_output(&self, now: Instant) -> Option<Duration> {
        self.waiting.then(|| self.spacing(now))
    }

    /// Whether output waiting for its frame is due now.
    pub fn output_due(&self, now: Instant) -> bool {
        self.until_output(now).is_some_and(|left| left.is_zero())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A program printing without pause draws a frame at most every 16 ms, the next as soon
    /// as that's up; a key's echo draws at once, however soon after the last frame.
    #[test]
    fn output_draws_once_a_frame_but_an_echo_at_once() {
        let start = Instant::now();
        let ms = |n: u64| start + Duration::from_millis(n);
        let mut pace = Pace::new(start);
        assert!(pace.printed(ms(0)), "the first output after a pause draws");
        pace.framed(ms(0));
        // More every 4 ms: it waits for the frame's time.
        assert!(!pace.printed(ms(4)));
        assert_eq!(pace.until_output(ms(4)), Some(Duration::from_millis(12)));
        assert!(!pace.printed(ms(8)));
        assert!(!pace.output_due(ms(12)));
        assert!(pace.output_due(ms(16)), "drawn when the frame's time comes");
        pace.framed(ms(16));
        assert_eq!(pace.until_output(ms(17)), None, "nothing waits");
        // A key, then its echo 3 ms after the key's own frame: drawn at once.
        pace.typed(ms(20));
        pace.framed(ms(21));
        assert!(pace.printed(ms(24)), "the echo");
        pace.framed(ms(24));
        // Past the echo's time, the frame's time holds again.
        pace.framed(ms(53));
        assert!(!pace.printed(ms(56)));
        // At most sixteen echo frames for one key.
        let mut pace = Pace::new(start);
        pace.typed(ms(0));
        let drawn = (0..30)
            .filter(|n| {
                let now = ms(n / 2);
                let draw = pace.printed(now);
                if draw {
                    pace.framed(now);
                }
                draw
            })
            .count();
        assert_eq!(drawn, ECHO_FRAMES as usize);
    }

    /// Output out of sight draws once at the next tick however much of it there is, and
    /// nothing after that until there's more.
    #[test]
    fn output_out_of_sight_draws_once_a_tick() {
        let start = Instant::now();
        let mut pace = Pace::new(start);
        pace.drew(1);
        let mut drawn = 0;
        for step in 1..=8u32 {
            // Four tabs at 60 lines a second each: a hundred and twenty a tick.
            for _ in 0..120 {
                pace.out_of_sight();
            }
            if pace.ticked(start + TICK * step, 1) {
                drawn += 1;
                pace.drew(1);
            }
        }
        assert_eq!(drawn, 8, "one frame a tick");
        assert!(
            !pace.ticked(start + TICK * 9, 1),
            "and none with nothing new"
        );
    }

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
