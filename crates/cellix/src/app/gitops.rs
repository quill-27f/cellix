//! Git beyond staging and committing: conflicts, stashes, comparing with another version,
//! and history. Kept apart from `app.rs`, which reaches here through a handful of calls.

use super::*;

use cellix_review::conflict::{self, Operation, Resolution};
use cellix_review::{Keep, apply_stash, drop_stash, pop_stash, stash, stashes};

use crate::palette::GitPick;
use crate::review::Fixed;
use crate::source::{Hit as SourceHit, StashAction};

/// How many commits Compare With lists.
const COMPARE_COMMITS: usize = 20;

/// A question about git, answered in a Confirm.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum GitChoice {
    Abort(Operation),
    DropStash(String),
}

impl App {
    /// Runs a git command from the palette, a menu or a key. Returns false for anything else.
    pub(super) fn run_git(&mut self, command: Command) -> bool {
        match command {
            Command::Stash => self.ask(
                "Stash message (optional)",
                PromptAction::Stash(Keep::Tracked),
            ),
            Command::StashUntracked => {
                self.ask(
                    "Stash message (optional)",
                    PromptAction::Stash(Keep::Untracked),
                );
            }
            Command::StashStaged => {
                self.ask(
                    "Stash message (optional)",
                    PromptAction::Stash(Keep::Staged),
                );
            }
            Command::PopStash => match stashes(self.workspace.root()).first() {
                Some(newest) => self.stash_action(StashAction::Pop, newest.hash.clone()),
                None => self.say("There are no stashes"),
            },
            Command::ApplyStash => self.stash_picker(StashAction::Apply),
            Command::PickStashToPop => self.stash_picker(StashAction::Pop),
            Command::DropStash => self.stash_picker(StashAction::Drop),
            Command::CompareWith => self.compare_with_list(),
            Command::ShowHistory => {
                if !self.rail_on(Command::ShowSourceControl) {
                    self.run(Command::ShowSourceControl);
                }
                self.source.show_graph();
            }
            Command::FileHistory => self.file_history_list(),
            Command::AcceptCurrent => self.settle_conflict(Resolution::Current),
            Command::AcceptIncoming => self.settle_conflict(Resolution::Incoming),
            Command::AcceptBoth => self.settle_conflict(Resolution::Both),
            Command::CompareConflict => self.compare_conflict(),
            Command::NextConflict => self.step_conflict(true),
            Command::PreviousConflict => self.step_conflict(false),
            Command::AbortMerge => self.ask_abort(),
            _ => return false,
        }
        true
    }

    /// Puts changes aside, once the prompt for its message is answered.
    pub(super) fn finish_stash(&mut self, keep: Keep, message: &str) {
        match stash(self.workspace.root(), message, keep) {
            Ok(()) => {
                self.say("Changes stashed");
                self.after_git_moved_files();
            }
            Err(why) => self.say(format!("Git refused: {why}")),
        }
    }

    /// Files on disk changed under the editors: reread them, the tree and git.
    fn after_git_moved_files(&mut self) {
        self.tick();
        self.refresh_tree();
        self.refresh_committed();
        self.source_settled();
        let root = self.workspace.root().to_path_buf();
        self.timeline.reread(&root);
    }

    fn stash_picker(&mut self, action: StashAction) {
        let list = stashes(self.workspace.root());
        if list.is_empty() {
            self.say("There are no stashes");
            return;
        }
        let now = now();
        let items = list
            .into_iter()
            .map(|s| {
                let title = s.title().to_string();
                Item::Git(
                    title,
                    cellix_review::age(s.time, now),
                    GitPick::Stash(action, s.hash),
                )
            })
            .collect();
        let hint = match action {
            StashAction::Apply => "Apply a stash, keeping it",
            StashAction::Pop => "Pop a stash: apply it and drop it",
            StashAction::Drop => "Drop a stash",
        };
        self.overlay = Overlay::Palette(Palette::asking(items, hint));
    }

    /// Applies, pops or drops a stash. Dropping asks first: nothing brings it back.
    fn stash_action(&mut self, action: StashAction, hash: String) {
        let root = self.workspace.root().to_path_buf();
        let result = match action {
            StashAction::Apply => apply_stash(&root, &hash).map(|()| "Stash applied"),
            StashAction::Pop => pop_stash(&root, &hash).map(|()| "Stash popped"),
            StashAction::Drop => {
                let title = stashes(&root)
                    .into_iter()
                    .find(|s| s.hash == hash)
                    .map_or_else(String::new, |s| s.title().to_string());
                self.overlay = Overlay::Confirm(Confirm {
                    title: format!("Drop the stash \u{201c}{title}\u{201d}?"),
                    detail: "Its changes will be gone.".to_string(),
                    buttons: vec![
                        ("Cancel", Choice::Cancel),
                        ("Drop", Choice::Git(GitChoice::DropStash(hash))),
                    ],
                    selected: 0,
                });
                return;
            }
        };
        match result {
            Ok(done) => {
                self.say(done);
                self.after_git_moved_files();
            }
            Err(why) => {
                self.say(format!("Git refused: {why}"));
                self.source_settled();
            }
        }
    }

    pub(super) fn choose_git(&mut self, choice: GitChoice) {
        let root = self.workspace.root().to_path_buf();
        let result = match choice {
            GitChoice::DropStash(hash) => drop_stash(&root, &hash).map(|()| "Stash dropped"),
            GitChoice::Abort(operation) => {
                conflict::abort(&root, operation).map(|()| match operation {
                    Operation::Merge => "Merge abandoned",
                    Operation::Rebase => "Rebase abandoned",
                    Operation::CherryPick => "Cherry-pick abandoned",
                    Operation::Revert => "Revert abandoned",
                })
            }
        };
        match result {
            Ok(done) => self.say(done),
            Err(why) => self.say(format!("Git refused: {why}")),
        }
        self.after_git_moved_files();
    }

    pub(super) fn pick_git(&mut self, pick: GitPick) {
        match pick {
            GitPick::Stash(action, hash) => self.stash_action(action, hash),
            GitPick::CompareRef(name, label) => self.compare_file(Some(name), label),
            GitPick::CompareClipboard => self.compare_file(None, "Clipboard".to_string()),
            GitPick::FileCommit(commit, path) => {
                self.show_fixed(Fixed::Commit(*commit), Some(&path))
            }
        }
    }

    /// Every commit that changed the file in view, to pick one and see what it did there.
    fn file_history_list(&mut self) {
        let Some((path, _)) = self.file_in_view() else {
            self.say("Open a file in the workspace to see its history");
            return;
        };
        let root = self.workspace.root().to_path_buf();
        let commits = cellix_review::file_history(&root, &path, crate::timeline::READ);
        if commits.is_empty() {
            self.say("This file isn't committed yet");
            return;
        }
        let now = now();
        let items = commits
            .into_iter()
            .map(|commit| {
                let label = format!("{}  {}", commit.short(), commit.summary);
                let note = format!(
                    "{}, {}",
                    commit.author,
                    cellix_review::age(commit.time, now)
                );
                let named = commit.path.clone().unwrap_or_else(|| path.clone());
                Item::Git(label, note, GitPick::FileCommit(Box::new(commit), named))
            })
            .collect();
        self.overlay = Overlay::Palette(Palette::asking(items, "Commits that changed this file"));
    }

    /// Keeps the timeline on the file in view, while the Explorer shows.
    pub(super) fn poll_timeline(&mut self) -> bool {
        let explorer = self.show_sidebar && self.sidebar_view == SidebarView::Explorer;
        if explorer {
            let path = self.file_in_view().map(|(p, _)| p);
            let root = self.workspace.root().to_path_buf();
            self.timeline.follow(&root, path);
        }
        self.timeline.poll()
    }

    pub(super) fn draw_timeline(&self, grid: &mut Grid, area: Rect) {
        let look = (now(), &*self.measure);
        self.timeline
            .draw(grid, area, self.hover, look, &self.theme);
    }

    /// A click in the timeline. Returns true if it was in it.
    pub(super) fn click_timeline(&mut self, x: u16, y: u16) -> bool {
        let Some(area) = self.sidebar_area() else {
            return false;
        };
        let Some(below) = self.timeline.split(area).1 else {
            return false;
        };
        let Some(hit) = self.timeline.hit(below, x, y) else {
            return below.contains(x, y);
        };
        match hit {
            crate::timeline::Hit::Fold => self.timeline.folded = !self.timeline.folded,
            crate::timeline::Hit::More => self.file_history_list(),
            crate::timeline::Hit::Commit(index) => {
                let Some(commit) = self.timeline.commits.get(index).cloned() else {
                    return true;
                };
                let fallback = self.timeline.path.clone().unwrap_or_default();
                let path = commit.path.clone().unwrap_or(fallback);
                self.show_fixed(Fixed::Commit(commit), Some(&path));
            }
        }
        true
    }
    /// The file in view, relative to the workspace, with its text as the editor has it.
    fn file_in_view(&self) -> Option<(PathBuf, String)> {
        let editor = self.editors.get(self.active())?;
        let path = editor.buffer.path()?;
        let root = self.workspace.root();
        let inside = path.strip_prefix(root).ok()?.to_path_buf();
        Some((inside, editor.buffer.text()))
    }

    fn clipboard_text(&self) -> String {
        system_clipboard().unwrap_or_else(|| self.clipboard.clone())
    }

    fn compare_with_list(&mut self) {
        if self.file_in_view().is_none() {
            self.say("Open a file in the workspace to compare it");
            return;
        }
        let root = self.workspace.root().to_path_buf();
        let mut items = Vec::new();
        let clipboard = self.clipboard_text();
        if !clipboard.trim().is_empty() {
            let lines = clipboard.lines().count();
            let note = crate::lines_word(lines);
            items.push(Item::Git(
                "Clipboard".into(),
                note,
                GitPick::CompareClipboard,
            ));
            items.push(Item::Gap);
        }
        let current = self.branch.clone();
        for branch in branches(&root) {
            let when = if current.as_deref() == Some(branch.name.as_str()) {
                "current".to_string()
            } else {
                branch.when
            };
            let pick = GitPick::CompareRef(branch.name.clone(), branch.name.clone());
            items.push(Item::Git(branch.name, when, pick));
        }
        let now = now();
        let commits = cellix_review::log(&root, COMPARE_COMMITS, false);
        if !commits.is_empty() {
            items.push(Item::Gap);
        }
        for commit in commits {
            let label = format!("{}  {}", commit.short(), commit.summary);
            let note = format!(
                "{}, {}",
                commit.author,
                cellix_review::age(commit.time, now)
            );
            let pick = GitPick::CompareRef(commit.hash.clone(), commit.short().to_string());
            items.push(Item::Git(label, note, pick));
        }
        let hint = "Compare the file with a branch, a commit or the clipboard";
        self.overlay = Overlay::Palette(Palette::asking(items, hint));
    }

    /// Shows the file in view against a branch or commit, or the clipboard for None.
    fn compare_file(&mut self, against: Option<String>, label: String) {
        let Some((path, text)) = self.file_in_view() else {
            return;
        };
        let root = self.workspace.root().to_path_buf();
        let old = match &against {
            Some(name) => cellix_review::text_at(&root, name, &path),
            None => self.clipboard_text(),
        };
        let fixed = Fixed::File {
            title: format!("{label} \u{2194} working copy"),
            old,
            new: text,
        };
        self.show_fixed(fixed, Some(&path));
    }

    /// Opens a commit, a stash or a comparison in the review view.
    pub(super) fn show_fixed(&mut self, fixed: Fixed, path: Option<&Path>) {
        let root = self.workspace.root().to_path_buf();
        let review = Review::fixed(root, self.registry.clone(), fixed, path);
        self.settings_page = None;
        self.review = Some(review);
    }

    /// The conflict at the caret, or the next one after it, in the file in view.
    fn conflict_here(&self) -> Option<(String, conflict::Conflict, usize)> {
        let editor = self.editors.get(self.active())?;
        let text = editor.buffer.text();
        let rope = editor.buffer.rope();
        let line = rope.char_to_line(editor.selections.primary().head);
        let found = conflict::conflicts(&text);
        let at = found
            .iter()
            .position(|c| c.start <= line && line <= c.end)
            .or_else(|| found.iter().position(|c| c.start > line))?;
        let count = found.len();
        Some((text, found.into_iter().nth(at)?, count))
    }

    fn settle_conflict(&mut self, how: Resolution) {
        let Some((text, found, count)) = self.conflict_here() else {
            self.say("No conflict here");
            return;
        };
        self.settle(&text, &found, how);
        let left = count - 1;
        if left == 0 {
            self.say("No conflicts left. Save, then Mark Resolved in Source Control");
        } else {
            self.say(format!("{left} conflicts left in this file"));
        }
    }

    /// Puts one side, or both, in place of a conflict, as one step to undo.
    pub(super) fn settle(&mut self, text: &str, found: &conflict::Conflict, how: Resolution) {
        let settled = conflict::resolve(text, found, how);
        let area = self.focused_area();
        let Some(editor) = self.editor_mut() else {
            return;
        };
        let rope = editor.buffer.rope().clone();
        let start = rope.line_to_char(found.start);
        let after = rope.len_chars()
            - if found.end + 1 < rope.len_lines() {
                rope.line_to_char(found.end + 1)
            } else {
                rope.len_chars()
            };
        let total = settled.chars().count();
        let middle: String = settled
            .chars()
            .skip(start)
            .take(total.saturating_sub(after + start))
            .collect();
        let end = rope.len_chars() - after;
        editor.prepare_edit(area);
        editor.apply_fix(start, end, &middle);
        editor.selections.set_single(Selection::cursor(start));
        editor.reveal(area);
    }

    /// A click on a conflict's choices. Returns true if it was one.
    pub(super) fn click_conflict(&mut self, area: Rect, x: u16, y: u16) -> bool {
        let Some(editor) = self.editors.get(self.active()) else {
            return false;
        };
        let Some((index, choice)) = crate::conflicts::hit(editor, area, x, y) else {
            return false;
        };
        let Some(found) = crate::conflicts::found(editor).get(index).cloned() else {
            return false;
        };
        let text = editor.buffer.text();
        let how = match choice {
            crate::conflicts::Choice::Current => Resolution::Current,
            crate::conflicts::Choice::Incoming => Resolution::Incoming,
            crate::conflicts::Choice::Both => Resolution::Both,
            crate::conflicts::Choice::Compare => {
                self.compare_one(&text, &found);
                return true;
            }
        };
        let left = crate::conflicts::found(editor).len() - 1;
        self.settle(&text, &found, how);
        if left == 0 {
            self.say("No conflicts left. Save, then Mark Resolved in Source Control");
        }
        true
    }

    fn compare_conflict(&mut self) {
        let Some((text, found, _)) = self.conflict_here() else {
            self.say("No conflict here");
            return;
        };
        self.compare_one(&text, &found);
    }

    /// One conflict's two sides in the diff, current on the left.
    fn compare_one(&mut self, text: &str, found: &conflict::Conflict) {
        let Some((path, _)) = self.file_in_view() else {
            return;
        };
        let (current, incoming) = conflict::sides(text, found);
        let name = |label: &str, fallback: &str| {
            if label.is_empty() {
                fallback.to_string()
            } else {
                label.to_string()
            }
        };
        let title = format!(
            "{} \u{2194} {}",
            name(&found.current, "Current"),
            name(&found.incoming, "Incoming")
        );
        let fixed = Fixed::File {
            title,
            old: current,
            new: incoming,
        };
        self.show_fixed(fixed, Some(&path));
    }

    fn step_conflict(&mut self, forward: bool) {
        let area = self.focused_area();
        let Some(editor) = self.editor_mut() else {
            return;
        };
        let found = conflict::conflicts(&editor.buffer.text());
        if found.is_empty() {
            self.say("No conflicts in this file");
            return;
        }
        let line = editor
            .buffer
            .rope()
            .char_to_line(editor.selections.primary().head);
        let starts: Vec<usize> = found.iter().map(|c| c.start).collect();
        let next = if forward {
            starts.iter().find(|&&s| s > line).or(starts.first())
        } else {
            starts.iter().rev().find(|&&s| s < line).or(starts.last())
        };
        if let Some(&start) = next {
            editor.go_to_line(start + 1, area);
            let index = starts.iter().position(|&s| s == start).unwrap_or(0);
            self.say(format!("Conflict {} of {}", index + 1, starts.len()));
        }
    }

    fn ask_abort(&mut self) {
        let Some(underway) = conflict::underway(self.workspace.root()) else {
            self.say("No merge or rebase is under way");
            return;
        };
        let what = match underway.operation {
            Operation::Merge => "merge",
            Operation::Rebase => "rebase",
            Operation::CherryPick => "cherry-pick",
            Operation::Revert => "revert",
        };
        self.overlay = Overlay::Confirm(Confirm {
            title: format!("Abandon the {what} of {}?", underway.incoming),
            detail: "Files go back to how they were before it, conflicts you settled too."
                .to_string(),
            buttons: vec![
                ("Cancel", Choice::Cancel),
                ("Abandon", Choice::Git(GitChoice::Abort(underway.operation))),
            ],
            selected: 0,
        });
    }

    /// Commits a merge, or goes on with a rebase, cherry-pick or revert. Returns false when
    /// nothing is under way, for the ordinary commit to go ahead.
    pub(super) fn finish_underway(&mut self) -> bool {
        let Some(underway) = self.source.underway.clone() else {
            return false;
        };
        let left = self.source.conflicted().len();
        if left > 0 {
            let files = if left == 1 { "file" } else { "files" };
            self.say(format!(
                "{left} {files} still in conflict: settle them and Mark Resolved"
            ));
            return true;
        }
        let root = self.workspace.root().to_path_buf();
        if self.source.staged().is_empty()
            && let Err(why) = cellix_review::stage(&root, &self.source.unstaged())
        {
            self.say(format!("Git refused: {why}"));
            return true;
        }
        let result = match underway.operation {
            Operation::Merge => {
                let mut message = self.source.message.trim().to_string();
                if message.is_empty() {
                    message = conflict::merge_message(&root)
                        .unwrap_or_else(|| format!("Merge {}", underway.incoming));
                }
                cellix_review::commit(&root, &message)
            }
            other => conflict::proceed(&root, other).map(|()| format!("{} went on", other.verb())),
        };
        match result {
            Ok(done) => {
                self.say(done);
                self.source.message.clear();
                self.source.focused = false;
            }
            Err(why) => self.say(format!("Git refused: {why}")),
        }
        self.after_git_moved_files();
        true
    }

    /// The panel's new rows: conflicts, the merge under way, stashes and its menu.
    pub(super) fn click_source_git(&mut self, hit: SourceHit, at: (u16, u16)) {
        let root = self.workspace.root().to_path_buf();
        match hit {
            SourceHit::More => {
                let item =
                    |label: &'static str, command| MenuItem::new(label, MenuAction::Run(command));
                let mut items = vec![
                    item("Stash", Command::Stash),
                    item("Stash, Including Untracked", Command::StashUntracked),
                    item("Stash Staged", Command::StashStaged),
                    item("Pop Latest Stash", Command::PopStash),
                    item("Apply Stash…", Command::ApplyStash),
                    item("Drop Stash…", Command::DropStash),
                    item("Show Git History", Command::ShowHistory),
                    item("Compare With…", Command::CompareWith),
                ];
                items[3].rule = true;
                items[6].rule = true;
                // Dropped from the button's left edge, under its row.
                self.show_menu((at.0.saturating_sub(1), at.1 + 1), items);
            }
            SourceHit::Abort => self.ask_abort(),
            SourceHit::OpenConflict(path) => {
                if let Err(e) = self.open(&root.join(&path)) {
                    self.say(format!("Could not open: {e}"));
                    return;
                }
                let area = self.focused_area();
                if let Some(editor) = self.editor_mut() {
                    editor.rendered = false;
                    editor.left = 0;
                    let first = conflict::conflicts(&editor.buffer.text())
                        .first()
                        .map(|c| c.start);
                    if let Some(line) = first {
                        editor.go_to_line(line + 1, area);
                    }
                }
            }
            SourceHit::Resolve(path) => {
                if let Err(why) = cellix_review::stage(&root, &[path]) {
                    self.say(format!("Git refused: {why}"));
                }
                self.source_settled();
            }
            SourceHit::ToggleStashes => self.source.stashes_folded = !self.source.stashes_folded,
            SourceHit::ShowStash(hash) => {
                let title = self
                    .source
                    .stashes
                    .iter()
                    .find(|s| s.hash == hash)
                    .map_or_else(String::new, |s| s.title().to_string());
                self.show_fixed(Fixed::Stash(hash, title), None);
                if let Some(review) = &mut self.review {
                    review.beside_panel = true;
                }
            }
            SourceHit::Stash(action, hash) => self.stash_action(action, hash),
            SourceHit::ToggleGraph => self.source.graph_folded = !self.source.graph_folded,
            SourceHit::OpenCommit(index) => {
                let graph = &mut self.source.graph;
                if graph.is_open(index) {
                    graph.open = None;
                } else if let Some(commit) = graph.commits.get(index) {
                    let files = cellix_review::commit_files(&root, commit);
                    graph.open = Some((commit.hash.clone(), files));
                }
            }
            SourceHit::CommitFile(index, path) => {
                let Some(commit) = self.source.graph.commits.get(index).cloned() else {
                    return;
                };
                self.show_fixed(Fixed::Commit(commit), Some(&path));
                if let Some(review) = &mut self.review {
                    review.beside_panel = true;
                }
            }
            _ => {}
        }
    }
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cellix_input::{Key, KeyEvent, Mods};
    use std::fs;
    use tempfile::TempDir;

    fn git(dir: &Path, args: &[&str]) {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .unwrap();
        assert!(output.status.success(), "git {args:?}: {output:?}");
    }

    fn git_fails(dir: &Path, args: &[&str]) {
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .unwrap()
            .status;
        assert!(!status.success(), "git {args:?} was expected to stop");
    }

    fn screen(app: &App) -> Grid {
        let mut grid = Grid::new(app.size().0, app.size().1, app.background());
        app.draw(&mut grid);
        grid
    }

    fn click(app: &mut App, x: u16, y: u16) {
        app.handle(Event::Mouse(MouseEvent {
            kind: MouseKind::Left,
            x,
            y,
            fine_x: x as f32,
            fine_y: y as f32 + 0.5,
            mods: Mods::default(),
        }));
    }

    fn typed(app: &mut App, text: &str) {
        for c in text.chars() {
            app.handle(Event::Key(KeyEvent::plain(Key::Char(c))));
        }
    }

    fn enter(app: &mut App) {
        app.handle(Event::Key(KeyEvent::plain(Key::Enter)));
    }

    /// Where `text` first shows on screen.
    fn find(grid: &Grid, text: &str) -> (u16, u16) {
        for y in 0..grid.height() {
            if let Some(i) = grid.row_text(y).find(text) {
                return (grid.row_text(y)[..i].chars().count() as u16, y);
            }
        }
        panic!("{text:?} not on screen:\n{}", grid.text());
    }

    fn repo() -> TempDir {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path();
        git(path, &["init", "-q", "-b", "main"]);
        git(path, &["config", "user.email", "t@example.com"]);
        git(path, &["config", "user.name", "Tester"]);
        fs::write(path.join("queue.rs"), "fn push() {\n    old();\n}\n").unwrap();
        git(path, &["add", "."]);
        git(path, &["commit", "-q", "-m", "Start the queue"]);
        dir
    }

    /// A repository stopped in a merge of `feature`, with one conflict in queue.rs.
    fn merging() -> (TempDir, App) {
        let dir = repo();
        let path = dir.path();
        git(path, &["checkout", "-q", "-b", "feature"]);
        fs::write(path.join("queue.rs"), "fn push() {\n    theirs();\n}\n").unwrap();
        git(path, &["commit", "-qam", "Theirs"]);
        git(path, &["checkout", "-q", "main"]);
        fs::write(path.join("queue.rs"), "fn push() {\n    mine();\n}\n").unwrap();
        git(path, &["commit", "-qam", "Mine"]);
        git_fails(path, &["merge", "-q", "feature"]);
        let mut app = App::new(path.to_path_buf(), (150, 40));
        app.run(Command::ShowSourceControl);
        app.source_settled();
        settle_reads(&mut app);
        (dir, app)
    }

    /// Reads the repository again and waits for what Source Control finds.
    fn settle_reads(app: &mut App) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        let root = app.workspace.root().to_path_buf();
        app.source.reread(&root);
        while app.source.is_reading() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(20));
            app.source.poll();
        }
    }

    /// Where `text` first shows in the sidebar.
    fn find_in_sidebar(app: &App, text: &str) -> (u16, u16) {
        let area = app.sidebar_area().unwrap();
        let grid = screen(app);
        for y in area.y..area.bottom() {
            let row: String = (area.x..area.right())
                .map(|x| grid.cell(x, y).symbol.to_string())
                .collect();
            if let Some(i) = row.find(text) {
                return (area.x + row[..i].chars().count() as u16, y);
            }
        }
        panic!("{text:?} not in the sidebar:\n{}", grid.text());
    }

    #[test]
    fn a_conflict_is_a_card_whose_choices_settle_it_and_the_merge_commits() {
        let (dir, mut app) = merging();
        let text = screen(&app).text();
        assert!(text.contains("MERGE CHANGES"), "{text}");
        assert!(text.contains("Merging feature"), "{text}");
        assert!(text.contains("Commit Merge"), "{text}");
        // The conflicted file opens in the editor, on its conflict.
        let (x, y) = find(&screen(&app), "queue.rs");
        click(&mut app, x, y);
        let grid = screen(&app);
        let text = grid.text();
        assert!(text.contains("Current · HEAD"), "{text}");
        assert!(text.contains("Incoming · feature"), "{text}");
        assert!(text.contains("end of conflict 1"), "{text}");
        assert!(
            !text.contains("<<<<<<<"),
            "the marker line is the card's header:\n{text}"
        );
        // The current side is tinted green, the incoming side blue.
        let (_, mine) = find(&grid, "mine();");
        let (_, theirs) = find(&grid, "theirs();");
        let ground = app.theme.text.bg;
        let bg = |y: u16| grid.cell(app.pane_area(0).x + 8, y).style.bg;
        assert_ne!(bg(mine), ground);
        assert_ne!(bg(theirs), ground);
        assert_ne!(bg(mine), bg(theirs));
        // Committing with the conflict left says why not.
        app.finish_underway();
        assert!(screen(&app).text().contains("still in conflict"));
        // Take theirs, as one step to undo.
        let (x, y) = find(&screen(&app), "[Incoming]");
        click(&mut app, x + 2, y);
        let editor = &app.editors[app.active()];
        assert_eq!(editor.buffer.text(), "fn push() {\n    theirs();\n}\n");
        app.run(Command::Undo);
        assert!(
            app.editors[app.active()]
                .buffer
                .text()
                .contains("<<<<<<< HEAD")
        );
        app.run(Command::Redo);
        app.run(Command::Save);
        // With no markers left, the row offers Mark Resolved, which stages it.
        app.source_settled();
        settle_reads(&mut app);
        assert_eq!(app.source.settled, vec![PathBuf::from("queue.rs")]);
        let area = app.sidebar_area().unwrap();
        let (_, y) = find_in_sidebar(&app, "queue.rs");
        click(&mut app, area.right() - crate::source::STAGE_FROM_RIGHT, y);
        settle_reads(&mut app);
        assert!(app.source.conflicted().is_empty());
        let (x, y) = find(&screen(&app), "Commit Merge");
        click(&mut app, x, y);
        assert!(cellix_review::conflict::underway(dir.path()).is_none());
        let log = cellix_review::log(dir.path(), 1, false);
        assert!(log[0].is_merge(), "{:?}", log[0]);
        assert_eq!(
            fs::read_to_string(dir.path().join("queue.rs")).unwrap(),
            "fn push() {\n    theirs();\n}\n"
        );
    }

    #[test]
    fn conflicts_can_be_stepped_through_and_settled_from_the_palette() {
        let dir = repo();
        let path = dir.path();
        let text = "a\n<<<<<<< HEAD\none\n=======\nuno\n>>>>>>> b\nmid\n<<<<<<< HEAD\ntwo\n=======\ndos\n>>>>>>> b\nz\n";
        fs::write(path.join("both.txt"), text).unwrap();
        let mut app = App::new(path.to_path_buf(), (150, 40));
        app.open(&path.join("both.txt")).unwrap();
        app.run(Command::NextConflict);
        assert!(screen(&app).text().contains("Conflict 1 of 2"));
        app.run(Command::NextConflict);
        assert!(screen(&app).text().contains("Conflict 2 of 2"));
        app.run(Command::AcceptBoth);
        app.run(Command::PreviousConflict);
        app.run(Command::AcceptCurrent);
        let text = app.editors[app.active()].buffer.text();
        assert_eq!(text, "a\none\nmid\ntwo\ndos\nz\n");
        assert!(screen(&app).text().contains("No conflicts left"));
    }

    #[test]
    fn a_merge_can_be_abandoned_from_the_panel() {
        let (dir, mut app) = merging();
        let (x, y) = find(&screen(&app), "Abort");
        click(&mut app, x + 1, y);
        assert!(
            screen(&app)
                .text()
                .contains("Abandon the merge of feature?")
        );
        let (x, y) = find(&screen(&app), "Abandon");
        // The dialog's own button, not the title's word.
        let (bx, by) = {
            let grid = screen(&app);
            let rows: Vec<u16> = (0..grid.height())
                .filter(|r| grid.row_text(*r).contains("Abandon"))
                .collect();
            let last = *rows.last().unwrap();
            let at = grid.row_text(last).rfind("Abandon").unwrap();
            (grid.row_text(last)[..at].chars().count() as u16, last)
        };
        let _ = (x, y);
        click(&mut app, bx + 1, by);
        assert!(cellix_review::conflict::underway(dir.path()).is_none());
        assert_eq!(
            fs::read_to_string(dir.path().join("queue.rs")).unwrap(),
            "fn push() {\n    mine();\n}\n"
        );
    }

    #[test]
    fn changes_are_stashed_from_the_panel_menu_and_popped_from_their_row() {
        let dir = repo();
        let path = dir.path();
        fs::write(path.join("queue.rs"), "fn push() {\n    halfway();\n}\n").unwrap();
        let mut app = App::new(path.to_path_buf(), (150, 40));
        app.run(Command::ShowSourceControl);
        app.source_settled();
        let area = app.sidebar_area().unwrap();
        let title = find(&screen(&app), "SOURCE CONTROL").1;
        click(
            &mut app,
            area.right() - crate::source::MORE_FROM_RIGHT,
            title,
        );
        let (x, y) = find(&screen(&app), "Stash, Including Untracked");
        assert!(screen(&app).text().contains("Pop Latest Stash"));
        let _ = x;
        // "Stash" itself is the row above.
        let (x, _) = find(&screen(&app), "Stash, Including");
        click(&mut app, x + 1, y - 1);
        typed(&mut app, "Halfway");
        enter(&mut app);
        assert_eq!(
            fs::read_to_string(path.join("queue.rs")).unwrap(),
            "fn push() {\n    old();\n}\n"
        );
        settle_reads(&mut app);
        let text = screen(&app).text();
        assert!(
            text.contains("STASHES") && text.contains("Halfway"),
            "{text}"
        );
        // Its row shows what it holds, and Pop brings it back.
        let (x, y) = find(&screen(&app), "Halfway");
        click(&mut app, x, y);
        assert!(screen(&app).text().contains("stash: Halfway"));
        assert!(screen(&app).text().contains("halfway();"));
        app.review = None;
        let (_, y) = find(&screen(&app), "Halfway");
        click(&mut app, area.right() - 9, y);
        assert_eq!(
            fs::read_to_string(path.join("queue.rs")).unwrap(),
            "fn push() {\n    halfway();\n}\n"
        );
        assert!(cellix_review::stashes(path).is_empty());
        settle_reads(&mut app);
        assert!(!screen(&app).text().contains("STASHES"));
    }

    #[test]
    fn a_stash_is_dropped_only_once_confirmed() {
        let dir = repo();
        let path = dir.path();
        fs::write(path.join("queue.rs"), "x\n").unwrap();
        cellix_review::stash(path, "Keep me", Keep::Tracked).unwrap();
        let mut app = App::new(path.to_path_buf(), (150, 40));
        app.run(Command::DropStash);
        typed(&mut app, "Keep");
        enter(&mut app);
        assert!(screen(&app).text().contains("Drop the stash"));
        assert_eq!(cellix_review::stashes(path).len(), 1);
        let hash = cellix_review::stashes(path)[0].hash.clone();
        app.overlay = Overlay::None;
        app.choose(Choice::Git(GitChoice::DropStash(hash)));
        assert!(cellix_review::stashes(path).is_empty());
    }

    #[test]
    fn the_file_compares_with_a_branch_and_with_the_clipboard() {
        let dir = repo();
        let path = dir.path();
        git(path, &["branch", "earlier"]);
        fs::write(path.join("queue.rs"), "fn push() {\n    newer();\n}\n").unwrap();
        git(path, &["commit", "-qam", "Newer"]);
        let mut app = App::new(path.to_path_buf(), (150, 40));
        app.open(&path.join("queue.rs")).unwrap();
        // Unsaved edits count: it's the file as the editor has it.
        app.run(Command::SelectAll);
        typed(&mut app, "typed\n");
        app.run(Command::CompareWith);
        let text = screen(&app).text();
        assert!(
            text.contains("earlier") && text.contains("Start the queue"),
            "{text}"
        );
        typed(&mut app, "earlier");
        enter(&mut app);
        let text = screen(&app).text();
        assert!(text.contains("earlier ↔ working copy"), "{text}");
        assert!(text.contains("old();") && text.contains("typed"), "{text}");
        app.review = None;
        app.clipboard = "pasted words\n".to_string();
        app.run(Command::CompareWith);
        typed(&mut app, "Clipboard");
        enter(&mut app);
        let text = screen(&app).text();
        assert!(text.contains("Clipboard ↔ working copy"), "{text}");
        assert!(text.contains("pasted words"), "{text}");
    }

    /// main with a merged branch: Start, then Mine on main and Side on side, then the merge.
    fn merged() -> TempDir {
        let dir = repo();
        let path = dir.path();
        git(path, &["checkout", "-q", "-b", "side"]);
        fs::write(path.join("side.txt"), "sideways words\n").unwrap();
        git(path, &["add", "."]);
        git(path, &["commit", "-qm", "Work on the side"]);
        git(path, &["checkout", "-q", "main"]);
        fs::write(path.join("queue.rs"), "fn push() {\n    mine();\n}\n").unwrap();
        git(path, &["commit", "-qam", "Mine"]);
        git(path, &["merge", "-q", "--no-edit", "side"]);
        dir
    }

    #[test]
    fn the_graph_lists_commits_in_lanes_and_a_commit_opens_to_its_diffs() {
        let dir = merged();
        let mut app = App::new(dir.path().to_path_buf(), (150, 40));
        app.run(Command::ShowHistory);
        settle_reads(&mut app);
        app.source.show_graph();
        let grid = screen(&app);
        let text = grid.text();
        assert!(text.contains("GRAPH"), "{text}");
        assert!(
            text.contains("Merge branch") && text.contains("Work on the s"),
            "{text}"
        );
        // The merge is a hollow dot, and the side branch has a lane of its own.
        let (_, merge) = find(&grid, "Merge branch");
        assert!(
            grid.row_text(merge).contains('○'),
            "{}",
            grid.row_text(merge)
        );
        // Main's line runs past the side commit in a lane of its own.
        let (_, side) = find(&grid, "Work on the s");
        let lanes = grid.row_text(side);
        assert!(lanes.contains('●') && lanes.contains('│'), "{lanes}");
        assert_eq!(app.source.graph.lanes(), 2);
        // A commit opens to its files; a file shows its diff against the commit before.
        let (x, y) = find(&grid, "Work on the s");
        click(&mut app, x, y);
        let (x, y) = find_in_sidebar(&app, "side.txt");
        click(&mut app, x, y);
        let text = screen(&app).text();
        assert!(text.contains("sideways words"), "{text}");
        assert!(matches!(
            app.review.as_ref().and_then(|r| r.fixed.as_ref()),
            Some(Fixed::Commit(c)) if c.summary == "Work on the side"
        ));
    }

    #[test]
    fn the_timeline_lists_the_open_files_commits_and_opens_one() {
        let dir = merged();
        let path = dir.path();
        let mut app = App::new(path.to_path_buf(), (150, 40));
        app.open(&path.join("queue.rs")).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while app.timeline.commits.is_empty() && std::time::Instant::now() < deadline {
            app.poll_timeline();
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let text = screen(&app).text();
        assert!(
            text.contains("TIMELINE") && text.contains("queue.rs"),
            "{text}"
        );
        assert!(
            text.contains("Mine") && text.contains("Start the queue"),
            "{text}"
        );
        assert!(
            !text.contains("Work on the side"),
            "only the file's commits:\n{text}"
        );
        let (x, y) = find_in_sidebar(&app, "Mine");
        click(&mut app, x, y);
        let text = screen(&app).text();
        assert!(
            text.contains("mine();") && text.contains("old();"),
            "{text}"
        );
        // The palette's list holds every one of them.
        app.review = None;
        app.run(Command::FileHistory);
        typed(&mut app, "Start");
        enter(&mut app);
        assert!(matches!(
            app.review.as_ref().and_then(|r| r.fixed.as_ref()),
            Some(Fixed::Commit(c)) if c.summary == "Start the queue"
        ));
    }
}
