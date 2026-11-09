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
/// What the side pane is for, in the order it happens.
const STEPS: [&str; 3] = [
    "Start an agent below. It runs here.",
    "Click a file on the left to open it beside it.",
    "Ask in the agent, or comment on what you select.",
];
/// The same, docked beside a chat with the file above: two lines, and no explorer to
/// point at.
const DOCKED_STEPS: [&str; 2] = [
    "An agent you start runs here, under the file.",
    "Select text in the file to comment or ask it.",
];
const AGENTS: [(&str, &str); 3] = [
    ("Claude Code", "claude"),
    ("Codex", "codex"),
    ("Gemini CLI", "gemini"),
];
/// The name of an agent's tab for the name its hooks give it: Gemini CLI's say "Gemini".
pub fn tab_name(agent: &str) -> &str {
    match agent {
        "Gemini" => "Gemini CLI",
        other => other,
    }
}

/// What the panel calls a plain shell, and what stands for it in `Hit::Start`.
pub const SHELL: (&str, &str) = ("Terminal", "shell");

/// The program a Terminal tab runs: the user's shell, or on Windows PowerShell, which every
/// Windows has. There `SHELL` is unset, or a Unix path Git Bash left behind.
pub fn shell() -> String {
    if cfg!(windows) {
        "powershell.exe".to_string()
    } else {
        std::env::var("SHELL").unwrap_or_else(|_| "sh".to_string())
    }
}

/// What opens a file or page in the system's own program for it. On Windows that isn't
/// `cmd /c start`, because cmd would read a link's `&` as the start of another command.
pub fn opener() -> Vec<String> {
    let argv: &[&str] = if cfg!(target_os = "macos") {
        &["open"]
    } else if cfg!(windows) {
        &["rundll32", "url.dll,FileProtocolHandler"]
    } else {
        &["xdg-open"]
    };
    argv.iter().map(|word| word.to_string()).collect()
}

/// What the panel calls a program.
pub fn display_name(program: &str) -> &'static str {
    let named = AGENTS.iter().find(|(_, p)| *p == program);
    named.map_or(SHELL.0, |(name, _)| name)
}

/// The program of an agent the panel calls `name`.
pub fn program_of(name: &str) -> Option<&'static str> {
    AGENTS.iter().find(|(n, _)| *n == name).map(|(_, p)| *p)
}

/// What resumes an agent's own session, put after its program: the agents that can be told
/// which session to pick up.
pub fn resume_args(name: &str, session: &str) -> Option<Vec<String>> {
    let words: &[&str] = match name {
        "Claude Code" => &["--resume"],
        "Codex" => &["resume"],
        _ => return None,
    };
    let mut args: Vec<String> = words.iter().map(|w| w.to_string()).collect();
    args.push(session.to_string());
    Some(args)
}

/// What a click in the panel lands on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Hit {
    /// The program of an agent to start.
    Start(&'static str),
    Worktree(PathBuf),
}

/// A row or two of the panel.
enum Line {
    Section(&'static str),
    Gap,
    /// The note that takes the running list's place.
    Step(usize),
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
    /// When an agent here last finished its turn, until one starts working again.
    pub finished: Option<std::time::Instant>,
    /// Docked beside a chat: short how-to, and one row for each agent.
    pub docked: bool,
}

/// Whether a process is still there.
#[cfg(unix)]
fn alive(pid: u32) -> bool {
    let Ok(pid) = libc::pid_t::try_from(pid) else {
        return false;
    };
    // SAFETY: signal 0 sends nothing; it only asks whether the process exists. A process
    // that exists but isn't ours answers EPERM, which still means it's there.
    let sent = unsafe { libc::kill(pid, 0) };
    sent == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

#[cfg(not(unix))]
fn alive(_pid: u32) -> bool {
    true
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
        match event.state {
            AgentState::Working | AgentState::NeedsYou => self.finished = None,
            AgentState::Idle if event.detail == "Finished" => {
                self.finished = Some(std::time::Instant::now());
            }
            _ => {}
        }
        // One process at a time is one session: news under a new id, as after /clear,
        // replaces what the old one said.
        let same = |a: &AgentEvent| {
            a.session == event.session || (event.pid.is_some() && a.pid == event.pid)
        };
        self.running.retain(|a| !same(a));
        if event.state != AgentState::Ended {
            self.running.push(event);
        }
    }

    /// Which agent's plan is waiting, by its name.
    pub fn plan_author(&self) -> Option<&str> {
        let waiting = self.running.iter().rev().find(|a| a.detail == PLAN_WAITING);
        waiting.map(|a| a.agent.as_str())
    }

    /// The answer went back, so the agent is working again before any hook says so.
    pub fn plan_answered(&mut self) {
        let waiting = self.running.iter_mut().filter(|a| a.detail == PLAN_WAITING);
        for agent in waiting {
            agent.state = AgentState::Working;
            agent.detail = "Working".to_string();
        }
    }

    /// The agent working on this project outside the pane, if there is one. It hears about
    /// comments through its hooks, with its next prompt.
    pub fn outside(&self) -> Option<String> {
        let reporting = self.running.first().map(|a| a.agent.clone());
        reporting.or_else(|| self.silent().into_iter().next())
    }

    /// Lets go of agents whose process has gone without saying so, as when one is killed,
    /// and gives their sessions. Without a process to look at, an agent stays.
    pub fn prune(&mut self) -> Vec<String> {
        let (gone, kept): (Vec<_>, Vec<_>) = std::mem::take(&mut self.running)
            .into_iter()
            .partition(|a| a.pid.is_some_and(|pid| !alive(pid)));
        self.running = kept;
        gone.into_iter().map(|a| a.session).collect()
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

    /// The panel, top to bottom. Drawing and clicking share this. With nothing running, it
    /// says what this is for and how it goes, and a way to start comes straight after.
    fn lines(&self) -> Vec<Line> {
        // The pane's tab already says what this is.
        let mut lines = vec![Line::Gap];
        let silent = self.silent();
        let idle = self.running.is_empty() && silent.is_empty();
        if idle && self.docked {
            lines.extend((0..DOCKED_STEPS.len()).map(Line::Step));
        } else if idle {
            lines.push(Line::Section("HOW IT WORKS"));
            lines.extend((0..STEPS.len()).map(Line::Step));
        } else {
            lines.push(Line::Section("RUNNING"));
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
        if !self.worktrees.is_empty() {
            lines.push(Line::Gap);
            lines.push(Line::Section("WORKTREES"));
            lines.extend((0..self.worktrees.len()).map(Line::Tree));
        }
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
        if self.docked {
            return 1;
        }
        match line {
            Line::Reporting(_) | Line::Silent(_) | Line::Start(_) => 2,
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
                Line::Section(text) => {
                    grid.label_at(side as f32, side, y, text, dim, area.right(), SECTION_SIZE);
                }
                Line::Step(index) if self.docked => {
                    grid.label(side, y, DOCKED_STEPS[index], dim, area.right());
                }
                Line::Step(index) => {
                    let number = (index + 1).to_string();
                    grid.label(side, y, &number, dim, side + 2);
                    grid.label(side + 2, y, STEPS[index], theme.sidebar, area.right());
                }
                Line::Reporting(index) => {
                    let agent = &self.running[index];
                    let (mark, color) = match agent.state {
                        AgentState::NeedsYou => ("needs you", theme.attention),
                        AgentState::Working => ("working", theme.git_added),
                        AgentState::Idle | AgentState::Ended => ("idle", theme.gutter.fg),
                    };
                    let end = draw_agent(grid, (area, y), &agent.agent, (mark, color), theme);
                    if self.docked {
                        // On the agent's own row, between its name and how it is.
                        let room = area.right().saturating_sub(mark.len() as u16 + 4);
                        grid.label(end + 2, y, &agent.detail, dim, room);
                    } else {
                        grid.label(side + 2, y + 1, &agent.detail, dim, area.right());
                    }
                }
                Line::Silent(agent) => {
                    draw_agent(grid, (area, y), &agent, ("running", theme.git_added), theme);
                    if self.docked {
                        y += rows;
                        continue;
                    }
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
                    let (label, program) = self.installed[index];
                    grid.label(side + 2, y, label, theme.sidebar, area.right());
                    if self.docked {
                        // What it runs, at the end of its one row.
                        let shown = if program == SHELL.1 { "shell" } else { program };
                        let right = area.right().saturating_sub(2);
                        grid.label_right(right, y, shown, dim, DETAIL_SIZE);
                        y += rows;
                        continue;
                    }
                    let note = if program == SHELL.1 {
                        "A plain terminal in this project"
                    } else {
                        "Starts here, working on this project"
                    };
                    let x = (side + 2) as f32;
                    grid.label_at(x, side + 2, y + 1, note, dim, area.right(), DETAIL_SIZE);
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

    /// Whether anyone has looked for what can be started yet.
    pub fn looked(&self) -> bool {
        !self.installed.is_empty()
    }

    /// Finds which agents can be started, which only changes when the panel opens.
    pub fn look_for_agents(&mut self) {
        let var = |name: &str| std::env::var(name).ok();
        self.installed = AGENTS
            .iter()
            .copied()
            .filter(|(_, program)| integrations::on_path(program, var))
            .chain([SHELL])
            .collect();
    }
}

fn draw_agent(
    grid: &mut Grid,
    (area, y): (Rect, u16),
    agent: &str,
    (mark, color): (&str, Rgb),
    theme: &Theme,
) -> u16 {
    let dot = Bounds::cells(area.x as f32 + 2.0, y as f32 + 0.32, 0.7, 0.36);
    grid.panel(dot, 18, Some(color), None);
    let name = Style {
        bold: true,
        ..theme.sidebar
    };
    let end = grid.label(area.x + 4, y, agent, name, area.right());
    let badge = Style {
        fg: color,
        ..theme.sidebar
    };
    grid.label_right(area.right().saturating_sub(2), y, mark, badge, DETAIL_SIZE);
    end
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
