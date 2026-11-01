use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use cellix_agent::hook::PLAN_WAITING;
use cellix_agent::{AgentEvent, AgentState};
use cellix_render::{Bounds, Grid, Rect, Rgb, Style};
use cellix_review::Worktree;

use crate::integrations;
use crate::theme::Theme;

const SECTION_SIZE: u8 = 80;
const DETAIL_SIZE: u8 = 85;
/// Agents that can be started from here, as what the panel calls them and their programs.
const AGENTS: [(&str, &str); 3] = [
    ("Claude Code", "claude"),
    ("Codex", "codex"),
    ("Gemini CLI", "gemini"),
];

/// What a click in the panel lands on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Hit {
    /// The program of an agent to start.
    Start(&'static str),
    Worktree(PathBuf),
}

/// A row or two of the panel.
enum Line {
    Title(&'static str),
    Section(&'static str),
    Gap,
    /// The note that takes the running list's place.
    Nothing,
    /// An index into `running`.
    Reporting(usize),
    /// An agent found running, which reports nothing.
    Silent(String),
    /// An index into `worktrees`.
    Tree(usize),
    /// An index into `installed`.
    Start(usize),
}

/// The Agents panel: who's running, who needs you, and where work is happening.
#[derive(Default)]
pub struct Agents {
    pub running: Vec<AgentEvent>,
    pub worktrees: Vec<Worktree>,
    /// Agents found running in this folder that haven't reported through a plugin.
    seen: Arc<Mutex<Vec<String>>>,
    /// The agents installed here.
    installed: Vec<(&'static str, &'static str)>,
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
                "gemini" => "Gemini CLI".to_string(),
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
                .args(["-a", "-d", "cwd", "-c", "claude", "-c", "codex"])
                .args(["-c", "gemini", "-Fcn"])
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

    /// The panel, top to bottom. Drawing and clicking share this. With nothing running, a
    /// way to start something comes straight after saying so.
    fn lines(&self) -> Vec<Line> {
        let mut lines = vec![Line::Title("Agents"), Line::Gap, Line::Section("RUNNING")];
        let silent = self.silent();
        let idle = self.running.is_empty() && silent.is_empty();
        if idle {
            lines.push(Line::Nothing);
        }
        lines.extend((0..self.running.len()).map(Line::Reporting));
        lines.extend(silent.into_iter().map(Line::Silent));
        let starts = |lines: &mut Vec<Line>| {
            if !self.installed.is_empty() {
                lines.push(Line::Gap);
                lines.push(Line::Section("START AN AGENT"));
                lines.extend((0..self.installed.len()).map(Line::Start));
            }
        };
        if idle {
            starts(&mut lines);
        }
        lines.push(Line::Gap);
        lines.push(Line::Section("WORKTREES"));
        lines.extend((0..self.worktrees.len()).map(Line::Tree));
        if !idle {
            starts(&mut lines);
        }
        lines
    }

    pub fn hit(&self, area: Rect, y: u16) -> Option<Hit> {
        let mut top = area.y;
        for line in self.lines() {
            let rows = self.height(&line);
            if y >= top && y < top + rows {
                return match line {
                    Line::Start(index) => Some(Hit::Start(self.installed[index].1)),
                    Line::Tree(index) => Some(Hit::Worktree(self.worktrees[index].path.clone())),
                    _ => None,
                };
            }
            top += rows;
        }
        None
    }

    fn height(&self, line: &Line) -> u16 {
        match line {
            Line::Nothing | Line::Reporting(_) | Line::Silent(_) => 2,
            // One worktree is just this folder, so it needs no second line to tell them apart.
            Line::Tree(_) if self.worktrees.len() > 1 => 2,
            _ => 1,
        }
    }

    pub fn draw(&self, grid: &mut Grid, area: Rect, hover: Option<(u16, u16)>, theme: &Theme) {
        grid.fill(area, theme.sidebar);
        let dim = Style {
            fg: theme.gutter.fg,
            ..theme.sidebar
        };
        let side = area.x + 2;
        let mut y = area.y;
        for line in self.lines() {
            let rows = self.height(&line);
            if y + rows > area.bottom() {
                break;
            }
            let row = Rect::new(area.x, y, area.w, rows);
            let hovered = hover.is_some_and(|h| row.contains(h.0, h.1));
            match line {
                Line::Gap => {}
                Line::Title(text) => {
                    let name = Style {
                        bold: true,
                        ..theme.sidebar
                    };
                    grid.label(side, y, text, name, area.right());
                }
                Line::Section(text) => {
                    grid.label_at(side as f32, side, y, text, dim, area.right(), SECTION_SIZE);
                }
                Line::Nothing => {
                    grid.label(side, y, "No agents running", dim, area.right());
                    let note = if self.installed.is_empty() {
                        "Install one, then review what it changes."
                    } else {
                        "Start one below, then review what it changes."
                    };
                    grid.label_at(
                        side as f32,
                        side,
                        y + 1,
                        note,
                        dim,
                        area.right(),
                        DETAIL_SIZE,
                    );
                }
                Line::Reporting(index) => {
                    let agent = &self.running[index];
                    let (mark, color) = match agent.state {
                        AgentState::NeedsYou => ("needs you", theme.attention),
                        AgentState::Working => ("working", theme.git_added),
                        AgentState::Idle | AgentState::Ended => ("idle", theme.gutter.fg),
                    };
                    draw_agent(grid, (area, y), &agent.agent, (mark, color), theme);
                    grid.label(side + 2, y + 1, &agent.detail, dim, area.right());
                }
                Line::Silent(agent) => {
                    draw_agent(grid, (area, y), &agent, ("running", theme.git_added), theme);
                    grid.label(
                        side + 2,
                        y + 1,
                        "No plugin, so no detail",
                        dim,
                        area.right(),
                    );
                }
                Line::Tree(index) => {
                    if hovered {
                        let wash = Bounds::inset(row, 0.4, 0.055);
                        grid.panel(wash, 18, Some(theme.row_hover), None);
                    }
                    self.draw_worktree(grid, area, (y, rows), index, theme);
                }
                Line::Start(index) => {
                    if hovered {
                        let wash = Bounds::inset(row, 0.4, 0.055);
                        grid.panel(wash, 18, Some(theme.row_hover), None);
                    }
                    let plus = Style {
                        fg: theme.gutter.fg,
                        ..theme.sidebar
                    };
                    grid.label(side, y, "+", plus, side + 2);
                    let label = self.installed[index].0;
                    grid.label(side + 2, y, label, theme.sidebar, area.right());
                }
            }
            y += rows;
        }
    }

    fn draw_worktree(
        &self,
        grid: &mut Grid,
        area: Rect,
        (y, rows): (u16, u16),
        index: usize,
        theme: &Theme,
    ) {
        let dim = Style {
            fg: theme.gutter.fg,
            ..theme.sidebar
        };
        let side = area.x + 2;
        let worktree = &self.worktrees[index];
        let path = &worktree.path;
        let folder = path.file_name().map(|n| n.to_string_lossy().into_owned());
        let detached = worktree.branch == "detached";
        let title = if detached {
            folder.clone().unwrap_or_else(|| worktree.branch.clone())
        } else {
            worktree.branch.clone()
        };
        grid.label(side, y, &title, theme.sidebar, area.right());
        if rows > 1 {
            let place = if detached {
                "Detached HEAD".to_string()
            } else {
                folder.unwrap_or_default()
            };
            let x = side as f32;
            grid.label_at(x, side, y + 1, &place, dim, area.right(), DETAIL_SIZE);
        }
        if worktree.changes > 0 {
            let count = format!("{} changed", worktree.changes);
            let badge = Style {
                fg: theme.git_modified,
                ..theme.sidebar
            };
            let right = area.right().saturating_sub(2);
            grid.label_right(right, y, &count, badge, DETAIL_SIZE);
        }
    }

    /// Finds which agents can be started, which only changes when the panel opens.
    pub fn look_for_agents(&mut self) {
        let var = |name: &str| std::env::var(name).ok();
        self.installed = AGENTS
            .iter()
            .copied()
            .filter(|(_, program)| integrations::on_path(program, var))
            .collect();
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
    fn a_click_finds_the_agent_to_start_below_the_worktrees() {
        let agents = Agents {
            installed: vec![("Codex", "codex")],
            worktrees: vec![Worktree {
                path: PathBuf::from("/work"),
                branch: "main".to_string(),
                changes: 0,
            }],
            ..Agents::default()
        };
        let area = Rect::new(0, 0, 30, 40);
        let at = |hit: &Hit| (0..40).find(|y| agents.hit(area, *y).as_ref() == Some(hit));
        let start = at(&Hit::Start("codex")).unwrap();
        let tree = at(&Hit::Worktree(PathBuf::from("/work"))).unwrap();
        assert!(
            start < tree,
            "with nothing running, starting one comes first"
        );
        assert_eq!(agents.hit(area, 0), None, "the title does nothing");
    }

    #[test]
    fn a_folder_with_a_similar_name_isnt_this_one() {
        assert!(agents_in(OUTPUT, Path::new("/work/proj")).is_empty());
    }
}
