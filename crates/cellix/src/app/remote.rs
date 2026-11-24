//! Reviewing a remote doc, on screen: the palette's Review Remote Doc… and `cellix review
//! <link>` have Claude Code read the doc through the MCP server picked for it (`crate::remote`),
//! in the background as Ask runs it; the doc opens as a page, its tab named by its title with a
//! cloud, and a strip under the tabs says where it's from, with Sync and Open in Browser.
//! Someone else's comments come in as threads under their names. The reviewer's own are drafts
//! until a press on that one card's Post to Doc: nothing else ever posts, not Sync, not Enter.

use std::collections::HashMap;

use super::*;
use crate::ask::{self, Cli, News};
use crate::remote::{self as doc, Meta};
use crate::threads::{Polish, Polishes};
use cellix_review::threads::{Remote, Sent};

/// Who reads and posts: Claude Code, whose `--allowedTools` limits a run to one MCP server.
/// Codex and Gemini CLI would each need their own way to allow one server's tools only.
const READER: Cli = Cli::Claude;
/// The strip's buttons, from its right end: the menu with End Review, then these.
pub(super) const SYNC: &str = "Sync";
pub(super) const OPEN_IN_BROWSER: &str = "Open in Browser";
const STRIP_MENU: &str = "\u{22ef}";

/// A read of a doc under way: its link and folder, the server it's read through, and whether
/// it's a Sync of one already here.
pub(super) struct Reading {
    pub link: String,
    pub slug: String,
    pub server: String,
    pub sync: bool,
    pub running: ask::Running,
    /// What it said last, should its result come without the answer.
    pub said: String,
}

/// A post under way: the comment, and the reply by its place where it's one of those.
pub(super) struct Posting {
    pub thread: u32,
    pub reply: Option<usize>,
    pub running: ask::Running,
    pub said: String,
}

/// Polish under way for a draft, and how many words it had.
pub(super) struct Polishing {
    pub thread: u32,
    pub from: usize,
    pub running: ask::Running,
    pub said: String,
}

#[derive(Default)]
pub(super) struct Remotes {
    /// The docs this project has read, by their folder's name.
    pub docs: HashMap<String, Meta>,
    pub reading: Option<Reading>,
    pub posts: Vec<Posting>,
    pub polishing: Vec<Polishing>,
    /// What Polish has for each draft, as the cards show it.
    pub polish: Polishes,
    /// A link waiting for the MCP server to be picked.
    pub waiting: Option<String>,
    /// Claude Code's config as a test names it (`HOME`, `CLAUDE_CONFIG_DIR`). A test without
    /// one reads none, so none reads the person's own.
    pub env: Option<HashMap<String, String>>,
    /// How long a run may take; a test makes it short.
    pub limit: Option<std::time::Duration>,
}

/// How a draft's polish run ends: its suggestion, or why there's none.
enum Polished {
    Ready(String),
    Failed(String),
}

impl App {
    /// Takes in the docs this project has read, and says of a post that was under way when
    /// Cellix last closed that it may or may not have reached the doc.
    pub(super) fn load_remote_docs(&mut self) {
        let root = self.workspace.root().to_path_buf();
        self.remote.docs = doc::load_all(&root).into_iter().collect();
        let mut unfinished = false;
        let settle = |remote: &mut Option<Remote>, unfinished: &mut bool| {
            if let Some(remote) = remote.as_mut().filter(|r| r.state == Sent::Posting) {
                remote.state = Sent::Failed {
                    why: "Cellix closed before the doc answered. Look in the doc before you \
                          retry, so it isn't posted twice."
                        .to_string(),
                };
                *unfinished = true;
            }
        };
        for thread in &mut self.threads {
            settle(&mut thread.remote, &mut unfinished);
            for reply in &mut thread.replies {
                settle(&mut reply.remote, &mut unfinished);
            }
        }
        if unfinished {
            self.save_threads();
        }
    }

    /// Opens every remote doc this project has, in the tabs after those it had open.
    pub(super) fn reopen_remote_docs(&mut self) {
        let root = self.workspace.root().to_path_buf();
        let mut slugs: Vec<String> = self.remote.docs.keys().cloned().collect();
        slugs.sort();
        for slug in slugs {
            let path = root.join(doc::doc_file(&slug));
            let open = self.editors.iter().any(|e| e.buffer.path() == Some(&path));
            if !open && path.is_file() {
                let _ = self.open(&path);
            }
        }
    }

    /// What a remote doc is, where `path` is one.
    pub(super) fn remote_meta_of(&self, path: &Path) -> Option<&Meta> {
        let relative = self.relative(path);
        let slug = doc::slug_of(&relative)?;
        self.remote.docs.get(slug)
    }

    /// The remote doc in view, by its folder's name, and what it is.
    pub(super) fn remote_in_view(&self) -> Option<(String, &Meta)> {
        if self.task_shown() || self.review.is_some() || self.settings_page.is_some() {
            return None;
        }
        let path = self.editors.get(self.active())?.buffer.path()?;
        let relative = self.relative(path);
        let slug = doc::slug_of(&relative)?;
        let meta = self.remote.docs.get(slug)?;
        Some((slug.to_string(), meta))
    }

    /// Whether a tab of the strip is a remote doc's, which has a cloud for its icon.
    pub(super) fn tab_is_remote(&self, index: usize) -> bool {
        self.editors
            .get(index)
            .and_then(|e| e.buffer.path())
            .is_some_and(|path| self.remote_meta_of(path).is_some())
    }

    /// What reads the config Claude Code keeps its MCP servers in.
    fn remote_var(&self) -> impl Fn(&str) -> Option<String> + use<> {
        let env = self.remote.env.clone();
        move |name: &str| match &env {
            Some(env) => env.get(name).cloned(),
            None if cfg!(test) => None,
            None => std::env::var(name).ok(),
        }
    }

    fn remote_servers(&self) -> Vec<String> {
        doc::servers(&self.remote_var(), self.workspace.root())
    }

    fn remote_limit(&self) -> std::time::Duration {
        self.remote.limit.unwrap_or(doc::TIME_LIMIT)
    }

    /// Asks for a link to review, as the palette's Review Remote Doc… does.
    pub(super) fn ask_remote_link(&mut self) {
        self.ask("Link to the doc", PromptAction::ReviewRemote);
    }

    /// Reviews the doc at `link`: through the MCP server the setting names, or once one is
    /// picked; Claude Code reads it in the background, and it opens when it's read.
    pub(super) fn review_remote(&mut self, link: &str) {
        let link = link.trim().to_string();
        if link.is_empty() {
            self.say("Paste the doc's link to review it");
            return;
        }
        if self.remote.reading.is_some() {
            self.say("A doc is being read already. Wait for it, then try again");
            return;
        }
        // A doc already read is synced, not read afresh, so its drafts stay with it.
        let slug = doc::slug(&link);
        if self.remote.docs.contains_key(&slug) {
            self.open_remote(&slug);
            self.sync_remote_doc();
            return;
        }
        let servers = self.remote_servers();
        let named = self.settings.remote_docs_mcp.clone();
        match named.filter(|name| servers.contains(name)) {
            Some(server) => self.start_reading(&link, &server, false),
            None if servers.is_empty() => self.say(
                "Claude Code has no MCP servers set up here. Add your doc tool's (claude mcp \
                 add), then review again",
            ),
            None => {
                self.remote.waiting = Some(link);
                self.pick_remote_server((self.main_area().x + 2, 2));
            }
        }
    }

    /// Lists the MCP servers Claude Code knows, to pick the doc tool's.
    pub(super) fn pick_remote_server(&mut self, at: (u16, u16)) {
        let servers = self.remote_servers();
        if servers.is_empty() {
            self.say("Claude Code has no MCP servers set up here");
            return;
        }
        let picked = self.settings.remote_docs_mcp.clone();
        let mut items = vec![MenuItem::heading("Read remote docs through")];
        for server in servers {
            let mut item = MenuItem::new(server.clone(), MenuAction::RemoteServer(server.clone()));
            if picked.as_deref() == Some(server.as_str()) {
                item.detail = Some("in use".into());
            }
            items.push(item);
        }
        self.show_menu(at, items);
    }

    /// Keeps the server picked for every review on this machine, and reads the doc waiting.
    pub(super) fn picked_remote_server(&mut self, server: String) {
        if let Err(e) = crate::settings::Settings::store(
            "remote_docs_mcp",
            serde_json::Value::String(server.clone()),
        ) {
            self.say(format!("Couldn't keep the pick in settings: {e}"));
        }
        self.settings.remote_docs_mcp = Some(server.clone());
        if let Some(link) = self.remote.waiting.take() {
            self.start_reading(&link, &server, false);
        } else {
            self.say(format!("Remote docs are read through {server}"));
        }
    }

    /// The command line that reads and posts, found on the PATH Ask uses.
    fn remote_program(&self) -> Option<PathBuf> {
        ask::find(READER.program(), &self.side_path())
    }

    fn start_remote_run(&self, args: Vec<String>, prompt: &str) -> Option<ask::Running> {
        let program = self.remote_program()?;
        let run = ask::Run {
            cli: READER,
            args,
            root: self.workspace.root(),
            prompt,
        };
        Some(ask::start_run(
            &program,
            run,
            &self.side_path(),
            self.remote_limit(),
        ))
    }

    fn start_reading(&mut self, link: &str, server: &str, sync: bool) {
        if self.remote_program().is_none() {
            self.say("Reviewing a remote doc needs Claude Code installed here");
            return;
        }
        let servers = self.remote_servers();
        let args = doc::tool_args(server, &servers);
        let prompt = doc::read_prompt(server, link);
        let Some(running) = self.start_remote_run(args, &prompt) else {
            return;
        };
        self.remote.reading = Some(Reading {
            link: link.to_string(),
            slug: doc::slug(link),
            server: server.to_string(),
            sync,
            running,
            said: String::new(),
        });
    }

    /// Reads the doc in view again: new comments and replies come in, changed ones are
    /// brought up to date and the doc's text replaced. Nothing is posted.
    pub(super) fn sync_remote_doc(&mut self) {
        if self.remote.reading.is_some() {
            return;
        }
        let Some((_, meta)) = self.remote_in_view() else {
            return;
        };
        let (link, server) = (meta.link.clone(), meta.server.clone());
        let servers = self.remote_servers();
        // The server it was read through, while Claude Code still has it.
        let server = if servers.contains(&server) {
            server
        } else {
            match self.settings.remote_docs_mcp.clone() {
                Some(named) if servers.contains(&named) => named,
                _ => {
                    self.say(format!("Claude Code no longer has the MCP server {server}"));
                    return;
                }
            }
        };
        self.start_reading(&link, &server, true);
    }

    /// Opens a remote doc that's been read, with its comments beside it.
    fn open_remote(&mut self, slug: &str) {
        let path = self.workspace.root().join(doc::doc_file(slug));
        if let Err(e) = self.open(&path) {
            self.say(format!("Could not open the doc: {e}"));
            return;
        }
        self.dock.open = true;
        self.dock.show_comments();
        self.dock.focused = false;
    }

    /// What the status bar says Cellix is waiting on for remote docs, with its spinner.
    pub(super) fn remote_waiting(&self) -> Option<String> {
        if let Some(reading) = &self.remote.reading {
            let what = if reading.sync { "Syncing" } else { "Reading" };
            return Some(format!("{what} the doc with {}", READER.name()));
        }
        if !self.remote.posts.is_empty() {
            return Some("Posting to the doc".to_string());
        }
        (!self.remote.polishing.is_empty()).then(|| "Polishing".to_string())
    }

    /// Takes in what the reads, posts and polishes said since. True when anything changed.
    pub(super) fn poll_remote(&mut self) -> bool {
        let mut changed = false;
        if let Some(reading) = self.remote.reading.as_mut() {
            match drain(&reading.running, &mut reading.said) {
                Some(Ok(answer)) => {
                    let reading = self.remote.reading.take();
                    if let Some(reading) = reading {
                        self.finish_reading(reading, &answer);
                    }
                    changed = true;
                }
                Some(Err(why)) => {
                    self.remote.reading = None;
                    self.say(format!("Couldn't read the doc: {why}"));
                    changed = true;
                }
                None => {}
            }
        }
        let mut ended = Vec::new();
        for (index, post) in self.remote.posts.iter_mut().enumerate() {
            if let Some(result) = drain(&post.running, &mut post.said) {
                ended.push((index, post.thread, post.reply, result));
            }
        }
        for (index, thread, reply, result) in ended.into_iter().rev() {
            self.remote.posts.remove(index);
            self.finish_post(thread, reply, result);
            changed = true;
        }
        let mut polished = Vec::new();
        for (index, run) in self.remote.polishing.iter_mut().enumerate() {
            if let Some(result) = drain(&run.running, &mut run.said) {
                let result = match result {
                    Ok(text) if !text.trim().is_empty() => Polished::Ready(text),
                    Ok(_) => Polished::Failed("it gave no text".to_string()),
                    Err(why) => Polished::Failed(why),
                };
                polished.push((index, run.thread, run.from, result));
            }
        }
        for (index, thread, from, result) in polished.into_iter().rev() {
            self.remote.polishing.remove(index);
            let shown = match result {
                Polished::Ready(text) => Polish::Ready {
                    text: clean_polish(&text),
                    by: READER.name().to_string(),
                    from,
                },
                Polished::Failed(why) => Polish::Failed(why),
            };
            self.remote.polish.insert(thread, shown);
            changed = true;
        }
        changed
    }

    /// A read's answer, written as the doc and its comments, and the doc shown.
    fn finish_reading(&mut self, reading: Reading, answer: &str) {
        let read = match doc::answer_json(answer).and_then(|v| doc::fetched(&v, &reading.link)) {
            Ok(read) => read,
            Err(why) => {
                self.say(format!("Couldn't read the doc: {why}"));
                return;
            }
        };
        let root = self.workspace.root().to_path_buf();
        let file = doc::doc_file(&reading.slug);
        let added = doc::merge(&mut self.threads, &file, &read);
        let others = self
            .threads
            .iter()
            .filter(|t| t.file == file && !t.author.is_empty())
            .count();
        let meta = Meta {
            title: read.title.clone(),
            link: read.link.clone(),
            version: read.version.clone(),
            server: reading.server.clone(),
            synced: cellix_review::threads::now(),
            others,
        };
        if let Err(e) = doc::write(&root, &reading.slug, &read, &meta) {
            self.say(format!("Couldn't keep the doc: {e}"));
            return;
        }
        self.remote.docs.insert(reading.slug.clone(), meta);
        self.threads_seen = None;
        self.save_threads();
        // A Sync puts the doc's new text in its tab; a first read opens it.
        let path = root.join(&file);
        if let Some(editor) = self
            .editors
            .iter_mut()
            .find(|e| e.buffer.path() == Some(path.as_path()))
        {
            let _ = editor.reload();
            editor.label = Some(read.title.clone());
        }
        if reading.sync {
            let news = match added {
                0 => "Synced: no new comments".to_string(),
                1 => "Synced: 1 new comment".to_string(),
                n => format!("Synced: {n} new comments"),
            };
            self.say(news);
        } else {
            self.open_remote(&reading.slug);
            self.say(format!("Read {} with {}", read.title, READER.name()));
        }
    }

    /// Posts one comment, or one reply, of the reviewer's to the doc: only ever from a press
    /// on its own Post button. One already posting, or posted, isn't posted again.
    pub(super) fn post_remote(&mut self, id: u32, reply: Option<usize>) {
        let Some(thread) = self.threads.iter().find(|t| t.id == id) else {
            return;
        };
        let Some(slug) = doc::slug_of(&thread.file).map(str::to_string) else {
            return;
        };
        let Some(meta) = self.remote.docs.get(&slug).cloned() else {
            return;
        };
        let (state, prompt) = match reply {
            None => {
                let Some(remote) = thread.remote.as_ref().filter(|_| thread.author.is_empty())
                else {
                    return;
                };
                let prompt =
                    doc::post_prompt(&meta.server, &meta.link, &thread.quote, &thread.text);
                (remote.state.clone(), prompt)
            }
            Some(n) => {
                let Some((answer, remote)) = thread
                    .replies
                    .get(n)
                    .filter(|r| r.author == YOU)
                    .and_then(|r| Some((r, r.remote.as_ref()?)))
                else {
                    return;
                };
                // A reply goes under the doc's own comment, by its id there.
                let Some(on) = thread
                    .remote
                    .as_ref()
                    .map(|r| r.id.clone())
                    .filter(|id| !id.is_empty())
                else {
                    self.say("Post the comment first: a reply goes under it in the doc");
                    return;
                };
                let prompt = doc::reply_prompt(&meta.server, &meta.link, &on, &answer.text);
                (remote.state.clone(), prompt)
            }
        };
        if matches!(state, Sent::Posting | Sent::Posted { .. }) {
            return;
        }
        if self.remote_program().is_none() {
            self.say("Posting to a remote doc needs Claude Code installed here");
            return;
        }
        let servers = self.remote_servers();
        let args = doc::tool_args(&meta.server, &servers);
        let Some(running) = self.start_remote_run(args, &prompt) else {
            return;
        };
        self.set_remote_state(id, reply, Sent::Posting);
        self.remote.posts.push(Posting {
            thread: id,
            reply,
            running,
            said: String::new(),
        });
    }

    fn set_remote_state(&mut self, id: u32, reply: Option<usize>, state: Sent) {
        let Some(thread) = self.threads.iter_mut().find(|t| t.id == id) else {
            return;
        };
        let remote = match reply {
            None => thread.remote.as_mut(),
            Some(n) => thread.replies.get_mut(n).and_then(|r| r.remote.as_mut()),
        };
        if let Some(remote) = remote {
            remote.state = state;
        }
        self.threads_seen = None;
        self.save_threads();
    }

    /// A post's end: the doc's id kept and the card saying Posted, or the draft kept as it
    /// was, saying why it didn't go.
    fn finish_post(&mut self, id: u32, reply: Option<usize>, result: Result<String, String>) {
        match result.and_then(|answer| doc::posted_id(&answer)) {
            Ok(posted) => {
                let now = cellix_review::threads::now();
                if let Some(thread) = self.threads.iter_mut().find(|t| t.id == id) {
                    let remote = match reply {
                        None => thread.remote.as_mut(),
                        Some(n) => thread.replies.get_mut(n).and_then(|r| r.remote.as_mut()),
                    };
                    if let Some(remote) = remote {
                        *remote = Remote::posted(posted, now);
                    }
                }
                self.threads_seen = None;
                self.save_threads();
                self.remote.polish.remove(&id);
                let what = if reply.is_some() { "reply" } else { "comment" };
                self.say(format!("Posted your {what} to the doc"));
            }
            Err(why) => {
                let why = doc::failure_words(&why);
                self.set_remote_state(id, reply, Sent::Failed { why });
                self.say("Couldn't post it. The card says why");
            }
        }
    }

    /// Puts a comment's words, or a reply's, on the clipboard, to paste into the doc.
    pub(super) fn copy_remote(&mut self, id: u32, reply: Option<usize>) {
        let Some(thread) = self.threads.iter().find(|t| t.id == id) else {
            return;
        };
        let text = match reply {
            None => thread.text.clone(),
            Some(n) => match thread.replies.get(n) {
                Some(reply) => reply.text.clone(),
                None => return,
            },
        };
        self.copy_text(text, "Copied. Paste it in the doc");
    }

    /// Has Claude Code make a draft short, clear and kind, with no tools: its suggestion
    /// shows under the draft, to use or not.
    pub(super) fn polish_remote(&mut self, id: u32) {
        let Some(thread) = self.threads.iter().find(|t| t.id == id && t.unposted()) else {
            return;
        };
        if self.remote.polishing.iter().any(|p| p.thread == id) {
            return;
        }
        let text = self
            .editors
            .iter()
            .find(|e| {
                e.buffer
                    .path()
                    .is_some_and(|p| self.relative(p) == thread.file)
            })
            .map(|e| e.buffer.text())
            .unwrap_or_default();
        let paragraph = doc::paragraph_around(&text, thread.anchor());
        let prompt = doc::polish_prompt(&thread.text, &thread.quote, &paragraph);
        let from = doc::words(&thread.text);
        if self.remote_program().is_none() {
            self.say("Polish needs Claude Code installed here");
            return;
        }
        let Some(running) = self.start_remote_run(doc::polish_args(), &prompt) else {
            return;
        };
        self.remote.polishing.push(Polishing {
            thread: id,
            from,
            running,
            said: String::new(),
        });
        self.remote
            .polish
            .insert(id, Polish::Running(READER.name().to_string()));
    }

    /// Takes Polish's suggestion as the draft's words. It's still a draft.
    pub(super) fn use_polish(&mut self, id: u32) {
        let Some(Polish::Ready { text, .. }) = self.remote.polish.remove(&id) else {
            return;
        };
        if let Some(thread) = self.threads.iter_mut().find(|t| t.id == id && t.unposted()) {
            thread.text = text;
        }
        self.threads_seen = None;
        self.save_threads();
    }

    pub(super) fn keep_mine(&mut self, id: u32) {
        self.remote.polish.remove(&id);
        self.remote.polishing.retain(|p| p.thread != id);
    }

    /// Opens the doc a comment is on in the system's browser, where the doc tool's sign-in is.
    pub(super) fn open_remote_in_browser(&mut self, slug: Option<&str>) {
        let slug = match slug {
            Some(slug) => slug.to_string(),
            None => match self.remote_in_view() {
                Some((slug, _)) => slug,
                None => return,
            },
        };
        let Some(link) = self.remote.docs.get(&slug).map(|m| m.link.clone()) else {
            return;
        };
        let opener = self.system_browser.clone();
        if opener.is_empty() {
            self.copy_text(link, "No browser here: the link is copied");
            return;
        }
        let (done, result) = crate::wake::channel();
        let page = link.clone();
        std::thread::spawn(move || {
            let _ = done.send(ran_well(&opener, &page));
        });
        self.browser_result = Some((link, result));
    }

    /// The doc a comment is on, opened in the browser from its card.
    pub(super) fn open_comment_in_doc(&mut self, id: u32) {
        let slug = self
            .threads
            .iter()
            .find(|t| t.id == id)
            .and_then(|t| doc::slug_of(&t.file).map(str::to_string));
        if let Some(slug) = slug {
            self.open_remote_in_browser(Some(&slug));
        }
    }

    /// The strip's buttons, each its words, its cells and the column it starts at, from the
    /// left: Sync, Open in Browser and the menu, the last a column short of the sheet's end.
    pub(super) fn remote_strip_buttons(&self) -> Vec<(&'static str, String, u16)> {
        let main = self.main_area();
        let mut end = main.right().saturating_sub(1);
        let mut placed = Vec::new();
        for label in [STRIP_MENU, OPEN_IN_BROWSER, SYNC] {
            let cells = measures::button_cells(label, &*self.measure);
            let x = end.saturating_sub(cells.chars().count() as u16);
            placed.push((label, cells, x));
            end = x.saturating_sub(BUTTON_GAP);
        }
        placed.reverse();
        placed
    }

    /// The strip under the tabs of a remote doc: where it's from and who read it, its
    /// version, when it was synced and how many comments are others', then its buttons.
    pub(super) fn draw_remote_strip(&self, grid: &mut Grid) -> bool {
        let Some((_, meta)) = self.remote_in_view().filter(|_| self.chrome_rows() == 2) else {
            return false;
        };
        let main = self.main_area();
        let theme = &self.theme;
        let style = theme.breadcrumb;
        let y = 1;
        grid.fill(Rect::new(main.x, y, main.w, 1), style);
        let buttons = self.remote_strip_buttons();
        let end = buttons
            .first()
            .map_or(main.right(), |(_, _, x)| x.saturating_sub(2));
        // The cloud's left edge under the tab's.
        let mut x = main.x as f32 + 1.05;
        if !self.measure.in_cells() {
            let size = 0.62;
            let across = size * 2.0;
            grid.paint.push(cellix_render::Paint::Icon {
                bounds: Bounds::cells(x, f32::from(y) + (1.0 - size) / 2.0, across, size),
                icon: Icon::Cloud,
                color: theme.status.fg,
            });
            x += across + 0.6;
        } else {
            x += 0.45;
        }
        let from = format!("From {}, read by {}", meta.server, READER.name());
        let strong = Style {
            fg: theme.text.fg,
            ..style
        };
        let after = grid.label_at(x, x.floor() as u16, y, &from, strong, end, 90);
        let ago = crate::threads::age(meta.synced, cellix_review::threads::now());
        let version = (!meta.version.is_empty()).then(|| format!("version {}", meta.version));
        let synced = ago.map(|ago| {
            if ago == "now" {
                "synced just now".to_string()
            } else {
                format!("synced {ago} ago")
            }
        });
        let others = Some(match meta.others {
            1 => "1 comment from others".to_string(),
            n => format!("{n} comments from others"),
        });
        let quiet = Style {
            fg: theme.status.fg,
            ..style
        };
        let font = cellix_render::Font {
            size: 90,
            ..cellix_render::Font::default()
        };
        // Two columns after the words as they're painted, which are narrower than their
        // cells: placed by the cells, the rest had no room beside Sync.
        let at = x + self.measure.width(&from, font) + 2.0;
        let cell = (after + 2).min(end);
        // As many of them as fit whole: the version goes first, then the count of others'
        // comments, which the Comments pane says too; when it was synced, beside Sync, stays.
        // Cut, "version 12" read "vers".
        let room = f32::from(end) - at;
        let mut parts = [version, synced, others];
        let joined = |parts: &[Option<String>; 3]| {
            parts
                .iter()
                .flatten()
                .cloned()
                .collect::<Vec<_>>()
                .join(" \u{b7} ")
        };
        for drop in [0, 2, 1] {
            if self.measure.width(&joined(&parts), font) <= room {
                break;
            }
            parts[drop] = None;
        }
        grid.label_at(at, cell, y, &joined(&parts), quiet, end, 90);
        // Its buttons quiet words, as a card's are, washed only under the pointer; the menu
        // its mark, as the toolbar's are.
        for (label, cells, x) in buttons {
            let width = cells.chars().count() as u16;
            let behind = grid.cell(x, y).style.bg;
            let on_it = self
                .hover
                .is_some_and(|h| h.1 == y && h.0 >= x && h.0 < x + width);
            let fill = if on_it {
                theme.secondary(behind, false)
            } else {
                behind
            };
            let words = Style {
                fg: theme.text.fg,
                bg: fill,
                ..style
            };
            let busy = label == SYNC && self.remote.reading.as_ref().is_some_and(|r| r.sync);
            let label = if busy { "Syncing" } else { label };
            if label == STRIP_MENU && !self.measure.in_cells() {
                grid.button(x, y, &cells, "", fill, words);
                let spot = Rect::new(x, y, width, 1);
                crate::toolbar::draw_icon(grid, (Icon::More, 0.6), spot, theme.text.fg, words);
            } else {
                grid.button(x, y, &cells, label, fill, words);
            }
        }
        // A line under it, as under the tabs, where rows have room for one.
        if !self.measure.in_cells() {
            let line = Bounds::cells(
                f32::from(main.x),
                f32::from(y) + 1.0,
                f32::from(main.w),
                0.0,
            );
            grid.hairline(line, theme.border);
        }
        true
    }

    /// A press on the strip: Sync, Open in Browser, or its menu.
    pub(super) fn click_remote_strip(&mut self, x: u16) -> bool {
        let Some((slug, _)) = self.remote_in_view() else {
            return false;
        };
        let pressed = self
            .remote_strip_buttons()
            .into_iter()
            .find(|(_, cells, at)| x >= *at && x < at + cells.chars().count() as u16);
        match pressed.map(|(label, _, at)| (label, at)) {
            Some((SYNC, _)) => self.sync_remote_doc(),
            Some((OPEN_IN_BROWSER, _)) => self.open_remote_in_browser(Some(&slug)),
            Some((_, at)) => {
                let items = vec![
                    MenuItem::new("Read Through Another Server…", MenuAction::PickRemoteServer),
                    MenuItem::new("End Review", MenuAction::EndReview(slug)),
                ];
                self.show_menu((at, 2), items);
            }
            None => return false,
        }
        true
    }

    /// Asks before End Review takes the doc and its comments off this machine.
    pub(super) fn ask_end_review(&mut self, slug: String) {
        let title = self
            .remote
            .docs
            .get(&slug)
            .map_or_else(|| "this doc".to_string(), |m| m.title.clone());
        let file = doc::doc_file(&slug);
        let drafts = self
            .threads
            .iter()
            .filter(|t| t.file == file && t.unposted())
            .count();
        let drafts = match drafts {
            0 => String::new(),
            1 => " Your 1 draft that isn't posted goes too.".to_string(),
            n => format!(" Your {n} drafts that aren't posted go too."),
        };
        self.overlay = Overlay::Confirm(Confirm {
            title: "End this review?".to_string(),
            detail: format!(
                "{title} and its comments are removed from this machine. The doc itself \
                 stays as it is.{drafts}"
            ),
            buttons: vec![
                ("End Review", Choice::EndReview(slug)),
                ("Cancel", Choice::Cancel),
            ],
            selected: 1,
        });
    }

    /// Takes a remote doc off this machine: its folder, its tab and its comments.
    pub(super) fn end_review(&mut self, slug: &str) {
        let root = self.workspace.root().to_path_buf();
        let file = doc::doc_file(slug);
        let path = root.join(&file);
        if let Some(index) = self
            .editors
            .iter()
            .position(|e| e.buffer.path() == Some(path.as_path()))
        {
            self.set_active(index);
            self.close_tab();
        }
        let ids: Vec<u32> = self
            .threads
            .iter()
            .filter(|t| t.file == file)
            .map(|t| t.id)
            .collect();
        self.threads.retain(|t| t.file != file);
        for id in ids {
            self.remote.polish.remove(&id);
            self.remote.posts.retain(|p| p.thread != id);
            self.remote.polishing.retain(|p| p.thread != id);
        }
        self.threads_seen = None;
        self.save_threads();
        self.remote.docs.remove(slug);
        let _ = std::fs::remove_dir_all(doc::folder(&root, slug));
        self.say("Review ended: the doc and its comments are gone from this machine");
    }
}

/// What a run said since, up to its end: its answer, or why there's none. None while it's
/// still going.
fn drain(running: &ask::Running, said: &mut String) -> Option<Result<String, String>> {
    for news in running.news.try_iter() {
        match news {
            News::Fresh => said.clear(),
            News::Text(words) => said.push_str(&words),
            News::Said(words) => *said = words,
            News::Done(words) => {
                let answer = if words.trim().is_empty() {
                    said.clone()
                } else {
                    words
                };
                return Some(Ok(answer));
            }
            News::Failed(failure) => return Some(Err(failure.words(READER))),
            News::Session(_) | News::Doing(_) => {}
        }
    }
    None
}

/// Polish's text as it's shown: without a fence or quotes round it.
fn clean_polish(text: &str) -> String {
    let text = text.trim();
    let text = text
        .strip_prefix("```")
        .and_then(|t| t.strip_suffix("```"))
        .map_or(text, |t| {
            t.trim_start_matches(|c: char| c.is_alphanumeric())
        });
    let text = text.trim();
    let quoted = text.len() > 1
        && ((text.starts_with('"') && text.ends_with('"'))
            || (text.starts_with('\u{201c}') && text.ends_with('\u{201d}')));
    if quoted {
        let inner: String = text.chars().skip(1).collect();
        let mut inner = inner;
        inner.pop();
        inner.trim().to_string()
    } else {
        text.to_string()
    }
}
