//! The Task page: one piece of agent work as a timeline you can step through and act on. Its
//! plan with every version and how it was answered, each prompt with when and which agent,
//! what each turn changed, the tests run during it and the comments made on it. What a task is
//! comes from `cellix_review::tasks`; this only shows it.

use std::cell::Cell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::time::SystemTime;

use cellix_agent::{AgentEvent, AgentState};
use cellix_render::{Bounds, Font, Grid, Measure, Rect, Style};
use cellix_review::tasks::{self, Event, Outcome, PlanStep, Step, Task, TestStep, TurnStep};
use cellix_review::{Thread, changed_between, changed_since, numstat, numstat_between};
use unicode_width::UnicodeWidthStr;

use crate::theme::Theme;

/// Below this the timeline and the step can't stand side by side, so the page shows the step
/// with arrows to the others.
const NARROW: u16 = 76;
const TIMELINE_MIN: u16 = 34;
const TIMELINE_MAX: u16 = 50;
/// Rows a step takes in the timeline: two of text and one of air.
const STEP_ROWS: u16 = 3;
/// Rows the header takes: the name, what it is, and air before the line under it.
const HEADER_ROWS: u16 = 3;
/// Lines of a prompt shown before the rest is cut.
const PROMPT_LINES: usize = 6;
const SMALL: u8 = 85;
const RADIUS: u8 = 20;

/// What a click on the page lands on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Hit {
    Step(usize),
    /// The arrow after the name, which lists the other tasks.
    Switch,
    ReviewAll,
    ReviewTurn,
    UndoTurn,
    /// A file the turn changed, to review in it.
    File(PathBuf),
    /// A test run of the turn, by its place in the turn.
    Test(usize),
    Thread(u32),
    Approve,
    SendBack,
    OpenPlan,
    ComparePlan,
    Previous,
    Next,
    /// The agent's transcript, as a page.
    Transcript,
}

/// One file a turn changed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Changed {
    pub path: PathBuf,
    pub status: char,
    pub added: usize,
    pub removed: usize,
}

/// What the page needs from the rest of the app to draw and to be clicked.
pub struct Context<'a> {
    pub threads: &'a [Thread],
    pub running: &'a [AgentEvent],
    /// The plan Cellix is holding for an agent, which Approve and Send Back answer.
    pub pending_plan: Option<&'a Path>,
    /// How wide text is drawn, so a cut ends in "…" where the room really runs out.
    pub measure: &'a dyn Measure,
}

/// A session's prompts, by the name they'd give a task, with the context before and after.
type Prompts = Vec<(String, (u64, u64))>;

/// The Task page, which stands where the file would.
pub struct TaskPage {
    root: PathBuf,
    pub tasks: Vec<Task>,
    /// The task in view, by index into `tasks`.
    pub shown: usize,
    /// The step chosen in it.
    pub step: usize,
    /// The first step in view in the timeline.
    top: usize,
    /// Rows the step's side is scrolled by.
    detail_top: usize,
    /// When the log was read.
    seen: Option<SystemTime>,
    /// What each turn changed, by `turn_key`, read off the main thread.
    changes: HashMap<String, Vec<Changed>>,
    reading: Option<Receiver<HashMap<String, Vec<Changed>>>>,
    /// Each session's prompts with how full the context was across each, from its transcript.
    contexts: HashMap<String, Prompts>,
    reading_context: Option<Receiver<(String, Prompts)>>,
    /// Columns the title row leaves free at its end, for the pane's icons.
    pub title_room: Cell<u16>,
    hover: Cell<Option<(u16, u16)>>,
}

/// The key a turn's changes are kept under.
fn turn_key(turn: &TurnStep) -> String {
    format!(
        "{}..{}",
        turn.commit.as_deref().unwrap_or(""),
        turn.until.as_deref().unwrap_or("now")
    )
}

/// What every turn changed, as git tells it. Slow enough on a big tree to keep off a click.
fn read_changes(
    root: &Path,
    turns: &[(String, String, Option<String>)],
) -> HashMap<String, Vec<Changed>> {
    let mut found = HashMap::new();
    for (key, commit, until) in turns {
        let (changes, stats) = match until {
            Some(until) => (
                changed_between(root, commit, until),
                numstat_between(root, commit, until),
            ),
            None => (changed_since(root, commit), numstat(root, Some(commit))),
        };
        let rows = changes
            .into_iter()
            .map(|c| {
                // git counts no lines for a file it doesn't track yet, so those are counted here.
                let (added, removed) = stats.get(&c.path).copied().unwrap_or_else(|| {
                    let new = until.is_none() && matches!(c.status, 'A' | '?');
                    let text = if new {
                        std::fs::read_to_string(root.join(&c.path)).ok()
                    } else {
                        None
                    };
                    (text.map_or(0, |t| t.lines().count()), 0)
                });
                Changed {
                    path: c.path,
                    status: c.status,
                    added,
                    removed,
                }
            })
            .collect();
        found.insert(key.clone(), rows);
    }
    found
}

/// A time of day as the clock shows it here, or the day for one before today.
pub fn clock(ms: u64) -> String {
    let (day, minutes) = local(ms / 1000);
    let (today, _) = local(tasks::now() / 1000);
    if day == today {
        format!("{:02}:{:02}", minutes / 60, minutes % 60)
    } else {
        const MONTHS: [&str; 12] = [
            "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
        ];
        let (month, date) = day;
        format!("{date} {}", MONTHS[month as usize % 12])
    }
}

/// The local month, day and minute of the day for a time in seconds.
#[cfg(unix)]
fn local(seconds: u64) -> ((u32, u32), u32) {
    let time = libc::time_t::try_from(seconds).unwrap_or_default();
    // SAFETY: `localtime_r` writes only into `parts`, which lives for the call.
    let mut parts: libc::tm = unsafe { std::mem::zeroed() };
    unsafe { libc::localtime_r(&time, &mut parts) };
    let minutes = (parts.tm_hour * 60 + parts.tm_min) as u32;
    ((parts.tm_mon as u32, parts.tm_mday as u32), minutes)
}

#[cfg(not(unix))]
fn local(seconds: u64) -> ((u32, u32), u32) {
    let days = seconds / 86_400;
    let minutes = ((seconds % 86_400) / 60) as u32;
    // Without the local zone, UTC; the day only needs to tell today from the rest.
    ((0, days as u32), minutes)
}

/// How long something took, said briefly.
fn took(ms: u64) -> String {
    let seconds = ms / 1000;
    match seconds {
        0..60 => format!("{seconds}s"),
        60..3600 => format!("{}m {:02}s", seconds / 60, seconds % 60),
        _ => format!("{}h {:02}m", seconds / 3600, seconds % 3600 / 60),
    }
}

/// Words broken into lines of at most `width` cells. A word longer than a line is cut.
fn wrap(text: &str, width: usize) -> Vec<String> {
    let width = width.max(8);
    let mut lines = Vec::new();
    for paragraph in text.lines() {
        let mut line = String::new();
        for word in paragraph.split_whitespace() {
            let word: String = word.chars().take(width).collect();
            if !line.is_empty() && line.width() + 1 + word.width() > width {
                lines.push(std::mem::take(&mut line));
            }
            if !line.is_empty() {
                line.push(' ');
            }
            line.push_str(&word);
        }
        lines.push(line);
    }
    while lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
    lines
}

/// Text cut to fit `room`, as `width` measures it, ending in "…" where it was cut, so
/// nothing stops mid-letter.
fn fit(text: &str, room: f32, width: impl Fn(&str) -> f32) -> std::borrow::Cow<'_, str> {
    if width(text) <= room {
        return std::borrow::Cow::Borrowed(text);
    }
    let mut cut: String = text.to_string();
    while !cut.is_empty() && width(&format!("{}…", cut.trim_end())) > room {
        cut.pop();
    }
    let cut = cut.trim_end();
    if cut.is_empty() {
        return std::borrow::Cow::Owned(String::new());
    }
    std::borrow::Cow::Owned(format!("{cut}…"))
}

/// One row on the step's side.
enum Line<'a> {
    Gap,
    Heading(String),
    Prompt(String),
    /// A note from the reviewer, as on a plan sent back.
    Said(String),
    Text(String),
    Dim(String),
    File(&'a Changed),
    Test(usize, &'a TestStep),
    Thread(&'a Thread),
}

/// Draws while it lays out, or only lays out when there's no grid, so a click always lands
/// on what was drawn.
struct Pen<'g> {
    grid: Option<&'g mut Grid>,
    measure: &'g dyn Measure,
    hover: Option<(u16, u16)>,
    spots: Vec<(Rect, Hit)>,
}

impl Pen<'_> {
    fn hovered(&self, rect: Rect) -> bool {
        self.hover.is_some_and(|(x, y)| rect.contains(x, y))
    }

    /// Writes a label, and gives the column after it.
    fn label(&mut self, x: u16, y: u16, text: &str, style: Style, max_x: u16) -> u16 {
        self.label_sized(x, y, text, style, max_x, 100)
    }

    fn label_sized(
        &mut self,
        x: u16,
        y: u16,
        text: &str,
        style: Style,
        max_x: u16,
        size: u8,
    ) -> u16 {
        let font = Font {
            bold: style.bold,
            size,
            ..Font::default()
        };
        let text = fit(text, max_x.saturating_sub(x) as f32, |t| {
            self.measure.width(t, font)
        });
        match self.grid.as_deref_mut() {
            Some(grid) => grid.label_at(x as f32, x, y, &text, style, max_x, size),
            None => x.saturating_add(text.width() as u16).min(max_x),
        }
    }

    fn label_right(&mut self, right: u16, y: u16, text: &str, style: Style) {
        if let Some(grid) = self.grid.as_deref_mut() {
            grid.label_right(right, y, text, style, SMALL);
        }
    }

    /// A button whose right edge is at `right`, giving the column it starts at.
    fn button(
        &mut self,
        right: u16,
        y: u16,
        label: &str,
        primary: bool,
        theme: &Theme,
        hit: Hit,
    ) -> u16 {
        let cells = format!("[{label}]");
        let x = right.saturating_sub(cells.width() as u16);
        let spot = Rect::new(x, y, cells.width() as u16, 1);
        let hovered = self.hovered(spot);
        if let Some(grid) = self.grid.as_deref_mut() {
            let (fill, fg) = if primary {
                (theme.button, theme.button_text)
            } else {
                (theme.tint, theme.text.fg)
            };
            let fill = if hovered {
                theme.quiet_button(fill)
            } else {
                fill
            };
            let style = Style { fg, ..theme.text };
            grid.button(x, y, &cells, label, fill, style);
        }
        self.spots.push((spot, hit));
        x
    }

    /// A row that can be clicked, washed when the pointer is on it or it's chosen.
    fn row(&mut self, rect: Rect, chosen: bool, theme: &Theme, hit: Hit) {
        let hovered = self.hovered(rect);
        if let Some(grid) = self.grid.as_deref_mut()
            && (chosen || hovered)
        {
            let fill = if chosen {
                theme.row_active
            } else {
                theme.row_hover
            };
            let style = Style {
                bg: fill,
                ..theme.text
            };
            grid.fill(rect, style);
            grid.panel(Bounds::inset(rect, 0.0, 0.0), 0, Some(theme.text.bg), None);
            grid.panel(Bounds::inset(rect, 0.3, 0.06), RADIUS, Some(fill), None);
        }
        self.spots.push((rect, hit));
    }
}

impl TaskPage {
    pub fn new(root: PathBuf) -> Self {
        let mut page = Self {
            root,
            tasks: Vec::new(),
            shown: 0,
            step: 0,
            top: 0,
            detail_top: 0,
            seen: None,
            changes: HashMap::new(),
            reading: None,
            contexts: HashMap::new(),
            reading_context: None,
            title_room: Cell::new(0),
            hover: Cell::new(None),
        };
        page.reload();
        page.show_latest();
        page
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Reads the log again, keeping the task and step in view where they still are.
    pub fn reload(&mut self) {
        let keep = self.task().map(|t| (t.id.clone(), self.step));
        self.seen = tasks::changed(&self.root);
        self.tasks = tasks::tasks(&tasks::load(&self.root), tasks::plan_title);
        if let Some((id, step)) = keep {
            match self.tasks.iter().position(|t| t.id == id) {
                Some(index) => {
                    self.shown = index;
                    self.step = step.min(self.tasks[index].steps.len().saturating_sub(1));
                }
                None => self.show_latest(),
            }
        }
        self.read_changes_later();
        self.read_context_later();
    }

    /// Reads again what each turn changed, in the background.
    pub fn read_changes_later(&mut self) {
        let turns: Vec<(String, String, Option<String>)> = self
            .tasks
            .iter()
            .flat_map(|t| t.turns())
            .filter_map(|t| Some((turn_key(t), t.commit.clone()?, t.until.clone())))
            // A finished turn's changes never change, so they're read once.
            .filter(|(key, _, until)| until.is_none() || !self.changes.contains_key(key))
            .collect();
        if turns.is_empty() {
            return;
        }
        let root = self.root.clone();
        let (sender, receiver) = mpsc::channel();
        self.reading = Some(receiver);
        std::thread::spawn(move || {
            let _ = sender.send(read_changes(&root, &turns));
        });
    }

    /// Takes in what changed, and reads the log again if an agent added to it. Says whether
    /// anything did.
    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        if let Some(found) = self.reading.as_ref().and_then(|r| r.try_recv().ok()) {
            self.reading = None;
            self.changes.extend(found);
            changed = true;
        }
        if let Some((session, prompts)) = self
            .reading_context
            .as_ref()
            .and_then(|r| r.try_recv().ok())
        {
            self.reading_context = None;
            self.contexts.insert(session, prompts);
            changed = true;
        }
        if tasks::changed(&self.root) != self.seen {
            self.reload();
            changed = true;
        }
        changed
    }

    /// Reads, off the main thread, how full the shown task's agent's context was over each
    /// prompt, from the transcript the agent keeps. Read again each time, as it grows.
    pub fn read_context_later(&mut self) {
        let Some(task) = self.task() else {
            return;
        };
        if task.session.is_empty() || self.reading_context.is_some() {
            return;
        }
        let (agent, session) = (task.agent.clone(), task.session.clone());
        let (sender, receiver) = mpsc::channel();
        self.reading_context = Some(receiver);
        std::thread::spawn(move || {
            let read = crate::transcript::find(&agent, &session)
                .and_then(|path| crate::transcript::read(&path).ok());
            let prompts = read.map_or_else(Vec::new, |t| {
                t.turns
                    .into_iter()
                    .filter_map(|turn| Some((tasks::name_from(&turn.prompt), turn.context?)))
                    .collect()
            });
            let _ = sender.send((session, prompts));
        });
    }

    /// How full the context was as a turn began and as it ended, where the transcript says.
    pub fn context_of(&self, task: &Task, turn: &TurnStep) -> Option<(u64, u64)> {
        let name = tasks::name_from(&turn.text);
        self.contexts
            .get(&task.session)?
            .iter()
            .find(|(prompt, _)| *prompt == name)
            .map(|(_, context)| *context)
    }

    /// Whether a read of the changes is still out.
    pub fn is_reading(&self) -> bool {
        self.reading.is_some()
    }

    /// Shows the task worked on last, at what needs the reviewer: a waiting plan, or else
    /// its latest step.
    pub fn show_latest(&mut self) {
        let latest = (0..self.tasks.len()).max_by_key(|&i| self.tasks[i].latest());
        if let Some(index) = latest {
            self.show_task(index);
        }
    }

    pub fn show_task(&mut self, index: usize) {
        let Some(task) = self.tasks.get(index) else {
            return;
        };
        self.shown = index;
        let waiting = task
            .steps
            .iter()
            .rposition(|s| matches!(s, Step::Plan(p) if p.outcome == Outcome::Waiting));
        self.step = waiting.unwrap_or(task.steps.len().saturating_sub(1));
        // From the start: the timeline scrolls itself to keep the chosen step in view.
        self.top = 0;
        self.detail_top = 0;
        self.read_context_later();
    }

    pub fn task(&self) -> Option<&Task> {
        self.tasks.get(self.shown)
    }

    pub fn chosen(&self) -> Option<&Step> {
        self.task()?.steps.get(self.step)
    }

    pub fn chosen_turn(&self) -> Option<&TurnStep> {
        match self.chosen()? {
            Step::Turn(turn) => Some(turn),
            Step::Plan(_) => None,
        }
    }

    pub fn chosen_plan(&self) -> Option<&PlanStep> {
        match self.chosen()? {
            Step::Plan(plan) => Some(plan),
            Step::Turn(_) => None,
        }
    }

    /// How many turns of the task come after the chosen one.
    pub fn turns_after(&self) -> usize {
        let Some(turn) = self.chosen_turn() else {
            return 0;
        };
        self.task()
            .map_or(0, |t| t.turns().filter(|o| o.at > turn.at).count())
    }

    pub fn choose(&mut self, step: usize) {
        if self.task().is_some_and(|t| step < t.steps.len()) {
            self.step = step;
            self.detail_top = 0;
            if step < self.top {
                self.top = step;
            }
        }
    }

    /// Moves to the step before or after.
    pub fn step_by(&mut self, delta: isize) {
        let count = self.task().map_or(0, |t| t.steps.len());
        if count == 0 {
            return;
        }
        let step = (self.step as isize + delta).clamp(0, count as isize - 1) as usize;
        self.choose(step);
    }

    /// The first turn of the task, where reviewing all of it starts.
    pub fn first_commit(&self) -> Option<String> {
        self.task()?.turns().find_map(|t| t.commit.clone())
    }

    /// Where the chosen version of a plan is.
    pub fn plan_path(&self, task: &Task, plan: &PlanStep) -> Option<PathBuf> {
        let same: Vec<&PlanStep> = task.plans().filter(|p| p.file == plan.file).collect();
        let index = same.iter().position(|p| p.at == plan.at)?;
        tasks::plan_versions(task, plan).get(index).cloned()
    }

    /// The event that starts a new task at the chosen turn, unless it starts one already.
    pub fn split_here(&self) -> Option<Event> {
        let (task, turn) = (self.task()?, self.chosen_turn()?);
        (turn.number > 1).then(|| Event::Split {
            at: turn.at,
            session: task.session.clone(),
        })
    }

    /// The event that names the task in view.
    pub fn rename(&self, name: &str) -> Option<Event> {
        let name = name.trim();
        let task = self.task()?.id.clone();
        (!name.is_empty()).then(|| Event::Rename {
            at: tasks::now(),
            task,
            name: name.to_string(),
        })
    }

    /// What a turn changed, once read.
    pub fn changes_of(&self, turn: &TurnStep) -> Option<&Vec<Changed>> {
        self.changes.get(&turn_key(turn))
    }

    pub fn scroll_by(&mut self, area: Rect, x: u16, delta: isize) {
        let (timeline, _) = Self::columns(area);
        if timeline.is_some_and(|t| x < t.right()) {
            let count = self.task().map_or(0, |t| t.steps.len());
            self.top =
                (self.top as isize + delta).clamp(0, count.saturating_sub(1) as isize) as usize;
        } else {
            self.detail_top = (self.detail_top as isize + delta).max(0) as usize;
        }
    }

    /// The timeline's column, if there's room for one, and the step's.
    fn columns(area: Rect) -> (Option<Rect>, Rect) {
        let top = area.y + HEADER_ROWS;
        let height = area.bottom().saturating_sub(top);
        if area.w < NARROW {
            return (None, Rect::new(area.x, top, area.w, height));
        }
        let width = (area.w * 2 / 5).clamp(TIMELINE_MIN, TIMELINE_MAX);
        let timeline = Rect::new(area.x, top, width, height);
        let detail = Rect::new(
            area.x + width + 1,
            top,
            area.w.saturating_sub(width + 1),
            height,
        );
        (Some(timeline), detail)
    }

    pub fn hit(&self, area: Rect, x: u16, y: u16, context: &Context, theme: &Theme) -> Option<Hit> {
        let mut pen = Pen {
            grid: None,
            measure: context.measure,
            hover: None,
            spots: Vec::new(),
        };
        self.walk(&mut pen, area, context, theme);
        pen.spots
            .into_iter()
            .rev()
            .find(|(rect, _)| rect.contains(x, y))
            .map(|(_, hit)| hit)
    }

    pub fn draw(
        &self,
        grid: &mut Grid,
        area: Rect,
        hover: Option<(u16, u16)>,
        context: &Context,
        theme: &Theme,
    ) {
        self.hover.set(hover);
        grid.fill(area, theme.text);
        let mut pen = Pen {
            grid: Some(grid),
            measure: context.measure,
            hover,
            spots: Vec::new(),
        };
        self.walk(&mut pen, area, context, theme);
    }

    fn walk(&self, pen: &mut Pen, area: Rect, context: &Context, theme: &Theme) {
        let Some(task) = self.task() else {
            self.walk_empty(pen, area, theme);
            return;
        };
        self.walk_header(pen, area, task, context, theme);
        let (timeline, detail) = Self::columns(area);
        if let Some(timeline) = timeline {
            self.walk_timeline(pen, timeline, task, context, theme);
            if let Some(grid) = pen.grid.as_deref_mut() {
                let line = Bounds::cells(
                    timeline.right() as f32 + 0.5,
                    detail.y as f32,
                    0.0,
                    detail.h as f32,
                );
                grid.hairline(line, theme.border);
                for y in detail.y..detail.bottom() {
                    let style = Style {
                        fg: theme.border,
                        ..theme.text
                    };
                    grid.put_str(timeline.right(), y, "│", style, timeline.right() + 1);
                }
            }
        }
        self.walk_detail(pen, detail, timeline.is_none(), task, context, theme);
    }

    fn walk_empty(&self, pen: &mut Pen, area: Rect, theme: &Theme) {
        let mid = area.y + area.h / 2;
        let heading = Style {
            bold: true,
            fg: theme.heading,
            ..theme.text
        };
        let dim = Style {
            fg: theme.status.fg,
            ..theme.text
        };
        let title = "No agent work in this folder yet";
        let x = area.x + area.w.saturating_sub(title.width() as u16) / 2;
        pen.label(x, mid.saturating_sub(2), title, heading, area.right());
        let words = "A task starts with your first prompt to an agent, and a new one with each \
                     plan after an approved one. Its plan, turns, tests and comments gather here.";
        let width = area.w.saturating_sub(8).min(72) as usize;
        for (i, line) in wrap(words, width).iter().enumerate() {
            let x = area.x + area.w.saturating_sub(line.width() as u16) / 2;
            pen.label(x, mid + i as u16, line, dim, area.right());
        }
    }

    fn status(&self, task: &Task, context: &Context, theme: &Theme) -> Option<(String, Style)> {
        let style = |fg| Style { fg, ..theme.text };
        if task.waiting_plan().is_some() {
            return Some(("● Plan waiting for you".to_string(), style(theme.attention)));
        }
        let newest = self
            .tasks
            .iter()
            .filter(|t| t.session == task.session)
            .max_by_key(|t| t.started())
            .is_some_and(|t| t.id == task.id);
        let event = context
            .running
            .iter()
            .find(|e| e.session == task.session && !task.session.is_empty());
        match (newest, event.map(|e| e.state)) {
            (true, Some(AgentState::Working)) => {
                let turn = task.turns().count();
                Some((format!("● Working on turn {turn}"), style(theme.accent)))
            }
            (true, Some(AgentState::NeedsYou)) => {
                Some(("● Needs you".to_string(), style(theme.attention)))
            }
            _ => None,
        }
    }

    fn walk_header(
        &self,
        pen: &mut Pen,
        area: Rect,
        task: &Task,
        context: &Context,
        theme: &Theme,
    ) {
        let y = area.y;
        let heading = Style {
            bold: true,
            fg: theme.heading,
            ..theme.text
        };
        let dim = Style {
            fg: theme.status.fg,
            ..theme.text
        };
        let right = area.right().saturating_sub(self.title_room.get() + 1);
        let mut buttons = right;
        if self.first_commit().is_some() && area.w >= NARROW {
            buttons = pen.button(right, y, "Review All Changes", false, theme, Hit::ReviewAll);
        }
        if !task.session.is_empty() && area.w >= NARROW {
            let at = if buttons == right {
                right
            } else {
                buttons.saturating_sub(1)
            };
            buttons = pen.button(at, y, "Open Transcript", false, theme, Hit::Transcript);
        }
        let end = pen.label(
            area.x + 2,
            y,
            &task.name,
            heading,
            buttons.saturating_sub(4),
        );
        let switch = Rect::new(end + 1, y, 2, 1);
        if self.tasks.len() > 1 {
            pen.row(switch, false, theme, Hit::Switch);
            pen.label(end + 1, y, "▾", dim, end + 3);
        }
        let after = if self.tasks.len() > 1 {
            end + 4
        } else {
            end + 2
        };
        if let Some((text, style)) = self.status(task, context, theme) {
            pen.label_sized(after, y, &text, style, buttons.saturating_sub(1), SMALL);
        }
        let turns = task.turns().count();
        let tests: usize = task.turns().map(|t| t.tests.len()).sum();
        let mut meta = vec![task.agent.clone()];
        meta.push(format!("since {}", clock(task.started())));
        let plans = task.plans().count();
        if let Some(approved) = task
            .plans()
            .find(|p| matches!(p.outcome, Outcome::Approved(_)))
        {
            let at = approved.answered.map(clock).unwrap_or_default();
            meta.push(format!("plan v{} approved {at}", approved.version));
        } else if plans > 0 {
            meta.push(format!(
                "{plans} plan version{}",
                if plans == 1 { "" } else { "s" }
            ));
        }
        meta.push(format!("{turns} turn{}", if turns == 1 { "" } else { "s" }));
        if tests > 0 {
            meta.push(format!(
                "{tests} test run{}",
                if tests == 1 { "" } else { "s" }
            ));
        }
        pen.label(
            area.x + 2,
            y + 1,
            &meta.join("  ·  "),
            dim,
            area.right().saturating_sub(1),
        );
        if let Some(grid) = pen.grid.as_deref_mut() {
            let line = Bounds::cells(
                area.x as f32,
                (y + HEADER_ROWS) as f32 - 0.7,
                area.w as f32,
                0.0,
            );
            grid.hairline(line, theme.border);
        }
    }

    /// The two lines a step shows in the timeline.
    fn summary(&self, step: &Step, context: &Context) -> (String, String) {
        match step {
            Step::Plan(plan) => {
                let state = match &plan.outcome {
                    Outcome::Waiting => "waiting for you".to_string(),
                    Outcome::SentBack(_) => "sent back".to_string(),
                    Outcome::Approved(_) => {
                        format!("approved {}", plan.answered.map(clock).unwrap_or_default())
                    }
                };
                let below = match &plan.outcome {
                    Outcome::SentBack(note) if !note.trim().is_empty() => {
                        format!("You: \u{201c}{}\u{201d}", tasks::name_from(note))
                    }
                    Outcome::Approved(_) => "The turns after it follow this version".to_string(),
                    _ => String::new(),
                };
                (format!("Plan v{}  {state}", plan.version), below)
            }
            Step::Turn(turn) => {
                let mut parts = Vec::new();
                match self.changes_of(turn) {
                    Some(files) if !files.is_empty() => {
                        let added: usize = files.iter().map(|f| f.added).sum();
                        let removed: usize = files.iter().map(|f| f.removed).sum();
                        let n = files.len();
                        parts.push(format!(
                            "{n} file{} +{added} -{removed}",
                            if n == 1 { "" } else { "s" }
                        ));
                    }
                    Some(_) => parts.push("no changes".to_string()),
                    None if turn.commit.is_none() => {}
                    None => parts.push("…".to_string()),
                }
                if let Some(last) = turn.tests.last() {
                    parts.push(
                        if last.passed {
                            "tests passed"
                        } else {
                            "tests failed"
                        }
                        .to_string(),
                    );
                }
                let comments = context
                    .threads
                    .iter()
                    .filter(|t| turn.holds_thread(t.id))
                    .count();
                if comments > 0 {
                    parts.push(format!(
                        "{comments} comment{}",
                        if comments == 1 { "" } else { "s" }
                    ));
                }
                (tasks::name_from(&turn.text), parts.join(" · "))
            }
        }
    }

    fn walk_timeline(
        &self,
        pen: &mut Pen,
        column: Rect,
        task: &Task,
        context: &Context,
        theme: &Theme,
    ) {
        let dim = Style {
            fg: theme.status.fg,
            ..theme.text
        };
        let quiet = Style {
            fg: theme.gutter.fg,
            ..theme.text
        };
        let fits = (column.h.saturating_sub(1) / STEP_ROWS).max(1) as usize;
        // Keep the chosen step in view.
        let top = if self.step >= self.top + fits {
            self.step + 1 - fits
        } else {
            self.top.min(self.step)
        };
        for (index, step) in task.steps.iter().enumerate().skip(top).take(fits) {
            let y = column.y + 1 + (index - top) as u16 * STEP_ROWS;
            let rect = Rect::new(column.x + 1, y, column.w.saturating_sub(2), 2);
            let chosen = index == self.step;
            pen.row(rect, chosen, theme, Hit::Step(index));
            let bg = if chosen {
                theme.row_active
            } else if pen.hovered(rect) {
                theme.row_hover
            } else {
                theme.text.bg
            };
            let on = |style: Style| Style { bg, ..style };
            let time = clock(step.at());
            pen.label_sized(rect.x + 1, y, &time, on(quiet), rect.x + 7, SMALL);
            let (mark, color) = match step {
                Step::Plan(plan) => match plan.outcome {
                    Outcome::Waiting => ("◆", theme.attention),
                    Outcome::SentBack(_) => ("◇", theme.status.fg),
                    Outcome::Approved(_) => ("✓", theme.git_added),
                },
                Step::Turn(turn) if turn.tests_failing() => ("●", theme.error),
                Step::Turn(_) => ("●", theme.accent),
            };
            let marker = Style {
                fg: color,
                ..on(theme.text)
            };
            pen.label(rect.x + 8, y, mark, marker, rect.x + 10);
            let (title, below) = self.summary(step, context);
            let title = match step {
                Step::Turn(turn) => format!("{}  {title}", turn.number),
                Step::Plan(_) => title,
            };
            let strong = Style {
                bold: chosen,
                ..on(theme.text)
            };
            pen.label(
                rect.x + 10,
                y,
                &title,
                strong,
                rect.right().saturating_sub(1),
            );
            pen.label_sized(
                rect.x + 10,
                y + 1,
                &below,
                on(dim),
                rect.right().saturating_sub(1),
                SMALL,
            );
            // One rail from each step's mark down to the next one's.
            if index + 1 < task.steps.len()
                && let Some(grid) = pen.grid.as_deref_mut()
            {
                let rail = Style {
                    fg: theme.border,
                    ..theme.text
                };
                grid.put_str(rect.x + 8, y + 1, "│", on(rail), rect.x + 9);
                grid.put_str(rect.x + 8, y + 2, "│", rail, rect.x + 9);
                let line = Bounds::cells(rect.x as f32 + 8.5, y as f32 + 0.8, 0.0, 2.4);
                grid.hairline(line, theme.border);
            }
        }
    }

    fn lines<'a>(
        &'a self,
        task: &'a Task,
        step: &'a Step,
        width: usize,
        context: &'a Context,
    ) -> Vec<Line<'a>> {
        let mut lines = Vec::new();
        match step {
            Step::Turn(turn) => {
                let mut prompt = wrap(&turn.text, width.saturating_sub(8));
                if prompt.len() > PROMPT_LINES {
                    prompt.truncate(PROMPT_LINES);
                    if let Some(last) = prompt.last_mut() {
                        last.push('…');
                    }
                }
                lines.extend(prompt.into_iter().map(Line::Prompt));
                lines.push(Line::Gap);
                match self.changes_of(turn) {
                    Some(files) => {
                        let n = files.len();
                        lines.push(Line::Heading(format!(
                            "CHANGED  {n} file{}",
                            if n == 1 { "" } else { "s" }
                        )));
                        if files.is_empty() {
                            lines.push(Line::Dim("Nothing changed in this turn.".to_string()));
                        }
                        lines.extend(files.iter().map(Line::File));
                    }
                    None if turn.commit.is_none() => {
                        lines.push(Line::Heading("CHANGED".to_string()));
                        lines.push(Line::Dim(
                            "No checkpoint: this folder isn't in git.".to_string(),
                        ));
                    }
                    None => {
                        lines.push(Line::Heading("CHANGED".to_string()));
                        lines.push(Line::Dim("Reading…".to_string()));
                    }
                }
                if !turn.tests.is_empty() {
                    lines.push(Line::Gap);
                    let n = turn.tests.len();
                    lines.push(Line::Heading(format!(
                        "TESTS  {n} run{}",
                        if n == 1 { "" } else { "s" }
                    )));
                    lines.extend(turn.tests.iter().enumerate().map(|(i, t)| Line::Test(i, t)));
                }
                let threads: Vec<&Thread> = context
                    .threads
                    .iter()
                    .filter(|t| turn.holds_thread(t.id))
                    .collect();
                if !threads.is_empty() {
                    lines.push(Line::Gap);
                    let open = threads.iter().filter(|t| !t.resolved).count();
                    lines.push(Line::Heading(format!("COMMENTS  {open} open")));
                    lines.extend(threads.into_iter().map(Line::Thread));
                }
            }
            Step::Plan(plan) => {
                if let Outcome::SentBack(note) = &plan.outcome
                    && !note.trim().is_empty()
                {
                    lines.extend(
                        wrap(note, width.saturating_sub(7))
                            .into_iter()
                            .map(Line::Said),
                    );
                    lines.push(Line::Gap);
                }
                match self
                    .plan_path(task, plan)
                    .and_then(|p| std::fs::read_to_string(p).ok())
                {
                    Some(text) => lines.extend(wrap(&text, width).into_iter().map(Line::Text)),
                    None => lines.push(Line::Dim(
                        "This version is no longer on disk; it went with the agent's session."
                            .to_string(),
                    )),
                }
            }
        }
        lines
    }

    fn walk_detail(
        &self,
        pen: &mut Pen,
        column: Rect,
        narrow: bool,
        task: &Task,
        context: &Context,
        theme: &Theme,
    ) {
        let Some(step) = task.steps.get(self.step) else {
            return;
        };
        let left = column.x + 2;
        let right = column.right().saturating_sub(2);
        let heading = Style {
            bold: true,
            fg: theme.heading,
            ..theme.text
        };
        let dim = Style {
            fg: theme.status.fg,
            ..theme.text
        };
        let mut y = column.y + 1;
        if narrow {
            let place = format!("Step {} of {}", self.step + 1, task.steps.len());
            let after = pen.button(left + 3, y, "◂", false, theme, Hit::Previous);
            let end = pen.label(after + 4, y, &place, dim, right);
            pen.button(end + 4, y, "▸", false, theme, Hit::Next);
            y += 2;
        }
        // The step's own title, and what can be done with it.
        let mut buttons = right;
        let title = match step {
            Step::Turn(turn) => {
                buttons = pen.button(buttons, y, "Review Turn", true, theme, Hit::ReviewTurn);
                if turn.commit.is_some() {
                    let undo = if self.turns_after() == 0 {
                        "Undo Turn"
                    } else {
                        "Undo From Here"
                    };
                    buttons = pen.button(
                        buttons.saturating_sub(1),
                        y,
                        undo,
                        false,
                        theme,
                        Hit::UndoTurn,
                    );
                }
                let mut said = vec![clock(turn.at)];
                if let Some(ended) = turn.ended {
                    said.push(took(ended.saturating_sub(turn.at)));
                }
                if let Some((from, to)) = self.context_of(task, turn) {
                    let tokens = crate::transcript::tokens;
                    said.push(format!("context {} → {}", tokens(from), tokens(to)));
                }
                said.push(task.agent.clone());
                (format!("Turn {}", turn.number), said.join(" · "))
            }
            Step::Plan(plan) => {
                let waiting = plan.outcome == Outcome::Waiting
                    && context
                        .pending_plan
                        .is_some_and(|p| p.display().to_string() == plan.file);
                if waiting {
                    buttons = pen.button(buttons, y, "Approve", true, theme, Hit::Approve);
                    buttons = pen.button(
                        buttons.saturating_sub(1),
                        y,
                        "Send Back",
                        false,
                        theme,
                        Hit::SendBack,
                    );
                    if plan.version > 1 {
                        buttons = pen.button(
                            buttons.saturating_sub(1),
                            y,
                            "Compare",
                            false,
                            theme,
                            Hit::ComparePlan,
                        );
                    }
                }
                if self.plan_path(task, plan).is_some_and(|p| p.exists()) {
                    let at = if buttons == right {
                        right
                    } else {
                        buttons.saturating_sub(1)
                    };
                    buttons = pen.button(at, y, "Open Plan", !waiting, theme, Hit::OpenPlan);
                }
                let state = match &plan.outcome {
                    Outcome::Waiting => format!("sent {} · waiting for you", clock(plan.at)),
                    Outcome::SentBack(_) => format!(
                        "sent {} · sent back {}",
                        clock(plan.at),
                        plan.answered.map(clock).unwrap_or_default()
                    ),
                    Outcome::Approved(_) => format!(
                        "sent {} · approved {}",
                        clock(plan.at),
                        plan.answered.map(clock).unwrap_or_default()
                    ),
                };
                (format!("Plan v{}", plan.version), state)
            }
        };
        let end = pen.label(left, y, &title.0, heading, buttons.saturating_sub(2));
        // One column for what it is, whatever the title's length.
        let meta = (left + 10).max(end + 2);
        pen.label_sized(meta, y, &title.1, dim, buttons.saturating_sub(2), SMALL);
        y += 2;
        let width = right.saturating_sub(left) as usize;
        let lines = self.lines(task, step, width, context);
        let prompt_rows = lines
            .iter()
            .take_while(|l| matches!(l, Line::Prompt(_)))
            .count();
        if prompt_rows > 0
            && self.detail_top == 0
            && let Some(grid) = pen.grid.as_deref_mut()
        {
            // The prompt stands in a box of its own, as the agent got it.
            let rows = (prompt_rows as u16).min(column.bottom().saturating_sub(y));
            let rect = Rect::new(left, y, right.saturating_sub(left), rows);
            grid.fill(rect, theme.sidebar);
            grid.panel(Bounds::inset(rect, 0.0, 0.0), 0, Some(theme.text.bg), None);
            grid.panel(
                Bounds::inset(rect, 0.0, 0.1),
                RADIUS,
                Some(theme.sidebar.bg),
                None,
            );
        }
        for line in lines.iter().skip(self.detail_top) {
            if y >= column.bottom() {
                break;
            }
            let row = Rect::new(left, y, right.saturating_sub(left), 1);
            match line {
                Line::Gap => {}
                Line::Heading(text) => {
                    let style = Style {
                        fg: theme.gutter.fg,
                        bold: true,
                        ..theme.text
                    };
                    pen.label_sized(left, y, text, style, right, 80);
                }
                Line::Prompt(text) => {
                    let who = Style {
                        fg: theme.status.fg,
                        ..theme.sidebar
                    };
                    if line_is_first(&lines, line) {
                        pen.label(left + 1, y, "You", who, left + 5);
                    }
                    pen.label(left + 6, y, text, theme.sidebar, right.saturating_sub(1));
                }
                Line::Said(text) => {
                    let who = Style {
                        fg: theme.status.fg,
                        ..theme.text
                    };
                    if line_is_first(&lines, line) {
                        pen.label(left + 1, y, "You", who, left + 5);
                    }
                    pen.label(left + 6, y, text, theme.text, right);
                }
                Line::Text(text) => {
                    let bare = text.trim_start_matches('#');
                    if bare.len() < text.len() && bare.starts_with(' ') {
                        pen.label(left, y, bare.trim(), heading, right);
                    } else {
                        pen.label(left, y, text, theme.text, right);
                    }
                }
                Line::Dim(text) => {
                    pen.label(left, y, text, dim, right);
                }
                Line::File(file) => {
                    pen.row(row, false, theme, Hit::File(file.path.clone()));
                    let name = file
                        .path
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    let folder = file
                        .path
                        .parent()
                        .map(|p| p.display().to_string())
                        .unwrap_or_default();
                    let mut end = right.saturating_sub(1);
                    let (letter, color) = match file.status {
                        'A' | '?' => ("A", theme.git_added),
                        'D' => ("D", theme.error),
                        _ => ("M", theme.git_modified),
                    };
                    pen.label_right(
                        end,
                        y,
                        letter,
                        Style {
                            fg: color,
                            ..theme.text
                        },
                    );
                    end = end.saturating_sub(3);
                    let removed = format!("-{}", file.removed);
                    pen.label_right(
                        end,
                        y,
                        &removed,
                        Style {
                            fg: theme.error,
                            ..theme.text
                        },
                    );
                    end = end.saturating_sub(removed.width() as u16 + 1);
                    let added = format!("+{}", file.added);
                    pen.label_right(
                        end,
                        y,
                        &added,
                        Style {
                            fg: theme.git_added,
                            ..theme.text
                        },
                    );
                    end = end.saturating_sub(added.width() as u16 + 2);
                    let after = pen.label(left + 1, y, &name, theme.text, end);
                    pen.label_sized(after + 2, y, &folder, dim, end, SMALL);
                }
                Line::Test(index, run) => {
                    pen.row(row, false, theme, Hit::Test(*index));
                    let (mark, color, word) = if run.passed {
                        ("✓", theme.git_added, "passed")
                    } else {
                        ("✗", theme.error, "failed")
                    };
                    pen.label(
                        left + 1,
                        y,
                        mark,
                        Style {
                            fg: color,
                            ..theme.text
                        },
                        left + 3,
                    );
                    let end = pen.label(
                        left + 3,
                        y,
                        &run.command,
                        theme.text,
                        right.saturating_sub(16),
                    );
                    pen.label_sized(
                        end + 2,
                        y,
                        word,
                        Style {
                            fg: color,
                            ..theme.text
                        },
                        right,
                        SMALL,
                    );
                    pen.label_right(right.saturating_sub(1), y, &clock(run.at), dim);
                }
                Line::Thread(thread) => {
                    pen.row(row, false, theme, Hit::Thread(thread.id));
                    let place = format!("{}:{}", thread.file, thread.line);
                    let style = if thread.resolved { dim } else { theme.text };
                    let end = pen.label_sized(left + 1, y, &place, dim, right, SMALL);
                    let words = tasks::name_from(&thread.text);
                    pen.label(end + 2, y, &words, style, right.saturating_sub(12));
                    let state = if thread.resolved {
                        "Resolved".to_string()
                    } else if thread.replies.is_empty() {
                        String::new()
                    } else {
                        format!(
                            "{} repl{}",
                            thread.replies.len(),
                            if thread.replies.len() == 1 {
                                "y"
                            } else {
                                "ies"
                            }
                        )
                    };
                    pen.label_right(right.saturating_sub(1), y, &state, dim);
                }
            }
            y += 1;
        }
    }
}

/// Whether a line is the first of its kind in a run, where its label goes.
fn line_is_first(lines: &[Line], line: &Line) -> bool {
    let at = lines
        .iter()
        .position(|l| std::ptr::eq(l, line))
        .unwrap_or(0);
    at == 0 || std::mem::discriminant(&lines[at - 1]) != std::mem::discriminant(line)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_wrap_to_the_width_and_keep_paragraphs() {
        let lines = wrap(
            "Rename refund_full to refund, and keep the old name.\n\nThen test.",
            20,
        );
        assert_eq!(
            lines,
            [
                "Rename refund_full",
                "to refund, and keep",
                "the old name.",
                "",
                "Then test."
            ]
        );
    }

    #[test]
    fn a_turn_says_how_full_the_agents_context_grew() {
        let dir = tempfile::tempdir().unwrap();
        let prompt = Event::Prompt {
            at: tasks::now(),
            session: "s1".into(),
            agent: "Claude Code".into(),
            text: "Add a refund route".into(),
            commit: None,
            threads: 0,
        };
        tasks::record(dir.path(), &prompt).unwrap();
        let mut page = TaskPage::new(dir.path().to_path_buf());
        // As the transcript tells it, for this session.
        page.contexts.insert(
            "s1".into(),
            vec![("Add a refund route".into(), (41_000, 88_000))],
        );
        let mut grid = Grid::new(140, 30, Style::default());
        let context = Context {
            threads: &[],
            running: &[],
            pending_plan: None,
            measure: &cellix_render::CellMeasure,
        };
        let area = grid.area();
        page.draw(&mut grid, area, None, &context, &Theme::named(None));
        let text = grid.text();
        assert!(text.contains("context 41k → 88k"), "{text}");
        assert!(text.contains("[Open Transcript]"), "{text}");
    }

    #[test]
    fn durations_read_briefly() {
        assert_eq!(took(42_000), "42s");
        assert_eq!(took(220_000), "3m 40s");
        assert_eq!(took(3_900_000), "1h 05m");
    }
}
