//! A right click on a link, wherever Cellix shows one the person can click, opens a menu at the
//! pointer: Open Link, which does what a click (or Ctrl+click, where a click is something
//! else's) does, and Copy Link, which puts its address on the clipboard the way every copy goes,
//! by OSC 52 through the terminal, so it reaches the person's clipboard over SSH and inside tmux
//! too. The address is what the link goes to, a Markdown link's target and not its words. A
//! link to a file, as a `file:line` in a terminal's output, has Open File and Copy Path instead,
//! and a picture's name on a comment's card Open Picture and Copy Path. Where a right click
//! already opens a menu, as on a page, the link's two rows stand at its top, over a rule.
//! A link's menu drops from under the pointer's row, at its column, so the link it's about
//! stays in view: from the pointer's own row it covered all of the address but its start.
//!
//! The places: a comment's words and its replies in the Comments pane; a page's text, rendered
//! or as source, in any file (transcripts are pages); a terminal's output in the side pane, and
//! in peek.

use super::*;

/// What a link goes to.
pub(super) enum Linked {
    /// A web address, or anything else a browser opens.
    Web(String),
    /// A file, at a line where the link names one.
    File(PathBuf, Option<usize>),
    /// A picture a comment names, by the name it's given there.
    Picture(String),
    /// Another comment, by its number: "#33", "comment 33".
    Comment(u32),
}

impl App {
    /// The two rows a link's menu starts with.
    pub(super) fn link_items(&self, linked: Linked) -> Vec<MenuItem> {
        match linked {
            Linked::Web(address) => vec![
                MenuItem::new("Open Link", MenuAction::OpenLink(address.clone())),
                MenuItem::new("Copy Link", MenuAction::CopyLink(address)),
            ],
            Linked::File(path, line) => vec![
                MenuItem::new("Open File", MenuAction::OpenFile(path.clone(), line)),
                MenuItem::new("Copy Path", MenuAction::CopyPath(path)),
            ],
            Linked::Picture(name) => {
                let path = self.comment_picture_path(&name);
                vec![
                    MenuItem::new("Open Picture", MenuAction::OpenFile(path.clone(), None)),
                    MenuItem::new("Copy Path", MenuAction::CopyPath(path)),
                ]
            }
            Linked::Comment(id) => vec![MenuItem::new(
                format!("Go to Comment #{id}"),
                MenuAction::ShowComment(id),
            )],
        }
    }

    /// A link's menu, dropping from under the row at `(x, y)`, the pointer's. Over a card, as
    /// a comment's or peek's, it's every menu's colour, its line and shadow setting it off.
    pub(super) fn show_link_menu(&mut self, (x, y): (u16, u16), items: Vec<MenuItem>) {
        self.show_menu((x, y + 1), items);
    }

    /// A menu that already had `rest` starts with the link's two rows, a rule between.
    pub(super) fn with_link(&self, linked: Linked, mut rest: Vec<MenuItem>) -> Vec<MenuItem> {
        if let Some(first) = rest.first_mut() {
            first.rule = true;
        }
        let mut items = self.link_items(linked);
        items.extend(rest);
        items
    }

    /// Goes where a link goes, as a click on it does.
    pub(super) fn open_linked(&mut self, linked: Linked) {
        match linked {
            Linked::Web(address) => self.open_page(address),
            Linked::File(path, line) => self.open_file_at(&path, line),
            Linked::Picture(name) => self.open_comment_picture(&name),
            Linked::Comment(id) => self.go_to_comment(id),
        }
    }

    /// Goes to the comment a mention names, as a press on a line's ◆ does: the Comments pane
    /// on its card, lit and in view, and its words in the file. One resolved is listed with the
    /// resolved ones, and one not here says so.
    pub(super) fn go_to_comment(&mut self, id: u32) {
        let Some(thread) = self.threads.iter().find(|t| t.id == id) else {
            self.say(format!("There's no comment #{id} in this project"));
            return;
        };
        let resolved = thread.resolved;
        // A kind listed alone would hide it.
        if self.comment_kind.is_some_and(|kind| kind != thread.kind) {
            self.comment_kind = None;
        }
        if resolved {
            self.show_resolved = true;
        }
        self.show_thread(id);
        if resolved {
            self.say(format!("Comment #{id} is resolved. Reopen brings it back"));
        }
    }

    /// Opens a file at a line, counting from one, and takes the keys to it.
    pub(super) fn open_file_at(&mut self, path: &Path, line: Option<usize>) {
        if let Err(e) = self.open(path) {
            self.say(format!("Could not open {}: {e}", self.relative(path)));
            return;
        }
        let area = self.focused_area();
        if let (Some(line), Some(editor)) = (line, self.editor_mut()) {
            editor.go_to_line(line, area);
        }
        self.dock.focused = false;
    }

    /// The `index`th link of comment `id`, counting through its words and then its replies, as
    /// its card shows them: a web address, or the name of a picture that can't stand on it.
    pub(super) fn comment_linked(&self, id: u32, index: usize) -> Option<Linked> {
        let view = self.comments_view();
        let thread = self.threads.iter().find(|t| t.id == id)?;
        let link = threads::links(thread, view).into_iter().nth(index)?;
        Some(if let Some(id) = threads::comment_ref(&link) {
            Linked::Comment(id)
        } else if link.contains("://") {
            Linked::Web(link)
        } else {
            Linked::Picture(link)
        })
    }

    /// A right click in the Comments pane: on a card, Copy where words on the cards are
    /// selected, Copy Comment and Copy Quote; on a link, the link's rows over those (the user,
    /// 20 November: "I cannot do copy paste from comments, which is a crucial feature").
    pub(super) fn comment_link_menu(&mut self, screen: Rect, at: (u16, u16, f32)) {
        let top = self.dock.tops.of(Front::Comments);
        let (hit, card) = self.card_rows(|shown, measure| {
            let hit = threads::hit(screen, shown, top, at, measure);
            (hit, threads::card_at(screen, shown, top, at.1, measure))
        });
        let mut items = Vec::new();
        if self.card_selected_text().is_some() {
            items.push(MenuItem::run("Copy", Command::Copy));
        }
        if let Some(thread) = card.and_then(|id| self.threads.iter().find(|t| t.id == id)) {
            let words = thread.text.clone();
            items.push(MenuItem::new(
                "Copy Comment",
                MenuAction::CopyWords(words, "Copied the comment"),
            ));
            // What it's about, as its agent is told it.
            let about = if thread.selection.is_empty() {
                &thread.quote
            } else {
                &thread.selection
            };
            if !about.trim().is_empty() {
                items.push(MenuItem::new(
                    "Copy Quote",
                    MenuAction::CopyWords(about.clone(), "Copied the words it's about"),
                ));
            }
        }
        let linked = match hit {
            Some(ThreadHit::Link(id, index)) => self.comment_linked(id, index),
            _ => None,
        };
        let items = match linked {
            Some(linked) if items.is_empty() => self.link_items(linked),
            Some(linked) => self.with_link(linked, items),
            None => items,
        };
        if !items.is_empty() {
            self.show_link_menu((at.0, at.1), items);
        }
    }

    /// What a link in session `index`'s text at `spot` goes to: a web address, or a file of
    /// the project its output names, as Ctrl+click opens them.
    pub(super) fn session_linked(&self, index: usize, spot: (u16, u16)) -> Option<Linked> {
        let session = self.dock.sessions.get(index)?;
        if let Some(address) = session.link_at(spot) {
            return Some(Linked::Web(address));
        }
        if let Some(id) = session.comment_at(spot) {
            return Some(Linked::Comment(id));
        }
        let (file, line) = session.path_at(spot)?;
        let (path, _) = self.file_named(&file)?;
        Some(Linked::File(path, line))
    }

    /// A right click on a terminal's text at `at`, its place in it being `spot`: on a link,
    /// its menu. Returns whether it was on one; elsewhere the click is what it was before.
    pub(super) fn session_link_menu(
        &mut self,
        index: usize,
        spot: (u16, u16),
        at: (u16, u16),
    ) -> bool {
        let Some(linked) = self.session_linked(index, spot) else {
            return false;
        };
        let items = self.link_items(linked);
        self.show_link_menu(at, items);
        true
    }

    /// What a link at char `at` of the file in front goes to, the pointer being on row `y` of
    /// `area`: a page's row that links to a file, as a paper's `\input` does, or the address
    /// of a link in the text there.
    pub(super) fn editor_linked(&self, area: Rect, y: u16, at: usize) -> Option<Linked> {
        let editor = self.editors.get(self.active())?;
        if editor.rendered
            && let Some(Target::File(path)) = editor.row_target(area, y)
        {
            return Some(Linked::File(path, None));
        }
        let rope = editor.buffer.rope();
        let at = at.min(rope.len_chars());
        let line = rope.char_to_line(at);
        let text = rope.line(line).to_string();
        let address = rendered::link_at(text.trim_end(), at - rope.line_to_char(line))?;
        if address.contains("://") || address.starts_with("mailto:") {
            return Some(Linked::Web(address));
        }
        // A link to a file beside the page, as `[notes](notes.md)`; one to a place on the page
        // itself, or to nothing that's there, isn't one to open or to copy.
        let file = address.split('#').next().filter(|file| !file.is_empty())?;
        let root = self.workspace.root();
        let folder = editor.buffer.path().and_then(Path::parent).unwrap_or(root);
        let path = folder.join(file);
        path.is_file().then_some(Linked::File(path, None))
    }
}
