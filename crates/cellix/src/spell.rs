//! Checks writing on a thread of its own: starting the checker and reading a long document
//! both take a moment, and typing must never wait for either.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};

use cellix_doc::prose::{Checker, Issue, Source};

enum Job {
    Check {
        file: PathBuf,
        revision: u64,
        text: String,
        source: Source,
    },
    Learn(String),
}

/// What came back for one revision of a file.
pub struct Found {
    pub file: PathBuf,
    pub revision: u64,
    pub issues: Vec<Issue>,
}

pub struct Spell {
    jobs: Sender<Job>,
    found: Receiver<Found>,
    words: Option<PathBuf>,
}

/// The words someone has said are right.
fn read_words(file: Option<&Path>) -> HashSet<String> {
    let text = file.and_then(|f| std::fs::read_to_string(f).ok());
    let lines = text.iter().flat_map(|t| t.lines());
    lines.map(|l| l.trim().to_lowercase()).collect()
}

impl Spell {
    pub fn start(words: Option<PathBuf>) -> Self {
        let (jobs, inbox) = mpsc::channel::<Job>();
        let (done, found) = mpsc::channel();
        let known = read_words(words.as_deref());
        std::thread::spawn(move || {
            let mut checker = None;
            while let Ok(job) = inbox.recv() {
                // Only the newest text matters, and whatever it takes to reach it.
                let mut job = job;
                let mut learned = Vec::new();
                while let Job::Learn(word) = job {
                    learned.push(word);
                    match inbox.recv() {
                        Ok(next) => job = next,
                        Err(_) => return,
                    }
                }
                let Job::Check {
                    file,
                    revision,
                    text,
                    source,
                } = job
                else {
                    continue;
                };
                let checker = checker.get_or_insert_with(|| Checker::new(known.clone()));
                checker.known.extend(learned);
                let issues = checker.check(&text, source);
                let found = Found {
                    file,
                    revision,
                    issues,
                };
                if done.send(found).is_err() {
                    return;
                }
            }
        });
        Self { jobs, found, words }
    }

    pub fn check(&self, file: PathBuf, revision: u64, text: String, source: Source) {
        let _ = self.jobs.send(Job::Check {
            file,
            revision,
            text,
            source,
        });
    }

    /// Remembers a word, for this run and the next.
    pub fn learn(&self, word: &str) {
        let word = word.to_lowercase();
        let _ = self.jobs.send(Job::Learn(word.clone()));
        if let Some(file) = &self.words {
            let mut text = std::fs::read_to_string(file).unwrap_or_default();
            text.push_str(&word);
            text.push('\n');
            if let Some(folder) = file.parent() {
                let _ = std::fs::create_dir_all(folder);
            }
            let _ = std::fs::write(file, text);
        }
    }

    pub fn poll(&self) -> Option<Found> {
        self.found.try_recv().ok()
    }
}
