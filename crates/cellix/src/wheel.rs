//! How far one wheel event moves the page. Claude Code's fullscreen view and vim feel right
//! because each event moves the text at once, so this keeps to that. A terminal sends a
//! trackpad's travel as a flood of events, each already worth a line. Slower events are a
//! wheel's notches, which ramp up while they keep coming and fade when they stop.

use std::time::{Duration, Instant};

/// Closer together than this, events are one gesture arriving in a batch.
const FLOOD: Duration = Duration::from_millis(6);
/// Further apart than this, it's a new gesture.
const NEW_GESTURE: Duration = Duration::from_millis(250);
/// How quickly built-up speed fades.
const HALF_LIFE: Duration = Duration::from_millis(150);
/// Lines an event adds to the speed, and the fastest it gets.
const RAMP: f32 = 0.7;
const FASTEST: f32 = 6.0;

#[derive(Debug, Default)]
pub struct Wheel {
    last: Option<(Instant, bool)>,
    speed: f32,
    /// The part of a line carried over to the next event.
    carried: f32,
}

impl Wheel {
    /// Lines to move for an event at `now`.
    pub fn lines(&mut self, down: bool, now: Instant) -> usize {
        let last = self.last.replace((now, down));
        let gap = last
            .filter(|(_, was_down)| *was_down == down)
            .map(|(at, _)| now.duration_since(at))
            .filter(|gap| *gap <= NEW_GESTURE);
        let Some(gap) = gap else {
            self.speed = 1.0;
            self.carried = 0.0;
            return 1;
        };
        if gap < FLOOD {
            return 1;
        }
        let fade = 0.5_f32.powf(gap.as_secs_f32() / HALF_LIFE.as_secs_f32());
        self.speed = (1.0 + (self.speed - 1.0 + RAMP) * fade).min(FASTEST);
        let lines = self.speed + self.carried;
        self.carried = lines.fract();
        lines as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The lines moved by events spaced `gap` apart.
    fn run(wheel: &mut Wheel, gap: Duration, count: u32) -> Vec<usize> {
        let start = Instant::now();
        (0..count)
            .map(|i| wheel.lines(true, start + gap * i))
            .collect()
    }

    #[test]
    fn a_trackpads_flood_moves_a_line_an_event() {
        let lines = run(&mut Wheel::default(), Duration::from_millis(2), 30);
        assert!(lines.iter().all(|line| *line == 1), "{lines:?}");
    }

    #[test]
    fn the_first_event_never_jumps() {
        assert_eq!(Wheel::default().lines(true, Instant::now()), 1);
    }

    #[test]
    fn a_wheel_spun_quickly_gathers_speed() {
        let lines = run(&mut Wheel::default(), Duration::from_millis(30), 40);
        assert!(lines[lines.len() - 1] > 1, "{lines:?}");
        assert!(
            lines.iter().all(|line| *line <= FASTEST as usize + 1),
            "{lines:?}"
        );
    }

    #[test]
    fn a_slow_wheel_stays_slow() {
        let lines = run(&mut Wheel::default(), Duration::from_millis(300), 8);
        assert!(lines.iter().all(|line| *line == 1), "{lines:?}");
    }

    #[test]
    fn turning_round_starts_again() {
        let mut wheel = Wheel::default();
        let start = Instant::now();
        let gap = Duration::from_millis(30);
        for i in 0..30 {
            wheel.lines(true, start + gap * i);
        }
        assert_eq!(wheel.lines(false, start + gap * 30), 1);
    }
}
