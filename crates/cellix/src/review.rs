use std::collections::HashSet;
use std::path::{Path, PathBuf};

use cellix_render::{Grid, Rect, Rgb, Style};
use cellix_review::{Change, FileDiff, Hunk, Row, RowKind, Side, changed_files, head_text};

use crate::theme::Theme;

const LIST_WIDTH: u16 = 24;
const ACCEPT: &str = "[Accept]";
const REJECT: &str = "[Reject]";
const NUMBER_WIDTH: u16 = 5;

/// What a click in the review view lands on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Hit {
    File(usize),
    Accept(usize),
    Reject(usize),
    /// A line of the new version, or the old one if it was deleted.
    Line(String, usize),
}

/// The Changes view: files that differ from HEAD and a side-by-side diff of one of them.
pub struct Review {
    root: PathBuf,
    pub changes: Vec<Change>,
    pub selected: usize,
    pub diff: FileDiff,
    old: String,
    new: String,
    top: usize,
    accepted: HashSet<(PathBuf, Hunk)>,
}

fn tint(style: Style, bg: Rgb) -> Style {
    Style { bg, ..style }
}

fn draw_side(
    grid: &mut Grid,
    area: Rect,
    y: u16,
    side: Option<&Side>,
    style: Style,
    number_fg: Rgb,
) {
    grid.fill(Rect::new(area.x, y, area.w, 1), style);
    let Some(side) = side else {
        return;
    };
    let number = format!("{:>4} ", side.number);
    let gutter = Style {
        fg: number_fg,
        ..style
    };
    grid.put_str(area.x, y, &number, gutter, area.right());
    let text = side.text.replace('\t', "    ");
    grid.put_str(area.x + NUMBER_WIDTH, y, &text, style, area.right());
}

impl Review {
    pub fn new(root: PathBuf) -> Self {
        let mut review = Self {
            root,
            changes: Vec::new(),
            selected: 0,
            diff: FileDiff::default(),
            old: String::new(),
            new: String::new(),
            top: 0,
            accepted: HashSet::new(),
        };
        review.refresh();
        review
    }

    /// Rereads git and the selected file. Keeps the selection where it can.
    pub fn refresh(&mut self) {
        let current = self.current_path().map(Path::to_path_buf);
        self.changes = changed_files(&self.root);
        self.changes.sort_by(|a, b| a.path.cmp(&b.path));
        self.selected = current
            .and_then(|p| self.changes.iter().position(|c| c.path == p))
            .unwrap_or(0);
        self.load();
    }

    fn current_path(&self) -> Option<&Path> {
        self.changes.get(self.selected).map(|c| c.path.as_path())
    }

    fn load(&mut self) {
        let Some(path) = self.current_path().map(Path::to_path_buf) else {
            self.diff = FileDiff::default();
            return;
        };
        self.old = head_text(&self.root, &path);
        self.new = std::fs::read_to_string(self.root.join(&path)).unwrap_or_default();
        self.diff = FileDiff::new(&self.old, &self.new);
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

    pub fn accept(&mut self, hunk: usize) {
        if let (Some(path), Some(h)) = (self.current_path(), self.diff.hunks.get(hunk)) {
            let key = (path.to_path_buf(), *h);
            self.accepted.insert(key);
        }
    }

    pub fn is_accepted(&self, hunk: usize) -> bool {
        match (self.current_path(), self.diff.hunks.get(hunk)) {
            (Some(path), Some(h)) => self.accepted.contains(&(path.to_path_buf(), *h)),
            _ => false,
        }
    }

    /// The file's text with one hunk put back, to be written by the caller.
    pub fn rejected_text(&self, hunk: usize) -> Option<String> {
        let hunk = self.diff.hunks.get(hunk)?;
        Some(cellix_review::reject_hunk(&self.old, &self.new, hunk))
    }

    fn columns(area: Rect) -> (Rect, u16) {
        let list = Rect::new(area.x, area.y, LIST_WIDTH.min(area.w), area.h);
        (list, area.w.saturating_sub(list.w) / 2)
    }

    pub fn hit(&self, area: Rect, x: u16, y: u16) -> Option<Hit> {
        let (list, width) = Self::columns(area);
        let row = (y - area.y) as usize;
        if x < list.right() {
            return (row < self.changes.len()).then_some(Hit::File(row));
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

    pub fn draw(
        &self,
        grid: &mut Grid,
        area: Rect,
        theme: &Theme,
        commented: impl Fn(&str, usize) -> bool,
    ) {
        grid.fill(area, theme.text);
        let (list, width) = Self::columns(area);
        grid.fill(list, theme.sidebar);
        for (i, change) in self.changes.iter().enumerate() {
            let y = list.y + i as u16;
            if y >= list.bottom() {
                break;
            }
            let style = if i == self.selected {
                tint(theme.sidebar, theme.row_hover)
            } else {
                theme.sidebar
            };
            grid.fill(Rect::new(list.x, y, list.w, 1), style);
            let text = format!(" {} {}", change.status, change.path.display());
            grid.put_str(list.x, y, &text, style, list.right());
        }
        if self.changes.is_empty() {
            grid.put_str(
                list.x + 1,
                list.y,
                "No changes",
                theme.sidebar,
                list.right(),
            );
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
                        "accepted"
                    } else {
                        ""
                    };
                    let text = format!(" {ACCEPT} {REJECT}  hunk {}  {state}", hunk + 1);
                    grid.put_str(line.x, y, &text, theme.bar, line.right());
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
                    draw_side(
                        grid,
                        left,
                        y,
                        l.as_ref(),
                        tint(theme.text, old_bg),
                        number_fg(l),
                    );
                    draw_side(
                        grid,
                        right,
                        y,
                        r.as_ref(),
                        tint(theme.text, new_bg),
                        number_fg(r),
                    );
                }
            }
        }
    }
}
