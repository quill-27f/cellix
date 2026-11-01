use std::collections::HashMap;
use std::path::PathBuf;

use cellix_core::{Entry, Workspace};
use cellix_render::{Bounds, Grid, Icon, Rect, Rgb, Style};

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

    pub fn draw(
        &self,
        grid: &mut Grid,
        area: Rect,
        root_name: &str,
        hover: Option<(u16, u16)>,
        theme: &Theme,
    ) {
        grid.fill(area, theme.sidebar);
        let title = Style {
            fg: theme.gutter.fg,
            ..theme.sidebar
        };
        grid.label_at(
            area.x as f32 + 2.0,
            area.x + 2,
            area.y,
            "EXPLORER",
            title,
            area.right(),
            80,
        );
        let heading = Style {
            bold: true,
            ..theme.sidebar
        };
        let name = root_name.to_uppercase();
        grid.label_at(
            area.x as f32 + 2.0,
            area.x + 3,
            area.y + 1,
            &name,
            heading,
            area.right(),
            85,
        );
        for (row, entry) in self.entries.iter().skip(self.top).enumerate() {
            let y = area.y + HEADER_ROWS + row as u16;
            if y >= area.bottom() {
                break;
            }
            let selected = self.selected == Some(self.top + row);
            let hovered = hover.is_some_and(|h| h.1 == y && area.contains(h.0, h.1));
            if selected || hovered {
                let fill = if selected {
                    theme.row_active
                } else {
                    theme.row_hover
                };
                let line = Rect::new(area.x, y, area.w, 1);
                grid.panel(Bounds::inset(line, 0.4, 0.03), 14, Some(fill), None);
            }
            let status = self.status.get(&entry.path).copied();
            let style = match status {
                Some('?' | 'A') => Style {
                    fg: theme.git_added,
                    ..theme.sidebar
                },
                Some('•' | 'M') => Style {
                    fg: theme.git_modified,
                    ..theme.sidebar
                },
                Some(_) => Style {
                    fg: theme.error,
                    ..theme.sidebar
                },
                None => theme.sidebar,
            };
            let x = area.x + entry.depth as u16 * 2 + 2;
            let end = area.right().saturating_sub(2);
            let (fallback, chevron, icon) = match (entry.is_dir, entry.expanded) {
                (true, true) => ("▾", Some(Icon::ChevronDown), Icon::FolderOpen),
                (true, false) => ("▸", Some(Icon::ChevronRight), Icon::Folder),
                (false, _) => ("▪", None, Icon::File),
            };
            let color = if entry.is_dir {
                theme.folder
            } else {
                file_color(&entry.name, theme)
            };
            if let Some(chevron) = chevron {
                grid.glyph(
                    x as f32 - 1.3,
                    y,
                    chevron,
                    theme.sidebar.fg,
                    " ",
                    theme.sidebar,
                );
            }
            grid.glyph(x as f32, y, icon, color, fallback, theme.sidebar);
            grid.label_at(x as f32 + 2.0, x + 2, y, &entry.name, style, end, 100);
            if let Some(status) = status {
                let shown = if status == '?' { 'U' } else { status };
                grid.label_right(
                    area.right().saturating_sub(1),
                    y,
                    &shown.to_string(),
                    style,
                    90,
                );
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
