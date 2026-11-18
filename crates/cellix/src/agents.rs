use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use cellix_agent::hook::PLAN_WAITING;
use cellix_agent::{AgentEvent, AgentState};
use std::time::Instant;

use cellix_render::{Bounds, Font, Grid, Icon, Measure, Paint, Rect, Rgb, Style};
use cellix_review::Worktree;

use crate::integrations;
use crate::theme::Theme;

const SECTION_SIZE: u8 = 80;
const DETAIL_SIZE: u8 = 85;
/// Room between a section's heading and how many are in it, in cells.
const COUNT_GAP: f32 = 0.6;
/// Room at a row's end for what the agent is doing, and between that and how long ago.
const STATE_WIDTH: f32 = 2.0;
const AGO_GAP: f32 = 0.8;
/// The chip that says an agent needs the person: its words, their size, its air either side
/// of them, its height and how strong its wash is.
const NEEDS_YOU: &str = "needs you";
const CHIP_SIZE: u8 = 80;
const CHIP_PAD: f32 = 0.7;
const CHIP_HIGH: f32 = 0.66;
const CHIP_WASH: f32 = 0.16;
/// What a worktree's row says a click on it opens, once the pointer is on it.
const REVIEW: &str = "Review";
/// How tall the icon before that word is, in rows, and the room between the two, in cells.
const REVIEW_ICON: (f32, f32) = (0.6, 0.4);
/// Cells between that word and the count of changes after it.
const REVIEW_GAP: f32 = 1.0;
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
pub const AGENTS: [(&str, &str); 6] = [
    ("Claude Code", "claude"),
    ("Codex", "codex"),
    ("Gemini CLI", "gemini"),
    ("OpenCode", "opencode"),
    ("Copilot CLI", "copilot"),
    ("Cursor CLI", "cursor-agent"),
];
/// The name of an agent's tab for the name its hooks give it: Gemini CLI's say "Gemini",
/// Copilot CLI's "Copilot" and Cursor's "Cursor".
pub fn tab_name(agent: &str) -> &str {
    match agent {
        "Gemini" => "Gemini CLI",
        "Copilot" => "Copilot CLI",
        "Cursor" => "Cursor CLI",
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

/// The mark that stands for a kind of agent beside its name, as its tab has it, and the one
/// character the cell tier shows for it.
pub fn mark_of(kind: &str) -> (Icon, &'static str) {
    let icon = crate::dock::program_icon(display_name(kind), false, false);
    (icon, crate::toolbar::fallback(icon))
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

/// What gives an agent its first prompt as it starts, put after its program. The agent takes
/// it once it's ready: after what it asks first, such as whether to trust the folder, which
/// is the person's to answer and never Cellix's. After `--`, so a prompt that starts with a
/// `-`, as a list does, isn't read as an option. None for an agent Cellix doesn't know how to
/// tell: one added to `AGENTS` says here how it takes a prompt.
pub fn first_prompt(program: &str, prompt: &str) -> Option<Vec<String>> {
    match program {
        "claude" | "codex" => Some(vec!["--".to_string(), prompt.to_string()]),
        "gemini" => Some(vec![format!("--prompt-interactive={prompt}")]),
        _ => None,
    }
}

/// What resumes an agent's own session, put after its program: the agents that can be told
/// which session to pick up.
pub fn resume_args(name: &str, session: &str) -> Option<Vec<String>> {
    let words: &[&str] = match name {
        "Claude Code" | "Copilot CLI" | "Cursor CLI" => &["--resume"],
        "Codex" => &["resume"],
        "OpenCode" => &["--session"],
        _ => return None,
    };
    let mut args: Vec<String> = words.iter().map(|w| w.to_string()).collect();
    args.push(session.to_string());
    Some(args)
}

/// The mark an agent's rows carry, and its colour: the one its tab has in the side pane,
/// Cellix's own and never a vendor's logo. A terminal is its prompt.
pub fn mark(program: &str, theme: &Theme) -> (Icon, Rgb) {
    let shell = program == SHELL.1;
    let icon = crate::dock::program_icon(display_name(program), shell, false);
    (icon, crate::dock::icon_colour(icon, theme))
}

/// What a click in the panel lands on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Hit {
    /// The program of an agent to start.
    Start(&'static str),
    Worktree(PathBuf),
    /// An agent's row, by its session's id.
    Agent(String),
}

/// How much an agent wants the person, which is the order the list keeps.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Urgency {
    NeedsYou,
    /// Finished while nobody was looking at it, until somebody does.
    Done,
    Working,
    Idle,
}

impl Urgency {
    const ALL: [Urgency; 4] = [Self::NeedsYou, Self::Done, Self::Working, Self::Idle];

    fn heading(self) -> &'static str {
        match self {
            Self::NeedsYou => "NEEDS YOU",
            Self::Done => "DONE",
            Self::Working => "WORKING",
            Self::Idle => "IDLE",
        }
    }
}

/// One agent in the list: its program's mark, its name, how long since what it's doing
/// changed, and what that is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub icon: Icon,
    pub name: String,
    pub urgency: Urgency,
    /// When what it's doing last changed, if it ever said.
    pub since: Option<Instant>,
    /// What it's asking, on the row under it while it needs you.
    pub detail: String,
    /// Its own session's id, which a helper names as its parent.
    pub session: String,
    /// The agent that started it, by session id: its row stands under that one's, in that
    /// one's section, however it is itself. None for one the person started.
    pub parent: Option<String>,
}

/// What hooks say that's no more than the state a row's mark already shows.
const PLAIN: [&str; 7] = [
    "",
    "Working",
    "Finished",
    "Ready",
    "Idle",
    "Waiting for you",
    "Ended",
];

impl Row {
    /// Whether it has something to say on the row under it: what it's asking, while it needs
    /// you, and otherwise what it says that's more than its state's own word, as a program's
    /// "Working · 40%".
    pub fn tells(&self) -> bool {
        self.urgency == Urgency::NeedsYou || !PLAIN.contains(&self.detail.as_str())
    }
}

/// The rows in the list's order: by urgency, the most first, and in the order they came
/// within that; each row's helpers straight under it, deeper by one, in the same order.
/// Returns each row with how deep it stands.
pub fn ordered(rows: &[Row]) -> Vec<(&Row, u8)> {
    let known = |id: &str| rows.iter().any(|row| row.session == id);
    let mut tops: Vec<&Row> = rows
        .iter()
        .filter(|row| row.parent.as_deref().is_none_or(|parent| !known(parent)))
        .collect();
    tops.sort_by_key(|row| row.urgency);
    let mut out = Vec::new();
    fn under<'a>(row: &'a Row, depth: u8, rows: &'a [Row], out: &mut Vec<(&'a Row, u8)>) {
        out.push((row, depth));
        if depth > 8 {
            return;
        }
        for child in rows
            .iter()
            .filter(|c| c.parent.as_deref() == Some(&row.session))
        {
            under(child, depth + 1, rows, out);
        }
    }
    for row in tops {
        under(row, 0, rows, &mut out);
    }
    out
}

/// How long ago, as the list says it: under a minute, then minutes, hours and days.
pub fn ago(at: Instant) -> String {
    let seconds = at.elapsed().as_secs();
    match seconds {
        0..60 => "<1m".to_string(),
        60..3600 => format!("{}m", seconds / 60),
        3600..86_400 => format!("{}h", seconds / 3600),
        _ => format!("{}d", seconds / 86_400),
    }
}

/// A row or two of the panel.
enum Line {
    Section(&'static str),
    /// A section of agents, by how much they want the person, with how many are in it.
    Group(Urgency, usize),
    Gap,
    /// The note that takes the running list's place.
    Step(usize),
    /// An index into the rows in their order.
    Agent(usize),
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
    /// Tests say which agents are installed, rather than whatever this machine has.
    #[cfg(test)]
    pretend: Option<Vec<&'static str>>,
    /// When an agent here last finished its turn, until one starts working again.
    pub finished: Option<std::time::Instant>,
    /// Docked beside a chat: short how-to, and one row for each agent.
    pub docked: bool,
    /// When what each agent is doing last changed, by its session's id.
    pub since: std::collections::HashMap<String, Instant>,
    /// Agents that finished a turn nobody has looked at since, by their session's id.
    pub unseen: std::collections::HashSet<String>,
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
            let named = AGENTS.iter().find(|(_, program)| *program == command);
            found.push(named.map_or_else(|| command.to_string(), |(name, _)| name.to_string()));
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
        // How long ago counts from when what it's doing last changed, not from each word.
        let before = self.running.iter().find(|a| same(a));
        let changed = before.is_none_or(|a| (a.state, &a.detail) != (event.state, &event.detail));
        if changed {
            self.since.insert(event.session.clone(), Instant::now());
        }
        match event.state {
            AgentState::Idle if event.detail == "Finished" => {
                self.unseen.insert(event.session.clone());
            }
            AgentState::Idle => {}
            AgentState::Working | AgentState::NeedsYou => {
                self.unseen.remove(&event.session);
            }
            AgentState::Ended => {
                self.unseen.remove(&event.session);
                self.since.remove(&event.session);
            }
        }
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
        // A program that only says how it is has no hooks to hand it a comment.
        let hooked = self.running.iter().find(|a| !crate::program::reported(a));
        let reporting = hooked.map(|a| a.agent.clone());
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
                .args(["-c", "gemini", "-c", "opencode", "-c", "copilot"])
                .args(["-c", "cursor-agent", "-Fcn"])
                .output();
            if let (Ok(output), Ok(mut seen)) = (output, seen.lock()) {
                *seen = agents_in(&String::from_utf8_lossy(&output.stdout), &root);
            }
        });
    }

    /// What the list shows that can change between events, as a number for the tick: those
    /// running, those found by looking, the worktrees and the agents installed.
    pub fn mark(&self) -> (usize, usize, usize, usize) {
        let seen = self.seen.lock().map_or(0, |s| s.len());
        let worktrees = self.worktrees.len();
        (self.running.len(), seen, worktrees, self.installed.len())
    }

    /// Agents found running that the plugin isn't already reporting.
    /// Agents found running that the plugin isn't reporting, as rows of the list: idle, as
    /// far as anyone can tell, with no time since anything.
    pub fn silent_rows(&self) -> Vec<Row> {
        let rows = self.silent().into_iter().map(|name| Row {
            icon: crate::dock::program_icon(&name, false, false),
            name: name.clone(),
            urgency: Urgency::Idle,
            since: None,
            detail: String::new(),
            session: name,
            parent: None,
        });
        rows.collect()
    }

    fn silent(&self) -> Vec<String> {
        let seen = self.seen.lock().map(|s| s.clone()).unwrap_or_default();
        let hooked = self.running.iter().filter(|a| !crate::program::reported(a));
        seen.into_iter().skip(hooked.count()).collect()
    }

    /// The panel, top to bottom. Drawing and clicking share this. With nothing running, it
    /// says what this is for and how it goes; either way a way to start comes next.
    fn lines(&self, rows: &[Row]) -> Vec<Line> {
        // The pane's tab already says what this is.
        let mut lines = vec![Line::Gap];
        if rows.is_empty() && self.docked {
            lines.extend((0..DOCKED_STEPS.len()).map(Line::Step));
        } else if rows.is_empty() {
            lines.push(Line::Section("HOW IT WORKS"));
            lines.extend((0..STEPS.len()).map(Line::Step));
        }
        // Who wants the person most comes first, a section each with how many are in it, and
        // a section with nobody in it isn't there. A helper stands in its agent's section.
        let order = ordered(rows);
        let mut sections = Vec::with_capacity(order.len());
        let mut current = Urgency::Idle;
        for (row, depth) in &order {
            if *depth == 0 {
                current = row.urgency;
            }
            sections.push(current);
        }
        for urgency in Urgency::ALL {
            let section: Vec<usize> = (0..order.len())
                .filter(|at| sections[*at] == urgency)
                .collect();
            if section.is_empty() {
                continue;
            }
            if lines.len() > 1 {
                lines.push(Line::Gap);
            }
            lines.push(Line::Group(urgency, section.len()));
            lines.extend(section.into_iter().map(Line::Agent));
        }
        // Starting one comes before the worktrees, which can be a long list to scroll past.
        // A terminal isn't an agent, so it stands apart from them, under a name that fits both.
        if !self.installed.is_empty() {
            lines.push(Line::Gap);
            lines.push(Line::Section("START"));
            for (index, (_, program)) in self.installed.iter().enumerate() {
                if *program == SHELL.1 && index > 0 && !self.docked {
                    lines.push(Line::Gap);
                }
                lines.push(Line::Start(index));
            }
        }
        if !self.worktrees.is_empty() {
            lines.push(Line::Gap);
            lines.push(Line::Section("WORKTREES"));
            lines.extend((0..self.worktrees.len()).map(Line::Tree));
        }
        lines
    }

    /// The furthest the panel scrolls, in lines of it: where the last of them just fits in
    /// `height` rows.
    pub fn last_top(&self, height: u16, agents: &[Row]) -> usize {
        let lines = self.lines(agents);
        let order = ordered(agents);
        let mut room = height;
        let mut fit = 0;
        for line in lines.iter().rev() {
            let rows = self.height(line, &order);
            if rows > room {
                break;
            }
            room -= rows;
            fit += 1;
        }
        lines.len() - fit
    }

    /// What's at row `y`, with the first `scrolled` lines of the panel above its top.
    pub fn hit(&self, area: Rect, y: u16, scrolled: usize, agents: &[Row]) -> Option<Hit> {
        let mut top = area.y;
        let scrolled = scrolled.min(self.last_top(area.h, agents));
        let order = ordered(agents);
        for line in self.lines(agents).into_iter().skip(scrolled) {
            let rows = self.height(&line, &order);
            if y >= top && y < top + rows {
                return match line {
                    Line::Start(index) => Some(Hit::Start(self.installed[index].1)),
                    Line::Tree(index) => Some(Hit::Worktree(self.worktrees[index].path.clone())),
                    Line::Agent(at) => Some(Hit::Agent(order[at].0.session.clone())),
                    _ => None,
                };
            }
            top += rows;
        }
        None
    }

    fn height(&self, line: &Line, order: &[(&Row, u8)]) -> u16 {
        if self.docked {
            return 1;
        }
        match line {
            // What one that needs you is asking is on the row under it, and so is anything
            // else one says that's more than its state's own word.
            Line::Agent(at) if order[*at].0.tells() => 2,
            Line::Agent(_) => 1,
            Line::Start(_) => 2,
            // One worktree is just this folder, so it needs no second line to tell them apart.
            Line::Tree(_) if self.worktrees.len() > 1 => 2,
            _ => 1,
        }
    }

    /// Draws the panel with its first `scrolled` lines above the top.
    pub fn draw(
        &self,
        grid: &mut Grid,
        area: Rect,
        (scrolled, agents, edge): (usize, &[Row], f32),
        hover: Option<(u16, u16)>,
        (measure, theme): (&dyn Measure, &Theme),
    ) {
        grid.fill(area, theme.sidebar);
        // The band behind a pointed row, as far in from the sheet's line on both sides.
        let band = |row: Rect| crate::measures::band(row, edge, 0.055, measure);
        let scrolled = scrolled.min(self.last_top(area.h, agents));
        let order = ordered(agents);
        let dim = Style {
            fg: theme.gutter.fg,
            ..theme.sidebar
        };
        let side = area.x + 2;
        let mut y = area.y;
        for line in self.lines(agents).into_iter().skip(scrolled) {
            let rows = self.height(&line, &order);
            if y + rows > area.bottom() {
                break;
            }
            let row = Rect::new(area.x, y, area.w, rows);
            let hovered = hover.is_some_and(|h| row.contains(h.0, h.1));
            match line {
                Line::Gap => {}
                Line::Section(text) => {
                    // Small, bold and quiet, as a section's heading is in every panel.
                    let heading = Style { bold: true, ..dim };
                    grid.label_at(
                        side as f32,
                        side,
                        y,
                        text,
                        heading,
                        area.right(),
                        SECTION_SIZE,
                    );
                }
                Line::Step(index) if self.docked => {
                    grid.label(side, y, DOCKED_STEPS[index], dim, area.right());
                }
                Line::Step(index) => {
                    let number = (index + 1).to_string();
                    grid.label(side, y, &number, dim, side + 2);
                    grid.label(side + 2, y, STEPS[index], theme.sidebar, area.right());
                }
                Line::Group(urgency, count) => {
                    // Small, bold and quiet, as a section's heading is in every panel, with
                    // how many are in it after it, plain.
                    let heading = Style { bold: true, ..dim };
                    let text = urgency.heading();
                    grid.label_at(
                        side as f32,
                        side,
                        y,
                        text,
                        heading,
                        area.right(),
                        SECTION_SIZE,
                    );
                    let font = Font {
                        bold: true,
                        size: SECTION_SIZE,
                        ..Font::default()
                    };
                    let after = side as f32 + measure.width(text, font) + COUNT_GAP;
                    let cell = side + text.chars().count() as u16 + 1;
                    let count = count.to_string();
                    grid.label_at(after, cell, y, &count, dim, area.right(), SECTION_SIZE);
                }
                Line::Agent(at) => {
                    let (row, depth) = order[at];
                    if hovered {
                        let wash = band(Rect::new(area.x, y, area.w, 1));
                        grid.panel(wash, 18, Some(theme.row_hover), None);
                    }
                    draw_row(grid, (area, y), (row, depth), (measure, theme));
                    if rows > 1 {
                        let x = side + 2 + 2 * u16::from(depth);
                        let at = x as f32;
                        grid.label_at(at, x, y + 1, &row.detail, dim, area.right(), DETAIL_SIZE);
                    }
                }
                Line::Tree(index) => {
                    if hovered {
                        let wash = band(row);
                        grid.panel(wash, 18, Some(theme.row_hover), None);
                    }
                    let row = (y, rows, hovered);
                    self.draw_worktree(grid, area, row, index, (measure, theme));
                }
                Line::Start(index) => {
                    if hovered {
                        let wash = band(row);
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

    /// One worktree: its branch, how much has changed in it, and under the pointer what a
    /// click on it opens, before that count.
    fn draw_worktree(
        &self,
        grid: &mut Grid,
        area: Rect,
        (y, rows, hovered): (u16, u16, bool),
        index: usize,
        (measure, theme): (&dyn Measure, &Theme),
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
        let right = area.right().saturating_sub(2);
        // Where what stands at the row's end starts, which the branch's name stops short of.
        let mut end = right as f32;
        if worktree.changes > 0 {
            let count = format!("{} changed", worktree.changes);
            let badge = Style {
                fg: theme.git_modified,
                ..theme.sidebar
            };
            grid.label_right(right, y, &count, badge, DETAIL_SIZE);
            let font = Font {
                size: DETAIL_SIZE,
                ..Font::default()
            };
            end -= measure.width(&count, font) + REVIEW_GAP;
        }
        if hovered {
            let ground = theme.sidebar;
            let (tall, gap) = REVIEW_ICON;
            let words = end - measure.width(REVIEW, Font::default());
            let icon = words - gap - tall * 2.0;
            if icon > side as f32 + 4.0 {
                let top = y as f32 + (1.0 - tall) / 2.0;
                grid.paint.push(Paint::Icon {
                    bounds: Bounds::cells(icon, top, tall * 2.0, tall),
                    icon: Icon::Diff,
                    color: theme.sidebar.fg,
                });
                let cell = words.floor() as u16;
                let shown = Style {
                    painted: true,
                    ..ground
                };
                grid.put_str(cell.saturating_sub(2), y, "±", shown, cell);
                grid.label_at(words, cell, y, REVIEW, ground, right, 100);
                end = icon;
            }
        }
        let stop = (end.floor() as u16).saturating_sub(1).max(side);
        grid.label(side, y, &title, theme.sidebar, stop);
        if rows > 1 {
            let place = if detached {
                "Detached HEAD".to_string()
            } else {
                folder.unwrap_or_default()
            };
            let x = side as f32;
            grid.label_at(x, side, y + 1, &place, dim, area.right(), DETAIL_SIZE);
        }
    }

    /// Whether anyone has looked for what can be started yet.
    pub fn looked(&self) -> bool {
        !self.installed.is_empty()
    }

    /// Finds which agents can be started, which only changes when the panel opens.
    pub fn look_for_agents(&mut self) {
        let var = |name: &str| std::env::var(name).ok();
        #[cfg(test)]
        let pretend = self.pretend.clone();
        let found = |program: &'static str| {
            #[cfg(test)]
            if let Some(pretend) = &pretend {
                return pretend.contains(&program);
            }
            integrations::on_path(program, var)
        };
        self.installed = AGENTS
            .iter()
            .copied()
            .filter(|(_, program)| found(program))
            .chain([SHELL])
            .collect();
    }

    /// The programs of the agents installed here, as last looked for; not a terminal's.
    pub fn installed_programs(&self) -> Vec<&'static str> {
        let agents = self.installed.iter().map(|(_, program)| *program);
        agents.filter(|program| *program != SHELL.1).collect()
    }

    /// Says which agents count as installed from now on, whatever this machine has.
    #[cfg(test)]
    pub fn pretend_installed(&mut self, programs: &[&'static str]) {
        self.pretend = Some(programs.to_vec());
        self.look_for_agents();
    }
}

/// One agent's row: its program's mark, its name, and at the row's end how long since what
/// it's doing changed and what that is: turning while it works, a chip while it needs you, a
/// green dot when it's done and nobody has looked, and nothing while it's idle. A helper
/// stands two columns in for each agent over it, with a line bent round from that one's mark.
fn draw_row(
    grid: &mut Grid,
    (area, y): (Rect, u16),
    (row, depth): (&Row, u8),
    (measure, theme): (&dyn Measure, &Theme),
) {
    let side = area.x + 2 + 2 * u16::from(depth);
    if depth > 0 {
        let elbow = side.saturating_sub(2);
        if measure.in_cells() {
            let line = Style {
                fg: theme.border,
                ..theme.sidebar
            };
            grid.put_str(elbow, y, "└", line, elbow + 1);
        } else {
            let x = elbow as f32 + 0.9;
            grid.hairline(Bounds::cells(x, y as f32 - 0.3, 0.0, 0.8), theme.border);
            grid.hairline(Bounds::cells(x, y as f32 + 0.5, 0.8, 0.0), theme.border);
        }
    }
    let color = crate::dock::icon_colour(row.icon, theme);
    let fallback = crate::toolbar::fallback(row.icon);
    grid.glyph(side as f32, y, row.icon, color, fallback, theme.sidebar);
    let right = area.right().saturating_sub(2);
    // The state at the very end, and how long ago before it.
    let state = right as f32 - STATE_WIDTH;
    match row.urgency {
        Urgency::Working => {
            let accent = theme.accent;
            grid.spinner(
                state + 0.3,
                y,
                crate::dock::MARK_SIZE,
                accent,
                theme.sidebar,
            );
        }
        Urgency::Done => {
            let green = theme.git_added;
            grid.dot(state + 0.3, y, crate::dock::MARK_SIZE, green, theme.sidebar);
        }
        Urgency::NeedsYou | Urgency::Idle => {}
    }
    let mut end = state - AGO_GAP;
    if row.urgency == Urgency::NeedsYou {
        end = draw_needs_you(grid, (right, y), (measure, theme)) - AGO_GAP;
    }
    let dim = Style {
        fg: theme.gutter.fg,
        ..theme.sidebar
    };
    if let Some(since) = row.since {
        let said = ago(since);
        let font = Font {
            size: DETAIL_SIZE,
            ..Font::default()
        };
        let wide = measure.width(&said, font);
        grid.label_right(end.floor() as u16, y, &said, dim, DETAIL_SIZE);
        end -= wide + AGO_GAP;
    }
    // Whoever is doing nothing is quieter, and not bold.
    let name = if row.urgency == Urgency::Idle {
        Style {
            fg: theme.tab.fg,
            ..theme.sidebar
        }
    } else {
        Style {
            bold: true,
            ..theme.sidebar
        }
    };
    let stop = (end.floor() as u16).max(side + 3);
    let text = crate::context::elide(&row.name, f32::from(stop - side - 2), measure);
    let after = grid.label(side + 2, y, &text, name, stop);
    // A helper says it is one, quietly, after its name, where there's room.
    if depth > 0 && text == row.name {
        let font = Font {
            size: DETAIL_SIZE,
            ..Font::default()
        };
        let named = (side + 2) as f32
            + measure.width(
                &text,
                Font {
                    bold: name.bold,
                    ..Font::default()
                },
            );
        let x = named + HELPER_GAP;
        if x + measure.width(HELPER, font) < f32::from(stop) {
            grid.label_at(x, after + 1, y, HELPER, dim, stop, DETAIL_SIZE);
        }
    }
}

/// The word after a helper's name, and the room before it, in cells.
const HELPER: &str = "helper";
const HELPER_GAP: f32 = 1.0;

/// The chip that says an agent needs the person, at the end of its row, in the attention
/// colour on a wash of it. Returns where it starts.
fn draw_needs_you(
    grid: &mut Grid,
    (right, y): (u16, u16),
    (measure, theme): (&dyn Measure, &Theme),
) -> f32 {
    let font = Font {
        size: CHIP_SIZE,
        bold: true,
        ..Font::default()
    };
    let words = measure.width(NEEDS_YOU, font);
    let wide = words + CHIP_PAD * 2.0;
    let left = f32::from(right) - wide;
    let wash = mix(theme.attention, theme.sidebar.bg, CHIP_WASH);
    let top = y as f32 + (1.0 - CHIP_HIGH) / 2.0;
    if !measure.in_cells() {
        grid.panel(
            Bounds::cells(left, top, wide, CHIP_HIGH),
            22,
            Some(wash),
            None,
        );
    }
    let style = Style {
        fg: theme.attention,
        bold: true,
        ..theme.sidebar
    };
    let cell = right.saturating_sub(NEEDS_YOU.len() as u16);
    grid.label_at(left + CHIP_PAD, cell, y, NEEDS_YOU, style, right, CHIP_SIZE);
    if measure.in_cells() {
        cell as f32
    } else {
        left
    }
}

/// `color` over `ground` at `share` of its strength.
fn mix(color: Rgb, ground: Rgb, share: f32) -> Rgb {
    let one = |a: u8, b: u8| (f32::from(a) * share + f32::from(b) * (1.0 - share)).round() as u8;
    Rgb(
        one(color.0, ground.0),
        one(color.1, ground.1),
        one(color.2, ground.2),
    )
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
        let at = |hit: &Hit| (0..40).find(|y| agents.hit(area, *y, 0, &[]).as_ref() == Some(hit));
        let start = at(&Hit::Start("codex")).unwrap();
        let tree = at(&Hit::Worktree(PathBuf::from("/work"))).unwrap();
        assert!(
            start < tree,
            "with nothing running, starting one comes first"
        );
        assert_eq!(agents.hit(area, 0, 0, &[]), None, "the title does nothing");
    }

    #[test]
    fn starting_an_agent_stays_in_reach_beside_many_worktrees() {
        let running = vec!["Claude Code".to_string(); 6];
        let agents = Agents {
            installed: vec![("Claude Code", "claude"), SHELL],
            seen: Arc::new(Mutex::new(running)),
            worktrees: (0..11)
                .map(|i| Worktree {
                    path: PathBuf::from(format!("/work/tree-{i}")),
                    branch: format!("claude/tree-{i}"),
                    changes: 0,
                })
                .collect(),
            ..Agents::default()
        };
        // As tall as a side pane in a laptop's window: the worktrees don't all fit.
        let area = Rect::new(0, 0, 40, 36);
        let rows = agents.silent_rows();
        let found =
            (0..area.h).find(|y| agents.hit(area, *y, 0, &rows) == Some(Hit::Start("claude")));
        assert!(found.is_some(), "Claude Code can be started");
        let theme = Theme::default();
        let look: (&dyn Measure, &Theme) = (&cellix_render::CellMeasure, &theme);
        let mut grid = Grid::new(40, 36, Style::default());
        agents.draw(&mut grid, area, (0, &rows, 0.0), None, look);
        assert!(grid.text().contains("START"), "{}", grid.text());

        // The worktrees that don't fit are a scroll away, and it stops at the last of them.
        assert!(!grid.text().contains("claude/tree-10"), "{}", grid.text());
        let last = agents.last_top(area.h, &rows);
        assert!(last > 0);
        let mut grid = Grid::new(40, 36, Style::default());
        agents.draw(&mut grid, area, (usize::MAX, &rows, 0.0), None, look);
        assert!(grid.text().contains("claude/tree-10"), "{}", grid.text());
        let tree = Hit::Worktree(PathBuf::from("/work/tree-10"));
        let found = (0..area.h).find(|y| agents.hit(area, *y, last, &rows) == Some(tree.clone()));
        assert!(found.is_some(), "and the last one can be clicked");
    }

    #[test]
    fn the_terminal_stands_apart_from_the_agents() {
        let agents = Agents {
            installed: vec![("Claude Code", "claude"), ("Codex", "codex"), SHELL],
            ..Agents::default()
        };
        let area = Rect::new(0, 0, 40, 30);
        let rows = |program: &'static str| -> Vec<u16> {
            let hit = Some(Hit::Start(program));
            (0..area.h)
                .filter(|y| agents.hit(area, *y, 0, &[]) == hit)
                .collect()
        };
        let (claude, codex, shell) = (rows("claude"), rows("codex"), rows(SHELL.1));
        assert_eq!(codex[0], claude[1] + 1, "one agent right under the other");
        assert_eq!(
            shell[0],
            codex[1] + 2,
            "and a row of air before the terminal"
        );
        assert_eq!(agents.hit(area, codex[1] + 1, 0, &[]), None);

        // With no agent installed there's nothing to stand apart from.
        let alone = Agents {
            installed: vec![SHELL],
            ..Agents::default()
        };
        let theme = Theme::default();
        let look: (&dyn Measure, &Theme) = (&cellix_render::CellMeasure, &theme);
        let mut grid = Grid::new(40, 30, Style::default());
        alone.draw(&mut grid, area, (0, &[], 0.0), None, look);
        let text = grid.text();
        let lines: Vec<&str> = text.lines().collect();
        let start = lines.iter().position(|l| l.contains("START")).unwrap();
        assert!(lines[start + 1].contains("Terminal"), "{text}");
    }

    #[test]
    fn a_folder_with_a_similar_name_isnt_this_one() {
        assert!(agents_in(OUTPUT, Path::new("/work/proj")).is_empty());
    }
}
