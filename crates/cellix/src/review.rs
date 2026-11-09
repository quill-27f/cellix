use std::cell::Cell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::mpsc::{self, Receiver};

use cellix_agent::TestRun;
use cellix_lang::{Registry, Token};
use cellix_render::{
    BOTTOM_LEFT, BOTTOM_RIGHT, Bounds, Grid, Rect, Rgb, Style, TOP_LEFT, TOP_RIGHT,
};
use cellix_review::{
    Change, Comment, FileDiff, Row, RowKind, Side, Turn, changed_between, changed_files,
    changed_since, commit, head_text, hunk_patch, index_text, numstat, numstat_between,
    rank_changes, stage_patch, staged_count, text_at, turns,
};

use crate::theme::Theme;

const LIST_WIDTH: u16 = 24;
const ACCEPT: &str = "[Accept]";
const REJECT: &str = "[Reject]";
const NUMBER_WIDTH: u16 = 5;
/// How far a card's top edge reaches into the gap above it, and how far its header's
/// words and buttons are raised, in rows: air of about 11 px above and below the buttons.
const HEADER_REACH: f32 = 0.4;
const HEADER_LIFT: f32 = 0.2;
/// How far a card reaches under its last line, in rows: what's left of the gap to the next
/// card's top edge stays clear of the 8 px rule.
const CARD_FOOT: f32 = 0.3;
/// Columns from the end of Review's title to its ×: about 45 px, as a tab's.
const CLOSE_GAP: f32 = 2.6;
/// Below this, two code columns are too narrow to read, so lines stack in one.
/// Narrower than this, the list of files makes way for the cards.
pub const MIN_SIDE_BY_SIDE: u16 = 90;
/// Corners as a percentage of a row's height.
const CARD_RADIUS: u8 = 30;
const HUNK_LABEL: usize = 120;
/// Turns listed in the timeline, newest first.
const SHOWN_TURNS: usize = 6;
const SHOWN_TESTS: usize = 3;

/// Columns of a stacked line that hold its number.
const LINE_NUMBER_WIDTH: u16 = 8;

/// What a click in the review view lands on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Hit {
    File(usize),
    Accept(usize),
    Reject(usize),
    /// A line number, which starts a comment on that line of the new version, or the old
    /// one if it was deleted.
    /// A line number: the file, the line, its text, and whether the change took it out.
    Line(String, usize, String, bool),
    /// The text of a line, which a double click opens in its file.
    Text(String, usize),
    /// Compare against this turn, or HEAD for None.
    Base(Option<usize>),
    Commit,
    /// Show a test run's output.
    Test(usize),
    AcceptProposal,
    RejectProposal,
    /// Step through the files, in the stacked layout.
    PreviousFile,
    NextFile,
    /// The × at the end of the title: back to the file that was open.
    Close,
}

/// A row of the stacked layout.
enum Stacked<'a> {
    /// Space between two cards.
    Gap,
    Header(usize),
    Line(&'a Side, RowKind),
    Note(&'a str),
    /// After a file's last change, what's next: the next file, or that it's all been seen.
    End,
}

/// A row of the column beside the diff.
#[derive(Clone, Debug, PartialEq, Eq)]
enum ListRow {
    Heading(&'static str),
    Base(Option<usize>),
    Commit,
    Test(usize),
    File(usize),
}

/// Something fixed to read, rather than the working tree against HEAD or a turn: nothing in
/// it can be accepted or rejected.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Fixed {
    /// A commit against its first parent.
    Commit(cellix_review::Commit),
    /// A stash against the commit it was made on: its commit and its title.
    Stash(String, String),
    /// One file against a branch, a commit or the clipboard: what the title row says, the
    /// old text, and the file as the editor has it.
    File {
        title: String,
        old: String,
        new: String,
    },
}

impl Fixed {
    /// What the title row says beside the file.
    fn describe(&self) -> String {
        match self {
            Fixed::Commit(commit) => format!("{} {}", commit.short(), commit.summary),
            Fixed::Stash(_, title) => format!("stash: {title}"),
            Fixed::File { title, .. } => title.clone(),
        }
    }

    fn heading(&self) -> &'static str {
        match self {
            Fixed::Commit(_) => "IN THIS COMMIT",
            Fixed::Stash(..) => "IN THIS STASH",
            Fixed::File { .. } => "COMPARED",
        }
    }
}

/// Colour spans for one line: start, end and token, in chars.
type LineTokens = Vec<(usize, usize, Token)>;

/// The Changes view: files that differ from a base, and a diff of one of them. The base is
/// HEAD, or the checkpoint taken at the start of an agent's turn.
pub struct Review {
    /// The Source Control panel stands beside this, so it has no list of its own.
    pub beside_panel: bool,
    /// How wide the interface's text is drawn, so the title's parts and its × sit by what's
    /// written, not by cells. None in cells.
    pub measure: Option<std::rc::Rc<dyn cellix_render::Measure>>,
    /// The column the title's × was drawn at, for a click to find.
    close_at: Cell<u16>,
    /// Columns the title row leaves free at its end, for what stands there.
    pub title_room: Cell<u16>,
    /// The list's width: the sidebar's, so every sidebar is the same.
    pub list_width: Cell<u16>,
    /// A file to show once the changes have been read.
    wanted: Option<PathBuf>,
    root: PathBuf,
    registry: Rc<Registry>,
    pub changes: Vec<Change>,
    pub selected: usize,
    pub diff: FileDiff,
    old: String,
    new: String,
    top: usize,
    pub turns: Vec<Turn>,
    /// The index into `turns` being compared against, or None for HEAD.
    pub base: Option<usize>,
    /// The checkpoint a past turn ended at, to show only what that turn changed. Nothing in
    /// it can be accepted or rejected: the tree has moved on since.
    pub until: Option<String>,
    staged: usize,
    /// Test commands agents have run, newest last.
    pub tests: Vec<TestRun>,
    /// Set when this shows a change an agent is proposing, rather than the working tree.
    /// One entry per hunk, true once it's been accepted.
    proposal: Option<Vec<bool>>,
    /// Lines added and removed, by file.
    stats: HashMap<PathBuf, (usize, usize)>,
    tokens: (Vec<LineTokens>, Vec<LineTokens>),
    /// A read of git on its way, so a click never waits for it.
    reading: Option<Receiver<Reading>>,
    /// Where the pointer is, set each time the view is drawn.
    hover: Cell<Option<(u16, u16)>>,
    /// A commit, stash or comparison to read, in place of the working tree.
    pub fixed: Option<Fixed>,
}

/// What git says about the tree, gathered off the main thread: several git commands take
/// long enough that doing them on a click makes the click feel ignored.
struct Reading {
    base: Option<usize>,
    turns: Vec<Turn>,
    changes: Vec<Change>,
    stats: HashMap<PathBuf, (usize, usize)>,
    staged: usize,
    /// The file that was selected, with its two sides.
    file: Option<(PathBuf, String, String)>,
}

impl Reading {
    fn take(
        root: &Path,
        base: Option<usize>,
        until: Option<&str>,
        selected: Option<PathBuf>,
    ) -> Self {
        let turns = turns(root);
        let base = base.filter(|b| *b < turns.len());
        let commit = base.map(|b| turns[b].commit.clone());
        let until = until.filter(|_| commit.is_some());
        let mut changes = match (&commit, until) {
            (Some(commit), Some(until)) => changed_between(root, commit, until),
            (Some(commit), None) => changed_since(root, commit),
            (None, _) => changed_files(root),
        };
        rank_changes(&mut changes);
        let stats = match (&commit, until) {
            (Some(commit), Some(until)) => numstat_between(root, commit, until),
            _ => numstat(root, commit.as_deref()),
        };
        let staged = staged_count(root);
        let path = selected
            .filter(|p| changes.iter().any(|c| c.path == *p))
            .or_else(|| changes.first().map(|c| c.path.clone()));
        let file = path.map(|path| {
            let old = match &commit {
                Some(commit) => text_at(root, commit, &path),
                None => head_text(root, &path),
            };
            let new = match until {
                Some(until) => text_at(root, until, &path),
                None => std::fs::read_to_string(root.join(&path)).unwrap_or_default(),
            };
            (path, old, new)
        });
        Self {
            base,
            turns,
            changes,
            stats,
            staged,
            file,
        }
    }
}

/// Splits a file's highlight spans into per-line columns.
fn line_tokens(registry: &Registry, path: &Path, text: &str) -> Vec<LineTokens> {
    let spans = registry.highlight(path, text);
    let mut lines = Vec::new();
    let mut byte = 0;
    let mut spans = spans.iter().peekable();
    for line in text.split_inclusive('\n') {
        let end = byte + line.len();
        let mut tokens = Vec::new();
        while let Some(span) = spans.peek() {
            if span.start >= end {
                break;
            }
            let from = span.start.max(byte) - byte;
            let to = span.end.min(end) - byte;
            if from < to && line.is_char_boundary(from) && line.is_char_boundary(to) {
                let start = line[..from].chars().count();
                tokens.push((start, start + line[from..to].chars().count(), span.token));
            }
            if span.end > end {
                break;
            }
            spans.next();
        }
        lines.push(tokens);
        byte = end;
    }
    lines
}

fn tint(style: Style, bg: Rgb) -> Style {
    Style { bg, ..style }
}

/// What decorates a side's text.
#[derive(Clone, Copy)]
struct SideStyle<'a> {
    base: Style,
    number_fg: Rgb,
    /// Background of the words that changed within the line.
    word_bg: Rgb,
    tokens: Option<&'a LineTokens>,
}

fn draw_side(
    grid: &mut Grid,
    area: Rect,
    y: u16,
    side: Option<&Side>,
    look: SideStyle,
    theme: &Theme,
) {
    let style = look.base;
    grid.fill(Rect::new(area.x, y, area.w, 1), style);
    let Some(side) = side else {
        return;
    };
    let number = format!("{:>4} ", side.number);
    let gutter = Style {
        fg: look.number_fg,
        ..style
    };
    grid.put_str(area.x, y, &number, gutter, area.right());
    let text = side.text.replace('\t', "    ");
    grid.put_str(area.x + NUMBER_WIDTH, y, &text, style, area.right());
    // Tabs widen when shown, so char positions need mapping to columns.
    let column = |at: usize| at + side.text.chars().take(at).filter(|c| *c == '\t').count() * 3;
    let start = area.x + NUMBER_WIDTH;
    let span = |from: usize, to: usize| {
        let x = start.saturating_add(column(from) as u16);
        let width = (column(to) - column(from)) as u16;
        Rect::new(x, y, width.min(area.right().saturating_sub(x)), 1)
    };
    for (from, to, token) in look.tokens.into_iter().flatten() {
        let fg = theme.token(*token);
        grid.set_style(span(*from, *to), |s| Style { fg, ..s });
    }
    for (from, to) in &side.changed {
        let bg = look.word_bg;
        grid.set_style(span(*from, *to), |s| Style { bg, ..s });
    }
}

impl Review {
    pub fn new(root: PathBuf, registry: Rc<Registry>) -> Self {
        let mut review = Self {
            root,
            registry,
            changes: Vec::new(),
            selected: 0,
            diff: FileDiff::default(),
            old: String::new(),
            new: String::new(),
            top: 0,
            turns: Vec::new(),
            base: None,
            until: None,
            staged: 0,
            tests: Vec::new(),
            proposal: None,
            stats: HashMap::new(),
            tokens: (Vec::new(), Vec::new()),
            reading: None,
            hover: Cell::new(None),
            beside_panel: false,
            title_room: Cell::new(0),
            measure: None,
            close_at: Cell::new(u16::MAX),
            list_width: Cell::new(LIST_WIDTH),
            wanted: None,
            fixed: None,
        };
        review.refresh();
        review
    }

    /// A commit, a stash, or one file against something else, to read. `path` is the file to
    /// show first.
    pub fn fixed(root: PathBuf, registry: Rc<Registry>, fixed: Fixed, path: Option<&Path>) -> Self {
        let changes = match &fixed {
            Fixed::Commit(commit) => cellix_review::commit_files(&root, commit),
            Fixed::Stash(hash, _) => cellix_review::stash_files(&root, hash),
            Fixed::File { .. } => path
                .map(|p| Change {
                    path: p.to_path_buf(),
                    status: 'M',
                })
                .into_iter()
                .collect(),
        };
        let mut review =
            Self::proposing(root, registry, PathBuf::new(), String::new(), String::new());
        review.proposal = None;
        review.changes = changes;
        review.fixed = Some(fixed);
        review.selected = path
            .and_then(|p| review.changes.iter().position(|c| c.path == p))
            .unwrap_or(0);
        review.load();
        review
    }

    /// A change an agent wants to make to a file. Nothing is on disk yet.
    pub fn proposing(
        root: PathBuf,
        registry: Rc<Registry>,
        path: PathBuf,
        old: String,
        new: String,
    ) -> Self {
        let mut review = Self {
            root,
            registry,
            changes: vec![Change { path, status: 'M' }],
            selected: 0,
            diff: FileDiff::default(),
            old,
            new,
            top: 0,
            turns: Vec::new(),
            base: None,
            until: None,
            staged: 0,
            tests: Vec::new(),
            proposal: Some(Vec::new()),
            stats: HashMap::new(),
            reading: None,
            hover: Cell::new(None),
            tokens: (Vec::new(), Vec::new()),
            beside_panel: false,
            title_room: Cell::new(0),
            measure: None,
            close_at: Cell::new(u16::MAX),
            list_width: Cell::new(LIST_WIDTH),
            wanted: None,
            fixed: None,
        };
        review.reload_proposal();
        review.proposal = Some(vec![false; review.diff.hunks.len()]);
        review
    }

    fn reload_proposal(&mut self) {
        self.diff = FileDiff::new(&self.old, &self.new);
        if let Some(path) = self.current_path().map(Path::to_path_buf) {
            self.tokens = (
                line_tokens(&self.registry, &path, &self.old),
                line_tokens(&self.registry, &path, &self.new),
            );
        }
        self.top = self.top.min(self.diff.rows.len().saturating_sub(1));
    }

    /// Where a hunk is, counting lines from one, and its first line: what was added, or what
    /// was removed when nothing was.
    pub fn hunk_place(&self, hunk: usize) -> Option<(usize, String, bool)> {
        let hunk = self.diff.hunks.get(hunk)?;
        let old: Vec<&str> = self.old.lines().collect();
        let new: Vec<&str> = self.new.lines().collect();
        // Past the context the hunk carries, to the first line that changed.
        let mut at = 0;
        while at < hunk.old_len.min(hunk.new_len)
            && old.get(hunk.old_start + at) == new.get(hunk.new_start + at)
        {
            at += 1;
        }
        let (lines, start, removed) = if at < hunk.new_len {
            (&new, hunk.new_start + at, false)
        } else {
            (&old, hunk.old_start + at, true)
        };
        let line = lines.get(start).map_or("", |l| l.trim()).to_string();
        Some((start + 1, line, removed))
    }

    pub fn is_proposal(&self) -> bool {
        self.proposal.is_some()
    }

    /// The proposed text, as the reviewer has left it.
    pub fn proposed_text(&self) -> &str {
        &self.new
    }

    pub fn proposed_unchanged(&self) -> bool {
        self.old == self.new
    }

    /// Keeps one proposed hunk. Returns whether every hunk is now decided.
    pub fn keep_proposed(&mut self, hunk: usize) -> bool {
        let Some(kept) = &mut self.proposal else {
            return false;
        };
        if let Some(entry) = kept.get_mut(hunk) {
            *entry = true;
        }
        kept.iter().all(|k| *k)
    }

    /// Drops one proposed hunk. Returns whether every hunk is now decided.
    pub fn drop_proposed(&mut self, hunk: usize) -> bool {
        let Some(text) = self.rejected_text(hunk) else {
            return false;
        };
        self.new = text;
        self.reload_proposal();
        let Some(kept) = &mut self.proposal else {
            return false;
        };
        if hunk < kept.len() {
            kept.remove(hunk);
        }
        kept.iter().all(|k| *k)
    }

    /// Rereads git and the selected file now, for when what follows depends on it. Keeps
    /// the selection where it can.
    pub fn refresh(&mut self) {
        if self.is_proposal() || self.fixed.is_some() {
            return;
        }
        self.reading = None;
        let current = self.current_path().map(Path::to_path_buf);
        let until = self.until.clone();
        self.apply(Reading::take(
            &self.root,
            self.base,
            until.as_deref(),
            current,
        ));
    }

    /// Rereads in the background. `poll` picks up the result.
    pub fn refresh_later(&mut self) {
        if self.is_proposal() || self.fixed.is_some() {
            return;
        }
        let (root, base, until) = (self.root.clone(), self.base, self.until.clone());
        let current = self.current_path().map(Path::to_path_buf);
        let (sender, receiver) = mpsc::channel();
        self.reading = Some(receiver);
        std::thread::spawn(move || {
            let _ = sender.send(Reading::take(&root, base, until.as_deref(), current));
        });
    }

    /// Whether a background read is still out.
    pub fn is_reading(&self) -> bool {
        self.reading.is_some()
    }

    /// Takes in a finished read, and says whether anything arrived.
    pub fn poll(&mut self) -> bool {
        let Some(reading) = self.reading.as_ref().and_then(|r| r.try_recv().ok()) else {
            return false;
        };
        self.reading = None;
        self.apply(reading);
        true
    }

    fn apply(&mut self, reading: Reading) {
        let current = self
            .wanted
            .take()
            .or_else(|| self.current_path().map(Path::to_path_buf));
        self.base = reading.base;
        self.turns = reading.turns;
        self.changes = reading.changes;
        self.stats = reading.stats;
        self.staged = reading.staged;
        self.selected = current
            .and_then(|p| self.changes.iter().position(|c| c.path == p))
            .unwrap_or(0);
        let Some((path, old, new)) = reading.file else {
            self.load();
            return;
        };
        if self.current_path() != Some(path.as_path()) {
            // The selection moved while this was being read.
            self.load();
        } else if old != self.old || new != self.new || self.diff.rows.is_empty() {
            self.show_file(&path, old, new);
        }
    }

    fn base_commit(&self) -> Option<String> {
        Some(self.turns.get(self.base?)?.commit.clone())
    }

    /// Compares against a turn, or HEAD for None.
    pub fn set_base(&mut self, base: Option<usize>) {
        self.base = base;
        self.until = None;
        self.top = 0;
        self.refresh_later();
    }

    /// Shows what one turn changed: from the checkpoint at `commit` to the one at `until`, or
    /// up to the tree now for the latest turn. Says whether that checkpoint is known.
    pub fn show_turn(&mut self, commit: &str, until: Option<String>) -> bool {
        let Some(base) = self.turns.iter().position(|t| t.commit == commit) else {
            return false;
        };
        self.base = Some(base);
        self.until = until;
        self.top = 0;
        self.refresh();
        true
    }

    /// Steps the base one turn back, from HEAD to the newest turn and on.
    pub fn previous_turn(&mut self) {
        let base = match self.base {
            None => self.turns.len().checked_sub(1),
            Some(index) => index.checked_sub(1),
        };
        self.set_base(base);
    }

    /// The commit an undo goes back to: the turn in view, or the newest.
    pub fn undo_target(&self) -> Option<Turn> {
        self.base
            .and_then(|b| self.turns.get(b))
            .or(self.turns.last())
            .cloned()
    }

    pub fn current_path(&self) -> Option<&Path> {
        self.changes.get(self.selected).map(|c| c.path.as_path())
    }

    fn load(&mut self) {
        let Some(path) = self.current_path().map(Path::to_path_buf) else {
            self.diff = FileDiff::default();
            self.tokens = (Vec::new(), Vec::new());
            return;
        };
        let (old, new) = match &self.fixed {
            Some(Fixed::Commit(commit)) => cellix_review::commit_sides(&self.root, commit, &path),
            Some(Fixed::Stash(hash, _)) => cellix_review::stash_sides(&self.root, hash, &path),
            Some(Fixed::File { old, new, .. }) => (old.clone(), new.clone()),
            None => {
                let old = match self.base_commit() {
                    Some(commit) => text_at(&self.root, &commit, &path),
                    None => head_text(&self.root, &path),
                };
                let new = std::fs::read_to_string(self.root.join(&path)).unwrap_or_default();
                (old, new)
            }
        };
        self.show_file(&path, old, new);
    }

    fn show_file(&mut self, path: &Path, old: String, new: String) {
        self.old = old;
        self.new = new;
        self.diff = FileDiff::new(&self.old, &self.new);
        self.tokens = (
            line_tokens(&self.registry, path, &self.old),
            line_tokens(&self.registry, path, &self.new),
        );
        self.top = self.top.min(self.diff.rows.len().saturating_sub(1));
    }

    /// Shows one file, now or when the list of changes has been read.
    pub fn show_path(&mut self, path: PathBuf) {
        match self.changes.iter().position(|c| c.path == path) {
            Some(index) => self.select(index),
            None => self.wanted = Some(path),
        }
    }

    pub fn select(&mut self, index: usize) {
        if self.is_proposal() {
            return;
        }
        self.selected = index;
        self.top = 0;
        self.load();
    }

    pub fn scroll_by(&mut self, delta: isize) {
        let last = self.stacked(&[]).len().saturating_sub(1) as isize;
        self.top = (self.top as isize + delta).clamp(0, last.max(0)) as usize;
    }

    pub fn absolute_path(&self) -> Option<PathBuf> {
        Some(self.root.join(self.current_path()?))
    }

    /// Stages one hunk in git.
    pub fn accept(&mut self, hunk: usize) -> Result<(), String> {
        let path = self.current_path().ok_or("No file selected")?.to_path_buf();
        let hunk = self.diff.hunks.get(hunk).ok_or("No such change")?;
        let patch = hunk_patch(&path, &self.old, &self.new, hunk);
        stage_patch(&self.root, &path, &patch)?;
        self.staged = staged_count(&self.root);
        Ok(())
    }

    /// Whether a hunk's new text has already reached the index.
    pub fn is_accepted(&self, hunk: usize) -> bool {
        if self.fixed.is_some() {
            return false;
        }
        if let Some(kept) = &self.proposal {
            return kept.get(hunk).copied().unwrap_or(false);
        }
        let (Some(path), Some(h)) = (self.current_path(), self.diff.hunks.get(hunk)) else {
            return false;
        };
        let index = index_text(&self.root, path);
        if index == self.new {
            return true;
        }
        let lines: Vec<&str> = self.new.split_inclusive('\n').collect();
        let block = lines[h.new_start..h.new_start + h.new_len].concat();
        let old_lines: Vec<&str> = self.old.split_inclusive('\n').collect();
        let before = old_lines[h.old_start..h.old_start + h.old_len].concat();
        !block.is_empty() && index.contains(&block) && !index.contains(&before)
    }

    pub fn commit(&mut self, message: &str) -> Result<String, String> {
        let result = commit(&self.root, message);
        self.refresh();
        result
    }

    /// The file's text with one hunk put back, to be written by the caller.
    pub fn rejected_text(&self, hunk: usize) -> Option<String> {
        let hunk = self.diff.hunks.get(hunk)?;
        Some(cellix_review::reject_hunk(&self.old, &self.new, hunk))
    }

    fn list_rows(&self) -> Vec<ListRow> {
        let mut rows = Vec::new();
        if self.is_proposal() {
            rows.push(ListRow::Heading("PROPOSED CHANGE"));
            rows.extend((0..self.changes.len()).map(ListRow::File));
            return rows;
        }
        if let Some(fixed) = &self.fixed {
            rows.push(ListRow::Heading(fixed.heading()));
            rows.extend((0..self.changes.len()).map(ListRow::File));
            return rows;
        }
        if !self.turns.is_empty() {
            rows.push(ListRow::Heading("COMPARE WITH"));
            rows.push(ListRow::Base(None));
            let newest = (0..self.turns.len()).rev().take(SHOWN_TURNS);
            rows.extend(newest.map(|i| ListRow::Base(Some(i))));
        }
        if !self.tests.is_empty() {
            rows.push(ListRow::Heading("TESTS"));
            let newest = (0..self.tests.len()).rev().take(SHOWN_TESTS);
            rows.extend(newest.map(ListRow::Test));
        }
        rows.push(ListRow::Heading("CHANGES"));
        rows.extend((0..self.changes.len()).map(ListRow::File));
        if self.staged > 0 {
            rows.push(ListRow::Commit);
        }
        rows
    }

    fn side_style<'a>(
        &self,
        side: &Option<Side>,
        bg: Rgb,
        tokens: Option<&'a LineTokens>,
        number_fg: Rgb,
        theme: &Theme,
    ) -> SideStyle<'a> {
        let moved = side.as_ref().is_some_and(|s| s.moved);
        SideStyle {
            base: tint(theme.text, if moved { theme.moved } else { bg }),
            number_fg,
            word_bg: if bg == theme.removed {
                theme.removed_word
            } else {
                theme.added_word
            },
            tokens,
        }
    }

    /// The diff as cards, one to a hunk: an old line, then its replacement under it, and
    /// any comment after the line it's on.
    fn stacked<'a>(&'a self, comments: &'a [Comment]) -> Vec<Stacked<'a>> {
        let file = self
            .current_path()
            .map(|p| p.to_string_lossy().into_owned());
        let mut rows = Vec::new();
        let note = |rows: &mut Vec<Stacked<'a>>, side: &Side| {
            let here = |c: &&Comment| Some(&c.file) == file.as_ref() && c.line == side.number;
            rows.extend(comments.iter().filter(here).map(|c| Stacked::Note(&c.text)));
        };
        for row in &self.diff.rows {
            match row {
                // A gap before every card, the first too: the card's top edge reaches into it,
                // so its header has air above as well as below.
                Row::Header(hunk) => {
                    rows.push(Stacked::Gap);
                    rows.push(Stacked::Header(*hunk));
                }
                Row::Line {
                    left,
                    right,
                    kind: RowKind::Same,
                } => {
                    if let Some(side) = right.as_ref().or(left.as_ref()) {
                        rows.push(Stacked::Line(side, RowKind::Same));
                        note(&mut rows, side);
                    }
                }
                Row::Line { left, right, .. } => {
                    if let Some(side) = left {
                        rows.push(Stacked::Line(side, RowKind::Removed));
                    }
                    if let Some(side) = right {
                        rows.push(Stacked::Line(side, RowKind::Added));
                        note(&mut rows, side);
                    }
                }
            }
        }
        if !rows.is_empty() {
            rows.push(Stacked::Gap);
            rows.push(Stacked::End);
        }
        rows
    }

    /// What the row after a file's last change says. Only the working tree's changes can be
    /// committed: a commit, a stash or a comparison just ends.
    fn end_words(&self) -> String {
        if let Some(next) = self.changes.get(self.selected + 1) {
            return format!("End of this file. Next: {}", next.path.display());
        }
        match &self.fixed {
            None => "That's every change. Commit when it's ready.",
            Some(Fixed::Commit(_)) => "That's every change in this commit.",
            Some(Fixed::Stash(..)) => "That's every change in this stash.",
            Some(Fixed::File { .. }) => "That's every difference.",
        }
        .to_string()
    }

    /// What code surrounds a hunk: the nearest line above that sits further out.
    fn hunk_label(&self, hunk: usize) -> String {
        let fallback = format!("Change {}", hunk + 1);
        let Some(h) = self.diff.hunks.get(hunk) else {
            return fallback;
        };
        let lines: Vec<&str> = self.new.lines().collect();
        let depth = |line: &str| line.len() - line.trim_start().len();
        let start = h.new_start.min(lines.len().saturating_sub(1));
        let here = lines.get(start).map_or(0, |line| depth(line));
        let above = lines[..start]
            .iter()
            .rev()
            .find(|line| !line.trim().is_empty() && depth(line) < here.max(1));
        above.map_or(fallback, |line| {
            line.trim()
                .trim_end_matches(['{', ' '])
                .chars()
                .take(HUNK_LABEL)
                .collect()
        })
    }

    /// Where the list and the cards go. The list makes way in a narrow pane.
    fn layout(&self, area: Rect) -> (Option<Rect>, Rect) {
        if area.w < MIN_SIDE_BY_SIDE || self.beside_panel {
            return (None, area);
        }
        let list = Rect::new(area.x, area.y, self.list_width.get().min(area.w), area.h);
        let x = list.right() + 1;
        (
            Some(list),
            Rect::new(x, area.y, area.right().saturating_sub(x + 1), area.h),
        )
    }

    /// The two buttons at the end of a row, as (accept x, reject x).
    fn button_spots(body: Rect) -> (u16, u16) {
        // As far in from the card's right as its name is from its left.
        let accept = body.right().saturating_sub(ACCEPT.len() as u16 + 2);
        (accept, accept.saturating_sub(REJECT.len() as u16 + 1))
    }

    /// The body with its width cut to what the title row may use.
    fn title_body(&self, body: Rect) -> Rect {
        let width = body.w.saturating_sub(self.title_room.get());
        Rect::new(body.x, body.y, width, body.h)
    }

    fn title_hit(&self, body: Rect, x: u16) -> Option<Hit> {
        let body = self.title_body(body);
        if self.is_proposal() {
            let (accept, reject) = Self::button_spots(body);
            return if x >= accept {
                Some(Hit::AcceptProposal)
            } else if x >= reject {
                Some(Hit::RejectProposal)
            } else {
                None
            };
        }
        if self.on_close(x) {
            return Some(Hit::Close);
        }
        Some(if x < body.x + body.w / 2 {
            Hit::PreviousFile
        } else {
            Hit::NextFile
        })
    }

    fn hit_cards(
        &self,
        list: bool,
        body: Rect,
        x: u16,
        y: u16,
        comments: &[Comment],
    ) -> Option<Hit> {
        if y == body.y {
            // With a list to pick files from, only the title's × answers.
            return if list && !self.is_proposal() {
                self.on_close(x).then_some(Hit::Close)
            } else {
                self.title_hit(body, x)
            };
        }
        let row = (y - body.y - 1) as usize;
        match self.stacked(comments).get(self.top + row)? {
            Stacked::Header(_) if self.fixed.is_some() || self.until.is_some() => None,
            Stacked::Header(hunk) => {
                let (accept, reject) = Self::button_spots(body);
                if x >= accept && x < accept + ACCEPT.len() as u16 {
                    Some(Hit::Accept(*hunk))
                } else if x >= reject && x < reject + REJECT.len() as u16 {
                    Some(Hit::Reject(*hunk))
                } else {
                    None
                }
            }
            Stacked::Line(side, kind) => {
                let file = self.current_path()?.to_string_lossy().into_owned();
                // Only the numbers: a click in the code is someone reading it.
                let in_numbers = x < body.x + LINE_NUMBER_WIDTH;
                let removed = *kind == RowKind::Removed;
                Some(if in_numbers {
                    Hit::Line(file, side.number, side.text.trim().to_string(), removed)
                } else {
                    Hit::Text(file, side.number)
                })
            }
            Stacked::End => (self.selected + 1 < self.changes.len()).then_some(Hit::NextFile),
            Stacked::Gap | Stacked::Note(_) => None,
        }
    }

    pub fn step_file(&mut self, delta: isize) {
        let count = self.changes.len() as isize;
        if count > 0 {
            self.select((self.selected as isize + delta).rem_euclid(count) as usize);
        }
    }

    fn draw_title(&self, grid: &mut Grid, body: Rect, narrow: bool, theme: &Theme) {
        let body = self.title_body(body);
        let y = body.y;
        let strong = Style {
            bold: true,
            fg: theme.heading,
            ..theme.sidebar
        };
        let dim = Style {
            fg: theme.status.fg,
            ..theme.sidebar
        };
        let Some(file) = self
            .current_path()
            .map(|p| p.to_string_lossy().into_owned())
        else {
            grid.label(body.x + 2, y, "No changes", dim, body.right());
            return;
        };
        if self.is_proposal() {
            let (accept, reject) = Self::button_spots(body);
            let primary = Style {
                fg: theme.button_text,
                ..theme.sidebar
            };
            let quiet = Style {
                fg: theme.text.fg,
                ..theme.sidebar
            };
            grid.label(body.x + 2, y, &file, strong, reject.saturating_sub(1));
            let reject_fill = if self.on_button(reject, y, REJECT) {
                theme.quiet_button(theme.tint)
            } else {
                theme.tint
            };
            let accept_fill = if self.on_button(accept, y, ACCEPT) {
                theme.quiet_button(theme.button)
            } else {
                theme.button
            };
            grid.button(reject, y, REJECT, "Reject", reject_fill, quiet);
            grid.button(accept, y, ACCEPT, "Accept", accept_fill, primary);
            return;
        }
        if narrow {
            let title = format!("◂ {}/{}  {file}", self.selected + 1, self.changes.len());
            let room = body.right().saturating_sub(5);
            // Beside Source Control there's no list, so the title says what this is.
            let fixed = self
                .fixed
                .as_ref()
                .map(|f| f.describe())
                .unwrap_or_default();
            let end = self.title_line(
                grid,
                (body.x + 2, y),
                &[(&title, strong), (&fixed, dim)],
                room,
            );
            grid.put_str(body.right().saturating_sub(3), y, "▸", dim, body.right());
            self.draw_close(grid, end, y, theme);
            return;
        }
        let room = body.right().saturating_sub(3);
        let described = match (&self.fixed, self.base.and_then(|b| self.turns.get(b))) {
            (Some(fixed), _) => fixed.describe(),
            (None, Some(turn)) if self.until.is_some() => {
                format!("in the turn \u{201c}{}\u{201d}", turn.label)
            }
            (None, Some(turn)) => format!("since \u{201c}{}\u{201d}", turn.label),
            (None, None) => "since the last commit".to_string(),
        };
        let end = self.title_line(
            grid,
            (body.x + 2, y),
            &[(&file, strong), (&described, dim)],
            room,
        );
        self.draw_close(grid, end, y, theme);
    }

    /// Writes the title's parts from `x`, a measured gap apart where the text is drawn in
    /// pixels and two columns apart in cells, and gives where the last one ends, in columns.
    fn title_line(
        &self,
        grid: &mut Grid,
        (x, y): (u16, u16),
        parts: &[(&str, Style)],
        room: u16,
    ) -> f32 {
        let measure = self.measure.as_ref().filter(|m| !m.in_cells());
        let mut at = f32::from(x);
        for (index, (text, style)) in parts.iter().enumerate() {
            if text.is_empty() {
                continue;
            }
            if index > 0 {
                at += if measure.is_some() { 1.0 } else { 2.0 };
            }
            let end = grid.label_at(at, at.round() as u16, y, text, *style, room, 100);
            at = match measure {
                Some(measure) => {
                    let font = cellix_render::Font {
                        bold: style.bold,
                        ..cellix_render::Font::default()
                    };
                    (at + measure.width(text, font)).min(f32::from(room))
                }
                None => f32::from(end),
            };
        }
        at
    }

    /// The × that closes the review, after its title as a tab's is after its name, with the
    /// tab's chip on hover.
    fn draw_close(&self, grid: &mut Grid, end: f32, y: u16, theme: &Theme) {
        let x = end + CLOSE_GAP;
        // The cell the glyph lands in, as `glyph` places it.
        let cell = x.floor() as u16;
        self.close_at.set(cell);
        let hovered = self.on_button(cell, y, "×");
        if hovered {
            let chip = Bounds::cells(x - 0.05, f32::from(y) + 0.12, 1.75, 0.76);
            grid.panel(chip, 16, Some(theme.row_hover), None);
        }
        let style = Style {
            fg: if hovered {
                theme.text.fg
            } else {
                theme.gutter.fg
            },
            ..theme.sidebar
        };
        grid.glyph(x, y, cellix_render::Icon::Close, style.fg, "×", style);
    }

    /// Whether column `x` of the title row is on its ×.
    fn on_close(&self, x: u16) -> bool {
        let at = self.close_at.get();
        x >= at && x < at.saturating_add(2)
    }

    /// A card's header: its name, and Reject and Accept. `first` is when it's the top row
    /// showing, with no gap above to reach into.
    fn draw_hunk_header(
        &self,
        grid: &mut Grid,
        card: Rect,
        (y, first): (u16, bool),
        hunk: usize,
        theme: &Theme,
    ) {
        grid.fill(Rect::new(card.x, y, card.w, 1), theme.text);
        if !first {
            // The card's top edge, in the gap above, so the header sits in air.
            let edge = Bounds::cells(
                card.x as f32,
                y as f32 - HEADER_REACH,
                card.w as f32,
                1.0 + HEADER_REACH,
            );
            grid.panel(edge, CARD_RADIUS, Some(theme.text.bg), None);
        }
        let from = grid.painted_so_far();
        self.draw_header_parts(grid, card, y, hunk, theme);
        if !first {
            grid.lift_since(from, HEADER_LIFT);
        }
    }

    fn draw_header_parts(&self, grid: &mut Grid, card: Rect, y: u16, hunk: usize, theme: &Theme) {
        let dim = Style {
            fg: theme.status.fg,
            ..theme.text
        };
        let (accept, reject) = Self::button_spots(card);
        grid.label(
            card.x + 2,
            y,
            &self.hunk_label(hunk),
            dim,
            reject.saturating_sub(1),
        );
        // A fixed change, or a past turn's, is only to read.
        if self.fixed.is_some() || self.until.is_some() {
            return;
        }
        if self.is_accepted(hunk) {
            let done = Style {
                fg: theme.git_added,
                ..theme.text
            };
            let word = if self.is_proposal() { "Kept" } else { "Staged" };
            grid.label_right(card.right().saturating_sub(2), y, word, done, 90);
            return;
        }
        // Accept is what a hunk is there for: the primary, filled with the accent.
        let primary = Style {
            fg: theme.button_text,
            ..theme.text
        };
        let quiet = Style {
            fg: theme.text.fg,
            ..theme.text
        };
        let accept_fill = if self.on_button(accept, y, ACCEPT) {
            theme.quiet_button(theme.button)
        } else {
            theme.button
        };
        // The secondary button every other surface has.
        let rest = theme.quiet_button(theme.text.bg);
        let reject_fill = if self.on_button(reject, y, REJECT) {
            theme.quiet_button(rest)
        } else {
            rest
        };
        grid.button(accept, y, ACCEPT, "Accept", accept_fill, primary);
        grid.button(reject, y, REJECT, "Reject", reject_fill, quiet);
    }

    fn draw_note(&self, grid: &mut Grid, card: Rect, y: u16, text: &str, theme: &Theme) {
        grid.fill(Rect::new(card.x, y, card.w, 1), theme.text);
        let inset = Rect::new(
            card.x + NUMBER_WIDTH,
            y,
            card.w.saturating_sub(NUMBER_WIDTH + 2),
            1,
        );
        grid.fill(inset, theme.sidebar);
        grid.panel(
            Bounds::inset(inset, 0.0, 0.04),
            20,
            Some(theme.sidebar.bg),
            None,
        );
        let who = Style {
            fg: theme.attention,
            ..theme.sidebar
        };
        let end = grid.label(inset.x + 1, y, "Your comment", who, inset.right());
        grid.label(end + 2, y, text, theme.sidebar, inset.right());
    }

    fn draw_cards(&self, grid: &mut Grid, body: Rect, theme: &Theme, comments: &[Comment]) {
        let rows = self.stacked(comments);
        let top = body.y + 1;
        let ground = theme.sidebar.bg;
        for (index, entry) in rows.iter().enumerate().skip(self.top) {
            let y = top + (index - self.top) as u16;
            if y >= body.bottom() {
                break;
            }
            let row = Rect::new(body.x, y, body.w, 1);
            match entry {
                Stacked::Gap => continue,
                Stacked::End => {
                    let quiet = Style {
                        fg: theme.gutter.fg,
                        ..theme.sidebar
                    };
                    // Level with the hunks' titles, and in the accent where a click goes on.
                    let style = if self.selected + 1 < self.changes.len() {
                        Style {
                            fg: theme.accent,
                            ..theme.sidebar
                        }
                    } else {
                        quiet
                    };
                    grid.label(body.x + 2, y, &self.end_words(), style, body.right());
                    continue;
                }
                Stacked::Header(hunk) => {
                    self.draw_hunk_header(grid, body, (y, y == top), *hunk, theme)
                }
                Stacked::Note(text) => self.draw_note(grid, body, y, text, theme),
                Stacked::Line(side, kind) => {
                    let bg = match kind {
                        RowKind::Removed => theme.removed,
                        RowKind::Added => theme.added,
                        _ => theme.text.bg,
                    };
                    let number_fg = theme.gutter.fg;
                    let lines = if *kind == RowKind::Removed {
                        &self.tokens.0
                    } else {
                        &self.tokens.1
                    };
                    let tokens = lines.get(side.number - 1);
                    let wrapped = Some((*side).clone());
                    let look = self.side_style(&wrapped, bg, tokens, number_fg, theme);
                    draw_side(grid, body, y, Some(side), look, theme);
                }
            }
            let bounds = Bounds::inset(row, 0.0, 0.0);
            // Its top edge is in the gap above, unless that's out of view.
            if matches!(entry, Stacked::Header(_)) && y == top {
                grid.corners(bounds, CARD_RADIUS, ground, TOP_LEFT | TOP_RIGHT);
            }
            if matches!(rows.get(index + 1), None | Some(Stacked::Gap)) {
                // As much under its last line as above its header, where the row below shows.
                if y + 1 < body.bottom() {
                    let foot = Bounds::cells(
                        f32::from(body.x),
                        f32::from(y + 1),
                        f32::from(body.w),
                        CARD_FOOT,
                    );
                    grid.panel(foot, 0, Some(theme.text.bg), None);
                    grid.corners(foot, CARD_RADIUS, ground, BOTTOM_LEFT | BOTTOM_RIGHT);
                } else {
                    grid.corners(bounds, CARD_RADIUS, ground, BOTTOM_LEFT | BOTTOM_RIGHT);
                }
            }
        }
    }

    /// The changed line at a place in the diff, as what undoing it takes, with its line number.
    pub fn changed_line_at(
        &self,
        area: Rect,
        y: u16,
        comments: &[Comment],
    ) -> Option<(cellix_review::LineUndo, usize, String)> {
        let (_, body) = self.layout(area);
        let row = y.checked_sub(body.y + 1)? as usize;
        let stacked = self.stacked(comments);
        let index = self.top + row;
        match stacked.get(index)? {
            Stacked::Line(side, RowKind::Added) => {
                let undo = cellix_review::LineUndo::Added(side.number - 1);
                Some((undo, side.number, side.text.trim().to_string()))
            }
            Stacked::Line(side, RowKind::Removed) => {
                // Back before the next line the new text has, or at its end.
                let next_new = stacked[index + 1..].iter().find_map(|s| match s {
                    Stacked::Line(side, RowKind::Added | RowKind::Same) => Some(side.number - 1),
                    Stacked::Header(_) => Some(usize::MAX),
                    _ => None,
                });
                let at = match next_new {
                    Some(usize::MAX) | None => self.new.lines().count(),
                    Some(at) => at,
                };
                let undo = cellix_review::LineUndo::Removed {
                    old: side.number - 1,
                    at,
                };
                Some((undo, side.number, side.text.trim().to_string()))
            }
            _ => None,
        }
    }

    /// The file's text with one line of the change undone, to be written by the caller.
    pub fn line_rejected_text(&self, undo: cellix_review::LineUndo) -> String {
        cellix_review::reject_line(&self.old, &self.new, undo)
    }

    pub fn hit(&self, area: Rect, x: u16, y: u16, comments: &[Comment]) -> Option<Hit> {
        let (list, body) = self.layout(area);
        if list.is_some_and(|l| x < l.right()) {
            return match self.list_rows().get((y - area.y) as usize)? {
                ListRow::Heading(_) => None,
                ListRow::Base(base) => Some(Hit::Base(*base)),
                ListRow::Commit => Some(Hit::Commit),
                ListRow::Test(index) => Some(Hit::Test(*index)),
                ListRow::File(index) => Some(Hit::File(*index)),
            };
        }
        self.hit_cards(list.is_some(), body, x, y, comments)
    }

    fn draw_list_row(&self, grid: &mut Grid, list: Rect, y: u16, row: &ListRow, theme: &Theme) {
        let line = Rect::new(list.x, y, list.w, 1);
        let pick = |on: bool| {
            if on {
                tint(theme.sidebar, theme.row_active)
            } else {
                theme.sidebar
            }
        };
        let selected = match row {
            ListRow::Base(base) => self.base == *base,
            ListRow::File(index) => self.selected == *index,
            _ => false,
        };
        let usable = matches!(row, ListRow::Base(_) | ListRow::Test(_) | ListRow::File(_));
        let hovered = usable && self.hover.get().is_some_and(|h| line.contains(h.0, h.1));
        if selected || hovered {
            // Pixels get a pill like every other list, over the cell tier's full row.
            let fill = if selected {
                theme.row_active
            } else {
                theme.row_hover
            };
            let whole = Bounds::inset(line, 0.0, 0.0);
            grid.panel(whole, 0, Some(theme.sidebar.bg), None);
            grid.panel(Bounds::inset(line, 0.4, 0.055), 18, Some(fill), None);
        }
        match row {
            ListRow::Heading(text) => {
                let dim = Style {
                    fg: theme.gutter.fg,
                    ..theme.sidebar
                };
                // Where every sidebar's title starts, two columns in.
                let x = list.x as f32 + 2.0;
                grid.label_at(x, list.x + 2, y, text, dim, list.right(), 80);
            }
            ListRow::Base(base) => {
                let style = pick(self.base == *base);
                grid.fill(line, style);
                let text = match base {
                    None => "Last commit".to_string(),
                    Some(index) => {
                        let turn = &self.turns[*index];
                        format!("{}  {}", turn.number, turn.label)
                    }
                };
                grid.label(list.x + 1, y, &text, style, list.right());
            }
            ListRow::Commit => {
                let style = Style {
                    fg: theme.button_text,
                    ..theme.sidebar
                };
                let text = format!("[Commit {} staged]", self.staged);
                grid.button(list.x + 1, y, &text, "Commit", theme.button, style);
            }
            ListRow::Test(index) => {
                let run = &self.tests[*index];
                let (mark, color) = if run.passed {
                    ("passed", theme.git_added)
                } else {
                    ("failed", theme.error)
                };
                let style = Style {
                    fg: color,
                    ..theme.sidebar
                };
                grid.label_right(list.right().saturating_sub(1), y, mark, style, 85);
                let end = list.right().saturating_sub(8);
                grid.label(list.x + 1, y, &run.command, theme.sidebar, end);
            }
            ListRow::File(index) => {
                let style = pick(*index == self.selected);
                grid.fill(line, style);
                let change = &self.changes[*index];
                let name = change.path.display().to_string();
                // As far in from the row's right as the name is from its left.
                let right = list.right().saturating_sub(2);
                let mut end = right.saturating_sub(2);
                if let Some((added, removed)) = self.stats.get(&change.path) {
                    let green = Style {
                        fg: theme.git_added,
                        ..style
                    };
                    let red = Style {
                        fg: theme.error,
                        ..style
                    };
                    // A count of nothing is only noise.
                    let counts = [(*removed, '-', red), (*added, '+', green)];
                    for (count, sign, colour) in counts.into_iter().filter(|c| c.0 > 0) {
                        let shown = format!("{sign}{count}");
                        grid.label_right(end, y, &shown, colour, 85);
                        end = end.saturating_sub(shown.len() as u16 + 1);
                    }
                }
                grid.label(list.x + 2, y, &name, style, end);
                // The letter Source Control shows for it, in the same colours.
                let letter = match change.status {
                    '?' => 'U',
                    other => other,
                };
                let mark = Style {
                    fg: match letter {
                        'A' | 'U' => theme.git_added,
                        'D' => theme.deleted(),
                        _ => theme.git_modified,
                    },
                    ..style
                };
                grid.label_right(right, y, letter.encode_utf8(&mut [0; 4]), mark, 100);
            }
        }
    }

    /// Whether the pointer is on a button at this place.
    fn on_button(&self, x: u16, y: u16, cells: &str) -> bool {
        let spot = Rect::new(x, y, cells.chars().count().max(2) as u16, 1);
        self.hover.get().is_some_and(|h| spot.contains(h.0, h.1))
    }

    pub fn draw(
        &self,
        grid: &mut Grid,
        area: Rect,
        hover: Option<(u16, u16)>,
        theme: &Theme,
        comments: &[Comment],
    ) {
        self.hover.set(hover);
        grid.fill(area, theme.sidebar);
        let (list, body) = self.layout(area);
        if let Some(list) = list {
            for (i, row) in self.list_rows().iter().enumerate() {
                let y = list.y + i as u16;
                if y >= list.bottom() {
                    break;
                }
                self.draw_list_row(grid, list, y, row, theme);
            }
        }
        self.draw_title(grid, body, list.is_none(), theme);
        self.draw_cards(grid, body, theme, comments);
    }
}
