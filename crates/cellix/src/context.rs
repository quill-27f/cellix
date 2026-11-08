//! Agent Context in the sidebar: what an agent works from in this project. Its instructions in
//! the order it reads them, its skills, subagents, commands, hooks and MCP servers, what its
//! last session read and how full its context got, and gentle checks on all of it. Every row
//! opens the file it comes from; nothing here writes.

use std::cell::Cell;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::mpsc::{self, Receiver};
use std::time::SystemTime;

use cellix_agent::context::{
    Agent, Check, Context, Homes, Instruction, Item, ItemKind, Scope, Session, checks,
    claude_project_dir, discover, read_session,
};
use cellix_render::{
    ALL_CORNERS, Bounds, CellMeasure, Font, Grid, Icon, Measure, Rect, Rgb, Style,
};

use crate::theme::Theme;
use crate::threads::wrap;
use crate::transcript::tokens;

const SIDE: u16 = 2;
/// Rows a section shows before "Show all".
const SHORT_LIST: usize = 8;
/// Checks wrap over several rows each, so fewer show before "Show N more".
const SHORT_CHECKS: usize = 3;
/// Rows a check's words take at most; the rest ends in "…".
const CHECK_ROWS: usize = 2;
/// Cells a quiet detail needs free beside a name before any of it is shown.
const ROOM: f32 = 3.0;
/// The space between a name and its quiet detail, as Source Control sets it.
const GAP: f32 = 0.8;
/// Where the context bar sits in its row, and how tall it is, in rows.
const BAR_TOP: f32 = 0.33;
const BAR_HEIGHT: f32 = 0.34;
/// Columns the row numbers of instructions take, so names line up with checks' words.
const NUMBER: u16 = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Section {
    Checks,
    Instructions,
    Skills,
    Commands,
    Hooks,
    Mcp,
    Read,
}

impl Section {
    fn title(self) -> &'static str {
        match self {
            Section::Checks => "CHECKS",
            Section::Instructions => "INSTRUCTIONS",
            Section::Skills => "SKILLS AND SUBAGENTS",
            Section::Commands => "COMMANDS",
            Section::Hooks => "HOOKS",
            Section::Mcp => "MCP SERVERS",
            Section::Read => "READ LAST SESSION",
        }
    }

    /// What an empty section says, so each names what would fill it.
    fn empty(self, agent: Agent) -> String {
        match self {
            Section::Checks => "Nothing to look at".into(),
            Section::Instructions => format!("No instructions for {}", agent.name()),
            Section::Skills => "None here or in your settings".into(),
            Section::Commands => "No commands of your own".into(),
            Section::Hooks => "No hooks".into(),
            Section::Mcp => "No MCP servers".into(),
            Section::Read => "No session yet".into(),
        }
    }
}

/// What a click in the panel lands on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Hit {
    /// The agent picker: show the next agent.
    Agent,
    Fold(Section),
    More(Section),
    /// A file to open, at a line counting from one.
    Open(PathBuf, Option<usize>),
    /// A row with nothing to open, as a plugin's skill: say why.
    Say(String),
}

#[derive(Clone, Debug, PartialEq)]
enum Line {
    Title,
    Agent,
    Meter,
    Bar,
    Note,
    Gap,
    Heading(Section, usize),
    /// One row of a check's words, which wrap rather than run off the panel.
    Check(usize, usize),
    Instruction(usize),
    Item(usize),
    /// A skill the session was told of that isn't in a file here, as a plugin's.
    Listed(usize),
    File(usize),
    More(Section, usize),
    Empty(Section),
}

/// Everything read for one agent, away from the screen.
struct Read {
    agent: Agent,
    context: Context,
    checks: Vec<Check>,
    session: Option<Session>,
    /// When the session last wrote its transcript.
    age: Option<SystemTime>,
}

pub struct ContextPanel {
    pub agent: Agent,
    context: Context,
    checks: Vec<Check>,
    session: Option<Session>,
    age: Option<SystemTime>,
    /// Skills from the session's listing with no file here: (name, what it's for).
    listed: Vec<(String, String)>,
    folded: HashSet<Section>,
    open: HashSet<Section>,
    top: usize,
    reading: Option<Receiver<Read>>,
    /// The panel's width when last drawn or clicked, which checks wrap to.
    width: Cell<u16>,
    /// How wide text is drawn, so checks wrap and details sit where the words end.
    measure: Rc<dyn Measure>,
    /// The file last opened from the panel, whose row stays marked.
    pub selected: Option<PathBuf>,
    /// The transcript of the Claude Code session running in the dock, when it's known; else
    /// the newest one written for this project is read.
    pub session_file: Option<PathBuf>,
    /// Where each agent keeps its settings. Tests point these at fixtures.
    pub homes: Homes,
    /// Whether anything has been read yet.
    pub loaded: bool,
}

impl Default for ContextPanel {
    fn default() -> Self {
        Self::new(Homes::from_env())
    }
}

/// The newest transcript Claude Code wrote for `project`, and when.
fn latest_transcript(homes: &Homes, project: &Path) -> Option<(PathBuf, SystemTime)> {
    let dir = claude_project_dir(homes.claude.as_ref()?, project);
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "jsonl"))
        .filter_map(|p| Some((std::fs::metadata(&p).ok()?.modified().ok()?, p)))
        .max()
        .map(|(when, path)| (path, when))
}

impl ContextPanel {
    pub fn new(homes: Homes) -> Self {
        let folded = [Section::Commands, Section::Hooks, Section::Mcp];
        Self {
            agent: Agent::Claude,
            context: Context::default(),
            checks: Vec::new(),
            session: None,
            age: None,
            listed: Vec::new(),
            folded: folded.into_iter().collect(),
            open: HashSet::new(),
            top: 0,
            reading: None,
            width: Cell::new(30),
            measure: Rc::new(CellMeasure),
            selected: None,
            session_file: None,
            homes,
            loaded: false,
        }
    }

    pub fn set_measure(&mut self, measure: Rc<dyn Measure>) {
        self.measure = measure;
    }

    /// Reads it all again, away from the screen.
    pub fn refresh(&mut self, project: &Path) {
        let (agent, homes, project) = (self.agent, self.homes.clone(), project.to_path_buf());
        let chosen = self.session_file.clone().filter(|p| p.is_file());
        let (done, reading) = mpsc::channel();
        std::thread::spawn(move || {
            let context = discover(agent, &project, &homes);
            let checks = checks(agent, &project, &context);
            let chosen = chosen.and_then(|path| {
                let when = std::fs::metadata(&path).ok()?.modified().ok()?;
                Some((path, when))
            });
            let found = (agent == Agent::Claude)
                .then(|| chosen.or_else(|| latest_transcript(&homes, &project)));
            let (session, age) = match found.flatten() {
                Some((path, when)) => (read_session(&path).ok(), Some(when)),
                None => (None, None),
            };
            let _ = done.send(Read {
                agent,
                context,
                checks,
                session,
                age,
            });
        });
        self.reading = Some(reading);
    }

    /// Takes in what was read. Returns true if it changed anything.
    pub fn poll(&mut self) -> bool {
        let Some(read) = self.reading.as_ref().and_then(|r| r.try_recv().ok()) else {
            return false;
        };
        self.reading = None;
        if read.agent != self.agent {
            return false;
        }
        self.listed = read
            .session
            .as_ref()
            .map(|s| listed_skills(&s.skills, &read.context.items))
            .unwrap_or_default();
        self.context = read.context;
        self.checks = read.checks;
        self.session = read.session;
        self.age = read.age;
        self.loaded = true;
        true
    }

    /// Shows the agent after this one.
    pub fn next_agent(&mut self, project: &Path) {
        let at = Agent::ALL
            .iter()
            .position(|a| *a == self.agent)
            .unwrap_or(0);
        self.show(Agent::ALL[(at + 1) % Agent::ALL.len()], project);
    }

    pub fn show(&mut self, agent: Agent, project: &Path) {
        if agent != self.agent {
            self.agent = agent;
            self.top = 0;
            self.context = Context::default();
            self.checks.clear();
            self.session = None;
            self.listed.clear();
            self.loaded = false;
        }
        self.refresh(project);
    }

    pub fn scroll_by(&mut self, delta: isize) {
        let most = self.lines().len().saturating_sub(1);
        self.top = self.top.saturating_add_signed(delta).min(most);
    }

    /// Cells a check's words have: the panel less its sides and the mark.
    fn check_width(&self) -> f32 {
        self.width.get().saturating_sub(SIDE * 2 + 3).max(12) as f32
    }

    /// A check's words in rows as wide as they're drawn, at most `CHECK_ROWS` of them.
    fn check_text(&self, check: &Check) -> Vec<String> {
        let (width, measure) = (self.check_width(), self.measure.as_ref());
        let mut rows = wrap(&check.message, width, measure);
        if rows.len() > CHECK_ROWS {
            let rest = rows[CHECK_ROWS - 1..].join(" ");
            rows.truncate(CHECK_ROWS - 1);
            rows.push(elide(&rest, width, measure));
        }
        rows
    }

    fn check_rows(&self) -> Vec<Line> {
        let each = self.checks.iter().enumerate();
        let rows = each.map(|(i, c)| (0..self.check_text(c).len()).map(move |p| Line::Check(i, p)));
        rows.flatten().collect()
    }

    /// Whether a section shows only its heading. One with nothing in it does until opened.
    fn is_folded(&self, section: Section, count: usize) -> bool {
        if count == 0 && section != Section::Checks {
            !self.open.contains(&section)
        } else {
            self.folded.contains(&section)
        }
    }

    /// The file a row stands for, which stays marked once opened.
    fn path_of(&self, line: &Line) -> Option<&Path> {
        match line {
            Line::Instruction(i) => self.context.instructions.get(*i).map(|f| f.path.as_path()),
            Line::Item(i) => self.context.items.get(*i).map(|f| f.path.as_path()),
            Line::File(i) => self
                .session
                .as_ref()?
                .files
                .get(*i)
                .map(|f| f.path.as_path()),
            _ => None,
        }
    }

    fn items(&self, section: Section) -> Vec<usize> {
        let wanted = |kind: ItemKind| match section {
            Section::Skills => matches!(
                kind,
                ItemKind::Skill | ItemKind::Subagent | ItemKind::Extension
            ),
            Section::Commands => kind == ItemKind::Command,
            Section::Hooks => kind == ItemKind::Hook,
            Section::Mcp => kind == ItemKind::Mcp,
            _ => false,
        };
        let items = self.context.items.iter().enumerate();
        items
            .filter(|(_, i)| wanted(i.kind))
            .map(|(n, _)| n)
            .collect()
    }

    fn lines(&self) -> Vec<Line> {
        let mut lines = vec![Line::Title, Line::Agent];
        if self.session.as_ref().is_some_and(|s| s.breakdown.total > 0) {
            lines.extend([Line::Meter, Line::Bar]);
        }
        lines.extend([Line::Note, Line::Gap]);
        if !self.loaded {
            return lines;
        }
        let section = |lines: &mut Vec<Line>, section: Section, rows: Vec<Line>| {
            if section == Section::Checks && rows.is_empty() {
                return;
            }
            // A check wrapped over rows is still one check: count and cut whole ones.
            let starts = |line: &Line| !matches!(line, Line::Check(_, part) if *part > 0);
            let count = rows.iter().filter(|l| starts(l)).count();
            let folded = self.is_folded(section, count);
            // Folded headings stack, as VS Code's do.
            if folded
                && matches!(lines.as_slice(), [.., Line::Heading(s, c), Line::Gap] if self.is_folded(*s, *c))
            {
                lines.pop();
            }
            lines.push(Line::Heading(section, count));
            if !folded {
                if rows.is_empty() {
                    lines.push(Line::Empty(section));
                }
                let short = if section == Section::Checks {
                    SHORT_CHECKS
                } else {
                    SHORT_LIST
                };
                let all = self.open.contains(&section) || count <= short + 1;
                let shown = if all { count } else { short };
                let mut seen = 0;
                lines.extend(rows.into_iter().take_while(|line| {
                    seen += usize::from(starts(line));
                    seen <= shown
                }));
                if count > shown {
                    lines.push(Line::More(section, count - shown));
                }
            }
            lines.push(Line::Gap);
        };
        section(&mut lines, Section::Checks, self.check_rows());
        let instructions = (0..self.context.instructions.len())
            .map(Line::Instruction)
            .collect();
        section(&mut lines, Section::Instructions, instructions);
        let mut skills: Vec<Line> = self
            .items(Section::Skills)
            .into_iter()
            .map(Line::Item)
            .collect();
        skills.extend((0..self.listed.len()).map(Line::Listed));
        section(&mut lines, Section::Skills, skills);
        if self.agent == Agent::Claude {
            let files = self.session.as_ref().map_or(0, |s| s.files.len());
            section(
                &mut lines,
                Section::Read,
                (0..files).map(Line::File).collect(),
            );
        }
        // Folded by default, so last: open sections don't sit between closed ones.
        for kind in [Section::Commands, Section::Hooks, Section::Mcp] {
            section(
                &mut lines,
                kind,
                self.items(kind).into_iter().map(Line::Item).collect(),
            );
        }
        lines
    }

    pub fn hit(&self, area: Rect, _x: u16, y: u16) -> Option<Hit> {
        self.width.set(area.w);
        let row = y.checked_sub(area.y)? as usize + self.top;
        match self.lines().into_iter().nth(row)? {
            Line::Agent => Some(Hit::Agent),
            Line::Heading(section, _) => Some(Hit::Fold(section)),
            Line::More(section, _) => Some(Hit::More(section)),
            Line::Check(i, _) => {
                let check = self.checks.get(i)?;
                Some(Hit::Open(check.path.clone(), check.line))
            }
            Line::Instruction(i) => {
                let file = self.context.instructions.get(i)?;
                if file.exists() {
                    Some(Hit::Open(file.path.clone(), None))
                } else {
                    Some(Hit::Say(format!(
                        "{} doesn't exist yet",
                        file.path.display()
                    )))
                }
            }
            Line::Item(i) => {
                let item = self.context.items.get(i)?;
                Some(Hit::Open(item.path.clone(), item.line))
            }
            Line::Listed(i) => {
                let (name, _) = self.listed.get(i)?;
                Some(Hit::Say(format!(
                    "{name} comes from a plugin, so it has no file here"
                )))
            }
            Line::File(i) => {
                let file = self.session.as_ref()?.files.get(i)?;
                Some(Hit::Open(file.path.clone(), None))
            }
            _ => None,
        }
    }

    /// Acts on a click that changes only the panel. Returns what's left for the app to do.
    pub fn click(&mut self, hit: Hit, project: &Path) -> Option<Hit> {
        match hit {
            Hit::Agent => self.next_agent(project),
            Hit::Fold(section) => {
                let count = self.lines().into_iter().find_map(|line| match line {
                    Line::Heading(s, count) if s == section => Some(count),
                    _ => None,
                });
                let set = if count == Some(0) && section != Section::Checks {
                    &mut self.open
                } else {
                    &mut self.folded
                };
                if !set.remove(&section) {
                    set.insert(section);
                }
            }
            Hit::More(section) => {
                self.open.insert(section);
            }
            other => {
                if let Hit::Open(path, _) = &other {
                    self.selected = Some(path.clone());
                }
                return Some(other);
            }
        }
        None
    }

    pub fn draw(
        &self,
        grid: &mut Grid,
        area: Rect,
        project: &Path,
        hover: Option<(u16, u16)>,
        theme: &Theme,
    ) {
        self.width.set(area.w);
        grid.fill(area, theme.sidebar);
        let measure = self.measure.as_ref();
        let quiet = Style {
            fg: theme.gutter.fg,
            ..theme.sidebar
        };
        let tone = |fg: Rgb| Style {
            fg,
            ..theme.sidebar
        };
        let left = area.x + SIDE;
        let right = area.right().saturating_sub(SIDE);
        let shown: Vec<Line> = self
            .lines()
            .into_iter()
            .skip(self.top)
            .take(area.h as usize)
            .collect();
        let hover_row = hover
            .filter(|h| area.contains(h.0, h.1))
            .map(|h| (h.1 - area.y) as usize);
        // A check wrapped over rows lights as one, as it opens as one.
        let hovered_check = hover_row.and_then(|r| match shown.get(r) {
            Some(Line::Check(i, _)) => Some(*i),
            _ => None,
        });
        for (row, line) in shown.iter().enumerate() {
            let y = area.y + row as u16;
            let whole = Rect::new(area.x, y, area.w, 1);
            let hovered = hover_row == Some(row);
            let clickable = !matches!(
                line,
                Line::Title
                    | Line::Meter
                    | Line::Bar
                    | Line::Note
                    | Line::Gap
                    | Line::Empty(_)
                    | Line::Agent
                    | Line::Check(..)
            );
            let selected = self.selected.is_some()
                && self.path_of(line) == self.selected.as_deref()
                && self.path_of(line).is_some();
            if (hovered && clickable) || selected {
                let wash = if selected {
                    theme.row_active
                } else {
                    theme.row_hover
                };
                grid.panel(Bounds::inset(whole, 0.4, 0.06), 14, Some(wash), None);
            }
            match line {
                Line::Title => {
                    crate::palette::sidebar_title(grid, whole, "Agent Context", theme);
                }
                Line::Agent => self.draw_agent(grid, (left, right, y), theme),
                Line::Meter => {
                    let Some(session) = &self.session else {
                        continue;
                    };
                    let b = session.breakdown;
                    let window = session.window();
                    let figure = format!(
                        "{} of {} · {}%",
                        tokens_of(b.total),
                        tokens_of(window),
                        b.total * 100 / window.max(1)
                    );
                    // The figure matters more than its name: drop "Context" when both don't fit.
                    let font = Font::default();
                    let named = measure.width("Context", font) + GAP + measure.width(&figure, font);
                    if named <= (right - left) as f32 {
                        grid.label(left, y, "Context", quiet, right);
                        let at = left as f32 + measure.width("Context", font) + GAP;
                        grid.label_at(at, at.ceil() as u16, y, &figure, theme.sidebar, right, 100);
                    } else {
                        grid.label(left, y, &figure, theme.sidebar, right);
                    }
                }
                Line::Bar => {
                    let Some(session) = &self.session else {
                        continue;
                    };
                    self.draw_bar(
                        grid,
                        Rect::new(left, y, right.saturating_sub(left), 1),
                        session,
                        theme,
                    );
                }
                Line::Note => {
                    let note = match &self.session {
                        Some(s) if s.breakdown.total > 0 => format!(
                            "{} at the start · {} since",
                            tokens_of(s.breakdown.before()),
                            tokens_of(s.breakdown.conversation())
                        ),
                        _ if !self.loaded => "Reading…".into(),
                        _ => format!("What {} would load here", self.agent.name()),
                    };
                    grid.label(left, y, &note, quiet, right);
                }
                Line::Gap => {}
                Line::Heading(section, count) => {
                    let warn =
                        *section == Section::Checks && self.checks.iter().any(|c| c.kind.warns());
                    let number = if warn {
                        theme.git_modified
                    } else {
                        theme.gutter.fg
                    };
                    let shown = count.to_string();
                    crate::palette::section_heading(
                        grid,
                        (left + 1, right, y),
                        (section.title(), Some((&shown, number))),
                        Some(self.is_folded(*section, *count)),
                        theme,
                    );
                }
                Line::Empty(section) => {
                    grid.label(left + 1, y, &section.empty(self.agent), quiet, right);
                }
                Line::More(_, more) => {
                    let link = tone(theme.accent);
                    grid.label(left + 1, y, &format!("Show {more} more"), link, right);
                }
                Line::Check(i, part) => {
                    let Some(check) = self.checks.get(*i) else {
                        continue;
                    };
                    let starts_here = *part == 0 || row == 0;
                    if hovered_check == Some(*i) && starts_here {
                        let rows = shown[row..]
                            .iter()
                            .take_while(|l| matches!(l, Line::Check(j, _) if j == i))
                            .count() as f32;
                        let wash = Bounds::cells(
                            area.x as f32 + 0.4,
                            y as f32 + 0.06,
                            area.w as f32 - 0.8,
                            rows - 0.12,
                        );
                        grid.panel(wash, 14, Some(theme.row_hover), None);
                    }
                    if *part == 0 {
                        let (mark, colour) = if check.kind.warns() {
                            ("▲", theme.git_modified)
                        } else {
                            ("●", theme.accent)
                        };
                        grid.label(left + 1, y, mark, tone(colour), right);
                    }
                    let rows = self.check_text(check);
                    let words = rows.get(*part).map_or("", String::as_str);
                    // The first row reads as the check's title, the rest as its detail.
                    let look = if *part == 0 { theme.sidebar } else { quiet };
                    grid.label(left + 3, y, words, look, right);
                }
                Line::Instruction(i) => {
                    let Some(file) = self.context.instructions.get(*i) else {
                        continue;
                    };
                    let number = self.number_of(*i);
                    self.draw_instruction(grid, (left, right, y), file, number, project, theme);
                }
                Line::Item(i) => {
                    let Some(item) = self.context.items.get(*i) else {
                        continue;
                    };
                    let scope = match item.kind {
                        ItemKind::Subagent => format!("agent · {}", item.scope.label()),
                        _ => item.scope.label().to_string(),
                    };
                    let name = (item.name.as_str(), theme.sidebar);
                    row_of(
                        grid,
                        measure,
                        (left + 1, right, y),
                        name,
                        &item.about,
                        (&scope, quiet),
                        theme,
                    );
                }
                Line::Listed(i) => {
                    let Some((name, about)) = self.listed.get(*i) else {
                        continue;
                    };
                    let name = (name.as_str(), theme.sidebar);
                    row_of(
                        grid,
                        measure,
                        (left + 1, right, y),
                        name,
                        about,
                        ("plugin", quiet),
                        theme,
                    );
                }
                Line::File(i) => {
                    let Some(file) = self.session.as_ref().and_then(|s| s.files.get(*i)) else {
                        continue;
                    };
                    let stale = self
                        .session
                        .as_ref()
                        .is_some_and(|s| s.stale().iter().any(|f| f.path == file.path));
                    draw_read(grid, measure, (left, right, y), file, stale, project, theme);
                }
            }
        }
    }

    /// The agent shown, in a field that says it can be changed.
    fn draw_agent(&self, grid: &mut Grid, (left, right, y): (u16, u16, u16), theme: &Theme) {
        let field = Rect::new(left - 1, y, right + 2 - left, 1);
        grid.field(field, theme.sidebar.bg, theme.input, theme.popup_border);
        let inside = Style {
            bg: theme.input,
            ..theme.sidebar
        };
        let quiet = Style {
            fg: theme.gutter.fg,
            ..inside
        };
        let dot = Style {
            fg: theme.accent,
            ..inside
        };
        grid.label(left, y, "●", dot, right);
        let source = match (&self.session, self.age) {
            (Some(_), Some(when)) => ago(when),
            _ if self.agent == Agent::Claude => "no session yet".into(),
            _ => "files only".into(),
        };
        let edge = right.saturating_sub(2);
        let name = (self.agent.name(), inside);
        row_of(
            grid,
            self.measure.as_ref(),
            (left + 2, edge, y),
            name,
            &source,
            ("", quiet),
            theme,
        );
        let chevron = (right - 2) as f32 + 0.2;
        grid.glyph(chevron, y, Icon::ChevronDown, theme.gutter.fg, "⌄", inside);
    }

    /// The number an instruction file is read as, counting only what the agent finds itself:
    /// imports and files read later share their parent's place.
    fn number_of(&self, index: usize) -> Option<usize> {
        let file = self.context.instructions.get(index)?;
        if file.imported_by.is_some() || file.later.is_some() || !file.exists() {
            return None;
        }
        let before = self.context.instructions[..index].iter();
        Some(
            before
                .filter(|f| f.imported_by.is_none() && f.later.is_none() && f.exists())
                .count()
                + 1,
        )
    }

    /// Name, then whose it is, then its size: the same columns on every row.
    fn draw_instruction(
        &self,
        grid: &mut Grid,
        (left, right, y): (u16, u16, u16),
        file: &Instruction,
        number: Option<usize>,
        project: &Path,
        theme: &Theme,
    ) {
        let quiet = Style {
            fg: theme.gutter.fg,
            ..theme.sidebar
        };
        // An import's arrow sits under its parent's name.
        let indent = file.depth as u16 * NUMBER;
        let x = left + 1 + indent;
        if let Some(n) = number {
            grid.label(x, y, &n.to_string(), quiet, right);
        }
        if file.imported_by.is_some() {
            let arrow = Style {
                fg: theme.accent,
                ..theme.sidebar
            };
            grid.label(x, y, "↳", arrow, right);
        }
        let name = display_name(&file.path, project);
        let warn = file.limit.is_some_and(|l| file.lines() > l);
        let (look, tail) = if !file.exists() {
            (quiet, "not there".to_string())
        } else {
            let lines = file.lines();
            let size = if lines == 1 {
                "1 line".into()
            } else {
                format!("{lines} lines")
            };
            (theme.sidebar, size)
        };
        let detail = if file.later.is_some() {
            "read later"
        } else if file.imported_by.is_some() {
            ""
        } else {
            scope_label(file.scope)
        };
        let tail_look = Style {
            fg: if warn {
                theme.git_modified
            } else {
                theme.gutter.fg
            },
            ..theme.sidebar
        };
        row_of(
            grid,
            self.measure.as_ref(),
            (x + NUMBER, right, y),
            (&name, look),
            detail,
            (&tail, tail_look),
            theme,
        );
    }

    fn draw_bar(&self, grid: &mut Grid, area: Rect, session: &Session, theme: &Theme) {
        let b = session.breakdown;
        let window = session.window().max(1) as f32;
        let parts = [
            (b.system + b.tools, theme.gutter.fg),
            (b.instructions, theme.accent),
            (b.listings + b.mcp, theme.kind),
            (b.files, theme.function),
            (b.output + b.messages, theme.string),
        ];
        // Paint, as the cell tier has the figure above it to go by.
        let width = area.w as f32;
        let (top, tall) = (area.y as f32 + BAR_TOP, BAR_HEIGHT);
        let track = Bounds::cells(area.x as f32, top, width, tall);
        grid.panel(track, 0, Some(theme.border), None);
        let mut x = area.x as f32;
        for (tokens, colour) in parts {
            let cells = (tokens as f32 / window * width).min(area.right() as f32 - x);
            if cells >= 0.05 {
                grid.panel(Bounds::cells(x, top, cells, tall), 0, Some(colour), None);
            }
            x += cells;
        }
        // Rounded ends, cut from whatever reaches them.
        grid.corners(track, 30, theme.sidebar.bg, ALL_CORNERS);
    }
}

/// One list row: a name, a quiet detail a space after it where there's room, and a tail at
/// the right. Each gives way, cut with "…", before the tail does.
fn row_of(
    grid: &mut Grid,
    measure: &dyn Measure,
    (x, right, y): (u16, u16, u16),
    (name, look): (&str, Style),
    detail: &str,
    (tail, tail_look): (&str, Style),
    theme: &Theme,
) {
    let font = Font::default();
    let tail_width = if tail.is_empty() {
        0.0
    } else {
        measure.width(tail, font) + 1.0
    };
    let edge = right as f32 - tail_width;
    let name = elide(name, edge - x as f32, measure);
    grid.label(x, y, &name, look, edge.floor() as u16);
    let at = x as f32 + measure.width(&name, font) + GAP;
    if !detail.is_empty() && edge - at >= ROOM {
        let quiet = Style {
            fg: theme.gutter.fg,
            ..look
        };
        let detail = elide(detail, edge - at, measure);
        grid.label_at(
            at,
            at.ceil() as u16,
            y,
            &detail,
            quiet,
            edge.floor() as u16,
            100,
        );
    }
    if !tail.is_empty() {
        grid.label_right(right, y, tail, tail_look, 100);
    }
}

/// `text` cut at its end with "…" until it fits `room` cells.
fn elide(text: &str, room: f32, measure: &dyn Measure) -> String {
    let font = Font::default();
    if measure.width(text, font) <= room {
        return text.to_string();
    }
    let mut chars: Vec<char> = text.chars().collect();
    while chars.pop().is_some() {
        let tried = format!("{}…", chars.iter().collect::<String>().trim_end());
        if measure.width(&tried, font) <= room {
            return tried;
        }
    }
    String::new()
}

fn scope_label(scope: Scope) -> &'static str {
    scope.label()
}

/// A path short enough for a row: inside the project, from its top; elsewhere, its name.
fn display_name(path: &Path, project: &Path) -> String {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let parent = path.parent().unwrap_or(Path::new(""));
    if parent == project {
        return name;
    }
    if let Ok(rel) = path.strip_prefix(project) {
        return rel.display().to_string();
    }
    // Outside the project, the scope beside it says whose it is.
    name
}

/// A file the session read: its name, its folder a space after, how it was used at the right.
fn draw_read(
    grid: &mut Grid,
    measure: &dyn Measure,
    (left, right, y): (u16, u16, u16),
    file: &cellix_agent::context::FileRead,
    stale: bool,
    project: &Path,
    theme: &Theme,
) {
    let name = file
        .path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut how = Vec::new();
    if file.reads > 0 {
        how.push(format!("{}×", file.reads));
    }
    if file.edits > 0 {
        how.push("edited".to_string());
    }
    if stale {
        how.push("changed".to_string());
    }
    if file.tokens > 0 {
        how.push(format!("≈{}", tokens_of(file.tokens)));
    }
    let tail = how.join(" · ");
    let folder = match file.path.parent() {
        Some(parent) if parent != project => display_name(parent, project),
        _ => String::new(),
    };
    let colour = if stale || file.reads > 2 {
        theme.git_modified
    } else {
        theme.gutter.fg
    };
    let tail_look = Style {
        fg: colour,
        ..theme.sidebar
    };
    let name = (name.as_str(), theme.sidebar);
    row_of(
        grid,
        measure,
        (left + 1, right, y),
        name,
        &folder,
        (&tail, tail_look),
        theme,
    );
}

/// Skills the session's listing names that no file here defines, as a plugin's.
fn listed_skills(listing: &str, items: &[Item]) -> Vec<(String, String)> {
    let known: HashSet<&str> = items
        .iter()
        .filter(|i| i.kind == ItemKind::Skill)
        .map(|i| i.name.as_str())
        .collect();
    listing
        .lines()
        .filter_map(|l| l.strip_prefix("- "))
        .filter_map(|l| l.split_once(": "))
        .map(|(name, about)| (name.trim(), about.trim()))
        .filter(|(name, _)| {
            let bare = name.rsplit(':').next().unwrap_or(name);
            !known.contains(name) && !known.contains(bare)
        })
        .map(|(name, about)| (name.to_string(), about.to_string()))
        .collect()
}

/// Tokens as the transcript page and the tabs write them: 940, 41k, 3.1M.
fn tokens_of(n: usize) -> String {
    tokens(n as u64)
}

fn ago(when: SystemTime) -> String {
    let secs = SystemTime::now()
        .duration_since(when)
        .map_or(0, |d| d.as_secs());
    match secs {
        0..60 => "just now".into(),
        60..3600 => format!("{} min ago", secs / 60),
        3600..86_400 => format!("{} h ago", secs / 3600),
        _ => format!("{} days ago", secs / 86_400),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_long_check_takes_two_rows_and_ends_in_an_ellipsis() {
        let panel = ContextPanel::new(Homes::default());
        panel.width.set(24);
        let check = Check {
            kind: cellix_agent::context::CheckKind::Large,
            message: "CLAUDE.md is 61,000 characters, which the agent reads in full every turn"
                .into(),
            path: PathBuf::from("CLAUDE.md"),
            line: None,
            other: None,
        };
        let rows = panel.check_text(&check);
        assert_eq!(rows.len(), 2, "{rows:?}");
        assert!(rows[1].ends_with('…'), "{rows:?}");
        assert!(rows.iter().all(|r| r.chars().count() <= 24 - 7), "{rows:?}");
    }

    #[test]
    fn a_name_too_long_for_its_row_ends_in_an_ellipsis() {
        assert_eq!(elide("docs/STYLE.md", 13.0, &CellMeasure), "docs/STYLE.md");
        assert_eq!(elide("docs/STYLE.md", 8.0, &CellMeasure), "docs/ST…");
    }

    #[test]
    fn counts_read_short() {
        assert_eq!(tokens_of(940), "940");
        assert_eq!(tokens_of(118_400), "118k");
        assert_eq!(tokens_of(1_000_000), "1M");
    }

    #[test]
    fn plugin_skills_are_those_with_no_file_here() {
        let items = vec![Item {
            kind: ItemKind::Skill,
            name: "terminal-browser".into(),
            about: String::new(),
            scope: Scope::User,
            path: PathBuf::from("/x/SKILL.md"),
            line: None,
        }];
        let listing = "- terminal-browser: A browser\n- cellix:cellix-review: Read comments\n";
        assert_eq!(
            listed_skills(listing, &items),
            [(
                "cellix:cellix-review".to_string(),
                "Read comments".to_string()
            )]
        );
    }
}
