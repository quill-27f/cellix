use std::path::{Path, PathBuf};
use std::rc::Rc;

use cellix_agent::TestRun;
use cellix_lang::{Registry, Token};
use cellix_render::{Grid, Rect, Rgb, Style};
use cellix_review::{
    Change, FileDiff, Row, RowKind, Side, Turn, changed_files, changed_since, commit, head_text,
    hunk_patch, index_text, rank_changes, stage_patch, staged_count, text_at, turns,
};

use crate::theme::Theme;

const LIST_WIDTH: u16 = 24;
const ACCEPT: &str = "[Accept]";
const REJECT: &str = "[Reject]";
const NUMBER_WIDTH: u16 = 5;
/// Below this, two code columns are too narrow to read, so lines stack in one.
const MIN_SIDE_BY_SIDE: u16 = 90;
/// Turns listed in the timeline, newest first.
const SHOWN_TURNS: usize = 6;
const SHOWN_TESTS: usize = 3;

/// What a click in the review view lands on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Hit {
    File(usize),
    Accept(usize),
    Reject(usize),
    /// A line of the new version, or the old one if it was deleted.
    Line(String, usize),
    /// Compare against this turn, or HEAD for None.
    Base(Option<usize>),
    Commit,
    /// Show a test run's output.
    Test(usize),
    /// Step through the files, in the stacked layout.
    PreviousFile,
    NextFile,
}

/// A row of the stacked layout.
enum Stacked<'a> {
    Header(usize),
    Line(&'a Side, RowKind),
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

/// Colour spans for one line: start, end and token, in chars.
type LineTokens = Vec<(usize, usize, Token)>;

/// The Changes view: files that differ from a base, and a diff of one of them. The base is
/// HEAD, or the checkpoint taken at the start of an agent's turn.
pub struct Review {
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
    staged: usize,
    /// Test commands agents have run, newest last.
    pub tests: Vec<TestRun>,
    tokens: (Vec<LineTokens>, Vec<LineTokens>),
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
            staged: 0,
            tests: Vec::new(),
            tokens: (Vec::new(), Vec::new()),
        };
        review.refresh();
        review
    }

    /// Rereads git and the selected file. Keeps the selection where it can.
    pub fn refresh(&mut self) {
        let current = self.current_path().map(Path::to_path_buf);
        self.turns = turns(&self.root);
        if self.base.is_some_and(|b| b >= self.turns.len()) {
            self.base = None;
        }
        self.changes = match self.base_commit() {
            Some(commit) => changed_since(&self.root, &commit),
            None => changed_files(&self.root),
        };
        rank_changes(&mut self.changes);
        self.staged = staged_count(&self.root);
        self.selected = current
            .and_then(|p| self.changes.iter().position(|c| c.path == p))
            .unwrap_or(0);
        self.load();
    }

    fn base_commit(&self) -> Option<String> {
        Some(self.turns.get(self.base?)?.commit.clone())
    }

    /// Compares against a turn, or HEAD for None.
    pub fn set_base(&mut self, base: Option<usize>) {
        self.base = base;
        self.top = 0;
        self.refresh();
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

    fn current_path(&self) -> Option<&Path> {
        self.changes.get(self.selected).map(|c| c.path.as_path())
    }

    fn load(&mut self) {
        let Some(path) = self.current_path().map(Path::to_path_buf) else {
            self.diff = FileDiff::default();
            self.tokens = (Vec::new(), Vec::new());
            return;
        };
        self.old = match self.base_commit() {
            Some(commit) => text_at(&self.root, &commit, &path),
            None => head_text(&self.root, &path),
        };
        self.new = std::fs::read_to_string(self.root.join(&path)).unwrap_or_default();
        self.diff = FileDiff::new(&self.old, &self.new);
        self.tokens = (
            line_tokens(&self.registry, &path, &self.old),
            line_tokens(&self.registry, &path, &self.new),
        );
        self.top = self.top.min(self.diff.rows.len().saturating_sub(1));
    }

    pub fn select(&mut self, index: usize) {
        self.selected = index;
        self.top = 0;
        self.load();
    }

    pub fn scroll_by(&mut self, delta: isize) {
        let last = self.diff.rows.len().saturating_sub(1) as isize;
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
        if self.staged > 0 {
            rows.push(ListRow::Commit);
        }
        rows.extend((0..self.changes.len()).map(ListRow::File));
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

    fn columns(area: Rect) -> (Rect, u16) {
        let list = Rect::new(area.x, area.y, LIST_WIDTH.min(area.w), area.h);
        (list, area.w.saturating_sub(list.w) / 2)
    }

    /// The diff as one column: an old line, then its replacement under it.
    fn stacked(&self) -> Vec<Stacked<'_>> {
        let mut rows = Vec::new();
        for row in &self.diff.rows {
            match row {
                Row::Header(hunk) => rows.push(Stacked::Header(*hunk)),
                Row::Line {
                    left,
                    right,
                    kind: RowKind::Same,
                } => {
                    let side = right.as_ref().or(left.as_ref());
                    rows.extend(side.map(|s| Stacked::Line(s, RowKind::Same)));
                }
                Row::Line { left, right, .. } => {
                    rows.extend(left.iter().map(|s| Stacked::Line(s, RowKind::Removed)));
                    rows.extend(right.iter().map(|s| Stacked::Line(s, RowKind::Added)));
                }
            }
        }
        rows
    }

    fn hit_stacked(&self, area: Rect, x: u16, y: u16) -> Option<Hit> {
        if y == area.y {
            return Some(if x < area.x + area.w / 2 {
                Hit::PreviousFile
            } else {
                Hit::NextFile
            });
        }
        let row = (y - area.y - 1) as usize;
        match self.stacked().get(self.top + row)? {
            Stacked::Header(hunk) => {
                let at = x.checked_sub(area.x + 1)?;
                let accept = ACCEPT.len() as u16;
                if at < accept {
                    Some(Hit::Accept(*hunk))
                } else if at > accept && at <= accept * 2 + 1 {
                    Some(Hit::Reject(*hunk))
                } else {
                    None
                }
            }
            Stacked::Line(side, _) => {
                let file = self.current_path()?.to_string_lossy().into_owned();
                Some(Hit::Line(file, side.number))
            }
        }
    }

    pub fn step_file(&mut self, delta: isize) {
        let count = self.changes.len() as isize;
        if count > 0 {
            self.select((self.selected as isize + delta).rem_euclid(count) as usize);
        }
    }

    fn draw_buttons(&self, grid: &mut Grid, x: u16, y: u16, theme: &Theme) {
        let on_button = Style {
            fg: theme.button_text,
            ..theme.bar
        };
        grid.button(x, y, ACCEPT, "Accept", theme.button, on_button);
        let secondary = Style {
            fg: theme.text.fg,
            ..theme.bar
        };
        let next = x + ACCEPT.len() as u16 + 1;
        grid.button(next, y, REJECT, "Reject", theme.input, secondary);
    }

    fn draw_stacked(
        &self,
        grid: &mut Grid,
        area: Rect,
        theme: &Theme,
        commented: impl Fn(&str, usize) -> bool,
    ) {
        grid.fill(area, theme.text);
        grid.fill(Rect::new(area.x, area.y, area.w, 1), theme.bar);
        let file = self
            .current_path()
            .map(|p| p.to_string_lossy().into_owned());
        let title = match &file {
            Some(file) => format!(" ◂ {}/{}  {file}", self.selected + 1, self.changes.len()),
            None => " No changes".to_string(),
        };
        grid.put_str(
            area.x,
            area.y,
            &title,
            theme.bar,
            area.right().saturating_sub(2),
        );
        if file.is_some() {
            grid.put_str(
                area.right().saturating_sub(2),
                area.y,
                "▸",
                theme.bar,
                area.right(),
            );
        }
        let file = file.unwrap_or_default();
        for (row, entry) in self.stacked().iter().skip(self.top).enumerate() {
            let y = area.y + 1 + row as u16;
            if y >= area.bottom() {
                break;
            }
            match entry {
                Stacked::Header(hunk) => {
                    grid.fill(Rect::new(area.x, y, area.w, 1), theme.bar);
                    let state = if self.is_accepted(*hunk) {
                        "staged"
                    } else {
                        ""
                    };
                    self.draw_buttons(grid, area.x + 1, y, theme);
                    let after = area.x + ACCEPT.len() as u16 * 2 + 3;
                    grid.label(after, y, state, theme.bar, area.right());
                }
                Stacked::Line(side, kind) => {
                    let bg = match kind {
                        RowKind::Removed => theme.removed,
                        RowKind::Added => theme.added,
                        _ => theme.text.bg,
                    };
                    let number_fg = if commented(&file, side.number) {
                        theme.accent
                    } else {
                        theme.gutter.fg
                    };
                    let lines = if *kind == RowKind::Removed {
                        &self.tokens.0
                    } else {
                        &self.tokens.1
                    };
                    let tokens = lines.get(side.number - 1);
                    let wrapped = Some((*side).clone());
                    let look = self.side_style(&wrapped, bg, tokens, number_fg, theme);
                    draw_side(grid, area, y, Some(side), look, theme);
                }
            }
        }
    }

    pub fn hit(&self, area: Rect, x: u16, y: u16) -> Option<Hit> {
        if area.w < MIN_SIDE_BY_SIDE {
            return self.hit_stacked(area, x, y);
        }
        let (list, width) = Self::columns(area);
        let row = (y - area.y) as usize;
        if x < list.right() {
            return match self.list_rows().get(row)? {
                ListRow::Heading(_) => None,
                ListRow::Base(base) => Some(Hit::Base(*base)),
                ListRow::Commit => Some(Hit::Commit),
                ListRow::Test(index) => Some(Hit::Test(*index)),
                ListRow::File(index) => Some(Hit::File(*index)),
            };
        }
        match self.diff.rows.get(self.top + row)? {
            Row::Header(hunk) => {
                let at = x.checked_sub(list.right() + 1)?;
                let accept = ACCEPT.len() as u16;
                if at < accept {
                    Some(Hit::Accept(*hunk))
                } else if at > accept && at <= accept * 2 + 1 {
                    Some(Hit::Reject(*hunk))
                } else {
                    None
                }
            }
            Row::Line { left, right, .. } => {
                let in_left = x < list.right() + width;
                let side = if in_left { left } else { right };
                let side = side.as_ref().or(left.as_ref()).or(right.as_ref())?;
                let file = self.current_path()?.to_string_lossy().into_owned();
                Some(Hit::Line(file, side.number))
            }
        }
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
        match row {
            ListRow::Heading(text) => {
                let dim = Style {
                    fg: theme.gutter.fg,
                    ..theme.sidebar
                };
                let x = list.x as f32 + 1.0;
                grid.label_at(x, list.x + 1, y, text, dim, list.right(), 80);
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
                let status = change.status.to_string();
                grid.put_str(list.x + 1, y, &status, style, list.right());
                grid.label(list.x + 3, y, &name, style, list.right());
            }
        }
    }

    pub fn draw(
        &self,
        grid: &mut Grid,
        area: Rect,
        theme: &Theme,
        commented: impl Fn(&str, usize) -> bool,
    ) {
        if area.w < MIN_SIDE_BY_SIDE {
            self.draw_stacked(grid, area, theme, commented);
            return;
        }
        grid.fill(area, theme.text);
        let (list, width) = Self::columns(area);
        grid.fill(list, theme.sidebar);
        for (i, row) in self.list_rows().iter().enumerate() {
            let y = list.y + i as u16;
            if y >= list.bottom() {
                break;
            }
            self.draw_list_row(grid, list, y, row, theme);
        }
        if self.changes.is_empty() {
            let y = list.y + self.list_rows().len() as u16;
            grid.label(list.x + 1, y, "No changes", theme.sidebar, list.right());
        }
        let left = Rect::new(list.right(), area.y, width, area.h);
        let right = Rect::new(left.right(), area.y, area.right() - left.right(), area.h);
        let file = self
            .current_path()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        for (row, entry) in self.diff.rows.iter().skip(self.top).enumerate() {
            let y = area.y + row as u16;
            if y >= area.bottom() {
                break;
            }
            match entry {
                Row::Header(hunk) => {
                    let line = Rect::new(left.x, y, left.w + right.w, 1);
                    grid.fill(line, theme.bar);
                    let state = if self.is_accepted(*hunk) {
                        "staged"
                    } else {
                        ""
                    };
                    let x = line.x + 1;
                    self.draw_buttons(grid, x, y, theme);
                    let text = format!("Change {}   {state}", hunk + 1);
                    let after = x + ACCEPT.len() as u16 * 2 + 2;
                    grid.label(after, y, &text, theme.bar, line.right());
                }
                Row::Line {
                    left: l,
                    right: r,
                    kind,
                } => {
                    let (old_bg, new_bg) = match kind {
                        RowKind::Same => (theme.text.bg, theme.text.bg),
                        RowKind::Removed | RowKind::Changed if r.is_none() => {
                            (theme.removed, theme.text.bg)
                        }
                        RowKind::Added => (theme.text.bg, theme.added),
                        _ => (theme.removed, theme.added),
                    };
                    let number_fg = |s: &Option<Side>| {
                        if s.as_ref().is_some_and(|s| commented(&file, s.number)) {
                            theme.accent
                        } else {
                            theme.gutter.fg
                        }
                    };
                    let tokens = |side: &Option<Side>, lines: &'_ [LineTokens]| {
                        side.as_ref().and_then(|s| lines.get(s.number - 1)).cloned()
                    };
                    let (old_tokens, new_tokens) =
                        (tokens(l, &self.tokens.0), tokens(r, &self.tokens.1));
                    let look = self.side_style(l, old_bg, old_tokens.as_ref(), number_fg(l), theme);
                    draw_side(grid, left, y, l.as_ref(), look, theme);
                    let look = self.side_style(r, new_bg, new_tokens.as_ref(), number_fg(r), theme);
                    draw_side(grid, right, y, r.as_ref(), look, theme);
                }
            }
        }
    }
}
