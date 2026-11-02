//! Source Control in the sidebar: what's staged, what isn't, and a message to commit it with.
//! Review is for reading what changed. This is for deciding what goes in.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};

use cellix_render::{Bounds, Font, Grid, Measure, Rect, Style};
use cellix_review::{Entry, ahead_behind, status_entries};

use crate::theme::Theme;
use crate::threads::wrap;

const SIDE: u16 = 2;
/// Rows of the message box.
const MESSAGE_ROWS: u16 = 3;
/// Where a row's marks stand, from the panel's right edge.
pub const LETTER_FROM_RIGHT: u16 = 3;
pub const STAGE_FROM_RIGHT: u16 = 5;
pub const DISCARD_FROM_RIGHT: u16 = 7;
/// How wide the button beside Commit is.
const SYNC_WIDTH: u16 = 10;

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
}

/// One row of the panel.
#[derive(Clone, Debug, PartialEq)]
enum Line {
    Title,
    Message(u16),
    Gap,
    Buttons,
    Heading(bool, usize),
    Change(usize, bool),
    Clean,
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
    top: usize,
    reading: Option<Receiver<(Vec<Entry>, Remote)>>,
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
            let _ = done.send((entries, remote));
        });
        self.reading = Some(reading);
    }

    /// Takes in what was read. Returns true if it changed anything.
    pub fn poll(&mut self) -> bool {
        let Some(read) = self.reading.as_ref().and_then(|r| r.try_recv().ok()) else {
            return false;
        };
        self.reading = None;
        let changed = self.entries != read.0 || self.remote != read.1;
        (self.entries, self.remote) = read;
        changed
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
        lines.extend([Line::Gap, Line::Buttons, Line::Gap]);
        let list = |lines: &mut Vec<Line>, staged: bool| {
            let matching = |entry: &&Entry| {
                if staged {
                    entry.is_staged()
                } else {
                    entry.is_unstaged()
                }
            };
            let count = self.entries.iter().filter(matching).count();
            if count == 0 {
                return;
            }
            lines.push(Line::Heading(staged, count));
            let rows = self.entries.iter().enumerate();
            let rows = rows.filter(|(_, entry)| matching(entry));
            lines.extend(rows.map(|(index, _)| Line::Change(index, staged)));
            lines.push(Line::Gap);
        };
        if self.entries.is_empty() {
            lines.push(Line::Clean);
        }
        list(&mut lines, true);
        list(&mut lines, false);
        lines
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
        let row = y.checked_sub(area.y)? as usize + self.top;
        let line = self.lines().into_iter().nth(row)?;
        let right = area.right();
        match line {
            Line::Message(_) => Some(Hit::Message),
            Line::Buttons => {
                let (commit, sync) = self.buttons(area, y);
                if sync.is_some_and(|b| b.contains(x, y)) {
                    return Some(Hit::Sync);
                }
                commit.contains(x, y).then_some(Hit::Commit)
            }
            Line::Heading(staged, _) => {
                let on_action = x + STAGE_FROM_RIGHT + 1 >= right;
                on_action.then_some(if staged {
                    Hit::UnstageAll
                } else {
                    Hit::StageAll
                })
            }
            Line::Change(index, staged) => {
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
        for (row, line) in lines
            .iter()
            .skip(self.top)
            .take(area.h as usize)
            .enumerate()
        {
            let y = area.y + row as u16;
            let whole = Rect::new(area.x, y, area.w, 1);
            let hovered = hover.is_some_and(|h| whole.contains(h.0, h.1));
            match line {
                Line::Title => {
                    grid.label(left, y, "SOURCE CONTROL", quiet, right);
                    if let Some(branch) = branch {
                        grid.label_right(right.saturating_sub(SIDE), y, branch, quiet, 90);
                    }
                }
                Line::Message(0) => self.draw_message(grid, area, y, measure, theme),
                Line::Message(_) | Line::Gap => {}
                Line::Buttons => self.draw_buttons(grid, area, y, hover, theme),
                Line::Clean => {
                    grid.label(left, y, "No changes", quiet, right);
                }
                Line::Heading(staged, count) => {
                    let name = if *staged { "STAGED CHANGES" } else { "CHANGES" };
                    grid.label(left, y, name, quiet, right);
                    let number = count.to_string();
                    let at = right.saturating_sub(LETTER_FROM_RIGHT);
                    grid.label(at, y, &number, quiet, right);
                    if hovered {
                        let mark = if *staged { "−" } else { "+" };
                        let x = right.saturating_sub(STAGE_FROM_RIGHT);
                        grid.label(x, y, mark, theme.sidebar, right);
                    }
                }
                Line::Change(index, staged) => {
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
                    self.draw_change(grid, area, y, (entry, *staged), hovered, theme);
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
        grid.panel(
            Bounds::inset(frame, 0.0, 0.03),
            22,
            Some(theme.text.bg),
            Some(edge),
        );
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

    fn draw_buttons(
        &self,
        grid: &mut Grid,
        area: Rect,
        y: u16,
        hover: Option<(u16, u16)>,
        theme: &Theme,
    ) {
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
            bg: fill,
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
                bg: fill,
                ..theme.sidebar
            };
            let cells = format!("{name:^0$}", button.w as usize);
            grid.large_button(button.x, y, &cells, &name, fill, label);
        }
    }

    fn draw_change(
        &self,
        grid: &mut Grid,
        area: Rect,
        y: u16,
        (entry, staged): (&Entry, bool),
        hovered: bool,
        theme: &Theme,
    ) {
        let right = area.right();
        let letter = entry.letter(staged);
        let colour = match letter {
            'A' | 'U' => theme.git_added,
            'D' => theme.deleted(),
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
        let after = grid.label(area.x + SIDE, y, &name, theme.sidebar, end);
        if let Some(folder) = entry.path.parent().filter(|f| !f.as_os_str().is_empty()) {
            let quiet = Style {
                fg: theme.gutter.fg,
                ..theme.sidebar
            };
            grid.label(after + 1, y, &folder.display().to_string(), quiet, end);
        }
        let mark = Style {
            fg: colour,
            ..theme.sidebar
        };
        let at = right.saturating_sub(LETTER_FROM_RIGHT);
        grid.label(at, y, letter.encode_utf8(&mut [0; 4]), mark, right);
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
