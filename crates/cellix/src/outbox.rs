//! Writes to a program off the main loop. A program that isn't reading, such as one paused or
//! busy, fills its terminal's buffer, and a write after that waits until it reads: on the main
//! loop, a large paste into one froze every frame and key. Here each write is queued and a
//! thread of its own sends them, in order, so keys and a terminal's replies keep theirs.

use std::io::{self, Write};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

/// The most that may wait to be sent. Past it, a write is refused whole rather than kept:
/// a program that has taken none of sixteen megabytes isn't going to.
pub const MOST: usize = 16 << 20;

/// A queue of writes to one program, and the thread that sends them.
#[derive(Clone)]
pub struct Outbox {
    sender: mpsc::Sender<Vec<u8>>,
    waiting: Arc<AtomicUsize>,
    /// A write failed: whatever was at the other end is gone.
    broken: Arc<AtomicBool>,
}

impl Outbox {
    pub fn new(mut writer: Box<dyn Write + Send>) -> Self {
        let (sender, jobs) = mpsc::channel::<Vec<u8>>();
        let waiting = Arc::new(AtomicUsize::new(0));
        let broken = Arc::new(AtomicBool::new(false));
        let (sent, failed) = (waiting.clone(), broken.clone());
        std::thread::spawn(move || {
            for bytes in jobs {
                // Once the other end is gone, what's left is let go as it comes.
                if !failed.load(Ordering::SeqCst) {
                    let written = writer.write_all(&bytes).and_then(|()| writer.flush());
                    failed.store(written.is_err(), Ordering::SeqCst);
                }
                sent.fetch_sub(bytes.len(), Ordering::SeqCst);
            }
        });
        Self {
            sender,
            waiting,
            broken,
        }
    }

    /// Waits up to `within` for everything queued to be written. Says whether it was, to an
    /// end that's still there: what must be heard before anything ends, such as a host's word
    /// that its program is over, is only queued once written.
    pub fn delivered(&self, within: Duration) -> bool {
        let until = Instant::now() + within;
        while self.waiting() > 0 && !self.broken.load(Ordering::SeqCst) && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(2));
        }
        self.waiting() == 0 && !self.broken.load(Ordering::SeqCst)
    }

    /// Bytes queued and not yet sent.
    pub fn waiting(&self) -> usize {
        self.waiting.load(Ordering::SeqCst)
    }
}

impl Write for Outbox {
    /// Queues all of `bytes`, or refuses them all when the queue is full.
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.is_empty() {
            return Ok(0);
        }
        let waiting = self.waiting();
        if waiting > 0 && waiting + bytes.len() > MOST {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "the program isn't reading what's sent to it",
            ));
        }
        self.waiting.fetch_add(bytes.len(), Ordering::SeqCst);
        self.sender
            .send(bytes.to_vec())
            .map_err(|_| io::Error::from(io::ErrorKind::BrokenPipe))?;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
