//! Keeps what runs in the side pane alive without Cellix. Each program runs under a small
//! process of its own, which owns the PTY and a Unix socket and remembers recent output, so
//! Cellix can quit, or an SSH connection drop, and the next Cellix finds it still running.
//!
//! That's Unix only. On Windows there's no host yet: `start` says so, Cellix runs the program
//! itself over ConPTY, and it ends when Cellix does.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[cfg(unix)]
mod host;
pub use host::{Held, Keys, left_running, serve, start};

/// What a host is told to run, kept beside its socket.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Job {
    pub name: String,
    pub argv: Vec<String>,
    pub env: Vec<String>,
    pub folder: PathBuf,
    pub size: (u16, u16),
    pub pixels: (u16, u16),
}

/// Where a project's sessions are kept.
pub fn folder_of(project: &Path) -> PathBuf {
    let mut hasher = DefaultHasher::new();
    project.hash(&mut hasher);
    let var = |name: &str| std::env::var_os(name).map(PathBuf::from);
    let cache = var("XDG_CACHE_HOME").or_else(|| var("HOME").map(|home| home.join(".cache")));
    cache
        .unwrap_or_else(std::env::temp_dir)
        .join("cellix")
        .join("sessions")
        .join(format!("{:016x}", hasher.finish()))
}

/// No host: nothing is ever held, so there's nothing to join, and a program in the side pane
/// is Cellix's own.
#[cfg(not(unix))]
mod host {
    use std::io::{self, ErrorKind, Write};
    use std::path::{Path, PathBuf};
    use std::sync::mpsc::Sender;

    use super::Job;

    fn unsupported() -> io::Error {
        io::Error::new(
            ErrorKind::Unsupported,
            "sessions that outlive Cellix need Unix",
        )
    }

    pub fn start(_program: &Path, _folder: &Path, _job: &Job) -> io::Result<Held> {
        Err(unsupported())
    }

    pub fn left_running(_folder: &Path) -> Vec<(String, PathBuf)> {
        Vec::new()
    }

    pub fn serve(_socket: &Path) -> io::Result<()> {
        Err(unsupported())
    }

    /// Can't be made, so none of its methods can be reached.
    pub enum Held {}

    pub enum Keys {}

    impl Write for Keys {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            match *self {}
        }

        fn flush(&mut self) -> io::Result<()> {
            match *self {}
        }
    }

    impl Held {
        pub fn connect(_socket: &Path) -> io::Result<Self> {
            Err(unsupported())
        }

        pub fn keys(&self) -> io::Result<Keys> {
            match *self {}
        }

        pub fn listen(&self, _output: Sender<Vec<u8>>, _done: Sender<()>) -> io::Result<()> {
            match *self {}
        }

        pub fn at_prompt(&self) -> bool {
            match *self {}
        }

        pub fn resize(&self, _size: (u16, u16), _pixels: (u16, u16)) {
            match *self {}
        }

        pub fn kill(&self) {
            match *self {}
        }
    }
}
