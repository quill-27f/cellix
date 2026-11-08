//! Source Control in the sidebar: what's staged, what isn't, and a message to commit it with.
//! Review is for reading what changed. This is for deciding what goes in.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};

use cellix_render::{Bounds, Font, Grid, Icon, Measure, Rect, Style};
use cellix_review::conflict::{Operation, Underway, has_conflicts, underway};
use cellix_review::{Commit, Entry, Stash, ahead_behind, stashes, status_entries};

use crate::graph::Graph;

use crate::explorer::file_color;
use crate::theme::Theme;
use crate::threads::wrap;

const SIDE: u16 = 2;
/// Rows of the message box.
const MESSAGE_ROWS: u16 = 3;
/// Where a row's marks stand, from the panel's right edge.
const LETTER_FROM_RIGHT: u16 = 3;
pub const STAGE_FROM_RIGHT: u16 = 5;
pub const DISCARD_FROM_RIGHT: u16 = 7;
/// How wide the button beside Commit is.
const SYNC_WIDTH: u16 = 10;
/// Columns from a row's icon to its name.
const NAME_FROM_ICON: u16 = 3;
/// Cells between a file's name and its folder.
const FOLDER_GAP: f32 = 0.8;
/// How far the message box stands past its rows, so its text has air above it.
const BOX_ROOM: f32 = 0.12;
/// Where the ⋯ button stands in the title row, from the panel's right edge.
pub const MORE_FROM_RIGHT: u16 = 4;
/// Commits the graph lists.
const GRAPH_COMMITS: usize = 100;
/// A stash row's actions on hover, each a word, from the right.
const STASH_ACTIONS: [(&str, StashAction); 3] = [
    ("Drop", StashAction::Drop),
    ("Pop", StashAction::Pop),
    ("Apply", StashAction::Apply),
];

/// What a click in the panel lands on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Hit {
    Message,
    Commit,
    Sync,
    StageAll,
    UnstageAll,
    Open(PathBuf),
    Stage(PathBuf),
    Unstage(PathBuf),
    Discard(PathBuf, bool),
    /// The ⋯ in the title row, which opens the panel's menu.
    More,
    /// Gives up the merge, rebase, cherry-pick or revert under way.
    Abort,
    /// A file with conflicts, to open in the editor.
    OpenConflict(PathBuf),
    /// Stages a file whose conflicts are all settled.
    Resolve(PathBuf),
    /// Opens or closes the list of stashes.
    ToggleStashes,
    /// What a stash holds, by its commit.
    ShowStash(String),
    Stash(StashAction, String),
    /// Opens or closes the graph.
    ToggleGraph,
    /// A commit in the graph, which opens to its files, or closes.
    OpenCommit(usize),
    /// A file a commit changed: the commit, and the file.
    CommitFile(usize, PathBuf),
}

/// What a stash row's words do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StashAction {
    Apply,
    Pop,
    Drop,
}

/// The groups files are listed in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Group {
    /// Both sides changed them and git couldn't merge them.
    Merge,
    Staged,
    Changes,
}

/// One row of the panel.
#[derive(Clone, Debug, PartialEq)]
enum Line {
    Title,
    Message(u16),
    Gap,
    Buttons,
    /// A merge or rebase stopped for conflicts, with Abort.
    Underway,
    Heading(Group, usize),
    Change(usize, Group),
    Clean,
    StashHeading(usize),
    Stash(usize),
    GraphHeading,
    Commit(usize),
    /// A file the open commit changed, by its place in that commit's files.
    CommitFile(usize, usize),
}

/// What a read of the repository found.
#[derive(Default)]
struct Read {
    entries: Vec<Entry>,
    remote: Remote,
    underway: Option<Underway>,
    stashes: Vec<Stash>,
    settled: Vec<PathBuf>,
    commits: Vec<Commit>,
}

/// How things stand with the remote.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Remote {
    /// Commits to push and to pull, or None with no upstream.
    pub counts: Option<(usize, usize)>,
    pub can_publish: bool,
}

#[derive(Default)]
pub struct Source {
    pub entries: Vec<Entry>,
    pub message: String,
    pub focused: bool,
    pub remote: Remote,
    /// Pushing or pulling, as what to say while it does.
    pub busy: Option<&'static str>,
    /// The file whose changes are showing beside the panel.
    pub shown: Option<PathBuf>,
    /// A merge, rebase, cherry-pick or revert waiting to be finished.
    pub underway: Option<Underway>,
    pub stashes: Vec<Stash>,
    /// Files listed as conflicted that no longer hold a conflict, ready to be marked resolved.
    pub settled: Vec<PathBuf>,
    /// Whether the stashes are listed or folded away.
    pub stashes_folded: bool,
    /// The repository's commits, drawn as a graph under everything else.
    pub graph: Graph,
    pub graph_folded: bool,
    /// Asked to show the graph before its commits were read: done once they are.
    graph_wanted: bool,
    top: usize,
    reading: Option<Receiver<Read>>,
}

impl Source {
    /// Reads the state of the repository, away from the screen.
    pub fn refresh(&mut self, root: &Path) {
        if self.reading.is_some() {
            return;
        }
        let root = root.to_path_buf();
        let (done, reading) = mpsc::channel();
        std::thread::spawn(move || {
            let entries = status_entries(&root);
            let counts = ahead_behind(&root);
            let remote = Remote {
                counts,
                can_publish: counts.is_none() && cellix_review::has_remote(&root),
            };
            let settled = settled(&root, &entries);
            let commits = cellix_review::log(&root, GRAPH_COMMITS, true);
            let _ = done.send(Read {
                commits,
                underway: underway(&root),
                stashes: stashes(&root),
                entries,
                remote,
                settled,
            });
        });
        self.reading = Some(reading);
    }

    /// Whether a read of the repository is on its way.
    #[cfg(test)]
    pub fn is_reading(&self) -> bool {
        self.reading.is_some()
    }

    /// Reads again after a change, so what was already on its way can't undo it.
    pub fn reread(&mut self, root: &Path) {
        self.reading = None;
        self.refresh(root);
    }

    /// Takes in what was read. Returns true if it changed anything.
    pub fn poll(&mut self) -> bool {
        let Some(read) = self.reading.as_ref().and_then(|r| r.try_recv().ok()) else {
            return false;
        };
        self.reading = None;
        let changed = self.entries != read.entries
            || self.remote != read.remote
            || self.underway != read.underway
            || self.stashes != read.stashes
            || self.settled != read.settled
            || self.graph.commits != read.commits;
        if self.graph.commits != read.commits {
            let open = self.graph.open.take();
            self.graph = Graph::new(read.commits);
            // A commit stays open while it's still listed.
            let listed = |hash: &str| self.graph.commits.iter().any(|c| c.hash == hash);
            self.graph.open = open.filter(|(hash, _)| listed(hash));
        }
        self.entries = read.entries;
        self.remote = read.remote;
        self.underway = read.underway;
        self.stashes = read.stashes;
        self.settled = read.settled;
        if self.graph_wanted && !self.graph.commits.is_empty() {
            self.show_graph();
        }
        changed
    }

    /// Files that still hold conflicts.
    pub fn conflicted(&self) -> Vec<PathBuf> {
        let conflicted = self.entries.iter().filter(|e| e.is_conflicted());
        conflicted.map(|e| e.path.clone()).collect()
    }

    pub fn staged(&self) -> Vec<PathBuf> {
        let staged = self.entries.iter().filter(|e| e.is_staged());
        staged.map(|e| e.path.clone()).collect()
    }

    pub fn unstaged(&self) -> Vec<PathBuf> {
        let open = self.entries.iter().filter(|e| e.is_unstaged());
        open.map(|e| e.path.clone()).collect()
    }

    pub fn scroll_by(&mut self, delta: isize) {
        self.top = self.top.saturating_add_signed(delta);
    }

    fn lines(&self) -> Vec<Line> {
        let mut lines = vec![Line::Title, Line::Gap, Line::Message(0)];
        lines.extend((1..MESSAGE_ROWS).map(Line::Message));
        // What's under way comes before the button that finishes it, so it's read first.
        if self.underway.is_some() {
            lines.extend([Line::Gap, Line::Underway]);
        }
        lines.extend([Line::Gap, Line::Buttons, Line::Gap]);
        let list = |lines: &mut Vec<Line>, group: Group| {
            let matching = |entry: &&Entry| match group {
                Group::Merge => entry.is_conflicted(),
                Group::Staged => entry.is_staged(),
                Group::Changes => entry.is_unstaged(),
            };
            let count = self.entries.iter().filter(matching).count();
            if count == 0 {
                return;
            }
            lines.push(Line::Heading(group, count));
            let rows = self.entries.iter().enumerate();
            let rows = rows.filter(|(_, entry)| matching(entry));
            lines.extend(rows.map(|(index, _)| Line::Change(index, group)));
            lines.push(Line::Gap);
        };
        if self.entries.is_empty() {
            lines.extend([Line::Clean, Line::Gap]);
        }
        list(&mut lines, Group::Merge);
        list(&mut lines, Group::Staged);
        list(&mut lines, Group::Changes);
        if !self.stashes.is_empty() {
            lines.push(Line::StashHeading(self.stashes.len()));
            if !self.stashes_folded {
                lines.extend((0..self.stashes.len()).map(Line::Stash));
            }
            lines.push(Line::Gap);
        }
        if !self.graph.commits.is_empty() {
            lines.push(Line::GraphHeading);
            if !self.graph_folded {
                for index in 0..self.graph.commits.len() {
                    lines.push(Line::Commit(index));
                    if self.graph.is_open(index)
                        && let Some((_, files)) = &self.graph.open
                    {
                        lines.extend((0..files.len()).map(|f| Line::CommitFile(index, f)));
                    }
                }
            }
        }
        lines
    }

    /// How many lines at the top stay put while the lists under them scroll: the title, the
    /// commit box, its buttons and what's under way. A wheel never takes the box out of
    /// reach, nor leaves it half drawn.
    fn pinned(lines: &[Line]) -> usize {
        let fixed = |line: &Line| {
            matches!(
                line,
                Line::Title | Line::Gap | Line::Message(_) | Line::Buttons | Line::Underway
            )
        };
        lines.iter().position(|l| !fixed(l)).unwrap_or(lines.len())
    }

    /// The line shown at `row` of the panel, counting the pinned ones.
    fn line_at(&self, lines: &[Line], row: usize) -> usize {
        let pinned = Self::pinned(lines);
        if row < pinned { row } else { row + self.top }
    }

    /// Scrolls so the graph's heading is at the top, opening it.
    pub fn show_graph(&mut self) {
        self.graph_folded = false;
        let at = self.lines().iter().position(|l| *l == Line::GraphHeading);
        self.graph_wanted = at.is_none();
        if let Some(at) = at {
            // Just under what stays put.
            self.top = at.saturating_sub(Self::pinned(&self.lines()));
        }
    }

    /// Where the Commit and Sync buttons stand on their row.
    fn buttons(&self, area: Rect, y: u16) -> (Rect, Option<Rect>) {
        let left = area.x + SIDE;
        let width = area.w.saturating_sub(SIDE * 2);
        if self.sync_label().is_some() && width > SYNC_WIDTH + 8 {
            let commit = Rect::new(left, y, width - SYNC_WIDTH - 1, 1);
            let sync = Rect::new(commit.right() + 1, y, SYNC_WIDTH, 1);
            (commit, Some(sync))
        } else {
            (Rect::new(left, y, width, 1), None)
        }
    }

    fn commit_label(&self) -> &'static str {
        match self.underway.as_ref().map(|u| u.operation) {
            Some(Operation::Merge) => return "Commit Merge",
            Some(Operation::Rebase) => return "Continue Rebase",
            Some(Operation::CherryPick) => return "Continue Cherry-Pick",
            Some(Operation::Revert) => return "Continue Revert",
            None => {}
        }
        if self.staged().is_empty() && !self.entries.is_empty() {
            "Commit All"
        } else {
            "Commit"
        }
    }

    /// What the button beside Commit says, if there's anything to do with the remote.
    pub fn sync_label(&self) -> Option<String> {
        if let Some(busy) = self.busy {
            return Some(busy.to_string());
        }
        match self.remote.counts {
            Some((0, 0)) => None,
            Some((ahead, 0)) => Some(format!("Push {ahead}")),
            Some((0, behind)) => Some(format!("Pull {behind}")),
            Some(_) => Some("Sync".to_string()),
            None if self.remote.can_publish => Some("Publish".to_string()),
            None => None,
        }
    }

    pub fn hit(&self, area: Rect, x: u16, y: u16) -> Option<Hit> {
        // The title row stays put while the rest scrolls under it.
        let offset = y.checked_sub(area.y)? as usize;
        let lines = self.lines();
        let row = self.line_at(&lines, offset);
        let line = lines.into_iter().nth(row)?;
        let right = area.right();
        match line {
            Line::Title => {
                let more = right.saturating_sub(MORE_FROM_RIGHT);
                (x >= more && x < more + 2).then_some(Hit::More)
            }
            Line::Underway => {
                let abort = right.saturating_sub(SIDE + 1 + "Abort".len() as u16);
                (x >= abort).then_some(Hit::Abort)
            }
            Line::StashHeading(_) => Some(Hit::ToggleStashes),
            Line::GraphHeading => Some(Hit::ToggleGraph),
            Line::Commit(index) => Some(Hit::OpenCommit(index)),
            Line::CommitFile(index, file) => {
                let (_, files) = self.graph.open.as_ref()?;
                Some(Hit::CommitFile(index, files.get(file)?.path.clone()))
            }
            Line::Stash(index) => {
                let stash = self.stashes.get(index)?;
                let mut end = right.saturating_sub(SIDE);
                for (word, action) in STASH_ACTIONS {
                    let start = end.saturating_sub(word.len() as u16);
                    if x >= start && x < end {
                        return Some(Hit::Stash(action, stash.hash.clone()));
                    }
                    end = start.saturating_sub(1);
                }
                Some(Hit::ShowStash(stash.hash.clone()))
            }
            Line::Message(_) => Some(Hit::Message),
            Line::Buttons => {
                let (commit, sync) = self.buttons(area, y);
                if sync.is_some_and(|b| b.contains(x, y)) {
                    return Some(Hit::Sync);
                }
                commit.contains(x, y).then_some(Hit::Commit)
            }
            Line::Heading(Group::Merge, _) => None,
            Line::Heading(group, _) => {
                let on_action = x + STAGE_FROM_RIGHT + 1 >= right;
                on_action.then_some(if group == Group::Staged {
                    Hit::UnstageAll
                } else {
                    Hit::StageAll
                })
            }
            Line::Change(index, Group::Merge) => {
                let entry = self.entries.get(index)?;
                let path = entry.path.clone();
                let stage = right.saturating_sub(STAGE_FROM_RIGHT);
                let on_stage = x == stage || x == stage + 1;
                if on_stage && self.settled.contains(&path) {
                    return Some(Hit::Resolve(path));
                }
                Some(Hit::OpenConflict(path))
            }
            Line::Change(index, group) => {
                let staged = group == Group::Staged;
                let entry = self.entries.get(index)?;
                let path = entry.path.clone();
                let stage = right.saturating_sub(STAGE_FROM_RIGHT);
                let discard = right.saturating_sub(DISCARD_FROM_RIGHT);
                if x == stage || x == stage + 1 {
                    return Some(if staged {
                        Hit::Unstage(path)
                    } else {
                        Hit::Stage(path)
                    });
                }
                if !staged && (x == discard || x == discard + 1) {
                    return Some(Hit::Discard(path, entry.is_untracked()));
                }
                Some(Hit::Open(path))
            }
            _ => None,
        }
    }

    pub fn draw(
        &self,
        grid: &mut Grid,
        area: Rect,
        branch: Option<&str>,
        hover: Option<(u16, u16)>,
        measure: &dyn Measure,
        theme: &Theme,
    ) {
        grid.fill(area, theme.sidebar);
        let quiet = Style {
            fg: theme.gutter.fg,
            ..theme.sidebar
        };
        let left = area.x + SIDE;
        let right = area.right();
        let lines = self.lines();
        let pinned = Self::pinned(&lines);
        let rest = lines.iter().skip(pinned + self.top);
        let shown = lines.iter().take(pinned).chain(rest);
        for (row, line) in shown.take(area.h as usize).enumerate() {
            let y = area.y + row as u16;
            let whole = Rect::new(area.x, y, area.w, 1);
            let hovered = hover.is_some_and(|h| whole.contains(h.0, h.1));
            match line {
                Line::Title => {
                    crate::palette::sidebar_title(grid, whole, "Source Control", theme);
                    let more = right.saturating_sub(MORE_FROM_RIGHT);
                    if let Some(branch) = branch {
                        grid.label_right(more.saturating_sub(1), y, branch, quiet, 90);
                    }
                    let on_more =
                        hover.is_some_and(|h| h.1 == y && (more..more + 2).contains(&h.0));
                    if on_more {
                        let chip = Bounds::cells(more as f32 - 0.2, y as f32 + 0.06, 2.4, 0.88);
                        grid.panel(chip, 30, Some(theme.row_hover), None);
                    }
                    grid.glyph(more as f32, y, Icon::More, quiet.fg, "⋯", quiet);
                }
                Line::Underway => self.draw_underway(grid, area, y, hover, (measure, theme)),
                Line::StashHeading(count) => {
                    let number = count.to_string();
                    crate::palette::section_heading(
                        grid,
                        (left, right.saturating_sub(SIDE), y),
                        ("STASHES", Some((&number, quiet.fg))),
                        Some(self.stashes_folded),
                        theme,
                    );
                }
                Line::GraphHeading => {
                    crate::palette::section_heading(
                        grid,
                        (left, right, y),
                        ("GRAPH", None),
                        Some(self.graph_folded),
                        theme,
                    );
                }
                Line::Commit(index) => {
                    let open = self.graph.is_open(*index);
                    if hovered || open {
                        let tone = if open {
                            theme.row_active
                        } else {
                            theme.row_hover
                        };
                        let wash = Bounds::inset(whole, 0.4, 0.06);
                        grid.panel(wash, 14, Some(tone), None);
                    }
                    let row = Rect::new(left, y, right.saturating_sub(left + SIDE), 1);
                    let place = (*index, row, now());
                    crate::graph::draw_commit(
                        grid,
                        &self.graph,
                        place,
                        measure,
                        theme,
                        theme.sidebar,
                    );
                }
                Line::CommitFile(index, file) => {
                    if let Some(lanes) = self.graph.rows.get(*index) {
                        let style = theme.sidebar;
                        crate::graph::draw_through(grid, (left, y), &lanes.below, theme, style);
                    }
                    let Some((_, files)) = &self.graph.open else {
                        continue;
                    };
                    let Some(change) = files.get(*file) else {
                        continue;
                    };
                    let showing = self.shown.as_ref() == Some(&change.path);
                    if hovered || showing {
                        let tone = if showing {
                            theme.row_active
                        } else {
                            theme.row_hover
                        };
                        let wash = Bounds::inset(whole, 0.4, 0.06);
                        grid.panel(wash, 14, Some(tone), None);
                    }
                    let indent = left + self.graph.text_from();
                    self.draw_file(
                        grid,
                        (indent, right, y),
                        &change.path,
                        change.status,
                        (measure, theme),
                    );
                }
                Line::Stash(index) => {
                    if let Some(stash) = self.stashes.get(*index) {
                        if hovered {
                            let wash = Bounds::inset(whole, 0.4, 0.06);
                            grid.panel(wash, 14, Some(theme.row_hover), None);
                        }
                        self.draw_stash(grid, (area, y), stash, hover, (measure, theme));
                    }
                }
                Line::Message(0) => self.draw_message(grid, area, y, measure, theme),
                Line::Message(_) | Line::Gap => {}
                Line::Buttons => {
                    self.draw_buttons(grid, (area, y), hover, measure.in_cells(), theme);
                }
                Line::Clean => {
                    grid.label(left, y, "No changes", quiet, right);
                }
                Line::Heading(group, count) => {
                    let name = match group {
                        Group::Merge => "MERGE CHANGES",
                        Group::Staged => "STAGED CHANGES",
                        Group::Changes => "CHANGES",
                    };
                    let number = count.to_string();
                    crate::palette::section_heading(
                        grid,
                        (left, right.saturating_sub(SIDE), y),
                        (name, Some((&number, quiet.fg))),
                        None,
                        theme,
                    );
                    if hovered && *group != Group::Merge {
                        let mark = if *group == Group::Staged { "−" } else { "+" };
                        let x = right.saturating_sub(STAGE_FROM_RIGHT);
                        grid.label(x, y, mark, theme.sidebar, right);
                    }
                }
                Line::Change(index, group) => {
                    let Some(entry) = self.entries.get(*index) else {
                        continue;
                    };
                    let showing = self.shown.as_ref() == Some(&entry.path);
                    if hovered || showing {
                        let tone = if showing {
                            theme.row_active
                        } else {
                            theme.row_hover
                        };
                        let wash = Bounds::inset(whole, 0.4, 0.06);
                        grid.panel(wash, 14, Some(tone), None);
                    }
                    let look = (measure, theme);
                    self.draw_change(grid, area, y, (entry, *group), hovered, look);
                }
            }
        }
    }

    fn draw_message(
        &self,
        grid: &mut Grid,
        area: Rect,
        y: u16,
        measure: &dyn Measure,
        theme: &Theme,
    ) {
        let left = area.x + SIDE;
        let width = area.w.saturating_sub(SIDE * 2);
        let frame = Rect::new(left, y, width, MESSAGE_ROWS);
        let edge = if self.focused {
            theme.accent
        } else {
            theme.popup_border
        };
        let tall = MESSAGE_ROWS as f32 + BOX_ROOM * 2.0;
        let outline = Bounds::cells(left as f32, y as f32 - BOX_ROOM, width as f32, tall);
        grid.panel(outline, 22, Some(theme.text.bg), Some(edge));
        let inner = width.saturating_sub(2).max(1) as f32;
        let hint = "Message (Enter to commit)";
        let rows = if self.message.is_empty() {
            Vec::new()
        } else {
            wrap(&format!("{} ", self.message), inner, measure)
        };
        let writing = Style {
            bg: theme.text.bg,
            ..theme.text
        };
        if rows.is_empty() {
            let quiet = Style {
                fg: theme.gutter.fg,
                ..writing
            };
            grid.label(left + 1, y, hint, quiet, frame.right());
        }
        let shown = rows.len().saturating_sub(MESSAGE_ROWS as usize);
        let visible = rows.iter().skip(shown).take(MESSAGE_ROWS as usize);
        for (offset, row) in visible.enumerate() {
            grid.label(left + 1, y + offset as u16, row, writing, frame.right());
        }
        if self.focused {
            let last = rows.len().saturating_sub(shown).saturating_sub(1) as u16;
            let across = rows.last().map_or(0.0, |r| {
                measure.width(r.trim_end_matches(' '), Font::default())
            });
            grid.set_caret(left as f32 + 1.0 + across, y + last, 0.62);
        }
    }

    /// Commit and Push or Pull. Painted, each is a chip inside its row on the sidebar's
    /// ground; in cells, the fill is all that shows a button.
    fn draw_buttons(
        &self,
        grid: &mut Grid,
        (area, y): (Rect, u16),
        hover: Option<(u16, u16)>,
        in_cells: bool,
        theme: &Theme,
    ) {
        let ground = |fill| if in_cells { fill } else { theme.sidebar.bg };
        let (commit, sync) = self.buttons(area, y);
        let ready = !self.entries.is_empty();
        let fill = if ready {
            theme.button
        } else {
            theme.quiet_button(theme.sidebar.bg)
        };
        let label = Style {
            fg: if ready {
                theme.button_text
            } else {
                theme.gutter.fg
            },
            bg: ground(fill),
            ..theme.sidebar
        };
        let name = self.commit_label();
        let cells = format!("{name:^0$}", commit.w as usize);
        grid.large_button(commit.x, y, &cells, name, fill, label);
        if let (Some(button), Some(name)) = (sync, self.sync_label()) {
            let on_it = hover.is_some_and(|h| button.contains(h.0, h.1));
            let fill = if on_it {
                theme.row_hover
            } else {
                theme.quiet_button(theme.sidebar.bg)
            };
            let label = Style {
                bg: ground(fill),
                ..theme.sidebar
            };
            let cells = format!("{name:^0$}", button.w as usize);
            grid.large_button(button.x, y, &cells, &name, fill, label);
        }
    }

    /// A file a commit changed: its icon, name, folder and letter.
    fn draw_file(
        &self,
        grid: &mut Grid,
        (left, right, y): (u16, u16, u16),
        path: &Path,
        status: char,
        (measure, theme): (&dyn Measure, &Theme),
    ) {
        let name = path
            .file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        grid.glyph(
            left as f32,
            y,
            Icon::File,
            file_color(&name),
            "▪",
            theme.sidebar,
        );
        let end = right.saturating_sub(SIDE + 1);
        let at = left + NAME_FROM_ICON;
        grid.label(at, y, &name, theme.sidebar, end);
        if let Some(folder) = path.parent().filter(|f| !f.as_os_str().is_empty()) {
            let quiet = Style {
                fg: theme.gutter.fg,
                ..theme.sidebar
            };
            let x = at as f32 + measure.width(&name, Font::default()) + FOLDER_GAP;
            let folder = folder.display().to_string();
            grid.label_at(x, x.ceil() as u16, y, &folder, quiet, end, 100);
        }
        let colour = match status {
            'A' => theme.git_added,
            'D' => theme.deleted(),
            _ => theme.git_modified,
        };
        let mark = Style {
            fg: colour,
            ..theme.sidebar
        };
        grid.label_right(
            right.saturating_sub(SIDE),
            y,
            status.encode_utf8(&mut [0; 4]),
            mark,
            100,
        );
    }

    fn draw_underway(
        &self,
        grid: &mut Grid,
        area: Rect,
        y: u16,
        hover: Option<(u16, u16)>,
        (measure, theme): (&dyn Measure, &Theme),
    ) {
        let Some(underway) = &self.underway else {
            return;
        };
        let left = area.x + SIDE;
        let width = area.w.saturating_sub(SIDE * 2);
        let card = Rect::new(left, y, width, 1);
        grid.fill(card, theme.sidebar);
        // What's under way says so in words, and Abort stands apart from them as a button
        // of its own, as VS Code has it.
        let ground = Style {
            fg: theme.gutter.fg,
            ..theme.sidebar
        };
        let abort = card.right().saturating_sub(1 + "Abort".len() as u16);
        let on_abort = hover.is_some_and(|h| h.1 == y && h.0 >= abort && h.0 < card.right());
        let verb = underway.operation.verb();
        // Up to two columns short of Abort, after the verb and its space.
        let room = abort.saturating_sub(left + 2) as f32
            - measure.width(&format!("{verb} "), Font::default());
        let text = format!("{verb} {}", clip_middle(&underway.incoming, room, measure));
        grid.label(left, y, &text, ground, abort.saturating_sub(2));
        let quiet = theme.quiet_button(theme.sidebar.bg);
        let fill = if on_abort {
            theme.quiet_button(quiet)
        } else {
            quiet
        };
        // Painted, the cells keep the row's ground and the chip alone is filled, as every
        // button is; in cells, the fill is all that shows it.
        let word = Style {
            bg: if measure.in_cells() {
                fill
            } else {
                theme.sidebar.bg
            },
            fg: theme.text.fg,
            ..theme.sidebar
        };
        grid.button(abort, y, "Abort", "Abort", fill, word);
    }

    fn draw_stash(
        &self,
        grid: &mut Grid,
        (area, y): (Rect, u16),
        stash: &Stash,
        hover: Option<(u16, u16)>,
        (measure, theme): (&dyn Measure, &Theme),
    ) {
        let left = area.x + SIDE;
        let right = area.right().saturating_sub(SIDE);
        let quiet = Style {
            fg: theme.gutter.fg,
            ..theme.sidebar
        };
        let on_row = hover.is_some_and(|h| h.1 == y && area.contains(h.0, h.1));
        let mut end = right;
        if on_row {
            for (word, _) in STASH_ACTIONS {
                let start = end.saturating_sub(word.len() as u16);
                let on_word = hover.is_some_and(|h| h.0 >= start && h.0 < end);
                let style = if on_word { theme.sidebar } else { quiet };
                grid.label(start, y, word, style, end);
                end = start.saturating_sub(1);
            }
        } else {
            let age = cellix_review::age(stash.time, now());
            grid.label_right(right, y, &age, quiet, 90);
            end = right.saturating_sub(age.chars().count() as u16 + 1);
        }
        let at = left + NAME_FROM_ICON;
        let title = clip(stash.title(), end.saturating_sub(at + 1) as f32, measure);
        grid.label(at, y, &title, theme.sidebar, end.saturating_sub(1));
    }

    fn draw_change(
        &self,
        grid: &mut Grid,
        area: Rect,
        y: u16,
        (entry, group): (&Entry, Group),
        hovered: bool,
        (measure, theme): (&dyn Measure, &Theme),
    ) {
        let right = area.right();
        let staged = group == Group::Staged;
        let letter = entry.letter(staged);
        let colour = match letter {
            'A' | 'U' => theme.git_added,
            'D' | 'C' => theme.deleted(),
            _ => theme.git_modified,
        };
        let name = entry
            .path
            .file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        let end = right.saturating_sub(if hovered {
            DISCARD_FROM_RIGHT + 1
        } else {
            LETTER_FROM_RIGHT + 1
        });
        let icon = area.x + SIDE;
        grid.glyph(
            icon as f32,
            y,
            Icon::File,
            file_color(&name),
            "▪",
            theme.sidebar,
        );
        let at = icon + NAME_FROM_ICON;
        grid.label(at, y, &name, theme.sidebar, end);
        if let Some(folder) = entry.path.parent().filter(|f| !f.as_os_str().is_empty()) {
            let quiet = Style {
                fg: theme.gutter.fg,
                ..theme.sidebar
            };
            // Set after the name as it's drawn, which isn't how many cells it would fill.
            let x = at as f32 + measure.width(&name, Font::default()) + FOLDER_GAP;
            let folder = folder.display().to_string();
            grid.label_at(x, x.ceil() as u16, y, &folder, quiet, end, 100);
        }
        let mark = Style {
            fg: colour,
            ..theme.sidebar
        };
        let edge = right.saturating_sub(SIDE);
        grid.label_right(edge, y, letter.encode_utf8(&mut [0; 4]), mark, 100);
        if group == Group::Merge {
            if hovered && self.settled.contains(&entry.path) {
                let stage = right.saturating_sub(STAGE_FROM_RIGHT);
                grid.glyph(
                    stage as f32,
                    y,
                    Icon::Check,
                    theme.git_added,
                    "✓",
                    theme.sidebar,
                );
            }
            return;
        }
        if hovered {
            let stage = right.saturating_sub(STAGE_FROM_RIGHT);
            grid.label(
                stage,
                y,
                if staged { "−" } else { "+" },
                theme.sidebar,
                stage + 2,
            );
            if !staged {
                let discard = right.saturating_sub(DISCARD_FROM_RIGHT);
                grid.label(discard, y, "↺", theme.sidebar, discard + 2);
            }
        }
    }
}

/// Text cut to `room` cells as it's drawn, ending in an ellipsis when it doesn't fit.
pub fn clip(text: &str, room: f32, measure: &dyn Measure) -> String {
    let wide = |t: &str| measure.width(t, Font::default());
    if wide(text) <= room {
        return text.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    let mut keep = chars.len();
    while keep > 0 {
        keep -= 1;
        let cut = format!("{}…", chars[..keep].iter().collect::<String>().trim_end());
        if wide(&cut) <= room {
            return cut;
        }
    }
    "…".to_string()
}

/// Text cut to `room` cells from the middle, so both ends of a branch's name still show.
fn clip_middle(text: &str, room: f32, measure: &dyn Measure) -> String {
    let wide = |t: &str| measure.width(t, Font::default());
    let chars: Vec<char> = text.chars().collect();
    if wide(text) <= room || chars.len() < 5 {
        return text.to_string();
    }
    let mut keep = chars.len() - 1;
    while keep > 2 {
        let head = keep / 2;
        let tail = keep - head;
        let start: String = chars[..head].iter().collect();
        let end: String = chars[chars.len() - tail..].iter().collect();
        let cut = format!("{start}…{end}");
        if wide(&cut) <= room {
            return cut;
        }
        keep -= 1;
    }
    clip(text, room, measure)
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

/// Conflicted files whose text no longer holds a conflict.
fn settled(root: &Path, entries: &[Entry]) -> Vec<PathBuf> {
    let conflicted = entries.iter().filter(|e| e.is_conflicted());
    conflicted
        .filter(|e| {
            // A file deleted on one side has no text to settle; staging it settles it.
            !std::fs::read_to_string(root.join(&e.path)).is_ok_and(|t| has_conflicts(&t))
        })
        .map(|e| e.path.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_read_from_before_a_change_doesnt_undo_it() {
        let dir = tempfile::tempdir().unwrap();
        let (late, reading) = mpsc::channel();
        let mut source = Source {
            reading: Some(reading),
            ..Source::default()
        };
        source.reread(dir.path());
        let stale = Entry {
            path: PathBuf::from("gone.txt"),
            index: 'M',
            worktree: ' ',
        };
        // The channel it came down was dropped, so nothing can arrive by it.
        let stale = Read {
            entries: vec![stale],
            ..Read::default()
        };
        assert!(late.send(stale).is_err());
    }
}
