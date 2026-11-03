//! Conversations on a document. A comment stays with its file and line, the agent and the
//! writer answer under it, and it's kept in the project until it's resolved.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Who the person at the keyboard is, in a thread.
pub const YOU: &str = "You";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reply {
    pub author: String,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Thread {
    pub id: u32,
    pub file: String,
    /// Counting from one.
    pub line: usize,
    /// What was selected, or the line it's on.
    pub quote: String,
    pub text: String,
    #[serde(default)]
    pub replies: Vec<Reply>,
    #[serde(default)]
    pub resolved: bool,
    /// How many replies it had when an agent was last told of it.
    #[serde(default)]
    pub told: Option<usize>,
    /// Someone pressed Ask agent, and no agent has it yet.
    #[serde(default)]
    pub asked: bool,
    /// What kind of comment it is, as a reviewer sorts them: Clarity, Correctness, Evidence,
    /// Structure or Reproducibility. Empty until someone says.
    #[serde(default)]
    pub kind: String,
    /// The agent that suggested it, while it's only a suggestion. Empty for the reviewer's own.
    #[serde(default)]
    pub suggested_by: String,
}

impl Thread {
    /// Whether it's waiting on an agent: open, new since it was last told, and not already
    /// answered by one.
    pub fn waiting(&self) -> bool {
        let last = self.replies.last();
        !self.resolved
            && self.told != Some(self.replies.len())
            && last.is_none_or(|r| r.author == YOU)
    }

    /// Whether the last word so far was the person's, so an agent's answer is still due.
    pub fn waiting_answer(&self) -> bool {
        !self.resolved && self.replies.last().is_none_or(|r| r.author == YOU)
    }

    /// Moves the thread to where its quoted text is now, the nearest place if it appears
    /// more than once. A quote rewrapped across lines is still found, and so is one with a
    /// word or two changed. Text that's gone leaves it where it was.
    pub fn follow(&mut self, text: &str) {
        if self.quote.trim().is_empty() {
            return;
        }
        let distance = |line: &usize| line.abs_diff(self.line);
        let found = text
            .lines()
            .enumerate()
            .filter(|(_, line)| line.contains(&self.quote))
            .map(|(index, _)| index + 1)
            .min_by_key(distance)
            .or_else(|| {
                rewrapped(text, &self.quote)
                    .into_iter()
                    .min_by_key(distance)
            })
            .or_else(|| reworded(text, &self.quote, self.line));
        if let Some(line) = found {
            self.line = line;
        }
    }

    /// The thread as a message for an agent, on one line, with how to answer.
    pub fn message(&self) -> String {
        let mut text = format!(
            "Comment {} on {} line {}, about \"{}\": {}",
            self.id, self.file, self.line, self.quote, self.text
        );
        for reply in &self.replies {
            text.push_str(&format!(" {}: {}", reply.author, reply.text));
        }
        let id = self.id;
        text.push_str(&format!(
            " Answer in the document by running: cellix reply {id} \"your answer\""
        ));
        text.replace('\n', " ")
    }
}

/// Where a project keeps its threads.
fn file(root: &Path) -> PathBuf {
    root.join(".cellix/threads.json")
}

/// The project's threads. A file that can't be read is left alone rather than replaced.
pub fn load(root: &Path) -> io::Result<Vec<Thread>> {
    match fs::read_to_string(file(root)) {
        Ok(text) => serde_json::from_str(&text).map_err(io::Error::other),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(e),
    }
}

pub fn save(root: &Path, threads: &[Thread]) -> io::Result<()> {
    let path = file(root);
    if let Some(folder) = path.parent() {
        fs::create_dir_all(folder)?;
        // A conversation isn't a change to the project, so it stays out of review.
        let ignore = folder.join(".gitignore");
        if !ignore.exists() {
            fs::write(
                ignore,
                "threads.json\nthreads.json.tmp\nplans/\n.gitignore\n",
            )?;
        }
    }
    let scratch = path.with_extension("json.tmp");
    fs::write(&scratch, serde_json::to_string_pretty(threads)?)?;
    fs::rename(scratch, path)
}

/// The next free id, so an id in a message never comes to mean another thread.
pub fn next_id(threads: &[Thread]) -> u32 {
    threads.iter().map(|t| t.id).max().unwrap_or(0) + 1
}

/// The lines where the quote starts when line breaks and runs of spaces don't count.
fn rewrapped(text: &str, quote: &str) -> Vec<usize> {
    let quote = quote.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut flat = String::with_capacity(text.len());
    // The line of each byte of `flat`.
    let mut lines = Vec::with_capacity(text.len());
    let mut line = 1;
    for c in text.chars() {
        if c.is_whitespace() {
            if !flat.ends_with(' ') && !flat.is_empty() {
                flat.push(' ');
                lines.push(line);
            }
        } else {
            flat.push(c);
            lines.extend(std::iter::repeat_n(line, c.len_utf8()));
        }
        if c == '\n' {
            line += 1;
        }
    }
    flat.match_indices(&quote)
        .map(|(at, _)| lines[at])
        .collect()
}

/// The line that shares most of the quote's words, if it shares most of them: a quote a word or
/// two was changed in. Ties go to the nearest.
fn reworded(text: &str, quote: &str, near: usize) -> Option<usize> {
    let words: Vec<&str> = quote.split_whitespace().collect();
    // A word or two is too little to tell one line from another.
    if words.len() < 4 {
        return None;
    }
    let score = |line: &str| {
        let theirs: Vec<&str> = line.split_whitespace().collect();
        words.iter().filter(|w| theirs.contains(w)).count()
    };
    text.lines()
        .enumerate()
        .map(|(index, line)| (score(line), index + 1))
        .filter(|(shared, _)| shared * 3 >= words.len() * 2)
        .max_by_key(|(shared, line)| (*shared, std::cmp::Reverse(line.abs_diff(near))))
        .map(|(_, line)| line)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn thread() -> Thread {
        Thread {
            id: 3,
            file: "docs/plan.md".into(),
            line: 12,
            quote: "the cache".into(),
            text: "Why here?\nIt looks late.".into(),
            replies: vec![Reply {
                author: "Claude Code".into(),
                text: "It saves a read.".into(),
            }],
            resolved: false,
            told: None,
            asked: false,
            kind: String::new(),
            suggested_by: String::new(),
        }
    }

    #[test]
    fn a_thread_waits_only_for_what_an_agent_hasnt_seen() {
        let mut thread = thread();
        assert!(!thread.waiting(), "an agent answered last");
        thread.replies.push(Reply {
            author: YOU.into(),
            text: "And for writes?".into(),
        });
        assert!(thread.waiting());
        thread.told = Some(thread.replies.len());
        assert!(!thread.waiting(), "told once");
        thread.replies.clear();
        thread.told = None;
        assert!(thread.waiting(), "a new comment");
        thread.resolved = true;
        assert!(!thread.waiting());
    }

    #[test]
    fn a_thread_follows_its_text() {
        let mut thread = thread();
        thread.line = 2;
        thread.follow("new\nnew\nnew\nabout the cache here\n");
        assert_eq!(thread.line, 4);
        thread.line = 6;
        thread.follow("the cache\nx\nx\nx\nx\nx\nthe cache\n");
        assert_eq!(thread.line, 7, "the nearest of two");
        thread.follow("nothing like it\n");
        assert_eq!(thread.line, 7, "gone, so it stays");
    }

    #[test]
    fn a_thread_follows_its_text_when_it_is_rewrapped_or_reworded() {
        let mut thread = Thread {
            quote: "the cache stays warm across runs".into(),
            line: 1,
            ..thread()
        };
        thread.follow("# Plan\n\nThen the cache stays\nwarm across runs, as before.\n");
        assert_eq!(thread.line, 3, "rewrapped");
        thread.follow("# Plan\n\nIntro.\n\nThen the cache stays hot across runs.\n");
        assert_eq!(thread.line, 5, "a word changed");
        thread.follow("# Plan\n\nSomething else entirely.\n");
        assert_eq!(thread.line, 5, "gone, so it stays");
    }

    #[test]
    fn an_agent_reads_a_thread_on_one_line_and_learns_how_to_answer() {
        let message = thread().message();
        assert!(!message.contains('\n'));
        assert!(message.contains("docs/plan.md line 12"));
        assert!(message.contains("the cache"));
        assert!(message.contains("Claude Code: It saves a read."));
        assert!(message.contains("cellix reply 3"));
    }

    #[test]
    fn threads_outlive_the_session() {
        let dir = tempfile::tempdir().unwrap();
        assert!(load(dir.path()).unwrap().is_empty());
        save(dir.path(), &[thread()]).unwrap();
        assert_eq!(load(dir.path()).unwrap(), [thread()]);
    }

    #[test]
    fn a_broken_file_is_an_error_not_an_empty_list() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join(".cellix")).unwrap();
        fs::write(dir.path().join(".cellix/threads.json"), "{ nope").unwrap();
        assert!(load(dir.path()).is_err());
    }

    #[test]
    fn ids_are_never_reused() {
        assert_eq!(next_id(&[]), 1);
        assert_eq!(next_id(&[thread()]), 4);
    }
}
