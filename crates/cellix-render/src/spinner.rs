//! The one mark that says something is under way: a faint ring with a quarter of it in full
//! colour, turning, where there are pictures, and turning braille on cells. Anything places it
//! on the grid (`Grid::spinner`), in the colour of what it means. Every spinner on a grid
//! stands at the same step, which the grid is given (`Grid::turn`), so one clock turns them
//! all and a turn is a cycle of ten pictures: once a row's ten tiles are in the terminal,
//! turning sends nothing but placements.
//!
//! With motion off a spinner is a still dot in its colour, and asks for no frames.

use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// Steps in one turn: the braille frames, and as many places for the ring's arc.
pub const STEPS: u8 = 10;
/// How long a step lasts: a turn in 0.9 s.
pub const STEP: Duration = Duration::from_millis(90);
/// The cell tier's frames, one a step.
pub const FRAMES: [&str; STEPS as usize] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
/// What stands for a spinner on cells with motion off, and for a state's dot: a still dot.
pub const STILL: &str = "●";

/// The clock every spinner turns by: time since it was first read. A test gives its grid a
/// step of its own instead.
pub fn clock() -> Duration {
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed()
}

/// The step a spinner stands at, at a time on that clock.
pub fn step_at(time: Duration) -> u8 {
    ((time.as_millis() / STEP.as_millis()) % u128::from(STEPS)) as u8
}

/// How long from a time on that clock until the next step begins: when a grid with something
/// turning on it wants drawing again.
pub fn until_next(time: Duration) -> Duration {
    let step = STEP.as_millis();
    let into = time.as_millis() % step;
    Duration::from_millis((step - into) as u64)
}

/// How much of a pixel at (u, v) in the spinner's square box is inked, as (the ring, the arc),
/// each 0 to 1: the ring is drawn at a quarter of the colour's strength and the arc over it at
/// full. The box is 24 units across, as an icon's, the ring 9 from the middle with a stroke
/// of 3.2, and the arc a quarter of it with round ends, starting at the top and going round
/// a tenth of a turn a step. `None` is the still dot: a disc half the box across.
pub fn inked(step: Option<u8>, u: f32, v: f32) -> (bool, bool) {
    let (x, y) = (u * 24.0 - 12.0, v * 24.0 - 12.0);
    let distance = (x * x + y * y).sqrt();
    let Some(step) = step else {
        return (false, distance < 6.0);
    };
    const RADIUS: f32 = 9.0;
    const HALF: f32 = 1.6;
    let ring = (distance - RADIUS).abs() < HALF;
    // Angles clockwise from the top, as a turn's share: 0 at the top, 0.25 at the right.
    let at = |x: f32, y: f32| (x.atan2(-y) / std::f32::consts::TAU).rem_euclid(1.0);
    let start = f32::from(step) / f32::from(STEPS);
    let into = (at(x, y) - start).rem_euclid(1.0);
    let arc = if into <= 0.25 {
        ring
    } else {
        // Past either end, the round cap: within a stroke's half of the end's middle.
        let end = |turn: f32| {
            let angle = turn * std::f32::consts::TAU;
            let (ex, ey) = (RADIUS * angle.sin(), -RADIUS * angle.cos());
            ((x - ex).powi(2) + (y - ey).powi(2)).sqrt() < HALF
        };
        end(start) || end(start + 0.25)
    };
    (ring, arc)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_turn_is_ten_steps_in_under_a_second() {
        assert_eq!(STEP * u32::from(STEPS), Duration::from_millis(900));
        assert_eq!(step_at(Duration::ZERO), 0);
        assert_eq!(step_at(Duration::from_millis(89)), 0);
        assert_eq!(step_at(Duration::from_millis(90)), 1);
        assert_eq!(step_at(Duration::from_millis(899)), 9);
        assert_eq!(step_at(Duration::from_millis(900)), 0, "round again");
        assert_eq!(until_next(Duration::from_millis(0)), STEP);
        assert_eq!(
            until_next(Duration::from_millis(100)),
            Duration::from_millis(80)
        );
        assert_eq!(
            until_next(Duration::from_millis(179)),
            Duration::from_millis(1)
        );
    }

    /// Where the arc is, as the share of a turn its ink's middle stands at from the top.
    fn arc_middle(step: u8) -> f32 {
        let (mut sx, mut sy, mut count) = (0.0, 0.0, 0);
        for j in 0..96 {
            for i in 0..96 {
                let (u, v) = ((i as f32 + 0.5) / 96.0, (j as f32 + 0.5) / 96.0);
                if inked(Some(step), u, v).1 {
                    sx += u - 0.5;
                    sy += v - 0.5;
                    count += 1;
                }
            }
        }
        assert!(count > 0);
        (f32::atan2(sx, -sy) / std::f32::consts::TAU).rem_euclid(1.0)
    }

    #[test]
    fn the_arc_is_a_quarter_of_the_ring_and_goes_round_a_tenth_a_step() {
        // At the first step it's the top right quarter: its middle an eighth of a turn round.
        assert!((arc_middle(0) - 0.125).abs() < 0.01, "{}", arc_middle(0));
        for step in 1..STEPS {
            let moved = (arc_middle(step) - arc_middle(step - 1)).rem_euclid(1.0);
            assert!((moved - 0.1).abs() < 0.01, "step {step} moved {moved}");
        }
        // A quarter of the ring, and its round ends past that: each a half stroke, 1.6 of
        // the ring's 9, about 0.028 of a turn. Nothing of it is off the ring.
        let (mut ring, mut arc, mut off) = (0, 0, 0);
        for j in 0..96 {
            for i in 0..96 {
                let (r, a) = inked(Some(3), (i as f32 + 0.5) / 96.0, (j as f32 + 0.5) / 96.0);
                ring += usize::from(r);
                arc += usize::from(a && r);
                off += usize::from(a && !r);
            }
        }
        let share = arc as f32 / ring as f32;
        let ends = 2.0 * 1.6 / 9.0 / std::f32::consts::TAU;
        assert!(share > 0.25 && share < 0.25 + ends + 0.01, "{share}");
        assert!(off < arc / 50, "{off} of {arc} off the ring");
    }

    #[test]
    fn with_motion_off_it_is_a_still_dot() {
        assert_eq!(inked(None, 0.5, 0.5), (false, true));
        assert_eq!(inked(None, 0.05, 0.5), (false, false));
        assert_eq!(inked(None, 0.2, 0.5), (false, false), "no ring round it");
    }
}
