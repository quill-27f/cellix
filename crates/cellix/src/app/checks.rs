//! Validate and Polish, on one of the reviewer's comments, on a page of the project and on a
//! remote doc alike. The Default agent (Settings › Integrations) runs once in the background,
//! as Ask's side questions do, with nothing but what it's told: Validate the whole document,
//! the words the comment is on and the comment, to say whether it holds up; Polish the
//! comment and the paragraph round its words, to make it short, clear and kind. Neither changes
//! the comment: what Validate says stands under it till it's dismissed, and Polish's words go
//! in only by Use This. Both are kept with the comment till its words change.

use super::remote::drain;
use super::*;
use crate::ask::{self, Cli};
use crate::remote as doc;
use crate::threads::Pending;
use cellix_review::threads::{Polished, Validation};

/// Which of the two a run is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Job {
    Validate,
    Polish,
}

impl Job {
    /// As the card and the status bar say it's under way.
    fn doing(self) -> &'static str {
        match self {
            Job::Validate => "Validating",
            Job::Polish => "Polishing",
        }
    }

    fn name(self) -> &'static str {
        match self {
            Job::Validate => "Validate",
            Job::Polish => "Polish",
        }
    }
}

/// A run under way on a comment: which agent, the comment's words as it was given them, and
/// how many words they were.
pub(super) struct Checking {
    pub thread: u32,
    pub job: Job,
    pub cli: Cli,
    pub on: String,
    pub running: ask::Running,
    pub said: String,
}

#[derive(Default)]
pub(super) struct Checks {
    pub running: Vec<Checking>,
    /// Each comment's Validate and Polish while it runs, or once it failed, as its card says.
    pub validate: crate::threads::Pendings,
    pub polish: crate::threads::Pendings,
}

impl Checks {
    fn pending(&mut self, job: Job) -> &mut crate::threads::Pendings {
        match job {
            Job::Validate => &mut self.validate,
            Job::Polish => &mut self.polish,
        }
    }

    pub(super) fn busy(&self, id: u32, job: Job) -> bool {
        self.running.iter().any(|r| r.thread == id && r.job == job)
    }
}

impl App {
    /// Says whether the reviewer's comment holds up: a press on its card's Validate. The
    /// Default agent reads the whole document, the words the comment is on and the comment,
    /// and answers one line; the comment stays as it is.
    pub(super) fn validate_comment(&mut self, id: u32) {
        self.start_check(id, Job::Validate);
    }

    /// Has the Default agent make the reviewer's comment short, clear and kind; its words
    /// stand under the comment, to use or not.
    pub(super) fn polish_comment(&mut self, id: u32) {
        self.start_check(id, Job::Polish);
    }

    /// Whether a comment is one Validate and Polish are for: the reviewer's own, open. Polish
    /// leaves a remote comment that's posted, or posting, as the doc has it.
    pub(super) fn checkable(&self, id: u32, job: Job) -> bool {
        let Some(thread) = self.threads.iter().find(|t| t.id == id) else {
            return false;
        };
        let yours = thread.author.is_empty() && thread.suggested_by.is_empty();
        let posted = thread.remote.is_some() && !thread.unposted();
        yours && !thread.resolved && !(job == Job::Polish && posted)
    }

    fn start_check(&mut self, id: u32, job: Job) -> bool {
        if !self.checkable(id, job) || self.checks.busy(id, job) {
            return false;
        }
        let Some(thread) = self.threads.iter().find(|t| t.id == id) else {
            return false;
        };
        let text = self.document_of(thread);
        let prompt = match job {
            Job::Validate => doc::validate_prompt(
                &crate::threads::place_of(thread),
                &text,
                &thread.quote,
                &thread.text,
            ),
            Job::Polish => {
                let paragraph = doc::paragraph_around(&text, thread.anchor());
                doc::polish_prompt(&thread.text, &thread.quote, &paragraph)
            }
        };
        let on = thread.text.clone();
        let Some((cli, running)) = self.start_toolless_run(job.name(), &prompt) else {
            return false;
        };
        self.checks
            .pending(job)
            .insert(id, Pending::Running(cli.name().to_string()));
        self.checks.running.push(Checking {
            thread: id,
            job,
            cli,
            on,
            running,
            said: String::new(),
        });
        true
    }

    /// The whole of what a comment is on, as it reads now: the file's text in its tab where
    /// it's open, else on disk; a terminal's output, the output it quotes.
    fn document_of(&self, thread: &Thread) -> String {
        if let Some(cellix_review::About::Output { .. }) = &thread.about {
            return thread.selection.clone();
        }
        let open = self.editors.iter().find(|e| {
            e.buffer
                .path()
                .is_some_and(|p| self.relative(p) == thread.file)
        });
        match open {
            Some(editor) => editor.buffer.text(),
            None => std::fs::read_to_string(self.workspace.root().join(&thread.file))
                .unwrap_or_default(),
        }
    }

    /// Starts the Default agent on `prompt` with no tools, as Validate and Polish run it; or
    /// says what's missing, `what` naming the action.
    pub(super) fn start_toolless_run(
        &mut self,
        what: &str,
        prompt: &str,
    ) -> Option<(Cli, ask::Running)> {
        let cli = match self.side_cli() {
            Ok(cli) => cli,
            Err(Some(named)) => {
                self.say(format!(
                    "{what} runs {}, the Default agent in Settings › Integrations, which isn't \
                     installed here",
                    named.name()
                ));
                return None;
            }
            Err(None) => {
                self.say(format!(
                    "{what} needs Claude Code, Codex or Gemini CLI installed here"
                ));
                return None;
            }
        };
        let path = self.side_path();
        let program = ask::find(cli.program(), &path)?;
        let root = self.workspace.root();
        let run = ask::Run {
            cli,
            args: ask::toolless_args(cli, root),
            root,
            prompt,
        };
        let limit = self.remote.limit.unwrap_or(ask::TIME_LIMIT);
        Some((cli, ask::start_run(&program, run, &path, limit)))
    }

    /// What the status bar says is under way, with its spinner.
    pub(super) fn checks_waiting(&self) -> Option<String> {
        let first = self.checks.running.first()?;
        Some(first.job.doing().to_string())
    }

    /// Takes in what the runs said since. True when anything changed.
    pub(super) fn poll_checks(&mut self) -> bool {
        let mut ended = Vec::new();
        for (index, run) in self.checks.running.iter_mut().enumerate() {
            if let Some(result) = drain(&run.running, &mut run.said, run.cli) {
                ended.push((index, result));
            }
        }
        let changed = !ended.is_empty();
        for (index, result) in ended.into_iter().rev() {
            let run = self.checks.running.remove(index);
            self.finish_check(run, result);
        }
        changed
    }

    /// A run's end: what it said kept with the comment, or why it said nothing on its card.
    /// What it said of words since changed is dropped, as it's said of words that are gone.
    fn finish_check(&mut self, run: Checking, result: Result<String, String>) {
        let by = run.cli.name().to_string();
        let kept = match (run.job, result) {
            (_, Err(why)) => Err(why),
            (Job::Validate, Ok(answer)) => {
                doc::validation_of(&answer).map(|(verdict, kind, why)| {
                    Kept::Validation(Validation {
                        verdict,
                        kind,
                        why,
                        by,
                        on: run.on.clone(),
                    })
                })
            }
            (Job::Polish, Ok(answer)) if answer.trim().is_empty() => {
                Err("it gave no text".to_string())
            }
            (Job::Polish, Ok(answer)) => Ok(Kept::Polished(Polished {
                text: doc::clean_polish(&answer),
                by,
                from: doc::words(&run.on),
                on: run.on.clone(),
            })),
        };
        let pending = self.checks.pending(run.job);
        match kept {
            Err(why) => {
                pending.insert(run.thread, Pending::Failed(why));
            }
            Ok(kept) => {
                pending.remove(&run.thread);
                let Some(thread) = self.threads.iter_mut().find(|t| t.id == run.thread) else {
                    return;
                };
                if thread.text != run.on {
                    return;
                }
                match kept {
                    Kept::Validation(v) => thread.validation = Some(v),
                    Kept::Polished(p) => thread.polished = Some(p),
                }
                self.threads_seen = None;
                self.save_threads();
            }
        }
    }

    /// Takes what Validate said off the comment's card. The comment is as it was.
    pub(super) fn dismiss_validation(&mut self, id: u32) {
        self.checks.validate.remove(&id);
        self.checks
            .running
            .retain(|r| !(r.thread == id && r.job == Job::Validate));
        if let Some(thread) = self.threads.iter_mut().find(|t| t.id == id) {
            thread.validation = None;
        }
        self.save_threads();
    }

    /// Takes Polish's words as the comment's, as Edit does: one an agent was told of already is
    /// an edited comment then, and what waits to go with the review changes with it. A remote
    /// draft is still a draft.
    pub(super) fn use_polish(&mut self, id: u32) {
        let Some(polished) = self
            .threads
            .iter_mut()
            .find(|t| t.id == id)
            .and_then(|t| t.polished.take())
        else {
            return;
        };
        self.checks.polish.remove(&id);
        self.set_comment_words(id, polished.text);
    }

    /// Leaves the comment as it is, and Polish's words go.
    pub(super) fn keep_mine(&mut self, id: u32) {
        self.checks.polish.remove(&id);
        self.checks
            .running
            .retain(|r| !(r.thread == id && r.job == Job::Polish));
        if let Some(thread) = self.threads.iter_mut().find(|t| t.id == id) {
            thread.polished = None;
        }
        self.save_threads();
    }

    /// What every run on a comment going away leaves: nothing.
    pub(super) fn forget_checks(&mut self, id: u32) {
        self.checks.running.retain(|r| r.thread != id);
        self.checks.validate.remove(&id);
        self.checks.polish.remove(&id);
    }
}

/// What a run leaves with its comment.
enum Kept {
    Validation(Validation),
    Polished(Polished),
}
