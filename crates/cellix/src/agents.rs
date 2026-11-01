use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use cellix_agent::hook::PLAN_WAITING;
use cellix_agent::{AgentEvent, AgentState};
use cellix_render::{Bounds, Grid, Rect, Rgb, Style};
use cellix_review::Worktree;

use crate::integrations;
use crate::theme::Theme;

/// Rows above the buttons: the panel's title and a gap.
const HEADER_ROWS: u16 = 2;
const CLAUDE_ROW: u16 = HEADER_ROWS;
const CODEX_ROW: u16 = HEADER_ROWS + 2;
/// Rows from the top of the panel to its first list.
const LISTS_TOP: u16 = HEADER_ROWS + 5;
const SECTION_SIZE: u8 = 80;
const DETAIL_SIZE: u8 = 85;

/// What a click in the panel lands on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Hit {
    OpenClaude,
    OpenCodex,
    Worktree(PathBuf),
}

/// The Agents panel: who's running, who needs you, and where work is happening.
#[derive(Default)]
pub struct Agents {
    pub running: Vec<AgentEvent>,
    pub worktrees: Vec<Worktree>,
    /// Agents found running in this folder that haven't reported through a plugin.
    seen: Arc<Mutex<Vec<String>>>,
    pub codex_found: bool,
}

/// Names of the agents whose working folder is `root`, from `lsof -Fcn` output.
fn agents_in(output: &str, root: &Path) -> Vec<String> {
    let mut found = Vec::new();
    let mut command = "";
    for line in output.lines() {
        if let Some(name) = line.strip_prefix('c') {
            command = name;
        } else if let Some(folder) = line.strip_prefix('n')
            && Path::new(folder).starts_with(root)
        {
            found.push(match command {
                "claude" => "Claude Code".to_string(),
                "codex" => "Codex".to_string(),
                other => other.to_string(),
            });
        }
    }
    found
}

impl Agents {
    pub fn update(&mut self, event: AgentEvent) {
        self.running.retain(|a| a.session != event.session);
        if event.state != AgentState::Ended {
            self.running.push(event);
        }
    }

    /// The answer went back, so the agent is working again before any hook says so.
    pub fn plan_answered(&mut self) {
        let waiting = self.running.iter_mut().filter(|a| a.detail == PLAN_WAITING);
        for agent in waiting {
            agent.state = AgentState::Working;
            agent.detail = "Working".to_string();
        }
    }

    pub fn needing_you(&self) -> usize {
        let waiting = |a: &&AgentEvent| a.state == AgentState::NeedsYou;
        self.running.iter().filter(waiting).count()
    }

    /// Looks for agents running here, off the main thread: `lsof` can take a moment, and
    /// agents that came up without the plugin never tell Cellix they exist.
    pub fn scan(&self, root: &Path) {
        let seen = self.seen.clone();
        let root = root.to_path_buf();
        std::thread::spawn(move || {
            let output = std::process::Command::new("lsof")
                .args(["-a", "-d", "cwd", "-c", "claude", "-c", "codex", "-Fcn"])
                .output();
            if let (Ok(output), Ok(mut seen)) = (output, seen.lock()) {
                *seen = agents_in(&String::from_utf8_lossy(&output.stdout), &root);
            }
        });
    }

    /// Agents found running that the plugin isn't already reporting.
    fn silent(&self) -> Vec<String> {
        let seen = self.seen.lock().map(|s| s.clone()).unwrap_or_default();
        seen.into_iter().skip(self.running.len()).collect()
    }

    fn worktrees_top(&self) -> u16 {
        let shown = self.running.len() + self.silent().len();
        LISTS_TOP + 1 + shown.max(1) as u16 * 2 + 1
    }

    pub fn hit(&self, area: Rect, y: u16) -> Option<Hit> {
        let row = y.checked_sub(area.y)?;
        if row == CLAUDE_ROW {
            return Some(Hit::OpenClaude);
        }
        if row == CODEX_ROW && self.codex_found {
            return Some(Hit::OpenCodex);
        }
        let below = row.checked_sub(self.worktrees_top() + 1)?;
        let tree = self.worktrees.get(below as usize / 2)?;
        Some(Hit::Worktree(tree.path.clone()))
    }

    pub fn draw(&self, grid: &mut Grid, area: Rect, hover: Option<(u16, u16)>, theme: &Theme) {
        grid.fill(area, theme.sidebar);
        let dim = Style {
            fg: theme.gutter.fg,
            ..theme.sidebar
        };
        let name = Style {
            bold: true,
            ..theme.sidebar
        };
        let side = area.x + 2;
        grid.label(side, area.y, "Agents", name, area.right());
        let width = area.w.saturating_sub(4);
        self.draw_buttons(grid, Rect::new(side, area.y, width, 1), hover, theme);
        let section = |grid: &mut Grid, y, text: &str| {
            grid.label_at(side as f32, side, y, text, dim, area.right(), SECTION_SIZE);
        };
        let mut y = area.y + LISTS_TOP;
        section(grid, y, "RUNNING HERE");
        y += 1;
        let silent = self.silent();
        if self.running.is_empty() && silent.is_empty() {
            grid.label(side, y, "No agents here", dim, area.right());
            y += 2;
        }
        for agent in &self.running {
            let (mark, color) = match agent.state {
                AgentState::NeedsYou => ("needs you", theme.attention),
                AgentState::Working => ("working", theme.git_added),
                AgentState::Idle | AgentState::Ended => ("idle", theme.gutter.fg),
            };
            draw_agent(grid, (area, y), &agent.agent, (mark, color), theme);
            let detail = side + 2;
            grid.label(detail, y + 1, &agent.detail, dim, area.right());
            y += 2;
        }
        for agent in &silent {
            draw_agent(grid, (area, y), agent, ("running", theme.git_added), theme);
            grid.label(side + 2, y + 1, "Without the plugin", dim, area.right());
            y += 2;
        }
        y += 1;
        section(grid, y, "WORKTREES");
        y += 1;
        for worktree in &self.worktrees {
            if y + 1 >= area.bottom() {
                break;
            }
            let row = Rect::new(area.x, y, area.w, 2);
            if hover.is_some_and(|h| row.contains(h.0, h.1)) {
                let wash = Bounds::inset(row, 0.4, 0.06);
                grid.panel(wash, 18, Some(theme.row_hover), None);
            }
            let path = &worktree.path;
            let folder = path.file_name().map(|n| n.to_string_lossy().into_owned());
            let title = if worktree.branch == "detached" {
                folder.clone().unwrap_or_else(|| worktree.branch.clone())
            } else {
                worktree.branch.clone()
            };
            grid.label(side, y, &title, theme.sidebar, area.right());
            let place = if worktree.branch == "detached" {
                "Detached HEAD".to_string()
            } else {
                folder.unwrap_or_default()
            };
            let x = side as f32;
            grid.label_at(x, side, y + 1, &place, dim, area.right(), DETAIL_SIZE);
            if worktree.changes > 0 {
                let count = format!("{} changed", worktree.changes);
                let badge = Style {
                    fg: theme.git_modified,
                    ..theme.sidebar
                };
                let right = area.right().saturating_sub(2);
                grid.label_right(right, y, &count, badge, DETAIL_SIZE);
            }
            y += 2;
        }
    }

    fn draw_buttons(&self, grid: &mut Grid, top: Rect, hover: Option<(u16, u16)>, theme: &Theme) {
        let on = |row: u16| hover.is_some_and(|h| h.1 == top.y + row);
        let rows: [(u16, &str, bool, Rgb, Rgb); 2] = [
            (
                CLAUDE_ROW,
                "Open Claude Code",
                true,
                theme.button,
                theme.button_text,
            ),
            (
                CODEX_ROW,
                if self.codex_found {
                    "Open Codex"
                } else {
                    "Codex isn't installed"
                },
                self.codex_found,
                theme.row_hover,
                theme.text.fg,
            ),
        ];
        for (row, label, usable, fill, fg) in rows {
            let fill = if usable && on(row) && row != CLAUDE_ROW {
                theme.pick
            } else {
                fill
            };
            let style = Style {
                fg: if usable { fg } else { theme.gutter.fg },
                ..theme.sidebar
            };
            let cells = format!("{label:^width$}", width = top.w as usize);
            grid.button(top.x, top.y + row, &cells, label, fill, style);
        }
    }

    /// Whether Codex can be opened, which only changes when the panel opens.
    pub fn look_for_codex(&mut self) {
        self.codex_found = integrations::on_path("codex", |name| std::env::var(name).ok());
    }
}

fn draw_agent(
    grid: &mut Grid,
    (area, y): (Rect, u16),
    agent: &str,
    (mark, color): (&str, Rgb),
    theme: &Theme,
) {
    let dot = Bounds::cells(area.x as f32 + 2.0, y as f32 + 0.32, 0.7, 0.36);
    grid.panel(dot, 18, Some(color), None);
    let name = Style {
        bold: true,
        ..theme.sidebar
    };
    grid.label(area.x + 4, y, agent, name, area.right());
    let badge = Style {
        fg: color,
        ..theme.sidebar
    };
    grid.label_right(area.right().saturating_sub(2), y, mark, badge, DETAIL_SIZE);
}

#[cfg(test)]
mod tests {
    use super::*;

    const OUTPUT: &str = "p1\ncclaude\nfcwd\nn/work/project\n\
p2\ncclaude\nfcwd\nn/work/other\n\
p3\nccodex\nfcwd\nn/work/project/sub\n";

    #[test]
    fn only_agents_in_this_folder_count() {
        let found = agents_in(OUTPUT, Path::new("/work/project"));
        assert_eq!(found, ["Claude Code", "Codex"]);
    }

    #[test]
    fn a_folder_with_a_similar_name_isnt_this_one() {
        assert!(agents_in(OUTPUT, Path::new("/work/proj")).is_empty());
    }
}
