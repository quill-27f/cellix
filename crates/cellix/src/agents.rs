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
/// Room at a row's end for a turning or done mark, and between what's doing and how long ago.
const STATE_WIDTH: f32 = 2.0;
const AGO_GAP: f32 = 0.8;
/// How far the mark of the row a helper stands under reaches down into its own row, in rows:
/// its line bent round starts 8 px under that, where it ran into the mark's foot.
const ELBOW_TOP: f32 = 0.04;
/// How far short of a helper's mark that line ends, in columns, about 9 px.
const ELBOW_END: f32 = 0.55;
/// How strong the line is, toward the words' colour: about #45474a on the dark sheet, where
/// the sheet's own hairline, 1.3 to 1, could hardly be seen.
const ELBOW_SHARE: f32 = 0.21;
/// The chip that says an agent needs the person: its words, their size, its air either side
/// of them, its height and how strong its wash is.
const NEEDS_YOU: &str = "needs you";
const CHIP_SIZE: u8 = 80;
const CHIP_PAD: f32 = 0.7;
/// A chip's share of the line, which a taller row makes the design's chip, 20/28 of the row
/// (`cellix_render::stood`), with room under its words' tails: at two thirds of the line it
/// kept the line's 24 px, and the "y" of "you" reached a pixel short of its foot (0.11.0's
/// final review, m2).
const CHIP_HIGH: f32 = 0.72;
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
pub const AGENTS: [(&str, &str); 7] = [
    ("Claude Code", "claude"),
    ("Codex", "codex"),
    ("Gemini CLI", "gemini"),
    ("OpenCode", "opencode"),
    ("Copilot CLI", "copilot"),
    ("Cursor CLI", "cursor-agent"),
    ("Kiro CLI", "kiro-cli"),
];
/// The name of an agent's tab for the name its hooks give it: Gemini CLI's say "Gemini",
/// Copilot CLI's "Copilot", Cursor's "Cursor" and Kiro CLI's "Kiro".
pub fn tab_name(agent: &str) -> &str {
    match agent {
        "Gemini" => "Gemini CLI",
        "Copilot" => "Copilot CLI",
        "Cursor" => "Cursor CLI",
        "Kiro" => "Kiro CLI",
        other => other,
    }
}

/// The agent a running process is, by its command's name: one of `AGENTS` by its program, or
/// Kiro CLI's chat, which may run as a process of its own (`kiro-cli-chat`) under `kiro-cli`.
/// `lsof` cuts a name to nine letters, which leaves that one `kiro-cli-`.
pub fn named_by_command(command: &str) -> Option<&'static str> {
    let named = AGENTS.iter().find(|(_, program)| *program == command);
    named
        .map(|(name, _)| *name)
        .or_else(|| command.starts_with("kiro-cli-").then_some("Kiro CLI"))
}

/// What an agent's program is given before anything else it's started with: Kiro CLI's chat
/// is a command of its program, `kiro-cli chat`.
pub fn launch_args(program: &str) -> &'static [&'static str] {
    match program {
        "kiro-cli" => &["chat"],
        _ => &[],
    }
}

/// What the panel calls a plain shell, and what stands for it in `Hit::Start`.
pub const SHELL: (&str, &str) = ("Terminal", "shell");

/// The program a Terminal tab runs: the user's shell, or on Windows PowerShell, which every
/// Windows has. There `SHELL` is unset, or a Unix path Git Bash left behind.
pub fn shell() -> String {
    shell_for(|name| std::env::var(name).ok(), cfg!(test))
}

/// The same, from `var`. A test's Terminal is always `/bin/sh`, never the person's own shell:
/// a test's zsh read its owner's startup files, one of which ran `gh completion`, which never
/// returned, and the whole gate hung on it.
pub fn shell_for(var: impl Fn(&str) -> Option<String>, testing: bool) -> String {
    if cfg!(windows) {
        "powershell.exe".to_string()
    } else if testing {
        "/bin/sh".to_string()
    } else {
        var("SHELL").unwrap_or_else(|| "sh".to_string())
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
        // Kiro CLI's chat takes its first question as its argument, `kiro-cli chat [INPUT]`.
        "claude" | "codex" | "kiro-cli" => Some(vec!["--".to_string(), prompt.to_string()]),
        "gemini" => Some(vec![format!("--prompt-interactive={prompt}")]),
        _ => None,
    }
}

/// What resumes an agent's own session, put after its program and what it's always given
/// (`launch_args`): the agents that can be told which session to pick up.
pub fn resume_args(name: &str, session: &str) -> Option<Vec<String>> {
    let words: &[&str] = match name {
        "Claude Code" | "Copilot CLI" | "Cursor CLI" => &["--resume"],
        "Codex" => &["resume"],
        "OpenCode" => &["--session"],
        // `kiro-cli chat --resume-id <id>`, which Kiro CLI prints as it exits.
        "Kiro CLI" => &["--resume-id"],
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
    /// The command of an agent to start.
    Start(String),
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

/// The agent a row's agent was started by, where that one is in the list too.
pub fn parent_of<'a>(row: &Row, rows: &'a [Row]) -> Option<&'a Row> {
    let parent = row.parent.as_deref()?;
    rows.iter().find(|other| other.session == parent)
}

/// Whether a helper stands under the agent that started it: always, where that one is in the
/// list too, with its own state's mark at its row's end, as peek's header says it. Stood in a
/// section of its own whenever it was doing something else than its agent, a helper read as
/// one more agent, and the list no longer showed what started it (0.11.0's release review,
/// 172 to 174).
fn nested(row: &Row, rows: &[Row]) -> bool {
    parent_of(row, rows).is_some()
}

/// The section an agent and its helpers stand in: its own state's, or where a helper of it
/// needs the person, "NEEDS YOU", so what asks is never under a quieter heading.
fn family_urgency(row: &Row, rows: &[Row], depth: u8) -> Urgency {
    let children = rows
        .iter()
        .filter(|c| c.parent.as_deref() == Some(&row.session) && nested(c, rows));
    let asking = depth < 8
        && children
            .into_iter()
            .any(|c| family_urgency(c, rows, depth + 1) == Urgency::NeedsYou);
    if asking {
        Urgency::NeedsYou
    } else {
        row.urgency
    }
}

/// The rows in the list's order: by urgency, the most first, and in the order they came
/// within that; each row's helpers straight under it, deeper by one, in the same order, so a
/// section holds agents and their helpers stand with them. Returns each row with how deep it
/// stands.
pub fn ordered(rows: &[Row]) -> Vec<(&Row, u8)> {
    let mut tops: Vec<&Row> = rows.iter().filter(|row| !nested(row, rows)).collect();
    tops.sort_by_key(|row| family_urgency(row, rows, 0));
    let mut out = Vec::new();
    fn under<'a>(row: &'a Row, depth: u8, rows: &'a [Row], out: &mut Vec<(&'a Row, u8)>) {
        out.push((row, depth));
        if depth > 8 {
            return;
        }
        for child in rows
            .iter()
            .filter(|c| c.parent.as_deref() == Some(&row.session) && nested(c, rows))
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
    /// A row with a line across its middle, between the terminal and the agents to start, as
    /// the + menu has; a blank row on cells, which have no hairline.
    Rule,
    /// Nothing to start but the terminal: every agent is off or uninstalled, and this says
    /// where to turn them on.
    NoneOn,
}

/// An agent Cellix offers to start: one of `AGENTS`, or the person's own runner from the
/// `agents` setting, treated everywhere as the agent its `like` names (that one's marks,
/// flags and treatment). Cellix only ever runs the command; it never reads the runner.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Offered {
    /// What the panel and every list calls it: the built-in's name, or the name given.
    pub name: String,
    /// The command that runs it: a built-in's program, or the runner's command, a name on
    /// the PATH or a path.
    pub program: String,
    /// The program of `AGENTS` it works like: a built-in's own, a custom runner's `like`.
    /// Empty where the like isn't an agent Cellix knows, which is offered nowhere but
    /// Settings, which says what's wrong.
    pub like: String,
    /// One the person added, as against a built-in.
    pub custom: bool,
}

impl Offered {
    /// One of `AGENTS`, by its table row.
    pub fn built_in(name: &'static str, program: &'static str) -> Self {
        Self {
            name: name.to_string(),
            program: program.to_string(),
            like: program.to_string(),
            custom: false,
        }
    }
}

/// The program of the agent an `Offered` works like, for the parts keyed by program.
pub fn like_of(offered: &Offered) -> &str {
    if offered.like.is_empty() {
        &offered.program
    } else {
        &offered.like
    }
}

/// The Agents panel: who's running, who needs you, and where work is happening.
#[derive(Default)]
pub struct Agents {
    pub running: Vec<AgentEvent>,
    pub worktrees: Vec<Worktree>,
    /// Agents found running in this folder that haven't reported through a plugin.
    seen: Arc<Mutex<Vec<String>>>,
    /// The agents offered here, built-ins installed and the person's own runners, in order.
    installed: Vec<Offered>,
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
            let named = named_by_command(command);
            found.push(named.map_or_else(|| command.to_string(), str::to_string));
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

    /// How many agents' news says they're asking, with a tab here or not. What's counted for
    /// the person is `App::needing_you`, of agents with a tab here.
    #[cfg(test)]
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
                .args(["-c", "cursor-agent", "-c", "kiro-cli", "-Fcn"])
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

    /// What a look at the folder found running, as a test has it.
    #[cfg(test)]
    pub fn found_by_looking(&self, names: &[&str]) {
        if let Ok(mut seen) = self.seen.lock() {
            *seen = names.iter().map(ToString::to_string).collect();
        }
    }

    /// Agents found running here that the plugin isn't reporting, by name. They hear about
    /// comments, and aren't in the list: there's nothing of theirs here to bring forward.
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
        // Who wants the person most comes first, a section each with how many agents are in
        // it, and a section with nobody in it isn't there. A helper stands under its agent, in
        // that one's section, and isn't counted in it: "WORKING 2" over one agent and the
        // helper under it read as two agents at work.
        let order = ordered(rows);
        let mut sections = Vec::with_capacity(order.len());
        let mut current = Urgency::Idle;
        for (row, depth) in &order {
            if *depth == 0 {
                current = family_urgency(row, rows, 0);
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
            let agents = section.iter().filter(|at| order[**at].1 == 0).count();
            lines.push(Line::Group(urgency, agents));
            lines.extend(section.into_iter().map(Line::Agent));
        }
        // Starting one comes before the worktrees, which can be a long list to scroll past.
        // In the + menu's order, one row each: the terminal first, which isn't an agent, then
        // a rule, then the agents.
        if !self.installed.is_empty() {
            lines.push(Line::Gap);
            lines.push(Line::Section("START"));
            let shell = |(_, offered): &(usize, &Offered)| offered.program == SHELL.1;
            let (terminal, agents): (Vec<_>, Vec<_>) =
                self.installed.iter().enumerate().partition(shell);
            lines.extend(terminal.iter().map(|(index, _)| Line::Start(*index)));
            if agents.is_empty() {
                lines.push(Line::NoneOn);
            } else {
                if !terminal.is_empty() {
                    lines.push(Line::Rule);
                }
                lines.extend(agents.iter().map(|(index, _)| Line::Start(*index)));
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
                    Line::Start(index) => Some(Hit::Start(self.installed[index].program.clone())),
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
        let end = crate::measures::row_end(area, edge, measure);
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
                    // A helper stands under its agent while it's doing what that one does;
                    // doing something else, it stands in its own section and says who
                    // started it.
                    let from = parent_of(row, agents).filter(|_| depth == 0);
                    let from = from.map(|parent| parent.name.as_str());
                    draw_row(grid, (area, y, end), (row, depth, from), (measure, theme));
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
                    self.draw_worktree(grid, (area, end), row, index, (measure, theme));
                }
                Line::Start(index) => {
                    if hovered {
                        let wash = band(row);
                        grid.panel(wash, 18, Some(theme.row_hover), None);
                    }
                    // As the + menu has it: the program's own mark, its name, and what it
                    // runs at the end of its one row.
                    let offered = &self.installed[index];
                    let (icon, color) = mark(&offered.like, theme);
                    let fallback = crate::toolbar::fallback(icon);
                    grid.glyph(side as f32, y, icon, color, fallback, theme.sidebar);
                    let shown = if offered.program == SHELL.1 {
                        "shell"
                    } else {
                        &offered.program
                    };
                    let right = area.right().saturating_sub(2);
                    let font = Font {
                        size: DETAIL_SIZE,
                        ..Font::default()
                    };
                    let wide = measure.width(shown, font).ceil() as u16;
                    let stop = right.saturating_sub(wide + 1).max(side + 3);
                    grid.label(side + 2, y, &offered.name, theme.sidebar, stop);
                    grid.label_right_at(end, right, y, shown, dim, DETAIL_SIZE);
                }
                Line::NoneOn => {
                    // Small and quiet, as a hint is, naming where the switches are.
                    let hint = Style {
                        fg: crate::theme::mix(theme.gutter.fg, theme.sidebar.bg, 0.35),
                        ..theme.sidebar
                    };
                    grid.label(
                        side,
                        y,
                        "No agents turned on · Settings › Integrations",
                        hint,
                        area.right(),
                    );
                }
                Line::Rule => {
                    // In the middle of its own row, from where the rows' marks start to where
                    // what stands at their ends ends, so nothing comes within 8 px of it: it
                    // stopped 6 px short of the rows (the release review of 0.12.0, 14).
                    let across = end - f32::from(side);
                    let rule = Bounds::cells(side as f32, y as f32 + 0.5, across, 0.0);
                    grid.hairline(rule, theme.border);
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
        (area, row_end): (Rect, f32),
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
        let mut end = row_end;
        if worktree.changes > 0 {
            let count = format!("{} changed", worktree.changes);
            let badge = Style {
                fg: theme.git_modified,
                ..theme.sidebar
            };
            grid.label_right_at(row_end, right, y, &count, badge, DETAIL_SIZE);
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

    /// Finds which agents can be started, which only changes when the panel opens: the
    /// built-ins installed here, and the person's own runners whose command is there, each
    /// with the agent its `like` names; the terminal last. `disabled` turns agents off,
    /// a built-in's by its command, a runner's by its name.
    pub fn look_for_agents(
        &mut self,
        added: &[crate::settings::CustomAgent],
        disabled: &[String],
        path: &std::ffi::OsString,
    ) {
        let var = |name: &str| std::env::var(name).ok();
        #[cfg(test)]
        let pretend = self.pretend.clone();
        let found = |program: &str| {
            #[cfg(test)]
            if let Some(pretend) = &pretend {
                return pretend.contains(&program);
            }
            integrations::on_path(program, var)
        };
        let custom_found = |command: &str| {
            crate::ask::find(command, path).is_some() || Path::new(command).is_file()
        };
        let like_known = |like: &str| AGENTS.iter().any(|(_, program)| *program == like);
        let off = |name: &str| disabled.iter().any(|d| d == name);
        self.installed = AGENTS
            .iter()
            .copied()
            .filter(|(_, program)| found(program) && !off(program))
            .map(|(name, program)| Offered::built_in(name, program))
            .chain(added.iter().filter_map(|agent| {
                let known =
                    like_known(&agent.like) && custom_found(&agent.command) && !off(&agent.name);
                known.then(|| Offered {
                    name: if agent.name.is_empty() {
                        agent.command.clone()
                    } else {
                        agent.name.clone()
                    },
                    program: agent.command.clone(),
                    like: agent.like.clone(),
                    custom: true,
                })
            }))
            .chain([Offered::built_in(SHELL.0, SHELL.1)])
            .collect();
    }

    /// The agents offered here, as last looked for, with the terminal last.
    pub fn installed_agents(&self) -> &[Offered] {
        &self.installed
    }

    /// Says which agents count as installed from now on, whatever this machine has.
    #[cfg(test)]
    pub fn pretend_installed(&mut self, programs: &[&'static str]) {
        self.pretend = Some(programs.to_vec());
        self.look_for_agents(&[], &[], &std::ffi::OsString::new());
    }
}

/// One agent's row: its program's mark, its name, and at the row's end how long since what
/// it's doing changed and what that is: turning while it works, a chip while it needs you, a
/// green dot when it's done and nobody has looked, and nothing while it's idle. How long ago
/// stands in one column down the list, before room as wide as the "needs you" chip. A helper
/// stands two columns in for each agent over it, with a line bent round from that one's mark;
/// one in a section of its own says who started it, `from`. What stands at its end ends at
/// `row_end` (`row_end`).
fn draw_row(
    grid: &mut Grid,
    (area, y, row_end): (Rect, u16, f32),
    (row, depth, from): (&Row, u8, Option<&str>),
    (measure, theme): (&dyn Measure, &Theme),
) {
    let side = area.x + 2 + 2 * u16::from(depth);
    if depth > 0 {
        let elbow = side.saturating_sub(2);
        let line = mix(theme.text.fg, theme.sidebar.bg, ELBOW_SHARE);
        if measure.in_cells() {
            let line = Style {
                fg: line,
                ..theme.sidebar
            };
            grid.put_str(elbow, y, "└", line, elbow + 1);
        } else {
            let x = elbow as f32 + 0.9;
            let top = y as f32 - ELBOW_TOP;
            let down = Bounds::cells(x, top, 0.0, 0.5 + ELBOW_TOP);
            grid.hairline(down, line);
            let across = (side as f32 - ELBOW_END - x).max(0.2);
            grid.hairline(Bounds::cells(x, y as f32 + 0.5, across, 0.0), line);
        }
    }
    let color = crate::dock::icon_colour(row.icon, theme);
    // Any other program's square has its initial, the letter itself on cells.
    let initial = crate::dock::initial(row.icon, &row.name);
    let mut buffer = [0; 4];
    let fallback = match initial {
        Some(letter) => &*letter.encode_utf8(&mut buffer),
        None => crate::toolbar::fallback(row.icon),
    };
    grid.glyph(side as f32, y, row.icon, color, fallback, theme.sidebar);
    if let Some(letter) = initial.filter(|_| !measure.in_cells()) {
        // In the middle of the glyph, which stands about its start.
        let middle = side as f32 + 1.0;
        let size = cellix_render::GLYPH;
        crate::dock::initial_over(grid, (middle, y), (size, letter), theme.sidebar.bg);
    }
    let right = area.right().saturating_sub(2);
    // The state at the very end, and how long ago before it. Where there are pictures, its
    // ink ends where the row's end is, as the "needs you" chip does: the green dot ended
    // 11 px short of the chip, 36 px from the band under the pointer (m2).
    let size = crate::dock::MARK_SIZE;
    let square = size * crate::measures::rows_to_columns(measure) / measure.row_air();
    let at = |ink: f32| {
        if measure.in_cells() {
            right as f32 - STATE_WIDTH + 0.3
        } else {
            row_end - square * ink / 24.0
        }
    };
    match row.urgency {
        Urgency::Working => {
            // The ring's ink ends 1.4 of its square's 24 parts before the square does.
            grid.spinner(at(22.6), y, size, theme.accent, theme.sidebar);
        }
        Urgency::Done => {
            // A dot's ink is 12 of the 24, from the 6th.
            grid.dot(at(18.0), y, size, theme.git_added, theme.sidebar);
        }
        Urgency::NeedsYou | Urgency::Idle => {}
    }
    if row.urgency == Urgency::NeedsYou {
        draw_needs_you(grid, (row_end, right, y), (measure, theme));
    }
    // Every row keeps the chip's room for its state, an idle row's empty, so how long ago is
    // one column down the list: one kind of value to a column. At an idle row's end, its time
    // stood 139 px right of every other row's, in the column the states are in (the release
    // review of 0.12.0, m2-agents-urgency). Its letters keep the cells they had, before the
    // chip's own.
    let room = chip_width(measure) + AGO_GAP;
    let end = row_end - room;
    let mut cells = f32::from(right) - room;
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
        grid.label_right_at(end, cells.floor() as u16, y, &said, dim, DETAIL_SIZE);
        cells -= wide + AGO_GAP;
    }
    // A name is in the words' own colour, for a press on its row brings its tab forward: in
    // the quiet colour an idle one's read as the menus' "not installed", which does nothing.
    // How long ago is what's quiet. An agent doing something is bold; one idle, and a helper,
    // which stands under the agent it works for, aren't: a helper as bold as its agent read
    // as its equal (0.11.0's review of `next`, m2 and 172).
    let helper = depth > 0 || from.is_some();
    let name = Style {
        bold: row.urgency != Urgency::Idle && !helper,
        ..theme.sidebar
    };
    let stop = (cells.floor() as u16).max(side + 3);
    let text = crate::context::elide(&row.name, f32::from(stop - side - 2), measure);
    let after = grid.label(side + 2, y, &text, name, stop);
    // A helper says it is one, quietly, after its name, where there's room: under its agent
    // "helper", and in a section of its own who started it, as peek's header says.
    let said = match from {
        Some(parent) => format!("from {parent}"),
        None => HELPER.to_string(),
    };
    if (depth > 0 || from.is_some()) && text == row.name {
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
        if x + measure.width(&said, font) < f32::from(stop) {
            grid.label_at(x, after + 1, y, &said, dim, stop, DETAIL_SIZE);
        }
    }
}

/// The word after a helper's name, and the room before it, in cells.
const HELPER: &str = "helper";
const HELPER_GAP: f32 = 1.0;

/// How wide the chip that says an agent needs the person is, in cells: its words and their
/// air, or on cells its words alone.
fn chip_width(measure: &dyn Measure) -> f32 {
    if measure.in_cells() {
        return NEEDS_YOU.len() as f32;
    }
    let font = Font {
        size: CHIP_SIZE,
        bold: true,
        ..Font::default()
    };
    measure.width(NEEDS_YOU, font) + CHIP_PAD * 2.0
}

/// The chip that says an agent needs the person, at the end of its row, in the attention
/// colour on a wash of it.
fn draw_needs_you(
    grid: &mut Grid,
    (end, right, y): (f32, u16, u16),
    (measure, theme): (&dyn Measure, &Theme),
) {
    let wide = chip_width(measure);
    let left = end - wide;
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
    // Its words in the attention colour, deepened where that reads under 4.5 to 1 on the
    // wash, as it did in the light theme (4.28).
    let style = Style {
        fg: crate::theme::readable(theme.attention, wash, 4.5),
        bold: true,
        ..theme.sidebar
    };
    let cell = right.saturating_sub(NEEDS_YOU.len() as u16);
    let from = grid.painted_so_far();
    grid.label_at(left + CHIP_PAD, cell, y, NEEDS_YOU, style, right, CHIP_SIZE);
    // Its ink in the chip's middle, the "d" as far from its top as the "y"'s tail from its
    // foot: words stand with their capitals in the middle of their row, and their tails
    // reach further down than their heads reach up. In the chip's middle they had 11 px over
    // them and 7 under (0.11.0's final review, T9, m2).
    grid.lift_since(
        from,
        crate::measures::FIELD_LIFT / measure.row_air().max(1.0),
    );
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

    /// Kiro CLI is known by its command, and by its chat's own process, which `lsof` cuts to
    /// nine letters; it starts as `kiro-cli chat` and resumes a session by its id after that,
    /// as Kiro CLI says to as it exits; its hooks' "Kiro" is its tab's "Kiro CLI".
    #[test]
    fn kiro_cli_is_known_by_its_command_and_resumes_its_chat_by_id() {
        for command in ["kiro-cli", "kiro-cli-chat", "kiro-cli-"] {
            assert_eq!(named_by_command(command), Some("Kiro CLI"), "{command}");
        }
        assert_eq!(named_by_command("kiro"), None);
        let lsof = "p4\nckiro-cli-\nfcwd\nn/work/project\n";
        assert_eq!(agents_in(lsof, Path::new("/work/project")), ["Kiro CLI"]);
        assert_eq!(launch_args("kiro-cli"), ["chat"]);
        assert!(launch_args("claude").is_empty());
        assert_eq!(
            resume_args("Kiro CLI", "f2946a26"),
            Some(vec!["--resume-id".to_string(), "f2946a26".to_string()])
        );
        assert_eq!(tab_name("Kiro"), "Kiro CLI");
        assert_eq!(display_name("kiro-cli"), "Kiro CLI");
    }

    #[test]
    fn a_click_finds_the_agent_to_start_below_the_worktrees() {
        let agents = Agents {
            installed: vec![Offered::built_in("Codex", "codex")],
            worktrees: vec![Worktree {
                path: PathBuf::from("/work"),
                branch: "main".to_string(),
                changes: 0,
            }],
            ..Agents::default()
        };
        let area = Rect::new(0, 0, 30, 40);
        let at = |hit: &Hit| (0..40).find(|y| agents.hit(area, *y, 0, &[]).as_ref() == Some(hit));
        let start = at(&Hit::Start("codex".to_string())).unwrap();
        let tree = at(&Hit::Worktree(PathBuf::from("/work"))).unwrap();
        assert!(
            start < tree,
            "with nothing running, starting one comes first"
        );
        assert_eq!(agents.hit(area, 0, 0, &[]), None, "the title does nothing");
    }

    #[test]
    fn starting_an_agent_stays_in_reach_beside_many_worktrees() {
        let agents = Agents {
            installed: vec![
                Offered::built_in("Claude Code", "claude"),
                Offered::built_in(SHELL.0, SHELL.1),
            ],
            worktrees: (0..11)
                .map(|i| Worktree {
                    path: PathBuf::from(format!("/work/tree-{i}")),
                    branch: format!("claude/tree-{i}"),
                    changes: 0,
                })
                .collect(),
            ..Agents::default()
        };
        // As tall as a side pane in a laptop's window: the worktrees don't all fit. Six agents
        // in tabs, idle.
        let area = Rect::new(0, 0, 40, 36);
        let rows: Vec<Row> = (0..6)
            .map(|i| Row {
                icon: crate::dock::program_icon("Claude Code", false, false),
                name: "Claude Code".to_string(),
                urgency: Urgency::Idle,
                since: None,
                detail: String::new(),
                session: format!("s{i}"),
                parent: None,
            })
            .collect();
        let found = (0..area.h)
            .find(|y| agents.hit(area, *y, 0, &rows) == Some(Hit::Start("claude".to_string())));
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

    /// A row's name is in the words' own colour, since a press on any row brings its tab
    /// forward: bold while its agent does something, and at regular weight while it's idle or
    /// is a helper under its agent. An idle row was in the quiet colour of "not installed",
    /// and a helper as bold as its agent read as its equal (0.11.0's review of `next`, m2,
    /// 172).
    #[test]
    fn names_read_as_rows_that_do_something() {
        let agents = Agents {
            installed: vec![Offered::built_in(SHELL.0, SHELL.1)],
            ..Agents::default()
        };
        let row = |name: &str, urgency, parent: Option<&str>| Row {
            icon: crate::dock::program_icon("Codex", false, false),
            name: name.to_string(),
            urgency,
            since: None,
            detail: String::new(),
            session: name.to_string(),
            parent: parent.map(str::to_string),
        };
        let rows = vec![
            row("lead", Urgency::Working, None),
            row("reviewer", Urgency::Working, Some("lead")),
            row("quiet one", Urgency::Idle, None),
        ];
        let theme = Theme::default();
        let measure = cellix_render::FontMeasure::new(17, 50).with_type_row(36);
        let area = Rect::new(0, 0, 50, 30);
        let mut grid = Grid::new(50, 30, Style::default());
        agents.draw(&mut grid, area, (0, &rows, 0.5), None, (&measure, &theme));
        let name = |words: &str| {
            let found = grid.paint.iter().find_map(|p| match p {
                Paint::Text {
                    text, color, font, ..
                } if text == words => Some((*color, font.bold)),
                _ => None,
            });
            found.unwrap_or_else(|| panic!("{words}: {}", grid.text()))
        };
        let words = theme.sidebar.fg;
        assert_eq!(name("lead"), (words, true), "doing something");
        assert_eq!(name("reviewer"), (words, false), "a helper");
        assert_eq!(name("quiet one"), (words, false), "idle");
    }

    /// What can be started is listed as the + menu lists it: the terminal first, a rule, then
    /// the agents, one row each with its mark and what it runs at the row's end.
    #[test]
    fn the_terminal_stands_apart_from_the_agents() {
        let agents = Agents {
            installed: vec![
                Offered::built_in("Claude Code", "claude"),
                Offered::built_in("Codex", "codex"),
                Offered::built_in(SHELL.0, SHELL.1),
            ],
            ..Agents::default()
        };
        let area = Rect::new(0, 0, 40, 30);
        let rows = |program: &str| -> Vec<u16> {
            let hit = Some(Hit::Start(program.to_string()));
            (0..area.h)
                .filter(|y| agents.hit(area, *y, 0, &[]) == hit)
                .collect()
        };
        let (claude, codex, shell) = (rows("claude"), rows("codex"), rows(SHELL.1));
        assert_eq!(
            (claude.len(), codex.len(), shell.len()),
            (1, 1, 1),
            "a row each"
        );
        assert_eq!(
            claude[0],
            shell[0] + 2,
            "the terminal first, and a rule's row"
        );
        assert_eq!(codex[0], claude[0] + 1, "one agent right under the other");
        assert_eq!(agents.hit(area, shell[0] + 1, 0, &[]), None, "the rule");
        let theme = Theme::default();
        let look: (&dyn Measure, &Theme) = (&cellix_render::CellMeasure, &theme);
        let mut grid = Grid::new(40, 30, Style::default());
        agents.draw(&mut grid, area, (0, &[], 0.0), None, look);
        let said = |y: u16| grid.row_text(y);
        assert!(
            said(shell[0]).trim_start().starts_with("> Terminal"),
            "{}",
            said(shell[0])
        );
        assert!(
            said(shell[0]).trim_end().ends_with("shell"),
            "{}",
            said(shell[0])
        );
        assert!(said(claude[0]).trim_start().starts_with("✻ Claude Code"));
        assert!(said(claude[0]).trim_end().ends_with("claude"));
        assert!(said(codex[0]).trim_start().starts_with("⬡ Codex"));
        assert!(!grid.text().contains("Starts here"), "{}", grid.text());

        // With no agent installed there's nothing to stand apart from.
        let alone = Agents {
            installed: vec![Offered::built_in(SHELL.0, SHELL.1)],
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

/// Runners found on `path` that nothing has added: executables named like an agent's
/// command (`claude-*`, `codex-*` and the rest), never the agents' own programs or ones
/// already added. Only each name and its executable bit are read, never a runner's
/// contents. Each comes with the agent its name's prefix says it wraps.
pub fn found_runners(
    path: &std::ffi::OsString,
    added: &[crate::settings::CustomAgent],
) -> Vec<(String, &'static str)> {
    let prefixes: [(&str, &'static str); 6] = [
        ("claude-", "claude"),
        ("codex-", "codex"),
        ("gemini-", "gemini"),
        ("opencode-", "opencode"),
        ("copilot-", "copilot"),
        ("kiro-cli-", "kiro-cli"),
    ];
    let taken = |name: &str| {
        AGENTS.iter().any(|(_, program)| *program == name)
            || added.iter().any(|agent| agent.command == name)
    };
    let executable = |entry: &std::fs::DirEntry| {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            entry
                .metadata()
                .is_ok_and(|m| m.permissions().mode() & 0o111 != 0)
        }
        #[cfg(not(unix))]
        {
            entry.metadata().map_or(false, |m| m.is_file())
        }
    };
    let mut found = Vec::new();
    for dir in std::env::split_paths(path) {
        let Ok(read) = std::fs::read_dir(dir) else {
            continue;
        };
        for entry in read.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let Some((_, like)) = prefixes.iter().find(|(prefix, _)| name.starts_with(prefix))
            else {
                continue;
            };
            if taken(&name) || !executable(&entry) {
                continue;
            }
            found.push((name, *like));
        }
    }
    found.sort();
    found.dedup();
    found
}

#[cfg(test)]
mod found_tests {
    use super::*;
    use std::fs;

    /// The scan reads only names and the executable bit: executables named like an agent's
    /// command, never the agents' own programs or ones already added, never a file's
    /// contents.
    #[test]
    fn the_found_runners_are_names_that_run_and_nothing_adds_itself() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("bin");
        fs::create_dir(&bin).unwrap();
        for name in ["claude-ds", "codex-fast", "gemini-2", "claude-zz"] {
            fs::write(bin.join(name), "#!/bin/sh\n").unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(bin.join(name), fs::Permissions::from_mode(0o755)).unwrap();
            }
        }
        // A runner's file that can't run, and one already added: neither is offered.
        fs::write(bin.join("opencode-frozen"), "#!/bin/sh\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(
                bin.join("opencode-frozen"),
                fs::Permissions::from_mode(0o644),
            )
            .unwrap();
        }
        let added = vec![crate::settings::CustomAgent {
            name: "Mine".to_string(),
            command: "gemini-2".to_string(),
            like: "gemini".to_string(),
        }];
        let path = std::ffi::OsString::from(bin.to_string_lossy().into_owned());
        let mut found = found_runners(&path, &added);
        found.sort();
        assert_eq!(
            found,
            vec![
                ("claude-ds".to_string(), "claude"),
                ("claude-zz".to_string(), "claude"),
                ("codex-fast".to_string(), "codex"),
            ],
        );
        // The scan never reads what a runner is: nothing of the stand-ins' contents.
    }
}
