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
    /// Who it is on the card: the agent's name, or a runner's own.
    pub by: String,
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
    /// Validate All, Polish All or Post All Drafts, under way.
    pub bulk: Option<Bulk>,
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
        let Some((cli, by, running)) = self.start_toolless_run(job.name(), &prompt) else {
            return false;
        };
        self.checks
            .pending(job)
            .insert(id, Pending::Running(by.clone()));
        self.checks.running.push(Checking {
            thread: id,
            job,
            cli,
            by,
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
    ) -> Option<(Cli, String, ask::Running)> {
        let (cli, command) = match self.side_runner() {
            Ok(chosen) => chosen,
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
        // A runner's own command is found on the PATH as agents' programs are.
        let program = ask::find(&command, &path)?;
        let root = self.workspace.root();
        let run = ask::Run {
            cli,
            args: ask::toolless_args(cli, root),
            root,
            prompt,
        };
        let limit = self.remote.limit.unwrap_or(ask::TIME_LIMIT);
        let by = self
            .settings
            .agents
            .iter()
            .find(|a| a.command == command)
            .map_or_else(|| cli.name().to_string(), |a| a.name.clone());
        Some((cli, by, ask::start_run(&program, run, &path, limit)))
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
        self.drive_bulk() || changed
    }

    /// A run's end: what it said kept with the comment, or why it said nothing on its card.
    /// What it said of words since changed is dropped, as it's said of words that are gone.
    fn finish_check(&mut self, run: Checking, result: Result<String, String>) {
        let by = run.by.clone();
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

/// What a press at the top of the Comments pane does to many comments at once.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum BulkJob {
    /// Validate All or Polish All, two at a time.
    Check(Job),
    /// Post All Drafts, one at a time, once the person has said Post.
    Post,
}

/// How many runs of Validate All or Polish All go at once.
const AT_ONCE: usize = 2;

/// One of those under way: the comments it takes, in the order they're listed, those not
/// started yet, and how many it has started.
pub(super) struct Bulk {
    pub job: BulkJob,
    pub ids: Vec<u32>,
    pub queue: std::collections::VecDeque<u32>,
    pub started: usize,
    pub stopped: bool,
}

impl App {
    /// The comments listed in the pane that a bulk press takes: those of the reviewer's with
    /// nothing from Validate, or Polish, yet; or the drafts of the remote doc in view. A kind
    /// picked on a page lists only it, and the press takes what the pane shows.
    fn bulk_ids(&self, job: BulkJob) -> Vec<u32> {
        let (listed, _) = self.shown_threads();
        listed
            .iter()
            .filter(|t| !t.resolved)
            .filter(|t| self.comment_kind.is_none_or(|kind| t.kind == kind))
            .filter(|t| match job {
                BulkJob::Check(Job::Validate) => {
                    crate::threads::wants_validation(t) && !self.checks.busy(t.id, Job::Validate)
                }
                BulkJob::Check(Job::Polish) => {
                    crate::threads::wants_polish(t) && !self.checks.busy(t.id, Job::Polish)
                }
                BulkJob::Post => t.unposted(),
            })
            .map(|t| t.id)
            .collect()
    }

    /// Validate All, Polish All, or Post All Drafts once the person has said Post: every
    /// comment it takes, queued, and the first started.
    pub(super) fn start_bulk(&mut self, job: BulkJob) {
        if self.checks.bulk.is_some() {
            return;
        }
        let ids = self.bulk_ids(job);
        if ids.is_empty() {
            return;
        }
        self.checks.bulk = Some(Bulk {
            job,
            queue: ids.iter().copied().collect(),
            ids,
            started: 0,
            stopped: false,
        });
        self.drive_bulk();
    }

    /// Asks before Post All Drafts posts anything: how many, Post or Cancel.
    pub(super) fn ask_post_all(&mut self) {
        let count = self.bulk_ids(BulkJob::Post).len();
        if count == 0 {
            return;
        }
        let title = match count {
            1 => "Post 1 comment to the doc?".to_string(),
            n => format!("Post {n} comments to the doc?"),
        };
        self.overlay = Overlay::Confirm(Confirm {
            title,
            detail: "Each is posted on its own, one at a time, as its Post to Doc posts it. A \
                     comment the doc tool refuses says why on its card, and the rest still go."
                .to_string(),
            buttons: vec![("Post", Choice::PostAll), ("Cancel", Choice::Cancel)],
            selected: 1,
        });
    }

    /// Stop: what's under way finishes, and nothing more starts.
    pub(super) fn stop_bulk(&mut self) {
        if let Some(bulk) = self.checks.bulk.as_mut() {
            bulk.queue.clear();
            bulk.stopped = true;
        }
        self.say("Stopped: what's under way finishes, and nothing more starts");
        self.drive_bulk();
    }

    /// Starts what the bulk press has room for: two of Validate or Polish at a time, one post
    /// at a time. When nothing's left to start and nothing it started is running, it's done.
    pub(super) fn drive_bulk(&mut self) -> bool {
        let Some(bulk) = self.checks.bulk.as_ref() else {
            return false;
        };
        let (job, ids) = (bulk.job, bulk.ids.clone());
        let running = |app: &App| match job {
            BulkJob::Check(job) => app
                .checks
                .running
                .iter()
                .filter(|r| r.job == job && ids.contains(&r.thread))
                .count(),
            BulkJob::Post => app
                .remote
                .posts
                .iter()
                .filter(|p| p.reply.is_none() && ids.contains(&p.thread))
                .count(),
        };
        let room = match job {
            BulkJob::Check(_) => AT_ONCE,
            BulkJob::Post => 1,
        };
        let mut changed = false;
        while running(self) < room {
            let Some(id) = self.checks.bulk.as_mut().and_then(|b| b.queue.pop_front()) else {
                break;
            };
            let started = match job {
                BulkJob::Check(job) => self.start_check(id, job),
                BulkJob::Post => {
                    self.post_remote(id, None);
                    self.remote.posts.iter().any(|p| p.thread == id)
                }
            };
            if let Some(bulk) = self.checks.bulk.as_mut() {
                if started {
                    bulk.started += 1;
                } else if self.checks.running.is_empty() && self.remote.posts.is_empty() {
                    // No agent to run: the rest would fail the same way, as the bar says.
                    bulk.queue.clear();
                }
            }
            changed = true;
        }
        let done = self
            .checks
            .bulk
            .as_ref()
            .is_some_and(|b| b.queue.is_empty())
            && running(self) == 0;
        if done && let Some(bulk) = self.checks.bulk.take() {
            let what = match job {
                BulkJob::Check(Job::Validate) => "Validated",
                BulkJob::Check(Job::Polish) => "Polished",
                BulkJob::Post => "Posted",
            };
            // What it says of: for posts, those that made it, since a refused one says why
            // on its card and counts nowhere.
            let n = match job {
                BulkJob::Post => self
                    .threads
                    .iter()
                    .filter(|t| {
                        ids.contains(&t.id)
                            && t.remote.as_ref().is_some_and(|r| {
                                matches!(r.state, cellix_review::threads::Sent::Posted { .. })
                            })
                    })
                    .count(),
                BulkJob::Check(_) => bulk.started,
            };
            if n > 0 {
                self.say(format!(
                    "{what} {n} {}",
                    if n == 1 { "comment" } else { "comments" }
                ));
            }
            changed = true;
        }
        changed
    }

    /// What the pane's head says of the bulk press under way.
    pub(super) fn bulk_shown(&self) -> Option<(&'static str, usize, usize, bool)> {
        let bulk = self.checks.bulk.as_ref()?;
        let doing = match bulk.job {
            BulkJob::Check(job) => job.doing(),
            BulkJob::Post => "Posting",
        };
        let total = if bulk.stopped {
            bulk.started
        } else {
            bulk.ids.len()
        };
        Some((doing, bulk.started, total, !bulk.stopped))
    }
}
