//! Wakes the main loop when a program on screen prints, rather than leaving its output to the
//! loop's next look. The loop waits on the keyboard and on a pipe of its own, which each
//! session's reader pokes as its output comes; a session out of sight doesn't poke unless
//! what it has printed is piling up, so it's taken in at the loop's slow pace and draws
//! nothing.
//!
//! Unix only: on Windows the console's input isn't a descriptor to wait on beside a pipe, so
//! there the loop keeps looking at the sessions every few milliseconds, as it did.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender};

/// Chunks of output that may wait to be taken in before a session's reader waits too. A
/// chunk is a read of up to 8 KB, so this is about half a megabyte a session; the reader
/// waiting is what tells a program printing faster than it can be shown to slow down.
pub const ROOM: usize = 64;

/// A session's reader's end: where its output goes.
pub struct Feed {
    sender: SyncSender<Vec<u8>>,
    shown: Arc<AtomicBool>,
    waiting: Arc<AtomicUsize>,
}

/// The session's end, which takes its output in, and says whether it's on screen.
pub struct Fed {
    output: Receiver<Vec<u8>>,
    pub shown: Arc<AtomicBool>,
    waiting: Arc<AtomicUsize>,
}

/// A new feed, from a reader to its session.
pub fn feed() -> (Feed, Fed) {
    let (sender, output) = mpsc::sync_channel(ROOM);
    let shown = Arc::new(AtomicBool::new(true));
    let waiting = Arc::new(AtomicUsize::new(0));
    let feed = Feed {
        sender,
        shown: shown.clone(),
        waiting: waiting.clone(),
    };
    let fed = Fed {
        output,
        shown,
        waiting,
    };
    (feed, fed)
}

/// Whether a chunk wakes the loop: one on screen always, one out of sight once its session
/// has half its room taken, so it's taken in before its reader has to wait.
fn wakes(shown: bool, waiting: usize) -> bool {
    shown || waiting >= ROOM / 2
}

impl Feed {
    /// Passes on a chunk of output, waiting while the session's queue is full. Says whether
    /// the session is still there to take it.
    pub fn send(&self, bytes: Vec<u8>) -> bool {
        let waiting = self.waiting.fetch_add(1, Ordering::SeqCst) + 1;
        if self.sender.send(bytes).is_err() {
            return false;
        }
        if wakes(self.shown.load(Ordering::Relaxed), waiting) {
            poke();
        }
        true
    }
}

impl Fed {
    /// The next chunk waiting, if any.
    pub fn take(&self) -> Option<Vec<u8>> {
        let bytes = self.output.try_recv().ok()?;
        self.waiting.fetch_sub(1, Ordering::SeqCst);
        Some(bytes)
    }
}

#[cfg(unix)]
mod pipe {
    use std::io;
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
    use std::sync::OnceLock;
    use std::time::Duration;

    static PIPE: OnceLock<(OwnedFd, OwnedFd)> = OnceLock::new();

    /// A pipe whose ends never block, and aren't passed to programs started.
    pub fn pipe() -> io::Result<(OwnedFd, OwnedFd)> {
        let mut fds = [0; 2];
        // SAFETY: two descriptors written into an array that outlives the call.
        if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
            return Err(io::Error::last_os_error());
        }
        for fd in fds {
            // SAFETY: a descriptor just made; flags are only added.
            unsafe {
                let flags = libc::fcntl(fd, libc::F_GETFL);
                libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
                libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC);
            }
        }
        // SAFETY: both are open, and owned from here on.
        Ok(unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) })
    }

    /// Makes the loop's pipe, once. Until then a poke does nothing.
    pub fn start() -> io::Result<()> {
        if PIPE.get().is_none() {
            let _ = PIPE.set(pipe()?);
        }
        Ok(())
    }

    pub fn poke() {
        if let Some((_, write)) = PIPE.get() {
            poke_fd(write.as_raw_fd());
        }
    }

    /// One byte down a pipe. A full pipe already has the loop's attention.
    pub fn poke_fd(fd: RawFd) {
        // SAFETY: one byte from a live buffer to an open descriptor that never blocks.
        unsafe { libc::write(fd, [1u8].as_ptr().cast(), 1) };
    }

    /// What ended a wait.
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub struct Woke {
        /// The keyboard has something.
        pub input: bool,
        /// A session poked.
        pub poked: bool,
    }

    /// Waits at most `timeout` for `input` to be readable or the loop's pipe to be poked.
    pub fn wait(input: RawFd, timeout: Duration) -> io::Result<Woke> {
        let Some((read, _)) = PIPE.get() else {
            return Ok(Woke::default());
        };
        wait_on(input, read.as_raw_fd(), timeout)
    }

    /// Waits at most `timeout` for either descriptor, and empties the pipe if it was poked.
    /// A signal ends the wait early, with neither.
    pub fn wait_on(input: RawFd, pipe: RawFd, timeout: Duration) -> io::Result<Woke> {
        let watch = |fd| libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        };
        let mut fds = [watch(input), watch(pipe)];
        let ms = timeout.as_millis().min(i32::MAX as u128) as i32;
        // SAFETY: two descriptors, in an array that outlives the call.
        let found = unsafe { libc::poll(fds.as_mut_ptr(), 2, ms) };
        if found < 0 {
            let error = io::Error::last_os_error();
            return match error.kind() {
                io::ErrorKind::Interrupted => Ok(Woke::default()),
                _ => Err(error),
            };
        }
        let poked = fds[1].revents & libc::POLLIN != 0;
        if poked {
            let mut drain = [0u8; 256];
            // SAFETY: reads into a live buffer of its own size from a pipe that never blocks.
            while unsafe { libc::read(pipe, drain.as_mut_ptr().cast(), drain.len()) } > 0 {}
        }
        Ok(Woke {
            input: fds[0].revents != 0,
            poked,
        })
    }
}

#[cfg(unix)]
pub use pipe::{start, wait};

/// Wakes the loop, where it waits on the pipe.
fn poke() {
    #[cfg(unix)]
    pipe::poke();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A session on screen wakes the loop with each chunk; one out of sight only once half
    /// its room is taken.
    #[test]
    fn output_out_of_sight_wakes_the_loop_only_when_it_piles_up() {
        assert!(wakes(true, 1));
        assert!(!wakes(false, 1));
        assert!(!wakes(false, ROOM / 2 - 1));
        assert!(wakes(false, ROOM / 2));
    }

    /// A reader's chunks queue in order, and its queue has a bound: a reader of a full one
    /// waits until the session takes some.
    #[test]
    fn a_feed_keeps_order_and_holds_its_reader_when_full() {
        let (feed, fed) = feed();
        fed.shown.store(false, Ordering::Relaxed);
        let reader = std::thread::spawn(move || {
            for n in 0..ROOM + 8 {
                assert!(feed.send(vec![n as u8]));
            }
        });
        std::thread::sleep(std::time::Duration::from_millis(100));
        assert!(!reader.is_finished(), "the reader waits on a full queue");
        let mut got = Vec::new();
        while got.len() < ROOM + 8 {
            match fed.take() {
                Some(bytes) => got.extend(bytes),
                None => std::thread::yield_now(),
            }
        }
        reader.join().unwrap();
        let wanted: Vec<u8> = (0..ROOM + 8).map(|n| n as u8).collect();
        assert_eq!(got, wanted);
    }

    /// The loop wakes as soon as the pipe is poked, well before its timeout, and empties it.
    #[cfg(unix)]
    #[test]
    fn a_poke_ends_the_loops_wait() {
        use std::os::fd::AsRawFd;
        use std::time::{Duration, Instant};
        let (keyboard, _typing) = pipe::pipe().unwrap();
        let (read, write) = pipe::pipe().unwrap();
        let writer = write.as_raw_fd();
        let poker = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(30));
            pipe::poke_fd(writer);
            write
        });
        let started = Instant::now();
        let woke = pipe::wait_on(
            keyboard.as_raw_fd(),
            read.as_raw_fd(),
            Duration::from_secs(5),
        );
        let took = started.elapsed();
        let _write = poker.join().unwrap();
        assert_eq!(
            woke.unwrap(),
            pipe::Woke {
                input: false,
                poked: true
            }
        );
        assert!(took < Duration::from_secs(2), "{took:?}");
        // Emptied: the next wait runs to its timeout.
        let again = pipe::wait_on(
            keyboard.as_raw_fd(),
            read.as_raw_fd(),
            Duration::from_millis(30),
        );
        assert_eq!(again.unwrap(), pipe::Woke::default());
    }
}
