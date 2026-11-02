//! Pieces of LaTeX only LaTeX can draw: TikZ pictures, commutative diagrams, circuits. Each
//! is built on its own in the background and kept by its content, so typing never waits and
//! a picture is only ever made once.

use std::collections::{HashSet, hash_map::DefaultHasher};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};

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
    Missing,
}

fn pdf_of(folder: &Path, key: u64) -> PathBuf {
    folder.join(format!("{key:016x}.pdf"))
}

fn error_of(folder: &Path, key: u64) -> PathBuf {
    folder.join(format!("{key:016x}.err"))
}

pub fn state(folder: &Path, key: u64) -> State {
    let pdf = pdf_of(folder, key);
    if pdf.is_file() {
        return State::Ready(pdf);
    }
    match std::fs::read_to_string(error_of(folder, key)) {
        Ok(reason) => State::Failed(reason),
        Err(_) => State::Missing,
    }
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
    let complaint = said.lines().find(|line| {
        let line = line.trim_start();
        line.starts_with('!') || line.starts_with("error:")
    });
    let complaint = complaint.unwrap_or("LaTeX couldn't build this");
    complaint.trim_start_matches(['!', ' ']).to_string()
}

fn build(engine: &[String], folder: &Path, snippet: &Snippet) {
    let room = folder.join(format!("{:016x}", snippet.key));
    let finish = |failure: Option<String>| {
        let pdf = room.join("snippet.pdf");
        match failure {
            None if pdf.is_file() => {
                let _ = std::fs::rename(&pdf, pdf_of(folder, snippet.key));
            }
            other => {
                let reason = other.unwrap_or_else(|| "LaTeX made no picture".to_string());
                let _ = std::fs::write(error_of(folder, snippet.key), reason);
            }
        }
        let _ = std::fs::remove_dir_all(&room);
    };
    if std::fs::create_dir_all(&room).is_err()
        || std::fs::write(room.join("snippet.tex"), &snippet.tex).is_err()
    {
        finish(Some("Couldn't write the picture's file".to_string()));
        return;
    }
    let output = std::process::Command::new(&engine[0])
        .args(&engine[1..])
        .arg("snippet.tex")
        .current_dir(&room)
        .stdin(std::process::Stdio::null())
        .output();
    finish(match output {
        Ok(done) if done.status.success() => None,
        Ok(done) => {
            let said = format!(
                "{}{}",
                String::from_utf8_lossy(&done.stdout),
                String::from_utf8_lossy(&done.stderr)
            );
            Some(reason(&said))
        }
        Err(e) => Some(format!("Couldn't start LaTeX: {e}")),
    });
}

/// Builds pieces one at a time on its own thread: they share LaTeX's caches.
pub struct Worker {
    jobs: Sender<Snippet>,
    done: Receiver<u64>,
    asked: HashSet<u64>,
}

impl Worker {
    pub fn start(engine: Vec<String>, folder: PathBuf) -> Self {
        let (jobs, inbox) = mpsc::channel::<Snippet>();
        let (finished, done) = mpsc::channel();
        std::thread::spawn(move || {
            while let Ok(snippet) = inbox.recv() {
                let _ = std::fs::create_dir_all(&folder);
                build(&engine, &folder, &snippet);
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
