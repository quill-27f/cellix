//! The open file's history under the Explorer, as VS Code's Timeline: the commits that
//! changed it, newest first, each opening its diff against the commit before.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};

use cellix_render::{Bounds, Grid, Measure, Rect, Style};
use cellix_review::Commit;

use crate::theme::Theme;

/// Commits listed under the Explorer; the rest are in Show File History.
pub const SHOWN: usize = 6;
/// Commits read for a file, for the palette's longer list.
pub const READ: usize = 200;
/// The sidebar needs this many rows before the timeline takes any from the tree.
const LEAST_SIDEBAR: u16 = 18;
const SIDE: u16 = 2;

#[derive(Default)]
pub struct Timeline {
    /// The file it's for, relative to the workspace.
    pub path: Option<PathBuf>,
    pub commits: Vec<Commit>,
    pub folded: bool,
    reading: Option<(PathBuf, Receiver<Vec<Commit>>)>,
}

/// What a click in the timeline lands on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Hit {
    Fold,
    Commit(usize),
    More,
}

impl Timeline {
    /// Follows the file in view, reading its history away from the screen when it changes.
    pub fn follow(&mut self, root: &Path, path: Option<PathBuf>) {
        let reading = self.reading.as_ref().map(|(p, _)| p);
        if path == self.path || (path.is_some() && path.as_ref() == reading) {
            return;
        }
        self.reading = None;
        self.commits.clear();
        self.path = path.clone();
        let Some(path) = path else {
            return;
        };
        let (done, receiver) = mpsc::channel();
        let (root, file) = (root.to_path_buf(), path.clone());
        std::thread::spawn(move || {
            let _ = done.send(cellix_review::file_history(&root, &file, READ));
        });
        self.reading = Some((path, receiver));
    }

    /// Reads it again, as after a commit.
    pub fn reread(&mut self, root: &Path) {
        let path = self.path.take();
        self.reading = None;
        self.follow(root, path);
    }

    /// Takes in a finished read. Returns true if it arrived.
    pub fn poll(&mut self) -> bool {
        let Some(commits) = self.reading.as_ref().and_then(|(_, r)| r.try_recv().ok()) else {
            return false;
        };
        self.reading = None;
        self.commits = commits;
        true
    }

    fn rows(&self) -> u16 {
        if self.path.is_none() {
            return 0;
        }
        if self.folded {
            return 1;
        }
        let listed = self.commits.len().min(SHOWN) as u16;
        let more = u16::from(self.commits.len() > SHOWN);
        // The header, the commits or a word that there are none, and air under them.
        1 + listed.max(1) + more + 1
    }

    /// The tree's part of the sidebar, and the timeline's under it.
    pub fn split(&self, area: Rect) -> (Rect, Option<Rect>) {
        let rows = self.rows();
        if rows == 0 || area.h < LEAST_SIDEBAR {
            return (area, None);
        }
        let tree = Rect::new(area.x, area.y, area.w, area.h - rows);
        let below = Rect::new(area.x, tree.bottom(), area.w, rows);
        (tree, Some(below))
    }

    pub fn hit(&self, area: Rect, x: u16, y: u16) -> Option<Hit> {
        if !area.contains(x, y) {
            return None;
        }
        let row = (y - area.y) as usize;
        if row == 0 {
            return Some(Hit::Fold);
        }
        if self.folded {
            return None;
        }
        let index = row - 1;
        if index < self.commits.len().min(SHOWN) {
            return Some(Hit::Commit(index));
        }
        (index == SHOWN && self.commits.len() > SHOWN).then_some(Hit::More)
    }

    pub fn draw(
        &self,
        grid: &mut Grid,
        area: Rect,
        hover: Option<(u16, u16)>,
        (now, measure): (i64, &dyn Measure),
        theme: &Theme,
    ) {
        grid.fill(area, theme.sidebar);
        let rule = Bounds::cells(area.x as f32, area.y as f32, area.w as f32, 0.0);
        grid.hairline(rule, theme.popup_border);
        let quiet = Style {
            fg: theme.gutter.fg,
            ..theme.sidebar
        };
        let left = area.x + SIDE;
        let right = area.right().saturating_sub(SIDE);
        let name = self
            .path
            .as_ref()
            .and_then(|p| p.file_name())
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        // The chevron in the margin, the name on the inset every sidebar heading has.
        let mark = if self.folded { "▸" } else { "▾" };
        grid.label(area.x, area.y, mark, quiet, left);
        let heading = format!("TIMELINE · {name}");
        let heading = crate::source::clip(&heading, (right - left) as f32, measure);
        grid.label(left, area.y, &heading, quiet, right);
        if self.folded {
            return;
        }
        if self.commits.is_empty() {
            let word = if self.reading.is_some() {
                "Reading…"
            } else {
                "Not committed yet"
            };
            grid.label(left + 1, area.y + 1, word, quiet, right);
            return;
        }
        for (index, commit) in self.commits.iter().take(SHOWN).enumerate() {
            let y = area.y + 1 + index as u16;
            let whole = Rect::new(area.x, y, area.w, 1);
            if hover.is_some_and(|h| whole.contains(h.0, h.1)) {
                let wash = Bounds::inset(whole, 0.4, 0.06);
                grid.panel(wash, 14, Some(theme.row_hover), None);
            }
            // A dot, as the graph marks a commit.
            let style = Style {
                fg: theme.accent,
                painted: true,
                ..theme.sidebar
            };
            grid.put_str(left, y, "●", style, left + 1);
            let dot = Bounds::cells(left as f32 + 0.18, y as f32 + 0.35, 0.64, 0.3);
            grid.panel(dot, 15, Some(theme.accent), None);
            let age = cellix_review::age(commit.time, now);
            grid.label_right(right, y, &age, quiet, 90);
            let end = right.saturating_sub(age.chars().count() as u16 + 1);
            let room = end.saturating_sub(left + 4) as f32;
            let summary = crate::source::clip(&commit.summary, room, measure);
            grid.label(left + 3, y, &summary, theme.sidebar, end.saturating_sub(1));
        }
        if self.commits.len() > SHOWN {
            let y = area.y + 1 + SHOWN as u16;
            let more = format!("Show all {}…", self.commits.len());
            grid.label(left + 3, y, &more, quiet, right);
        }
    }
}
