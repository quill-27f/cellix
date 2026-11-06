use std::collections::HashMap;
use std::path::PathBuf;

use cellix_core::{Entry, Workspace};
use cellix_render::{Bounds, Font, Grid, Icon, Measure, Paint, Rect, Rgb, Style};

use crate::theme::Theme;

/// Rows above the tree: the EXPLORER title and the folder name.
const HEADER_ROWS: u16 = 1;
const BRANCH_SIZE: u8 = 90;

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
        self.show(workspace.entries());
    }

    /// Shows a fresh list, keeping the selection on the same file.
    pub fn show(&mut self, entries: Vec<Entry>) {
        let selected = self
            .selected
            .and_then(|i| self.entries.get(i))
            .map(|e| e.path.clone());
        self.entries = entries;
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
        (root_name, branch): (&str, Option<&str>),
        measure: &dyn Measure,
        hover: Option<(u16, u16)>,
        theme: &Theme,
    ) {
        grid.fill(area, theme.sidebar);
        let heading = Style {
            bold: true,
            ..theme.sidebar
        };
        let end = grid.label(area.x + 2, area.y, root_name, heading, area.right());
        if let Some(branch) = branch {
            // Proportional text is narrower than its cells, so measure it.
            let bold = Font {
                bold: true,
                ..Font::default()
            };
            let small = Font {
                size: BRANCH_SIZE,
                ..Font::default()
            };
            let x = area.x as f32 + 2.0 + measure.width(root_name, bold) + 1.0;
            let chevron = 0.9;
            let wide = measure.width(branch, small) + 1.0 + chevron;
            let hovered = hover.is_some_and(|h| h.1 == area.y && area.contains(h.0, h.1));
            let fill = if hovered {
                theme.row_active
            } else {
                theme.row_hover
            };
            let pill = Bounds::cells(x - 0.5, area.y as f32 + 0.14, wide, 0.72);
            grid.panel(pill, 18, Some(fill), None);
            let dim = Style {
                fg: if hovered {
                    theme.text.fg
                } else {
                    theme.status.fg
                },
                ..theme.sidebar
            };
            grid.label_at(x, end + 1, area.y, branch, dim, area.right(), BRANCH_SIZE);
            grid.paint.push(Paint::Icon {
                bounds: Bounds::cells(x + wide - chevron - 0.8, area.y as f32 + 0.27, 0.8, 0.46),
                icon: Icon::ChevronDown,
                color: theme.status.fg,
            });
        }
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
                grid.panel(Bounds::inset(line, 0.4, 0.055), 18, Some(fill), None);
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
                // Ignored, it's half there, as in VS Code: still listed, but quiet.
                None if entry.ignored => Style {
                    fg: theme.ignored,
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
                folder_color(&entry.name)
            } else {
                file_color(&entry.name)
            };
            // The name's colour, so the icon fades as much as the name does.
            let color = if entry.ignored { theme.ignored } else { color };
            if let Some(chevron) = chevron {
                grid.glyph(
                    x as f32 - 1.3,
                    y,
                    chevron,
                    if entry.ignored {
                        theme.ignored
                    } else {
                        theme.sidebar.fg
                    },
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

/// A folder's colour, for the names that mean something, as VS Code's icon themes do.
fn folder_color(name: &str) -> Rgb {
    match name {
        "src" | "lib" | "app" | "crates" | "packages" => Rgb(120, 170, 240),
        "test" | "tests" | "spec" | "__tests__" => Rgb(120, 200, 150),
        "docs" | "doc" => Rgb(110, 190, 220),
        ".github" | ".git" | ".gitlab" => Rgb(235, 130, 110),
        ".claude" | ".codex" | ".cursor" => Rgb(230, 150, 110),
        "target" | "dist" | "build" | "out" | "node_modules" | ".venv" => Rgb(135, 135, 155),
        "scripts" | "harness" | "bin" | "tools" => Rgb(170, 200, 110),
        "terraform" | "infra" | "deploy" | "k8s" => Rgb(165, 140, 235),
        "assets" | "public" | "static" | "images" => Rgb(230, 190, 100),
        _ => Rgb(143, 168, 205),
    }
}

/// A file's colour, standing in for the logo a full icon theme would draw.
pub fn file_color(name: &str) -> Rgb {
    let lower = name.to_lowercase();
    if lower.starts_with(".env") {
        return Rgb(235, 200, 90);
    }
    if lower.starts_with("dockerfile") {
        return Rgb(90, 170, 235);
    }
    if lower.starts_with("license") {
        return Rgb(220, 190, 110);
    }
    match lower.rsplit('.').next().unwrap_or("") {
        "rs" => Rgb(235, 140, 90),
        "md" | "markdown" | "mdx" => Rgb(100, 170, 240),
        "json" | "jsonc" => Rgb(235, 200, 90),
        "toml" | "ini" | "cfg" | "conf" => Rgb(170, 170, 190),
        "yml" | "yaml" => Rgb(225, 110, 130),
        "tf" | "tfvars" | "hcl" => Rgb(165, 140, 235),
        "py" => Rgb(90, 165, 225),
        "ts" | "tsx" => Rgb(80, 150, 235),
        "js" | "jsx" | "mjs" => Rgb(240, 215, 100),
        "go" => Rgb(90, 200, 225),
        "sh" | "bash" | "zsh" => Rgb(140, 205, 110),
        "html" | "htm" => Rgb(235, 120, 80),
        "css" | "scss" => Rgb(110, 150, 240),
        "c" | "h" | "cpp" | "cc" | "hpp" => Rgb(110, 160, 220),
        "java" => Rgb(225, 120, 90),
        "rb" => Rgb(225, 90, 90),
        "sql" => Rgb(235, 180, 110),
        "lock" => Rgb(135, 135, 155),
        "gitignore" | "dockerignore" => Rgb(235, 130, 110),
        _ => Rgb(150, 150, 172),
    }
}
