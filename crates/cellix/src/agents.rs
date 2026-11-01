use std::path::PathBuf;

use cellix_agent::hook::PLAN_WAITING;
use cellix_agent::{AgentEvent, AgentState};
use cellix_render::{Bounds, Grid, Rect, Style};
use cellix_review::Worktree;

use crate::theme::Theme;

/// Rows above the list: the panel's title.
const HEADER_ROWS: u16 = 1;

/// The Agents panel: who's running, who needs you, and where work is happening.
#[derive(Default)]
pub struct Agents {
    pub running: Vec<AgentEvent>,
    pub worktrees: Vec<Worktree>,
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

    /// The worktree on a row, for a click.
    pub fn worktree_at(&self, area: Rect, y: u16) -> Option<PathBuf> {
        let row = y.checked_sub(area.y + self.worktrees_top())? as usize;
        self.worktrees.get(row).map(|w| w.path.clone())
    }

    /// Rows above the first worktree, inside the panel.
    fn worktrees_top(&self) -> u16 {
        HEADER_ROWS + self.running.len().max(1) as u16 * 2 + 2
    }

    pub fn draw(&self, grid: &mut Grid, area: Rect, theme: &Theme) {
        grid.fill(area, theme.sidebar);
        let dim = Style {
            fg: theme.gutter.fg,
            ..theme.sidebar
        };
        let title = |grid: &mut Grid, y, text| {
            grid.label_at(
                area.x as f32 + 2.0,
                area.x + 2,
                y,
                text,
                dim,
                area.right(),
                80,
            );
        };
        title(grid, area.y, "AGENTS");
        let mut y = area.y + HEADER_ROWS;
        if self.running.is_empty() {
            grid.label(area.x + 2, y, "None running", dim, area.right());
            y += 2;
        }
        for agent in &self.running {
            let (mark, color) = match agent.state {
                AgentState::NeedsYou => ("needs you", theme.git_modified),
                AgentState::Working => ("working", theme.git_added),
                AgentState::Idle | AgentState::Ended => ("idle", theme.gutter.fg),
            };
            let dot = Bounds::cells(area.x as f32 + 2.0, y as f32 + 0.32, 0.7, 0.36);
            grid.panel(dot, 18, Some(color), None);
            let name = Style {
                bold: true,
                ..theme.sidebar
            };
            grid.label(area.x + 4, y, &agent.agent, name, area.right());
            let badge = Style {
                fg: color,
                ..theme.sidebar
            };
            grid.label_right(area.right().saturating_sub(1), y, mark, badge, 85);
            grid.label(area.x + 4, y + 1, &agent.detail, dim, area.right());
            y += 2;
        }
        y += 1;
        title(grid, y, "WORKTREES");
        y += 1;
        for worktree in &self.worktrees {
            if y >= area.bottom() {
                break;
            }
            grid.label(area.x + 2, y, &worktree.branch, theme.sidebar, area.right());
            if worktree.changes > 0 {
                let count = worktree.changes.to_string();
                let badge = Style {
                    fg: theme.git_modified,
                    ..theme.sidebar
                };
                grid.label_right(area.right().saturating_sub(1), y, &count, badge, 85);
            }
            y += 1;
        }
    }
}
