//! Notes any frame slow enough to feel, with where the time went, so a stall on a real terminal
//! can be read afterwards. It holds timings and nothing else, and starts afresh each run.

use std::fs::{self, File};
use std::io::Write;
use std::path::PathBuf;
use std::time::Duration;

/// Slower than this, and typing stops feeling immediate.
const SLOW: Duration = Duration::from_millis(50);
/// Lines kept, so a bad run can't grow the file for long.
const MOST: usize = 500;

/// Where one trip round the main loop went.
#[derive(Clone, Copy, Debug, Default)]
pub struct Phases {
    /// Handling what arrived, from keys to ticks.
    pub events: Duration,
    /// Taking in what's come back from programs, servers and threads.
    pub poll: Duration,
    /// Laying the frame out.
    pub draw: Duration,
    /// Rasterizing it and writing it to the terminal, which blocks if the terminal falls behind.
    pub present: Duration,
}

impl Phases {
    fn line(&self) -> Option<String> {
        let total = self.events + self.poll + self.draw + self.present;
        let ms = |time: Duration| time.as_millis();
        (total >= SLOW).then(|| {
            format!(
                "{} ms: events {}, poll {}, draw {}, present {}",
                ms(total),
                ms(self.events),
                ms(self.poll),
                ms(self.draw),
                ms(self.present)
            )
        })
    }
}

fn log_path() -> Option<PathBuf> {
    let var = |name: &str| std::env::var_os(name).map(PathBuf::from);
    let cache = var("XDG_CACHE_HOME").or_else(|| var("HOME").map(|home| home.join(".cache")))?;
    Some(cache.join("cellix").join("slow-frames.log"))
}

/// The log of this run. Nothing here is allowed to get in the way of drawing.
pub struct Log {
    file: Option<File>,
    written: usize,
}

impl Log {
    pub fn start() -> Self {
        let file = log_path().and_then(|path| {
            fs::create_dir_all(path.parent()?).ok()?;
            File::create(path).ok()
        });
        Self { file, written: 0 }
    }

    pub fn note(&mut self, phases: &Phases) {
        let Some(line) = phases.line().filter(|_| self.written < MOST) else {
            return;
        };
        if let Some(file) = &mut self.file {
            let _ = writeln!(file, "{line}");
            self.written += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_quick_frame_isnt_noted_and_a_slow_one_says_where_it_went() {
        let quick = Phases {
            draw: Duration::from_millis(5),
            ..Phases::default()
        };
        assert_eq!(quick.line(), None);
        let slow = Phases {
            events: Duration::from_millis(2),
            present: Duration::from_millis(90),
            ..Phases::default()
        };
        let line = slow.line().unwrap();
        assert!(
            line.starts_with("92 ms") && line.contains("present 90"),
            "{line}"
        );
    }

    #[test]
    fn the_log_stops_at_its_limit() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("log");
        let mut log = Log {
            file: Some(File::create(&path).unwrap()),
            written: MOST - 1,
        };
        let slow = Phases {
            draw: SLOW,
            ..Phases::default()
        };
        log.note(&slow);
        log.note(&slow);
        assert_eq!(fs::read_to_string(path).unwrap().lines().count(), 1);
    }
}
