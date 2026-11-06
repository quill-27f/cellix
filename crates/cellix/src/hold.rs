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
pub use host::{Held, Keys, left_running, serve, start, stopped};

/// What a held program is told about where it's held: the host's socket.
pub const HELD_VAR: &str = cellix_agent::hook::HELD_VAR;
/// The folder, among a project's sessions, for those that stopped without finishing.
#[cfg(unix)]
const STOPPED: &str = "stopped";

/// A session whose host died without finishing, as when the machine restarted.
#[derive(Clone, Debug)]
pub struct Stopped {
    pub job: Job,
    /// What it last showed.
    pub output: Vec<u8>,
    /// The agent's own session, if its hooks said, which is what resumes it.
    pub agent: Option<cellix_agent::hook::Resumable>,
    /// When it last showed anything, which is about when it stopped.
    pub ended: Option<std::time::SystemTime>,
    /// Its job on disk, which names the rest.
    notes: PathBuf,
}

impl Stopped {
    /// Whether the machine has started since it stopped, so the restart is what stopped it.
    pub fn by_restart(&self) -> bool {
        matches!((self.ended, booted_at()), (Some(ended), Some(booted)) if ended <= booted)
    }

    /// Forgets it, once it has been resumed, started fresh or closed.
    pub fn forget(&self) {
        for extension in ["json", "log", "agent"] {
            let _ = std::fs::remove_file(self.notes.with_extension(extension));
        }
    }
}

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

/// When this machine last started.
pub fn booted_at() -> Option<std::time::SystemTime> {
    let seconds: u64 = if cfg!(target_os = "linux") {
        let stat = std::fs::read_to_string("/proc/stat").ok()?;
        let line = stat.lines().find(|l| l.starts_with("btime "))?;
        line[6..].trim().parse().ok()?
    } else if cfg!(target_os = "macos") {
        // `{ sec = 1730000000, usec = 0 } Mon Nov  4 …`
        let output = std::process::Command::new("sysctl")
            .args(["-n", "kern.boottime"])
            .output()
            .ok()?;
        let text = String::from_utf8_lossy(&output.stdout);
        let after = text.split("sec = ").nth(1)?;
        after.split(',').next()?.trim().parse().ok()?
    } else {
        return None;
    };
    Some(std::time::UNIX_EPOCH + std::time::Duration::from_secs(seconds))
}

/// Where every project's sessions are kept, each in a folder of its own.
pub fn sessions_base() -> PathBuf {
    let var = |name: &str| std::env::var_os(name).map(PathBuf::from);
    let cache = var("XDG_CACHE_HOME").or_else(|| var("HOME").map(|home| home.join(".cache")));
    cache
        .unwrap_or_else(std::env::temp_dir)
        .join("cellix")
        .join("sessions")
}

/// Where a project's sessions are kept.
pub fn folder_of(project: &Path) -> PathBuf {
    let mut hasher = DefaultHasher::new();
    project.hash(&mut hasher);
    sessions_base().join(format!("{:016x}", hasher.finish()))
}

/// Notes which project a sessions folder is for, so it can be found from the folder.
pub fn mark_project(project: &Path) {
    let folder = folder_of(project);
    let file = folder.join(PROJECT);
    // A path that isn't text can't be found again this way, which only costs the bar.
    let Some(named) = project.to_str() else {
        return;
    };
    if std::fs::read_to_string(&file).is_ok_and(|known| known == named) {
        return;
    }
    if std::fs::create_dir_all(&folder).is_ok() {
        let _ = std::fs::write(file, named);
    }
}

/// The file in a sessions folder that names its project.
const PROJECT: &str = "project";
/// Beside it while the project was a workspace when Cellix last quit, so it opens again with
/// nothing running in it, as an editor brings back the windows it had.
const OPEN: &str = "open";

/// Notes that a project was an open workspace when Cellix quit, so the next one opens it.
pub fn remember_open(project: &Path) {
    mark_project(project);
    let _ = std::fs::write(folder_of(project).join(OPEN), "");
}

/// Forgets that, once its workspace is closed on purpose.
pub fn forget_open(project: &Path) {
    let _ = std::fs::remove_file(folder_of(project).join(OPEN));
}

/// What's running for a project, or stopped without finishing and waiting to be resumed, and
/// whether it was open when Cellix last quit.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Activity {
    pub running: Vec<String>,
    pub stopped: Vec<String>,
    pub open: bool,
}

impl Activity {
    pub fn is_empty(&self) -> bool {
        self.running.is_empty() && self.stopped.is_empty() && !self.open
    }
}

/// What's running, or waiting to be resumed, for a project.
pub fn activity(project: &Path) -> Activity {
    let folder = folder_of(project);
    let stopped = stopped(&folder).into_iter().map(|s| s.job.name).collect();
    let running = left_running(&folder)
        .into_iter()
        .map(|(name, _)| name)
        .collect();
    let open = folder.join(OPEN).exists();
    Activity {
        running,
        stopped,
        open,
    }
}

/// The projects on this machine with something running, stopped and waiting to be resumed,
/// or left open when Cellix last quit, by their folders, sorted.
pub fn projects_with_activity() -> Vec<(PathBuf, Activity)> {
    let mut found = Vec::new();
    for entry in std::fs::read_dir(sessions_base())
        .into_iter()
        .flatten()
        .flatten()
    {
        let Ok(named) = std::fs::read_to_string(entry.path().join(PROJECT)) else {
            continue;
        };
        let project = PathBuf::from(named);
        // A folder whose name isn't this project's hash is someone else's, or stale.
        if folder_of(&project) != entry.path() || !project.is_dir() {
            continue;
        }
        let activity = activity(&project);
        if !activity.is_empty() {
            found.push((project, activity));
        }
    }
    found.sort_by(|a, b| a.0.cmp(&b.0));
    found
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

    pub fn stopped(_folder: &Path) -> Vec<super::Stopped> {
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
