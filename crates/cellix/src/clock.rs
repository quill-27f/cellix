//! The time for what waits on a pause or a limit: an auto-save's pause, a LaTeX piece's
//! limit, how long a held program's viewer has read nothing. Cellix reads the wall's. A test
//! stops it and moves it on itself, so a loaded machine can't make a pause pass before the
//! test looks, or a limit run out before what it limits has started: four tests lost to the
//! wall's clock that way with the machine at a load of 28.

use std::time::Instant;

/// Where the time comes from: the wall's, or in a test one it stops and moves on.
#[derive(Clone, Debug, Default)]
pub struct Clock {
    #[cfg(test)]
    stopped: Option<std::sync::Arc<Stopped>>,
}

impl Clock {
    pub fn now(&self) -> Instant {
        #[cfg(test)]
        if let Some(stopped) = &self.stopped {
            return stopped.read();
        }
        Instant::now()
    }
}

/// A stopped clock's time, shared by every copy, and how often it has been read.
#[cfg(test)]
#[derive(Debug)]
struct Stopped {
    at: std::sync::Mutex<Instant>,
    reads: std::sync::atomic::AtomicUsize,
}

#[cfg(test)]
impl Stopped {
    fn read(&self) -> Instant {
        self.reads.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        *self
            .at
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

#[cfg(test)]
impl Clock {
    /// Stopped where the wall's is now: it moves only as `advance` moves it, for every copy.
    pub fn stopped() -> Self {
        Self {
            stopped: Some(std::sync::Arc::new(Stopped {
                at: std::sync::Mutex::new(Instant::now()),
                reads: std::sync::atomic::AtomicUsize::new(0),
            })),
        }
    }

    /// Moves a stopped clock on by `by`. The wall's moves by itself.
    pub fn advance(&self, by: std::time::Duration) {
        if let Some(stopped) = &self.stopped {
            *stopped
                .at
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) += by;
        }
    }

    /// How many times a stopped clock has been read, so a test can tell that something has
    /// started waiting on it.
    pub fn reads(&self) -> usize {
        self.stopped.as_ref().map_or(0, |stopped| {
            stopped.reads.load(std::sync::atomic::Ordering::SeqCst)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// The wall's clock moves by itself; a stopped one only as it's moved, for every copy,
    /// and it counts its reads.
    #[test]
    fn a_stopped_clock_moves_only_as_it_is_moved() {
        let wall = Clock::default();
        let before = wall.now();
        std::thread::sleep(Duration::from_millis(5));
        assert!(wall.now() > before);

        let stopped = Clock::stopped();
        let copy = stopped.clone();
        let at = stopped.now();
        std::thread::sleep(Duration::from_millis(5));
        assert_eq!(copy.now(), at, "the wall moved, it didn't");
        stopped.advance(Duration::from_secs(2));
        assert_eq!(copy.now(), at + Duration::from_secs(2));
        assert_eq!(stopped.reads(), 3);
    }
}
