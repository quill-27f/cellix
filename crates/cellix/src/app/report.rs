//! The review as Markdown, for when it's done and has to go somewhere: a referee's report, a
//! review handed on. Comments live in `.cellix/threads.json`, which git ignores, so until this
//! a review had no output. Copy puts it on the clipboard the way every copy goes, through the
//! terminal (OSC 52); Save writes it into the project and opens it.
//!
//! It's every comment the reviewer kept, open and resolved, a file at a time and in each file
//! a kind at a time, in the document's order. An agent's suggestion goes in once it's kept,
//! which makes it the reviewer's; one dismissed or not yet decided doesn't.

use super::*;

/// The heading over comments that have no kind.
const NO_KIND: &str = "Other";
/// The name Save offers, or with a number after it when the project has one already.
const SAVED_AS: &str = "review";

/// Whether a comment goes in the review: it's the reviewer's own.
pub(super) fn kept(thread: &Thread) -> bool {
    thread.suggested_by.is_empty()
}

/// A kind's place: the five a careful review covers in the strip's order, any other kind
/// after them by name, and none last.
fn kind_rank(kind: &str) -> (usize, String) {
    match threads::KINDS.iter().position(|k| *k == kind) {
        Some(at) => (at, String::new()),
        None if kind.is_empty() => (threads::KINDS.len() + 1, String::new()),
        None => (threads::KINDS.len(), kind.to_lowercase()),
    }
}

fn kind_of(thread: &Thread) -> &str {
    if thread.kind.is_empty() {
        NO_KIND
    } else {
        &thread.kind
    }
}

/// "1 comment", "2 comments".
fn counted(count: usize, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

/// The first letter raised.
fn capital(text: &str) -> String {
    let mut chars = text.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(chars).collect()
    })
}

/// Text as a quote, a line of it at a time.
fn blockquote(text: &str) -> String {
    text.trim_end_matches('\n')
        .lines()
        .map(|line| {
            if line.trim().is_empty() {
                ">".to_string()
            } else {
                format!("> {line}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// How many comments, on how much, how many settled, and how many of each kind.
fn summary(kept: &[&Thread]) -> String {
    let mut files: Vec<&str> = kept
        .iter()
        .filter(|t| !on_output(t))
        .map(|t| t.file.as_str())
        .collect();
    files.sort_unstable();
    files.dedup();
    let output = kept.iter().any(|t| on_output(t));
    let on = match (files.len(), output) {
        (0, _) => "terminal output".to_string(),
        (n, false) => counted(n, "file", "files"),
        (n, true) => format!("{} and terminal output", counted(n, "file", "files")),
    };
    let resolved = kept.iter().filter(|t| t.resolved).count();
    let settled = if resolved > 0 {
        format!(", {resolved} resolved")
    } else {
        String::new()
    };
    let mut text = format!(
        "{} on {on}{settled}.\n\n",
        counted(kept.len(), "comment", "comments")
    );
    let mut kinds: Vec<&str> = kept.iter().map(|t| kind_of(t)).collect();
    kinds.sort_by_cached_key(|kind| kind_rank(if *kind == NO_KIND { "" } else { kind }));
    kinds.dedup();
    for kind in kinds {
        let count = kept.iter().filter(|t| kind_of(t) == kind).count();
        text.push_str(&format!("- {kind}: {count}\n"));
    }
    text
}

/// A comment: where it is and whether it's settled, what it's about, what was said, and
/// each answer under it with who gave it and, for an agent's verdict, which way it went.
fn entry(thread: &Thread) -> String {
    let at = if !thread.step.is_empty() {
        capital(&thread.step)
    } else if on_output(thread) {
        "Output".to_string()
    } else {
        format!("Line {}", thread.line)
    };
    let state = if thread.resolved { "Resolved" } else { "Open" };
    // Words a plan's later version changed or dropped, as the card's pill says.
    let earlier = thread.earlier.map_or_else(String::new, |version| {
        let what = if thread.reworded {
            "Changed"
        } else {
            "Removed"
        };
        format!(" · {what} in version {}", version + 1)
    });
    let mut text = format!("#### {at} · {state}{earlier}\n");
    let about = match &thread.about {
        Some(About::Problem { message }) => message.as_str(),
        _ if !thread.selection.is_empty() => thread.selection.as_str(),
        _ => thread.quote.as_str(),
    };
    if !about.trim().is_empty() {
        text.push_str(&format!("\n{}\n", blockquote(about)));
    }
    if !thread.text.trim().is_empty() {
        text.push_str(&format!("\n{}\n", thread.text.trim_end()));
    }
    for reply in &thread.replies {
        let (verdict, who, words) = threads::reply_parts(reply);
        let who = match verdict {
            Some(verdict) => format!("{who} {}", verdict.to_lowercase()),
            None => who,
        };
        let words = words.trim_end();
        if words.is_empty() {
            text.push_str(&format!("\n**{who}.**\n"));
        } else {
            text.push_str(&format!("\n**{who}:** {words}\n"));
        }
    }
    text
}

/// The review as Markdown, or None when there's no comment to put in it.
pub(super) fn markdown(threads: &[Thread]) -> Option<String> {
    let mut kept: Vec<&Thread> = threads.iter().filter(|t| kept(t)).collect();
    if kept.is_empty() {
        return None;
    }
    // A file at a time by name whatever its case, as the pane lists them with no file in
    // view, a terminal's output last; then a kind at a time; then down the document.
    kept.sort_by_cached_key(|t| {
        (
            on_output(t),
            threads::place_of(t).to_lowercase(),
            kind_rank(&t.kind),
            t.line,
            t.id,
        )
    });
    let mut out = format!("# Review\n\n{}", summary(&kept));
    let mut place: Option<String> = None;
    let mut kind: Option<&str> = None;
    for thread in kept {
        let here = threads::place_of(thread);
        if place.as_ref() != Some(&here) {
            out.push_str(&format!("\n## {here}\n"));
            place = Some(here);
            kind = None;
        }
        let filed = kind_of(thread);
        if kind != Some(filed) {
            out.push_str(&format!("\n### {filed}\n"));
            kind = Some(filed);
        }
        out.push('\n');
        out.push_str(&entry(thread));
    }
    Some(out)
}

impl App {
    /// Copy Review as Markdown: the review on the clipboard.
    pub(super) fn copy_review(&mut self) {
        let Some(text) = markdown(&self.threads) else {
            self.say("No comments to copy");
            return;
        };
        let count = self.threads.iter().filter(|t| kept(t)).count();
        let what = counted(count, "comment", "comments");
        self.copy_text(text, &format!("Copied the review as Markdown: {what}"));
    }

    /// Save Review as Markdown…: asks for a name in the project, offering one not taken.
    pub(super) fn ask_where_to_save_review(&mut self) {
        if !self.threads.iter().any(kept) {
            self.say("No comments to save");
            return;
        }
        let root = self.workspace.root();
        let free = (1..)
            .map(|n| match n {
                1 => format!("{SAVED_AS}.md"),
                n => format!("{SAVED_AS}-{n}.md"),
            })
            .find(|name| !root.join(name).exists())
            .unwrap_or_default();
        self.overlay = Overlay::Prompt(Prompt {
            label: "Save review as".to_string(),
            input: free,
            action: PromptAction::SaveReview,
        });
    }

    /// The name typed: a Markdown file in the project, asked about first when it's there.
    pub(super) fn save_review_as(&mut self, name: &str) {
        let typed = Path::new(name);
        let outside = typed.is_absolute()
            || typed
                .components()
                .any(|part| matches!(part, std::path::Component::ParentDir));
        if outside {
            self.say("Save the review inside the project");
            return;
        }
        let name = if typed.extension().is_none() {
            format!("{name}.md")
        } else {
            name.to_string()
        };
        let path = self.workspace.root().join(&name);
        if path.exists() {
            self.overlay = Overlay::Confirm(Confirm {
                title: format!("Replace {name}?"),
                detail: "It's already in the project. The review takes its place.".to_string(),
                buttons: vec![
                    ("Replace", Choice::SaveReview(path)),
                    ("Cancel", Choice::Cancel),
                ],
                selected: 1,
            });
            return;
        }
        self.write_review(&path);
    }

    /// Writes the review there and opens it, as a page.
    pub(super) fn write_review(&mut self, path: &Path) {
        let Some(text) = markdown(&self.threads) else {
            self.say("No comments to save");
            return;
        };
        let written = path
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| std::fs::write(path, text));
        let name = self.relative(path);
        if let Err(e) = written {
            self.say(format!("Could not save {name}: {e}"));
            return;
        }
        // A tab already open on it shows what's there now, unless it holds edits of its own.
        let open = self
            .editors
            .iter_mut()
            .find(|e| e.buffer.path() == Some(path));
        if let Some(editor) = open.filter(|e| !e.buffer.is_dirty()) {
            let _ = editor.reload();
        }
        self.refresh_tree();
        if let Err(e) = self.open(path) {
            self.say(format!("Could not open {name}: {e}"));
            return;
        }
        self.say(format!("Saved the review to {name}"));
    }
}
