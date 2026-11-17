//! Writes to a program off the main loop. A program that isn't reading, such as one paused or
//! busy, fills its terminal's buffer, and a write after that waits until it reads: on the main
//! loop, a large paste into one froze every frame and key. Here each write is queued and a
//! thread of its own sends them, in order, so keys and a terminal's replies keep theirs.

use std::io::{self, Write};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, mpsc};

/// The most that may wait to be sent. Past it, a write is refused whole rather than kept:
/// a program that has taken none of sixteen megabytes isn't going to.
pub const MOST: usize = 16 << 20;

/// A queue of writes to one program, and the thread that sends them.
#[derive(Clone)]
pub struct Outbox {
    sender: mpsc::Sender<Vec<u8>>,
    waiting: Arc<AtomicUsize>,
}

impl Outbox {
    pub fn new(mut writer: Box<dyn Write + Send>) -> Self {
        let (sender, jobs) = mpsc::channel::<Vec<u8>>();
        let waiting = Arc::new(AtomicUsize::new(0));
        let sent = waiting.clone();
        std::thread::spawn(move || {
            let mut open = true;
            for bytes in jobs {
                // Once the program is gone, what's left is let go as it comes.
                open = open
                    && writer
                        .write_all(&bytes)
                        .and_then(|()| writer.flush())
                        .is_ok();
                sent.fetch_sub(bytes.len(), Ordering::SeqCst);
            }
        });
        Self { sender, waiting }
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
