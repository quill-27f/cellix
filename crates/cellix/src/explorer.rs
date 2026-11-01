use std::collections::HashMap;
use std::path::PathBuf;

use cellix_core::{Entry, Workspace};
use cellix_render::{Grid, Rect, Rgb, Style};

use crate::theme::Theme;

/// Rows above the tree: the EXPLORER title and the folder name.
const HEADER_ROWS: u16 = 2;

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
        let row = self.top + y.checked_sub(area.y + HEADER_ROWS)? as usize;
        (row < self.entries.len()).then_some(row)
    }

    pub fn scroll_by(&mut self, delta: isize) {
        let last = self.entries.len().saturating_sub(1) as isize;
        self.top = (self.top as isize + delta).clamp(0, last.max(0)) as usize;
    }

    pub fn draw(&self, grid: &mut Grid, area: Rect, root_name: &str, theme: &Theme) {
        grid.fill(area, theme.sidebar);
        grid.put_str(area.x + 2, area.y, "EXPLORER", theme.sidebar, area.right());
        let heading = Style {
            bold: true,
            ..theme.sidebar
        };
        let title = format!("▾ {}", root_name.to_uppercase());
        grid.put_str(area.x + 1, area.y + 1, &title, heading, area.right());
        for (row, entry) in self.entries.iter().skip(self.top).enumerate() {
            let y = area.y + HEADER_ROWS + row as u16;
            if y >= area.bottom() {
                break;
            }
            let base = if self.selected == Some(self.top + row) {
                Style {
                    bg: theme.row_hover,
                    ..theme.sidebar
                }
            } else {
                theme.sidebar
            };
            grid.fill(Rect::new(area.x, y, area.w, 1), base);
            let status = self.status.get(&entry.path).copied();
            let style = match status {
                Some('?' | 'A') => Style {
                    fg: theme.git_added,
                    ..base
                },
                Some('•' | 'M') => Style {
                    fg: theme.git_modified,
                    ..base
                },
                Some(_) => Style {
                    fg: theme.error,
                    ..base
                },
                None => base,
            };
            let x = area.x + entry.depth as u16 * 2 + 2;
            let end = area.right().saturating_sub(2);
            let (icon, icon_fg) = match (entry.is_dir, entry.expanded) {
                (true, true) => ("▾ ", theme.sidebar.fg),
                (true, false) => ("▸ ", theme.sidebar.fg),
                (false, _) => ("▪ ", file_color(&entry.name, theme)),
            };
            let icon_style = Style {
                fg: icon_fg,
                ..base
            };
            let x = grid.put_str(x, y, icon, icon_style, end);
            grid.put_str(x, y, &entry.name, style, end);
            if let Some(status) = status {
                let shown = if status == '?' { 'U' } else { status };
                grid.put_str(end, y, &shown.to_string(), style, area.right());
            }
        }
    }
}

/// A colour for the file's kind, standing in for VS Code's file icons.
fn file_color(name: &str, theme: &Theme) -> Rgb {
    match name.rsplit('.').next().unwrap_or("") {
        "rs" => theme.string,
        "md" | "markdown" => theme.accent,
        "json" | "toml" | "yml" | "yaml" => theme.git_modified,
        "py" | "go" | "ts" | "tsx" | "js" | "jsx" => theme.kind,
        "sh" | "bash" | "zsh" => theme.comment,
        _ => theme.gutter.fg,
    }
}
