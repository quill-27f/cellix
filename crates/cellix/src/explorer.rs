use std::collections::HashMap;
use std::path::PathBuf;

use cellix_core::{Entry, Workspace};
use cellix_render::{Grid, Rect, Style};

use crate::theme::Theme;

/// The file tree sidebar.
#[derive(Default)]
pub struct Explorer {
    pub entries: Vec<Entry>,
    pub top: usize,
    pub selected: Option<usize>,
    /// Git status letters by absolute path, with directories marked for what they hold.
    pub status: HashMap<PathBuf, char>,
}

impl Explorer {
    pub fn refresh(&mut self, workspace: &Workspace) {
        let selected = self
            .selected
            .and_then(|i| self.entries.get(i))
            .map(|e| e.path.clone());
        self.entries = workspace.entries();
        self.selected = selected.and_then(|p| self.entries.iter().position(|e| e.path == p));
    }

    pub fn entry_at(&self, area: Rect, y: u16) -> Option<usize> {
        let row = self.top + y.checked_sub(area.y + 1)? as usize;
        (row < self.entries.len()).then_some(row)
    }

    pub fn scroll_by(&mut self, delta: isize) {
        let last = self.entries.len().saturating_sub(1) as isize;
        self.top = (self.top as isize + delta).clamp(0, last.max(0)) as usize;
    }

    pub fn draw(&self, grid: &mut Grid, area: Rect, root_name: &str, theme: &Theme) {
        grid.fill(area, theme.sidebar);
        let heading = Style {
            bold: true,
            fg: theme.accent,
            ..theme.sidebar
        };
        grid.put_str(
            area.x + 1,
            area.y,
            &root_name.to_uppercase(),
            heading,
            area.right(),
        );
        for (row, entry) in self.entries.iter().skip(self.top).enumerate() {
            let y = area.y + 1 + row as u16;
            if y >= area.bottom() {
                break;
            }
            let style = if self.selected == Some(self.top + row) {
                Style {
                    bg: theme.row_hover,
                    ..theme.sidebar
                }
            } else {
                theme.sidebar
            };
            grid.fill(Rect::new(area.x, y, area.w, 1), style);
            let marker = match (entry.is_dir, entry.expanded) {
                (true, true) => "▾ ",
                (true, false) => "▸ ",
                (false, _) => "  ",
            };
            let indent = " ".repeat(entry.depth * 2 + 1);
            let text = format!("{indent}{marker}{}", entry.name);
            grid.put_str(area.x, y, &text, style, area.right().saturating_sub(2));
            if let Some(status) = self.status.get(&entry.path) {
                let mark = Style {
                    fg: theme.accent,
                    ..style
                };
                let x = area.right().saturating_sub(2);
                grid.put_str(x, y, &status.to_string(), mark, area.right());
            }
        }
    }
}
