//! Pieces of LaTeX only LaTeX can draw: TikZ pictures, commutative diagrams, circuits. Each
//! is built on its own in the background and kept by its content, so typing never waits and
//! a picture is only ever made once.

use std::collections::{HashSet, hash_map::DefaultHasher};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

/// How long LaTeX may take over a piece before it's stopped and the page says so, where
/// "Drawing with LaTeX…" stood for ever (0.11.0's release check, P4). A first build with
/// Tectonic fetches its packages, which can take a while, so a minute.
pub const LIMIT: Duration = Duration::from_secs(60);

/// One piece to build, as a whole LaTeX file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snippet {
    pub key: u64,
    pub tex: String,
}

impl Snippet {
    pub fn new(tex: String) -> Self {
        let mut hasher = DefaultHasher::new();
        tex.hash(&mut hasher);
        Self {
            key: hasher.finish(),
            tex,
        }
    }
}

/// Where a piece stands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    Ready(PathBuf),
    /// LaTeX refused it, and said why.
    Failed(String),
    /// LaTeX gave no answer in time and was stopped, for this reason. What held it up may
    /// pass, as a first fetch of packages does, so a session asks for it once more.
    Stopped(String),
    Missing,
}

fn pdf_of(folder: &Path, key: u64) -> PathBuf {
    folder.join(format!("{key:016x}.pdf"))
}

fn error_of(folder: &Path, key: u64) -> PathBuf {
    folder.join(format!("{key:016x}.err"))
}

fn stopped_of(folder: &Path, key: u64) -> PathBuf {
    folder.join(format!("{key:016x}.slow"))
}

pub fn state(folder: &Path, key: u64) -> State {
    let pdf = pdf_of(folder, key);
    if pdf.is_file() {
        return State::Ready(pdf);
    }
    if let Ok(reason) = std::fs::read_to_string(error_of(folder, key)) {
        return State::Failed(reason);
    }
    match std::fs::read_to_string(stopped_of(folder, key)) {
        Ok(reason) => State::Stopped(reason),
        Err(_) => State::Missing,
    }
}

/// Notes that a piece can't be built here, as a stop a later session tries again, where
/// nothing has been said of it yet. True if that's new.
pub fn cannot(folder: &Path, snippet: &Snippet, why: &str) -> bool {
    if state(folder, snippet.key) != State::Missing {
        return false;
    }
    let _ = std::fs::create_dir_all(folder);
    std::fs::write(stopped_of(folder, snippet.key), why).is_ok()
}

/// Where pieces are kept between runs.
pub fn default_folder() -> PathBuf {
    let var = |name: &str| std::env::var_os(name).map(PathBuf::from);
    let cache = var("XDG_CACHE_HOME").or_else(|| var("HOME").map(|home| home.join(".cache")));
    cache
        .unwrap_or_else(std::env::temp_dir)
        .join("cellix")
        .join("figures")
}

/// The first thing LaTeX complained about.
fn reason(said: &str) -> String {
    complaint(said)
        .unwrap_or("LaTeX couldn't build this")
        .to_string()
}

/// The first line of what LaTeX said that's a complaint, if one is.
fn complaint(said: &str) -> Option<&str> {
    let line = said.lines().find(|line| {
        let line = line.trim_start();
        line.starts_with('!') || line.starts_with("error:")
    })?;
    Some(line.trim_start().trim_start_matches(['!', ' ']))
}

/// How a build ended.
enum Outcome {
    /// LaTeX finished happily, whether or not it made a picture.
    Made,
    /// It refused the piece, or couldn't be started, and said why.
    Refused(String),
    /// It gave no answer in time and was stopped.
    Stopped(String),
}

/// A time as it's said: "a minute", "2 minutes", "30 seconds".
fn said(limit: Duration) -> String {
    match limit.as_secs() {
        0 | 1 => "a second".to_string(),
        60 => "a minute".to_string(),
        secs if secs % 60 == 0 => format!("{} minutes", secs / 60),
        secs => format!("{secs} seconds"),
    }
}

/// Runs the engine on the piece in its room, its output kept in a file there so a full pipe
/// can't hold it up, and stops it, and whatever it started, once `limit` has passed.
fn run(engine: &[String], room: &Path, limit: Duration) -> Outcome {
    let said_by = room.join("snippet.log");
    let Ok(log) = std::fs::File::create(&said_by) else {
        return Outcome::Refused("Couldn't write the picture's file".to_string());
    };
    let mut command = std::process::Command::new(&engine[0]);
    command
        .args(&engine[1..])
        .arg("snippet.tex")
        .current_dir(room)
        .stdin(std::process::Stdio::null());
    if let Ok(copy) = log.try_clone() {
        command.stdout(copy);
    }
    command.stderr(log);
    // Its own group, so stopping it stops what it started too: latexmk runs pdflatex.
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut command, 0);
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(e) => return Outcome::Refused(format!("Couldn't start LaTeX: {e}")),
    };
    let deadline = Instant::now() + limit;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(50)),
            _ => break None,
        }
    };
    let output = std::fs::read_to_string(&said_by).unwrap_or_default();
    match status {
        Some(status) if status.success() => Outcome::Made,
        Some(_) => Outcome::Refused(reason(&output)),
        None => {
            #[cfg(unix)]
            if let Ok(group) = i32::try_from(child.id()) {
                // SAFETY: a signal to the group this child leads, which nothing else is in.
                unsafe { libc::kill(-group, libc::SIGKILL) };
            }
            let _ = child.kill();
            let _ = child.wait();
            let mut why = format!("LaTeX gave no answer in {}, so it was stopped", said(limit));
            if let Some(complaint) = complaint(&output) {
                why.push_str(": ");
                why.push_str(complaint);
            }
            Outcome::Stopped(why)
        }
    }
}

fn build(engine: &[String], folder: &Path, snippet: &Snippet, limit: Duration) {
    let room = folder.join(format!("{:016x}", snippet.key));
    let finish = |outcome: Outcome| {
        // What an earlier try said goes, whatever this one says.
        let _ = std::fs::remove_file(error_of(folder, snippet.key));
        let _ = std::fs::remove_file(stopped_of(folder, snippet.key));
        let pdf = room.join("snippet.pdf");
        match outcome {
            Outcome::Made if pdf.is_file() => {
                let _ = std::fs::rename(&pdf, pdf_of(folder, snippet.key));
            }
            Outcome::Made => {
                let _ = std::fs::write(error_of(folder, snippet.key), "LaTeX made no picture");
            }
            Outcome::Refused(reason) => {
                let _ = std::fs::write(error_of(folder, snippet.key), reason);
            }
            Outcome::Stopped(reason) => {
                let _ = std::fs::write(stopped_of(folder, snippet.key), reason);
            }
        }
        let _ = std::fs::remove_dir_all(&room);
    };
    if std::fs::create_dir_all(&room).is_err()
        || std::fs::write(room.join("snippet.tex"), &snippet.tex).is_err()
    {
        finish(Outcome::Refused(
            "Couldn't write the picture's file".to_string(),
        ));
        return;
    }
    finish(run(engine, &room, limit));
}

/// Builds pieces one at a time on its own thread: they share LaTeX's caches.
pub struct Worker {
    jobs: Sender<Snippet>,
    done: Receiver<u64>,
    asked: HashSet<u64>,
}

impl Worker {
    /// Starts the thread that builds pieces with `engine` into `folder`, stopping any that
    /// takes longer than `limit`.
    pub fn start(engine: Vec<String>, folder: PathBuf, limit: Duration) -> Self {
        let (jobs, inbox) = mpsc::channel::<Snippet>();
        let (finished, done) = crate::wake::channel();
        std::thread::spawn(move || {
            while let Ok(snippet) = inbox.recv() {
                let _ = std::fs::create_dir_all(&folder);
                build(&engine, &folder, &snippet, limit);
                if finished.send(snippet.key).is_err() {
                    break;
                }
            }
        });
        Self {
            jobs,
            done,
            asked: HashSet::new(),
        }
    }

    pub fn ask(&mut self, snippet: Snippet) {
        if self.asked.insert(snippet.key) {
            let _ = self.jobs.send(snippet);
        }
    }

    /// Whether any piece has finished since the last look.
    pub fn poll(&mut self) -> bool {
        let mut any = false;
        while self.done.try_recv().is_ok() {
            any = true;
        }
        any
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_complaint_is_read_out_of_the_log() {
        assert_eq!(
            reason("This is TeX\n! Undefined control sequence.\nl.3"),
            "Undefined control sequence."
        );
        assert_eq!(reason("nothing useful"), "LaTeX couldn't build this");
    }

    #[test]
    fn a_piece_is_known_by_its_content() {
        assert_eq!(Snippet::new("a".into()).key, Snippet::new("a".into()).key);
        assert_ne!(Snippet::new("a".into()).key, Snippet::new("b".into()).key);
    }
}
