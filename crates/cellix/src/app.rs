use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::io;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::mpsc::Sender;

use cellix_core::{Buffer, Selection, Workspace};
use cellix_input::{
    Command, Event, Format, Key, KeyEvent, MouseEvent, MouseKind, Vim, VimAction, VimMode,
    simple_keymap,
};
use cellix_lang::Registry;
use cellix_lang::complete::{self, Candidate, Source};
use cellix_lang::lsp::{Client, Diagnostic, LspEvent, TextEdit};
use cellix_render::{
    ALL_CORNERS, Bounds, CellMeasure, Font, Grid, Icon, Measure, Rect, Rgb, Style, TOP_LEFT,
    TOP_RIGHT,
};
use cellix_review::{
    Comment, Reply, Thread, append_comments, blame_line, branches, changed_files, create_branch,
    current_branch, head_text, restore, switch_branch, turns, worktrees,
};

use cellix_agent::hook::comments_message;
use cellix_agent::ide::{IdeCall, IdeServer};
use cellix_agent::{Decision, Request, SharedComments, TestRun};
use cellix_doc::table::{Align, Edit as TableEdit};

use serde_json::{Value, json};

use crate::agents::{Agents, Hit as AgentHit};
use crate::dock::{Dock, Session, TabHit};
use crate::editor::{Editor, Marks};
use crate::explorer::Explorer;
use crate::find::Find;
use crate::menu::{Action as MenuAction, Item as MenuItem, Menu};
use crate::neovim::{self, Neovim};
use crate::palette::{COMMANDS, Input, Item, Palette, input, popup, raised};
use crate::panels::{MathPanel, SYMBOLS, TablePicker};
use crate::rendered;
use crate::review::{Hit, Review};
use crate::search::{self, Field, Search};
use crate::settings::{SERVERS, Settings, default_server, formatter};
use crate::settings_page::{Control, Found, SettingsPage, Values};
use crate::theme::Theme;
use crate::threads::{self, Hit as ThreadHit};
use crate::toolbar::{self, Tool};

const ACTIVITY_WIDTH: u16 = 5;
/// Activity bar buttons, with what each one runs.
const ACTIVITY: [(Icon, Command); 5] = [
    (Icon::Files, Command::ToggleSidebar),
    (Icon::Search, Command::FindInProject),
    (Icon::Git, Command::OpenDiff),
    (Icon::Agents, Command::ShowAgents),
    (Icon::Settings, Command::OpenSettings),
];
/// Cells each activity icon spans.
const ACTIVITY_ICON: (u8, u8) = (3, 2);
/// Rows from one activity icon to the next.
const ACTIVITY_STEP: u16 = 3;
const APPROVE: &str = "[Approve]";
const FIND_WIDTH: u16 = 46;
const CONFIRM_WIDTH: u16 = 56;
const CONFIRM_HEIGHT: u16 = 6;
/// Columns around a dialog button's label.
const CONFIRM_PADDING: usize = 4;
const PLAN_BAR_WIDTH: u16 = 48;
/// Corners as percentages of a row's height.
const TAB_RADIUS: u8 = 22;
/// Columns of a tab that aren't its name: margins, the gap and the close button.
const TAB_PADDING: u16 = 5;
const PILL_RADIUS: u8 = 30;
/// What the explorer's status holds for a folder with changes inside, not a file.
const DIRECTORY_MARK: char = '•';
const BADGE_SIZE: u8 = 62;
const SUGGEST_WIDTH: u16 = 42;
const FIND_BUTTON: u16 = 3;
const FIND_BUTTONS: [&str; 3] = ["↑", "↓", "×"];
const SEND_BACK: &str = "[Send back]";
/// The card that offers a language's support.
const OFFER_WIDTH: u16 = 46;
const OFFER_HEIGHT: u16 = 4;
const OFFER_BUTTON_ROW: u16 = 3;
const OFFER_BUTTONS: [&str; 3] = ["Install", "Not now", "Never"];
/// A tooltip's height in rows, and the size of its text.
const TOOLTIP_HEIGHT: f32 = 1.22;
const TOOLTIP_SIZE: u8 = 92;
/// What each button is called, in order, since they're only icons and marks.
const FIND_NAMES: [&str; 3] = ["Previous match", "Next match", "Close"];
const RAIL_NAMES: [&str; 5] = ["Files", "Search", "Review Changes", "Agents", "Settings"];
/// Below this width, opening the side pane puts the sidebar away.
const CROWDED_WIDTH: u16 = 120;
const SIDEBAR_WIDTH: u16 = 28;
const SIDEBAR_MIN_WIDTH: u16 = 20;
/// Below this width the window is a side pane, as beside an agent: chrome gives way to
/// the code, and the sidebar becomes a drawer that opens over it.
const COMPACT_WIDTH: u16 = 70;
/// Columns for the menu button at the start of the tab bar, in the compact layout.
const MENU_BUTTON: u16 = 3;
/// Columns the status bar gives a branch's name, so a long one can't crowd the message.
const STATUS_BRANCH_WIDTH: u16 = 28;
/// Spaces per indent, which is what the status bar shows and formatters are told.
const TAB_SIZE: usize = 4;
const WHEEL_LINES: isize = 3;
/// Wheel events closer together than this are a trackpad's stream.
const WHEEL_STREAM_GAP: std::time::Duration = std::time::Duration::from_millis(60);
/// How close together presses must be to count as one double or triple click.
const MULTI_CLICK: std::time::Duration = std::time::Duration::from_millis(400);
/// Ticks between explorer refreshes, so files an agent creates appear on their own.
const EXPLORER_REFRESH_TICKS: u32 = 4;

/// Which part of the screen a press started in, so a drag stays with it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Drag {
    None,
    Text,
    Scrollbar,
    /// The side pane's edge.
    Dock,
    /// A selection in a session.
    Session,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum PromptAction {
    NewFile(PathBuf),
    NewFolder(PathBuf),
    Rename(PathBuf),
    Comment(String, usize),
    RenameSymbol,
    Commit,
    GoToLine,
    SendBackPlan,
    NewBranch,
    Image,
    Reply(u32),
}

struct Prompt {
    label: String,
    input: String,
    action: PromptAction,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SidebarView {
    Explorer,
    Search,
}

/// Suggestions showing at the caret while typing.
struct Suggest {
    items: Vec<Candidate>,
    selected: usize,
}

/// A change Claude Code is waiting on, shown in the review view.
struct Proposal {
    tab: String,
    reply: Sender<Vec<String>>,
}

/// A plan an agent is waiting on an answer for.
struct PendingPlan {
    path: PathBuf,
    reply: Sender<Decision>,
}

/// At most one of these takes input ahead of everything else.
enum Overlay {
    None,
    Palette(Palette),
    Prompt(Prompt),
    Menu(Menu),
    Find(Find),
    Confirm(Confirm),
    TablePicker(TablePicker),
    Math(MathPanel),
}

/// What's left to do after a choice about unsaved changes.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Leaving {
    Quit,
    CloseTab,
}

/// What a dialog's button does.
#[derive(Clone)]
enum Choice {
    Cancel,
    Save(Leaving),
    Discard(Leaving),
    Delete(PathBuf),
    Undo(String),
}

/// A question with buttons, for the keyboard or the mouse.
struct Confirm {
    title: String,
    detail: String,
    /// The first is what Enter does until the selection moves.
    buttons: Vec<(&'static str, Choice)>,
    selected: usize,
}

/// What the system's clipboard holds, where a local tool can read it. Over SSH that would
/// be the server's clipboard, so there Cellix keeps to its own.
fn system_clipboard() -> Option<String> {
    if cfg!(test) || std::env::var_os("SSH_CONNECTION").is_some() {
        return None;
    }
    let tools: [&[&str]; 3] = [
        &["pbpaste"],
        &["wl-paste", "-n"],
        &["xclip", "-o", "-sel", "c"],
    ];
    tools.iter().find_map(|tool| {
        let output = std::process::Command::new(tool[0])
            .args(&tool[1..])
            .output()
            .ok()?;
        let text = String::from_utf8(output.stdout).ok()?;
        (output.status.success() && !text.is_empty()).then_some(text)
    })
}

/// Runs a language server's install command, and says how it went.
fn install_server(index: usize) -> String {
    let Some(server) = SERVERS.get(index) else {
        return String::new();
    };
    install_with(server.install, server.command[0])
}

fn install_with(command: &str, tool: &str) -> String {
    // Not a login shell, whose profile would swap the PATH Cellix was started with.
    let done = std::process::Command::new("sh")
        .args(["-c", command])
        .stdin(std::process::Stdio::null())
        .output();
    match done {
        Ok(done) if done.status.success() => format!("Installed {tool}"),
        Ok(done) => {
            let said = String::from_utf8_lossy(&done.stderr);
            let first = said.lines().rfind(|l| !l.trim().is_empty());
            format!("Couldn't: {}", first.unwrap_or("the install failed").trim())
        }
        Err(e) => format!("Couldn't: {e}"),
    }
}

/// Runs text through a formatter on stdin, and returns what comes out.
fn format_with(tool: &[&str], path: &str, text: &str) -> Result<String, String> {
    use std::io::Write;
    use std::process::{Command as Process, Stdio};
    let (program, args) = tool.split_first().ok_or("No formatter")?;
    let mut command = Process::new(program);
    command.args(args);
    if args.last() == Some(&"--stdin-filepath") {
        command.arg(path);
    }
    let child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    let Ok(mut child) = child else {
        return Err(format!("Install {program} to format this file"));
    };
    let mut stdin = child.stdin.take().ok_or("No formatter input")?;
    let input = text.to_string();
    // On a thread, so a big file can't fill the pipe while nothing reads the other end.
    let writer = std::thread::spawn(move || stdin.write_all(input.as_bytes()));
    let done = child.wait_with_output().map_err(|e| e.to_string())?;
    let _ = writer.join();
    if done.status.success() {
        return String::from_utf8(done.stdout).map_err(|e| e.to_string());
    }
    let said = String::from_utf8_lossy(&done.stderr);
    let first = said.lines().next().unwrap_or("it failed");
    Err(format!("{program}: {first}"))
}

/// Saves a setting, and returns what to say if that failed.
fn store(key: &str, value: serde_json::Value) -> String {
    Settings::store(key, value).map_or_else(|e| format!("Couldn't save: {e}"), |()| String::new())
}

/// Runs a program and waits for it, for short commands whose outcome matters.
/// Runs a command to its end, for when starting isn't the same as working.
fn ran_well(command: &[String], arg: &str) -> bool {
    let Some((program, fixed)) = command.split_first() else {
        return false;
    };
    std::process::Command::new(program)
        .args(fixed)
        .arg(arg)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// All application state. It has no terminal: tests send events and read the grid.
pub struct App {
    pub workspace: Workspace,
    explorer: Explorer,
    editors: Vec<Editor>,
    /// The editor each side-by-side pane shows.
    panes: Vec<usize>,
    focus: usize,
    show_sidebar: bool,
    overlay: Overlay,
    size: (u16, u16),
    theme: Theme,
    registry: Rc<Registry>,
    drag: Drag,
    message: RefCell<String>,
    clipboard: String,
    /// Text for the terminal's clipboard, sent with OSC 52 by the terminal loop.
    pub outgoing_clipboard: Option<String>,
    ticks: u32,
    review: Option<Review>,
    /// Shared with the socket thread, so agents can read comments at any time.
    pub comments: SharedComments,
    /// With `--wait`: comments on this file are appended to it on exit, so an agent that
    /// only reads the file back still gets them.
    pub wait_file: Option<PathBuf>,
    pub settings: Settings,
    /// Set in Vim mode. Other key handling stays the same underneath it.
    vim: Option<Vim>,
    /// Running servers, by language.
    servers: HashMap<&'static str, Client>,
    diagnostics: HashMap<PathBuf, Vec<Diagnostic>>,
    /// The file and revision a rename was asked against. Its edits are only valid for
    /// that text, so one runs at a time and a stale reply is dropped.
    pending_rename: Option<(PathBuf, u64)>,
    branch: Option<String>,
    measure: Rc<dyn Measure>,
    agents: Agents,
    /// Set in the `neovim` keymap, when `nvim` could start.
    neovim: Option<Neovim>,
    /// What Neovim last saw of the open file: path, revision and cursor.
    neovim_file: Option<(PathBuf, u64, usize)>,
    neovim_mode: String,
    /// The `:` line being typed.
    neovim_line: String,
    last_wheel: Option<std::time::Instant>,
    /// Lines a wheel notch moves. Set by the terminal loop for the tier in use.
    pub wheel_lines: isize,
    suggest: Option<Suggest>,
    /// The language server's latest answer, kept to rank against each new letter.
    server_words: Vec<String>,
    /// Set when suggestions were asked for, so an empty prefix still gets them.
    suggest_asked: bool,
    /// Running as a pane inside Claude Code.
    pub in_pane: bool,
    /// Below this width the sidebar becomes a drawer. A pane beside an agent sets it higher.
    pub compact_width: u16,
    /// Why a pane should come forward, for the mod.
    attention: Option<&'static str>,
    /// What the wheel has moved since the last frame.
    scrolled: Option<(Rect, i32)>,
    /// The server Claude Code connects to. Set by the terminal loop.
    pub ide: Option<IdeServer>,
    proposal: Option<Proposal>,
    /// Diffs waiting their turn, because only one shows at a time.
    queued_diffs: VecDeque<IdeCall>,
    /// The last selection told to Claude Code, to send only changes.
    told_selection: Option<Value>,
    plan: Option<PendingPlan>,
    /// The side pane, where agents and shells run.
    dock: Dock,
    /// The comments on this project, kept with it.
    threads: Vec<Thread>,
    /// The file couldn't be read, so saving would lose what's in it.
    threads_broken: bool,
    tests: Vec<TestRun>,
    /// Where the pointer is, for hover highlights.
    hover: Option<(u16, u16)>,
    /// The last press in text, with where it was and how many it made in a row.
    last_click: Option<(std::time::Instant, u16, u16, u8)>,
    /// The Settings page, when it stands in place of the file.
    settings_page: Option<SettingsPage>,
    /// What the terminal said of itself, kept for when the theme changes.
    terminal_look: (Option<bool>, Option<(u8, u8, u8)>),
    /// A change to smooth scrolling for the terminal loop to pick up.
    smooth_change: Option<bool>,
    /// What the sidebar shows, as in VS Code's activity bar.
    sidebar_view: SidebarView,
    search: Search,
    /// In the compact layout, whether the sidebar is open over the code.
    drawer: bool,
    /// Languages that could have a server here and don't, with the server's index.
    missing_servers: HashMap<&'static str, usize>,
    /// Languages put off until the next run.
    not_now: Vec<String>,
    /// The language being installed from its offer, and how that went.
    offer_install: Option<(&'static str, std::sync::mpsc::Receiver<String>)>,
    /// Replaces a server's install command, so tests never touch the real package managers.
    server_install: Option<String>,
    /// What opens a page when nothing else does.
    system_browser: Vec<String>,
    /// A page on its way to the browser, and whether it got there.
    browser_result: Option<(String, std::sync::mpsc::Receiver<bool>)>,
    /// Set when the user asks to leave.
    pub quit: bool,
}

impl App {
    pub fn new(root: PathBuf, size: (u16, u16)) -> Self {
        let settings = Settings::load(&root);
        let search = Search::new(root.clone());
        let workspace = Workspace::new(root);
        let mut explorer = Explorer::default();
        explorer.refresh(&workspace);
        let threads = cellix_review::threads::load(workspace.root());
        let threads_broken = threads.is_err();
        let threads = threads.unwrap_or_default();
        let mut app = Self {
            workspace,
            explorer,
            editors: Vec::new(),
            panes: vec![0],
            focus: 0,
            show_sidebar: true,
            overlay: Overlay::None,
            size,
            theme: Theme::named(settings.theme.as_deref()),
            registry: Rc::default(),
            drag: Drag::None,
            message: RefCell::new(String::new()),
            clipboard: String::new(),
            outgoing_clipboard: None,
            ticks: 0,
            review: None,
            system_browser: vec![
                if cfg!(target_os = "macos") {
                    "open"
                } else {
                    "xdg-open"
                }
                .to_string(),
            ],
            browser_result: None,
            missing_servers: HashMap::new(),
            not_now: Vec::new(),
            offer_install: None,
            server_install: None,
            comments: SharedComments::default(),
            wait_file: None,
            vim: (settings.keymap.as_deref() == Some("vim")).then(Vim::default),
            neovim: (settings.keymap.as_deref() == Some("neovim"))
                .then(|| Neovim::start(&[]).ok())
                .flatten(),
            neovim_file: None,
            neovim_mode: String::new(),
            neovim_line: String::new(),
            settings,
            servers: HashMap::new(),
            diagnostics: HashMap::new(),
            pending_rename: None,
            branch: None,
            measure: Rc::new(CellMeasure),
            agents: Agents::default(),
            in_pane: false,
            compact_width: COMPACT_WIDTH,
            attention: None,
            suggest: None,
            server_words: Vec::new(),
            suggest_asked: false,
            last_wheel: None,
            wheel_lines: WHEEL_LINES,
            scrolled: None,
            ide: None,
            proposal: None,
            queued_diffs: VecDeque::new(),
            told_selection: None,
            plan: None,
            dock: Dock::default(),
            threads,
            threads_broken,
            tests: Vec::new(),
            hover: None,
            last_click: None,
            settings_page: None,
            terminal_look: (None, None),
            smooth_change: None,
            sidebar_view: SidebarView::Explorer,
            search,
            drawer: false,
            quit: false,
        };
        app.refresh_tree();
        app
    }

    /// Uses this measure for text from now on.
    pub fn set_measure(&mut self, measure: Rc<dyn Measure>) {
        for editor in &mut self.editors {
            editor.set_measure(measure.clone());
        }
        self.measure = measure;
    }

    /// In `auto`, matches the terminal's background. Terminals that don't say stay dark.
    pub fn follow_terminal_theme(&mut self, light: Option<bool>) {
        self.terminal_look.0 = light;
        let chosen = self.settings.theme.as_deref();
        if chosen.is_none() || chosen == Some("auto") {
            let name = if light == Some(true) { "light" } else { "dark" };
            self.theme = Theme::named(Some(name));
        }
    }

    /// Lines the theme up with the terminal, so no seam shows around the app.
    pub fn match_terminal_background(&mut self, background: Option<(u8, u8, u8)>) {
        self.terminal_look.1 = background;
        let chosen = self.settings.theme.as_deref();
        // A chosen theme that agrees with the terminal can sit on it too.
        let light = self.terminal_look.0 == Some(true);
        let agrees = chosen == Some(if light { "light" } else { "dark" });
        let follows = chosen.is_none() || chosen == Some("auto") || agrees;
        if let Some((r, g, b)) = background.filter(|_| follows) {
            self.theme = self.theme.on_background(cellix_render::Rgb(r, g, b));
        }
    }

    fn say(&self, text: impl Into<String>) {
        *self.message.borrow_mut() = text.into();
    }

    fn active(&self) -> usize {
        self.panes[self.focus]
    }

    fn set_active(&mut self, index: usize) {
        self.panes[self.focus] = index;
    }

    fn editor_mut(&mut self) -> Option<&mut Editor> {
        let index = self.active();
        self.editors.get_mut(index)
    }

    fn load_committed(&self, editor: &mut Editor) {
        let Some(path) = editor.buffer.path() else {
            return;
        };
        let root = self.workspace.root();
        let inside = path.strip_prefix(root).unwrap_or(path);
        let text = head_text(root, inside);
        // A file that isn't committed has nothing to compare with, and marking every line
        // as new says nothing.
        editor.committed = (!text.is_empty()).then_some(text);
        editor.refresh_changes();
    }

    pub fn open(&mut self, path: &Path) -> io::Result<()> {
        self.settings_page = None;
        if let Some(i) = self
            .editors
            .iter()
            .position(|e| e.buffer.path() == Some(path))
        {
            self.set_active(i);
            return Ok(());
        }
        let buffer = Buffer::open(path)?;
        let mut editor = Editor::new(buffer, self.measure.clone(), self.registry.clone());
        editor.minimap_on = self.settings.minimap != Some(false);
        self.load_committed(&mut editor);
        self.editors.push(editor);
        self.set_active(self.editors.len() - 1);
        self.attach_server(path);
        Ok(())
    }

    /// Starts this language's server if there is one. A missing server only costs the
    /// language features, so it isn't an error.
    fn attach_server(&mut self, path: &Path) {
        let Some(language) = Registry::language_name(path) else {
            return;
        };
        if !self.servers.contains_key(language) {
            let command = self
                .settings
                .language_servers
                .get(language)
                .cloned()
                .or_else(|| default_server(language));
            let root = self.workspace.root().to_path_buf();
            let custom = self.settings.language_servers.contains_key(language);
            match command.map(|c| Client::start(&c, &root)) {
                Some(Ok(client)) => {
                    self.servers.insert(language, client);
                }
                Some(Err(_)) if !custom => {
                    self.note_missing(language);
                    return;
                }
                _ => return,
            }
        }
        let index = self.active();
        let (Some(editor), Some(client)) =
            (self.editors.get_mut(index), self.servers.get_mut(language))
        else {
            return;
        };
        client.open(path, language, &editor.buffer.text());
        editor.synced = editor.buffer.revision();
    }

    /// Remembers that a language could have a server, so opening it can offer one. Markdown
    /// is left out: a page needs nothing from a server.
    fn note_missing(&mut self, language: &'static str) {
        let server = SERVERS.iter().position(|s| s.languages.contains(&language));
        if let Some(server) = server.filter(|_| language != "Markdown") {
            self.missing_servers.insert(language, server);
        }
    }

    /// The language support to offer for what's on screen, as its name and server.
    fn offer(&self) -> Option<(&'static str, usize)> {
        if self.settings_page.is_some() || self.review.is_some() || self.overlay_is_open() {
            return None;
        }
        let path = self.editors.get(self.active())?.buffer.path()?;
        let language = Registry::language_name(path)?;
        let declined = self.settings.declined_languages.iter();
        if declined.chain(&self.not_now).any(|l| l == language) {
            return None;
        }
        Some((language, *self.missing_servers.get(language)?))
    }

    fn offer_area(&self) -> Rect {
        let sheet = self.sheet_area();
        Rect::new(
            sheet.right().saturating_sub(OFFER_WIDTH + 2),
            sheet.bottom().saturating_sub(OFFER_HEIGHT + 1),
            OFFER_WIDTH,
            OFFER_HEIGHT,
        )
    }

    /// The offer's buttons, as (label, padded cells, start column).
    fn offer_buttons(&self) -> Vec<(&'static str, String, u16)> {
        let area = self.offer_area();
        let mut x = area.x + 2;
        OFFER_BUTTONS
            .iter()
            .map(|label| {
                let padding = " ".repeat(CONFIRM_PADDING / 2);
                let cells = format!("{padding}{label}{padding}");
                let at = x;
                x += cells.chars().count() as u16 + 1;
                (*label, cells, at)
            })
            .collect()
    }

    /// Handles a press on the offer. Returns true if the card took it.
    fn click_offer(&mut self, x: u16, y: u16) -> bool {
        let Some((language, server)) = self.offer() else {
            return false;
        };
        let area = self.offer_area();
        if !area.contains(x, y) {
            return false;
        }
        if self.offer_install.is_some() || y != area.y + OFFER_BUTTON_ROW {
            return true;
        }
        let hit = self
            .offer_buttons()
            .into_iter()
            .find(|(_, cells, at)| x >= *at && x < at + cells.chars().count() as u16);
        match hit.map(|(label, _, _)| label) {
            Some("Install") => {
                let (done, result) = std::sync::mpsc::channel();
                let command = self.server_install.clone();
                std::thread::spawn(move || {
                    let install = command.as_deref().unwrap_or(SERVERS[server].install);
                    let _ = done.send(install_with(install, SERVERS[server].command[0]));
                });
                self.offer_install = Some((language, result));
            }
            Some("Not now") => self.not_now.push(language.to_string()),
            Some("Never") => {
                self.settings.declined_languages.push(language.to_string());
                let declined = serde_json::json!(self.settings.declined_languages);
                let _ = Settings::store("declined_languages", declined);
            }
            _ => {}
        }
        true
    }

    /// Takes in a finished install, and connects the files it can now help.
    fn poll_offer(&mut self) -> bool {
        let finished = self.offer_install.as_ref().and_then(|(language, result)| {
            let note = result.try_recv().ok()?;
            Some((*language, note))
        });
        let Some((language, note)) = finished else {
            return false;
        };
        self.offer_install = None;
        let installed = note.starts_with("Installed");
        self.say(note);
        // Whatever happened, it's been asked and answered.
        self.not_now.push(language.to_string());
        if installed {
            self.missing_servers.remove(language);
            self.attach_open_files();
        }
        true
    }

    fn attach_open_files(&mut self) {
        let paths: Vec<PathBuf> = self
            .editors
            .iter()
            .filter_map(|e| e.buffer.path().map(Path::to_path_buf))
            .collect();
        let active = self.active();
        for (index, path) in paths.iter().enumerate() {
            self.set_active(index);
            self.attach_server(path);
        }
        self.set_active(active.min(self.editors.len().saturating_sub(1)));
    }

    fn draw_offer(&self, grid: &mut Grid) {
        let Some((language, server)) = self.offer() else {
            return;
        };
        let theme = &self.theme;
        let area = self.offer_area();
        crate::palette::raised(grid, area, theme);
        let base = Style {
            fg: theme.text.fg,
            bg: theme.popup,
            ..theme.bar
        };
        let strong = Style { bold: true, ..base };
        let dim = Style {
            fg: theme.popup_dim,
            ..base
        };
        let right = area.right().saturating_sub(1);
        let tool = SERVERS[server].command[0];
        if self.offer_install.is_some() {
            let title = format!("Installing {language} support…");
            grid.label(area.x + 2, area.y + 1, &title, strong, right);
            let detail = format!("You can keep working while {tool} installs.");
            grid.label(area.x + 2, area.y + 2, &detail, dim, right);
            return;
        }
        let title = format!("Install {language} support?");
        grid.label(area.x + 2, area.y, &title, strong, right);
        let detail = format!("{tool} adds completion, errors and rename.");
        grid.label(area.x + 2, area.y + 1, &detail, dim, right);
        let y = area.y + OFFER_BUTTON_ROW;
        for (index, (label, cells, x)) in self.offer_buttons().iter().enumerate() {
            let width = cells.chars().count() as u16;
            let hovered = self
                .hover
                .is_some_and(|h| h.1 == y && h.0 >= *x && h.0 < x + width);
            let fill = match (index, hovered) {
                (0, _) => theme.button,
                (_, true) => theme.popup_hover,
                _ => theme.quiet_button(theme.popup),
            };
            let style = Style {
                fg: if index == 0 {
                    theme.button_text
                } else {
                    theme.text.fg
                },
                bg: fill,
                ..base
            };
            grid.large_button(*x, y, cells, label, fill, style);
        }
    }

    /// Sends edits to servers and handles their replies. Returns true if anything changed.
    pub fn poll(&mut self) -> bool {
        for editor in &mut self.editors {
            if editor.synced == editor.buffer.revision() {
                continue;
            }
            let Some(path) = editor.buffer.path() else {
                continue;
            };
            if let Some(client) =
                Registry::language_name(path).and_then(|language| self.servers.get_mut(language))
            {
                client.change(path, &editor.buffer.text());
            }
            editor.synced = editor.buffer.revision();
        }
        let mut highlighted = false;
        for editor in &self.editors {
            highlighted |= editor.collect_highlights();
        }
        let mut calls = Vec::new();
        if let Some(ide) = &self.ide {
            calls.extend(ide.calls.try_iter());
        }
        let called = !calls.is_empty();
        for call in calls {
            self.ide_call(call);
        }
        self.tell_selection();
        let terminal = self.dock.poll(&self.theme);
        let searched = self.search.poll();
        let reviewed = self.review.as_mut().is_some_and(Review::poll);
        let installed = self.poll_install() | self.poll_browser() | self.poll_offer();
        let mut events = Vec::new();
        for client in self.servers.values() {
            events.extend(client.events.try_iter());
        }
        let changed = highlighted
            || called
            || searched
            || reviewed
            || installed
            || terminal
            || !events.is_empty();
        for event in events {
            self.lsp_event(event);
        }
        changed
    }

    fn file_uri(path: &Path) -> String {
        format!("file://{}", path.display())
    }

    /// A path from Claude Code, which may be absolute, a file URI, or relative to the root.
    fn ide_path(&self, text: &str) -> PathBuf {
        self.workspace
            .root()
            .join(text.strip_prefix("file://").unwrap_or(text))
    }

    fn line_column(editor: &Editor, at: usize) -> Value {
        let rope = editor.buffer.rope();
        let line = rope.char_to_line(at);
        json!({"line": line, "character": at - rope.line_to_char(line)})
    }

    /// The selection in the protocol's shape, or None with no file in view.
    fn ide_selection(&self) -> Option<Value> {
        let editor = self.editors.get(self.active())?;
        let path = editor.buffer.path()?;
        let selection = editor.selections.primary();
        Some(json!({
            "text": editor.selected_text().unwrap_or_default(),
            "filePath": path,
            "fileUrl": Self::file_uri(path),
            "selection": {
                "start": Self::line_column(editor, selection.start()),
                "end": Self::line_column(editor, selection.end()),
                "isEmpty": selection.is_empty(),
            },
        }))
    }

    fn tell_selection(&mut self) {
        let Some(ide) = self.ide.as_ref().filter(|ide| ide.connected()) else {
            return;
        };
        let current = self.ide_selection();
        if current != self.told_selection {
            if let Some(selection) = &current {
                ide.notify("selection_changed", selection.clone());
            }
            self.told_selection = current;
        }
    }

    /// `@`-mentions the selection in Claude Code's prompt.
    fn mention_selection(&mut self) {
        let (Some(ide), Some(editor)) = (&self.ide, self.editors.get(self.active())) else {
            self.say("Claude Code isn't connected");
            return;
        };
        let Some(path) = editor.buffer.path().filter(|_| ide.connected()) else {
            self.say("Claude Code isn't connected");
            return;
        };
        let selection = editor.selections.primary();
        let rope = editor.buffer.rope();
        ide.notify(
            "at_mentioned",
            json!({
                "filePath": path,
                "lineStart": rope.char_to_line(selection.start()),
                "lineEnd": rope.char_to_line(selection.end()),
            }),
        );
        self.say("Sent the selection to Claude Code");
    }

    fn ide_call(&mut self, call: IdeCall) {
        let args = &call.args;
        let text_arg = |name: &str| args[name].as_str().unwrap_or_default().to_string();
        let answer = match call.tool.as_str() {
            "openDiff" => return self.propose(call),
            "openFile" => {
                let path = self.ide_path(&text_arg("filePath"));
                let (start, end) = (text_arg("startText"), text_arg("endText"));
                self.ide_open(&path, &start, &end)
            }
            "getCurrentSelection" | "getLatestSelection" => {
                let none = json!({"success": false, "message": "No active editor"});
                self.ide_selection().unwrap_or(none).to_string()
            }
            "getOpenEditors" => self.ide_open_editors(),
            "getDiagnostics" => {
                let uri = args["uri"].as_str().map(|uri| self.ide_path(uri));
                self.ide_diagnostics(uri.as_deref())
            }
            "checkDocumentDirty" => {
                let path = self.ide_path(&text_arg("filePath"));
                let found = self.editors.iter().find(|e| e.buffer.path() == Some(&path));
                json!({
                    "success": true,
                    "filePath": path,
                    "isDirty": found.is_some_and(|e| e.buffer.is_dirty()),
                    "isUntitled": false,
                })
                .to_string()
            }
            "saveDocument" => {
                let path = self.ide_path(&text_arg("filePath"));
                let found = self
                    .editors
                    .iter_mut()
                    .find(|e| e.buffer.path() == Some(&path));
                let saved = found.map(|e| e.buffer.save());
                json!({"success": matches!(saved, Some(Ok(()))), "filePath": path}).to_string()
            }
            "close_tab" => {
                self.withdraw_diff(Some(&text_arg("tab_name")));
                "TAB_CLOSED".to_string()
            }
            "closeAllDiffTabs" => format!("CLOSED_{}_DIFF_TABS", self.withdraw_diff(None)),
            tool => format!("Cellix doesn't have {tool}"),
        };
        let _ = call.reply.send(vec![answer]);
    }

    fn ide_open(&mut self, path: &Path, start: &str, end: &str) -> String {
        if let Err(e) = self.open(path) {
            return format!("Could not open {}: {e}", path.display());
        }
        self.review = None;
        let index = self.active();
        if let Some(editor) = self.editors.get_mut(index).filter(|_| !start.is_empty()) {
            let text = editor.buffer.text();
            if let Some(from) = text.find(start) {
                let to = (!end.is_empty())
                    .then(|| text[from..].find(end).map(|at| from + at + end.len()))
                    .flatten()
                    .unwrap_or(from + start.len());
                let chars = |byte: usize| text[..byte].chars().count();
                let selection = Selection {
                    anchor: chars(from),
                    head: chars(to),
                    goal: None,
                };
                editor.selections.set_single(selection);
                let area = self.focused_area();
                if let Some(editor) = self.editors.get_mut(index) {
                    editor.reveal(area);
                }
            }
        }
        format!("Opened file: {}", path.display())
    }

    fn ide_open_editors(&self) -> String {
        let tabs: Vec<Value> = self
            .editors
            .iter()
            .enumerate()
            .filter_map(|(index, editor)| {
                let path = editor.buffer.path()?;
                Some(json!({
                    "uri": Self::file_uri(path),
                    "isActive": index == self.active(),
                    "label": editor.title(),
                    "languageId": Registry::language_name(path).unwrap_or("plaintext"),
                    "isDirty": editor.buffer.is_dirty(),
                }))
            })
            .collect();
        json!({"tabs": tabs}).to_string()
    }

    fn ide_diagnostics(&self, only: Option<&Path>) -> String {
        let files: Vec<Value> = self
            .diagnostics
            .iter()
            .filter(|(path, items)| !items.is_empty() && only.is_none_or(|o| o == path.as_path()))
            .map(|(path, items)| {
                let diagnostics: Vec<Value> = items
                    .iter()
                    .map(|item| {
                        let at = json!({"line": item.line, "character": item.column});
                        json!({
                            "message": item.message,
                            "severity": if item.severity == 1 { "Error" } else { "Warning" },
                            "range": {"start": at, "end": at},
                            "source": "language server",
                        })
                    })
                    .collect();
                json!({"uri": Self::file_uri(path), "diagnostics": diagnostics})
            })
            .collect();
        Value::Array(files).to_string()
    }

    /// Shows a change Claude Code wants to make, and holds its reply until it's decided.
    fn propose(&mut self, call: IdeCall) {
        let tab = call.args["tab_name"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        if let Some(current) = &self.proposal {
            if current.tab != tab {
                self.queued_diffs.push_back(call);
                return;
            }
            // The same tab again replaces the old proposal, and the old call gets rejected.
            self.finish_proposal(vec!["DIFF_REJECTED".to_string(), tab.clone()]);
        }
        let args = &call.args;
        let new = args["new_file_contents"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        let path = self.ide_path(args["new_file_path"].as_str().unwrap_or_default());
        let old_path = self.ide_path(args["old_file_path"].as_str().unwrap_or_default());
        // Missing means a new file, shown as all added.
        let old = std::fs::read_to_string(&old_path).unwrap_or_default();
        let root = self.workspace.root().to_path_buf();
        let shown = path.strip_prefix(&root).unwrap_or(&path).to_path_buf();
        self.review = Some(Review::proposing(
            root,
            self.registry.clone(),
            shown,
            old,
            new,
        ));
        self.drawer = false;
        self.proposal = Some(Proposal {
            tab,
            reply: call.reply,
        });
        self.attention = Some("change");
        self.say("Claude Code is proposing a change");
    }

    /// Answers the waiting call, closes the view, and shows the next diff in line.
    fn finish_proposal(&mut self, answer: Vec<String>) {
        if let Some(proposal) = self.proposal.take() {
            let _ = proposal.reply.send(answer);
        }
        self.review = None;
        if let Some(next) = self.queued_diffs.pop_front() {
            self.propose(next);
        }
    }

    fn accept_proposal(&mut self) {
        let Some(review) = self.review.as_ref().filter(|r| r.is_proposal()) else {
            self.say("No proposed change is waiting");
            return;
        };
        let text = review.proposed_text().to_string();
        let unchanged = review.proposed_unchanged();
        let tab = self
            .proposal
            .as_ref()
            .map(|p| p.tab.clone())
            .unwrap_or_default();
        self.finish_proposal(if unchanged {
            vec!["DIFF_REJECTED".to_string(), tab]
        } else {
            vec!["FILE_SAVED".to_string(), text]
        });
    }

    fn reject_proposal(&mut self) {
        let Some(proposal) = &self.proposal else {
            self.say("No proposed change is waiting");
            return;
        };
        let tab = proposal.tab.clone();
        self.finish_proposal(vec!["DIFF_REJECTED".to_string(), tab]);
    }

    /// Closes diffs that Claude Code no longer needs, usually because they were answered
    /// in its own prompt. Returns how many went.
    fn withdraw_diff(&mut self, tab: Option<&str>) -> usize {
        let wanted = |name: &str| tab.is_none_or(|t| t == name);
        let before = self.queued_diffs.len();
        self.queued_diffs
            .retain(|call| !wanted(call.args["tab_name"].as_str().unwrap_or_default()));
        let mut closed = before - self.queued_diffs.len();
        if self.proposal.as_ref().is_some_and(|p| wanted(&p.tab)) {
            let tab = self
                .proposal
                .as_ref()
                .map(|p| p.tab.clone())
                .unwrap_or_default();
            self.finish_proposal(vec!["DIFF_REJECTED".to_string(), tab]);
            closed += 1;
        }
        closed
    }

    fn lsp_event(&mut self, event: LspEvent) {
        match event {
            LspEvent::Diagnostics(path, items) => {
                self.diagnostics.insert(path, items);
            }
            LspEvent::Definition(path, line, column) => {
                if self.open(&path).is_ok() {
                    let area = self.focused_area();
                    if let Some(editor) = self.editor_mut() {
                        let at = editor.buffer.position(line, column);
                        editor.selections.set_single(Selection::cursor(at));
                        editor.reveal(area);
                    }
                }
            }
            LspEvent::Completion(labels) => {
                self.server_words = labels;
                self.rank_suggestions();
            }
            LspEvent::References(places) => self.show_references(places),
            LspEvent::Formatted(path, edits) => {
                let count = edits.len();
                let area = self.focused_area();
                match self.apply_edits(&path, edits) {
                    Ok(()) if count == 0 => self.say("Already formatted"),
                    Ok(()) => self.say("Formatted"),
                    Err(e) => self.say(format!("Could not format: {e}")),
                }
                if let Some(editor) = self.editor_mut() {
                    editor.reveal(area);
                }
            }
            LspEvent::Failed(message) => {
                self.pending_rename = None;
                self.say(message);
            }
            LspEvent::Rename(files) => {
                let Some((path, revision)) = self.pending_rename.take() else {
                    return;
                };
                let current = self
                    .editors
                    .iter()
                    .find(|e| e.buffer.path() == Some(path.as_path()))
                    .map(|e| e.buffer.revision());
                if current != Some(revision) {
                    self.say("The file changed, so the rename was dropped");
                    return;
                }
                let mut count = 0;
                for (path, edits) in files {
                    count += edits.len();
                    if let Err(e) = self.apply_edits(&path, edits) {
                        self.say(format!("Could not rename in {}: {e}", path.display()));
                    }
                }
                self.say(format!("Renamed {count} occurrences"));
            }
        }
    }

    /// Applies edits last to first, so earlier positions stay valid. Open files change in
    /// their buffers and stay unsaved, and closed files change on disk.
    fn apply_edits(&mut self, path: &Path, mut edits: Vec<TextEdit>) -> io::Result<()> {
        edits.sort_by_key(|e| std::cmp::Reverse(e.start));
        let (measure, registry) = (self.measure.clone(), self.registry.clone());
        let open = self
            .editors
            .iter_mut()
            .find(|e| e.buffer.path() == Some(path));
        let mut scratch = None;
        let editor = match open {
            Some(editor) => editor,
            None => {
                let buffer = Buffer::open(path)?;
                scratch.insert(Editor::new(buffer, measure, registry))
            }
        };
        for edit in edits {
            let start = editor.buffer.position(edit.start.0, edit.start.1);
            let end = editor.buffer.position(edit.end.0, edit.end.1);
            editor
                .buffer
                .replace_range(&mut editor.selections, start, end, &edit.text);
        }
        if let Some(mut closed) = scratch {
            closed.buffer.save()?;
        }
        Ok(())
    }

    /// Lists a symbol's references where search results go, as VS Code's panel does.
    fn show_references(&mut self, places: Vec<(PathBuf, usize, usize)>) {
        if places.is_empty() {
            self.say("No references found");
            return;
        }
        let word = self.typed_symbol();
        let mut found = Vec::new();
        for (path, line, column) in places {
            let open = self
                .editors
                .iter()
                .find(|e| e.buffer.path() == Some(path.as_path()));
            let text = match open {
                Some(editor) => editor.line_text(line),
                None => std::fs::read_to_string(&path)
                    .ok()
                    .and_then(|t| t.lines().nth(line).map(str::to_string))
                    .unwrap_or_default(),
            };
            found.push(cellix_core::Match {
                path,
                line,
                column,
                text,
            });
        }
        self.search.show(word, found);
        self.sidebar_view = SidebarView::Search;
        self.show_sidebar = true;
        self.drawer = self.compact();
    }

    /// The word at the caret.
    fn typed_symbol(&self) -> String {
        let Some(editor) = self.editors.get(self.active()) else {
            return String::new();
        };
        let (start, end) = editor.buffer.word_at(editor.selections.primary().head);
        editor.buffer.rope().slice(start..end).to_string()
    }

    /// Formats with the language server if there is one, and otherwise with a formatter
    /// that's on the path.
    fn format_document(&mut self) {
        self.poll();
        let Some(editor) = self.editors.get(self.active()) else {
            return;
        };
        let Some(path) = editor.buffer.path().map(Path::to_path_buf) else {
            return;
        };
        let text = editor.buffer.text();
        let language = Registry::language_name(&path).unwrap_or("Plain Text");
        if let Some(client) = self.servers.get_mut(language) {
            client.format(&path, TAB_SIZE);
            return;
        }
        let Some(tool) = formatter(language) else {
            self.say(format!("No formatter for {language}"));
            return;
        };
        let name = path.display().to_string();
        match format_with(tool, &name, &text) {
            Ok(formatted) if formatted == text => self.say("Already formatted"),
            Ok(formatted) => {
                let area = self.focused_area();
                if let Some(editor) = self.editor_mut() {
                    let end = editor.buffer.rope().len_chars();
                    let head = editor.selections.primary().head;
                    editor
                        .buffer
                        .replace_range(&mut editor.selections, 0, end, &formatted);
                    let last = editor.buffer.rope().len_chars();
                    editor
                        .selections
                        .set_single(Selection::cursor(head.min(last)));
                    editor.reveal(area);
                }
                self.say("Formatted");
            }
            Err(e) => self.say(e),
        }
    }

    fn lsp_target(&mut self) -> Option<(&'static str, PathBuf, usize, usize)> {
        let editor = self.editors.get(self.active())?;
        let path = editor.buffer.path()?.to_path_buf();
        let language = Registry::language_name(&path)?;
        let rope = editor.buffer.rope();
        let head = editor.selections.primary().head;
        let line = rope.char_to_line(head);
        Some((language, path, line, head - rope.line_to_char(line)))
    }

    fn lsp_request(&mut self, command: Command) {
        self.poll();
        let Some((language, path, line, column)) = self.lsp_target() else {
            return;
        };
        let Some(client) = self.servers.get_mut(language) else {
            self.say(format!("No language server for {language}"));
            return;
        };
        match command {
            Command::GoToDefinition => client.definition(&path, line, column),
            Command::GoToReferences => client.references(&path, line, column),
            _ => client.completion(&path, line, column),
        }
    }

    /// The word being typed, and where it starts.
    fn typed_word(&self) -> Option<(String, usize)> {
        let editor = self.editors.get(self.active())?;
        let primary = editor.selections.primary();
        if !primary.is_empty() {
            return None;
        }
        let (start, _) = editor.buffer.word_at(primary.head);
        let start = start.min(primary.head);
        let word = editor.buffer.rope().slice(start..primary.head).to_string();
        Some((word, start))
    }

    fn active_language(&self) -> &'static str {
        let path = self
            .editors
            .get(self.active())
            .and_then(|e| e.buffer.path());
        path.and_then(Registry::language_name).unwrap_or("")
    }

    /// Recomputes what shows from what's known, without asking any server.
    fn rank_suggestions(&mut self) {
        let Some((typed, _)) = self.typed_word() else {
            self.close_suggestions();
            return;
        };
        // Prose gets more letters first, so sentences don't sprout menus.
        let language = self.active_language();
        let needed = if language == "Markdown" { 3 } else { 2 };
        if typed.chars().count() < needed && !self.suggest_asked {
            self.close_suggestions();
            return;
        }
        let candidate = |source| move |label: String| Candidate { label, source };
        let mut all: Vec<Candidate> = self
            .server_words
            .iter()
            .cloned()
            .map(candidate(Source::Server))
            .collect();
        if language == "Env"
            && let Some(path) = self
                .editors
                .get(self.active())
                .and_then(|e| e.buffer.path())
        {
            all.extend(
                complete::env_keys(path)
                    .into_iter()
                    .map(candidate(Source::Key)),
            );
        }
        let keywords = complete::keywords(language).iter().map(ToString::to_string);
        all.extend(keywords.map(candidate(Source::Keyword)));
        let active = self.active();
        // The file in front first, then what else is open.
        let order = std::iter::once(active).chain((0..self.editors.len()).filter(|i| *i != active));
        for index in order {
            if let Some(editor) = self.editors.get(index) {
                let text = editor.buffer.text();
                let words = complete::words(&text).into_iter().map(str::to_string);
                all.extend(words.map(candidate(Source::Word)));
            }
        }
        let items = complete::rank(&typed, all);
        self.suggest = (!items.is_empty()).then_some(Suggest { items, selected: 0 });
    }

    /// Updates suggestions after a keystroke, and asks the language server too if there is one.
    fn refresh_suggestions(&mut self) {
        self.rank_suggestions();
        let target = self.lsp_target().filter(|_| {
            self.suggest_asked
                || self
                    .typed_word()
                    .is_some_and(|(w, _)| w.chars().count() >= 2)
        });
        if let Some((language, path, line, column)) = target
            && let Some(client) = self.servers.get_mut(language)
        {
            client.completion(&path, line, column);
        }
    }

    fn close_suggestions(&mut self) {
        self.suggest = None;
        self.suggest_asked = false;
        self.server_words.clear();
    }

    fn accept_suggestion(&mut self, index: usize) {
        let label = self
            .suggest
            .as_ref()
            .and_then(|s| s.items.get(index))
            .map(|c| c.label.clone());
        self.close_suggestions();
        if let Some(label) = label {
            self.insert_completion(&label);
        }
    }

    /// Handles a key while suggestions show. Returns true if it was theirs.
    fn suggest_key(&mut self, key: KeyEvent) -> bool {
        let Some(suggest) = &mut self.suggest else {
            return false;
        };
        let count = suggest.items.len();
        let selected = suggest.selected;
        match key.key {
            Key::Down => suggest.selected = (selected + 1) % count,
            Key::Up => suggest.selected = (selected + count - 1) % count,
            Key::Esc => self.close_suggestions(),
            Key::Tab => self.accept_suggestion(selected),
            // In prose Enter means a new line, so only Tab accepts there.
            Key::Enter if self.active_language() != "Markdown" => self.accept_suggestion(selected),
            _ => return false,
        }
        true
    }

    fn suggest_area(&self) -> Option<Rect> {
        let suggest = self.suggest.as_ref()?;
        let editor = self.editors.get(self.active())?;
        let (x, y) = editor.caret_cell(self.focused_area())?;
        let height = suggest.items.len() as u16 + 2;
        let bottom = self.size.1.saturating_sub(1);
        let y = if y + 1 + height <= bottom {
            y + 1
        } else {
            y.saturating_sub(height)
        };
        let x = (x as u16).min(self.size.0.saturating_sub(SUGGEST_WIDTH + 1));
        Some(Rect::new(x, y, SUGGEST_WIDTH.min(self.size.0), height))
    }

    fn draw_suggest(&self, grid: &mut Grid) {
        let (Some(suggest), Some(area)) = (&self.suggest, self.suggest_area()) else {
            return;
        };
        let theme = &self.theme;
        popup(grid, area, theme);
        for (i, item) in suggest.items.iter().enumerate() {
            let y = area.y + 1 + i as u16;
            let row = Rect::new(area.x, y, area.w, 1);
            let base = Style {
                fg: theme.text.fg,
                bg: theme.popup,
                ..theme.bar
            };
            let hovered = self.hover.is_some_and(|h| row.contains(h.0, h.1));
            if hovered && i != suggest.selected {
                let wash = Bounds::inset(row, 0.5, 0.055);
                grid.panel(wash, 18, Some(theme.quiet_button(theme.popup)), None);
            }
            if i == suggest.selected {
                grid.fill(
                    Rect::new(area.x + 1, y, area.w.saturating_sub(2), 1),
                    Style {
                        bg: theme.pick,
                        ..base
                    },
                );
                grid.panel(Bounds::inset(row, 0.5, 0.03), 14, Some(theme.pick), None);
            }
            let style = Style {
                bg: if i == suggest.selected {
                    theme.pick
                } else {
                    theme.popup
                },
                ..base
            };
            let kind = Style {
                fg: theme.gutter.fg,
                ..style
            };
            grid.label(
                area.x + 2,
                y,
                &item.label,
                style,
                area.right().saturating_sub(9),
            );
            grid.label_right(
                area.right().saturating_sub(2),
                y,
                item.source.label(),
                kind,
                80,
            );
        }
    }

    /// Replaces the word before the cursor with a completion.
    fn insert_completion(&mut self, text: &str) {
        if let Some(editor) = self.editor_mut() {
            let head = editor.selections.primary().head;
            let (start, _) = editor.buffer.word_at(head);
            editor.selections.set_single(Selection {
                anchor: start,
                head,
                goal: None,
            });
            editor.buffer.replace(&mut editor.selections, text);
        }
        self.reveal();
    }

    pub fn active_text(&self) -> Option<String> {
        self.editors.get(self.active()).map(|e| e.buffer.text())
    }

    pub fn selected_text(&self) -> Option<String> {
        self.editors.get(self.active())?.selected_text()
    }

    fn compact(&self) -> bool {
        self.size.0 < self.compact_width
    }

    fn activity_area(&self) -> Rect {
        let width = if self.compact() { 0 } else { ACTIVITY_WIDTH };
        Rect::new(0, 0, width, self.size.1.saturating_sub(1))
    }

    fn sidebar_area(&self) -> Option<Rect> {
        let activity = self.activity_area();
        if self.compact() {
            return self
                .drawer
                .then_some(Rect::new(0, 0, self.size.0, activity.h));
        }
        // Beside an agent, a small window gives its room to the two things being compared.
        if self.dock.open && self.size.0 < CROWDED_WIDTH {
            return None;
        }
        // Review uses the explorer's width: side-by-side code needs it more.
        (self.show_sidebar && self.review.is_none()).then(|| {
            let width = (self.size.0 / 4).clamp(SIDEBAR_MIN_WIDTH, SIDEBAR_WIDTH);
            Rect::new(activity.right(), 0, width, activity.h)
        })
    }

    /// Where the main area starts.
    fn main_left(&self) -> u16 {
        match self.sidebar_area() {
            Some(area) if !self.compact() => area.right(),
            _ => self.activity_area().right(),
        }
    }

    fn main_area(&self) -> Rect {
        let x = self.main_left();
        let beside = self.dock_area().map_or(0, |a| a.w);
        Rect::new(
            x,
            0,
            self.size.0.saturating_sub(x + beside),
            self.size.1.saturating_sub(1),
        )
    }

    fn tabs_area(&self) -> Rect {
        let main = self.main_area();
        let inset = if self.compact() { MENU_BUTTON } else { 0 };
        Rect::new(main.x + inset, main.y, main.w.saturating_sub(inset), 1)
    }

    /// Rows above the code: tabs, and breadcrumbs when there's room for them.
    fn chrome_rows(&self) -> u16 {
        if !self.compact() && self.size.1 >= 16 {
            2
        } else {
            1
        }
    }

    /// In the compact layout only the focused pane shows, at full width.
    fn pane_area(&self, pane: usize) -> Rect {
        let main = self.main_area();
        let rows = self.chrome_rows();
        let (y, h) = (main.y + rows, main.h.saturating_sub(rows));
        if self.compact() {
            let w = if pane == self.focus { main.w } else { 0 };
            return Rect::new(main.x, y, w, h);
        }
        let count = self.panes.len() as u16;
        let width = main.w / count.max(1);
        let x = main.x + width * pane as u16;
        let w = if pane + 1 == self.panes.len() {
            main.right() - x
        } else {
            width.saturating_sub(1)
        };
        Rect::new(x, y, w, h)
    }

    fn focused_area(&self) -> Rect {
        self.pane_area(self.focus)
    }

    pub fn take_attention(&mut self) -> Option<&'static str> {
        self.attention.take()
    }

    /// The editor region the wheel just moved, and by how many rows.
    pub fn take_scroll(&mut self) -> Option<(Rect, i32)> {
        self.scrolled.take()
    }

    pub fn handle(&mut self, event: Event) {
        // Ticks and pointer moves aren't the user moving on, so they leave messages up.
        let passive = matches!(
            event,
            Event::Tick
                | Event::Mouse(MouseEvent {
                    kind: MouseKind::Move | MouseKind::Up,
                    ..
                })
        );
        if !passive {
            self.message.borrow_mut().clear();
        }
        let wheel = matches!(
            event,
            Event::Mouse(MouseEvent {
                kind: MouseKind::ScrollUp | MouseKind::ScrollDown,
                ..
            })
        );
        // Anything else may change the page, and then the two frames don't line up.
        if !passive && !wheel {
            self.scrolled = None;
        }
        match event {
            Event::Key(key) => self.key(key),
            Event::Mouse(mouse) => self.mouse(mouse),
            Event::Paste(text) => self.paste(&text),
            Event::Resize(w, h) => self.size = (w, h),
            Event::Tick => self.tick(),
        }
        self.fit_dock();
    }

    fn paste(&mut self, text: &str) {
        if self.dock.focused
            && matches!(self.overlay, Overlay::None)
            && let Some(session) = self.dock.session_mut()
        {
            session.paste(text);
            return;
        }
        match &mut self.overlay {
            Overlay::Palette(palette) => {
                palette.query.push_str(text);
                palette.filter();
            }
            Overlay::Prompt(prompt) => prompt.input.push_str(text),
            Overlay::Find(find) => {
                find.query.push_str(text);
                self.search_file();
            }
            _ => {
                let area = self.focused_area();
                if let Some(editor) = self.editor_mut() {
                    editor.prepare_edit(area);
                    editor.buffer.replace(&mut editor.selections, text);
                }
                self.reveal();
            }
        }
    }

    fn tick(&mut self) {
        let mut note = None;
        let mut reloaded = Vec::new();
        for editor in &mut self.editors {
            if !editor.buffer.changed_on_disk() {
                continue;
            }
            if editor.buffer.is_dirty() {
                note = Some(format!(
                    "{} changed on disk, keeping your edits",
                    editor.title()
                ));
                editor.buffer.acknowledge_disk();
            } else {
                note = Some(match editor.buffer.reload(&mut editor.selections) {
                    Ok(()) => {
                        reloaded.extend(editor.buffer.path().map(Path::to_path_buf));
                        format!("Reloaded {}", editor.title())
                    }
                    Err(e) => format!("Could not reload {}: {e}", editor.title()),
                });
            }
        }
        if let Some(note) = note {
            self.say(note);
        }
        // A file an agent wrote is a saved file: servers only run their full check on save.
        if !reloaded.is_empty() {
            self.poll();
        }
        for path in reloaded {
            if let Some(client) =
                Registry::language_name(&path).and_then(|language| self.servers.get(language))
            {
                client.saved(&path);
            }
        }
        self.ticks += 1;
        if self.ticks.is_multiple_of(EXPLORER_REFRESH_TICKS) {
            self.refresh_tree();
            self.refresh_committed();
            if let Some(review) = self.review.as_mut().filter(|r| !r.is_reading()) {
                review.refresh_later();
            }
        }
    }

    /// Keeps margin marks true after a commit or a checkout.
    fn refresh_committed(&mut self) {
        let mut editors = std::mem::take(&mut self.editors);
        for editor in &mut editors {
            self.load_committed(editor);
        }
        self.editors = editors;
    }

    fn refresh_tree(&mut self) {
        self.explorer.refresh(&self.workspace);
        let root = self.workspace.root().to_path_buf();
        self.explorer.status.clear();
        self.branch = current_branch(&root);
        for change in changed_files(&root) {
            let path = root.join(&change.path);
            for parent in path
                .ancestors()
                .skip(1)
                .take_while(|p| p.starts_with(&root) && *p != root)
            {
                self.explorer
                    .status
                    .entry(parent.to_path_buf())
                    .or_insert('•');
            }
            self.explorer.status.insert(path, change.status);
        }
    }

    fn reveal(&mut self) {
        let area = self.focused_area();
        if let Some(editor) = self.editor_mut() {
            editor.reveal(area);
        }
    }

    fn key(&mut self, key: KeyEvent) {
        if self.dock.focused && matches!(self.overlay, Overlay::None) {
            // The session gets everything but what moves away from it.
            let leaves = matches!(
                simple_keymap(key),
                Some(Command::FocusAgent | Command::CommandPalette | Command::Quit)
            );
            if !leaves {
                if let Some(session) = self.dock.session_mut() {
                    session.key(key);
                }
                return;
            }
        }
        // These two work from anywhere, even with a palette or prompt open.
        let leaving = matches!(simple_keymap(key), Some(Command::Quit | Command::CloseTab));
        if !matches!(self.overlay, Overlay::None) && !leaving {
            self.overlay_key(key);
            return;
        }
        if leaving {
            self.overlay = Overlay::None;
        }
        if self.settings_page.is_some() && self.settings_key(key) {
            return;
        }
        if self.search.focus.is_some() && self.sidebar_area().is_some() && self.search_key(key) {
            return;
        }
        if self.suggest_key(key) {
            return;
        }
        if self.neovim_key(key) {
            return;
        }
        if let Some(vim) = &mut self.vim {
            match vim.key(key) {
                VimAction::Commands(commands) => {
                    for command in commands {
                        self.run(command);
                    }
                    return;
                }
                VimAction::Pending => return,
                VimAction::Passthrough => {}
            }
        }
        if let Some(command) = simple_keymap(key) {
            let open = self.suggest.is_some();
            self.run(command);
            match command {
                Command::Complete => {}
                Command::DeleteBackward if open => self.refresh_suggestions(),
                _ => self.close_suggestions(),
            }
        } else if let Key::Char(c) = key.key
            && !key.mods.ctrl
            && !key.mods.alt
        {
            let area = self.focused_area();
            if let Some(editor) = self.editor_mut() {
                editor.prepare_edit(area);
                editor.type_char(c);
            }
            self.reveal();
            if c.is_alphanumeric() || c == '_' {
                self.refresh_suggestions();
            } else {
                self.close_suggestions();
            }
        }
    }

    /// Sends a key to Neovim. Returns false if Cellix should handle it instead.
    fn neovim_key(&mut self, key: KeyEvent) -> bool {
        let Some(keys) = neovim::notation(key).filter(|_| self.neovim.is_some()) else {
            return false;
        };
        let index = self.active();
        let Some(editor) = self
            .editors
            .get(index)
            .filter(|e| e.buffer.path().is_some())
        else {
            return false;
        };
        let Some(path) = editor.buffer.path().map(Path::to_path_buf) else {
            return false;
        };
        let (revision, head) = (editor.buffer.revision(), editor.selections.primary().head);
        let text = editor.buffer.text();
        let rope = editor.buffer.rope();
        let line = rope.char_to_line(head);
        let column = rope.slice(rope.line_to_char(line)..head).len_bytes();
        // The `:` commands that mean something to the app, not to a scratch buffer.
        if key.key == Key::Enter && !self.neovim_line.is_empty() {
            let command = match self.neovim_line.as_str() {
                "w" => Some(vec![Command::Save]),
                "q" => Some(vec![Command::CloseTab]),
                "wq" | "x" => Some(vec![Command::Save, Command::CloseTab]),
                "qa" => Some(vec![Command::Quit]),
                _ => None,
            };
            if let Some(commands) = command {
                self.neovim_line.clear();
                if let Some(neovim) = &mut self.neovim {
                    neovim.input("<Esc>");
                }
                self.neovim_settle();
                for command in commands {
                    self.run(command);
                }
                return true;
            }
        }
        let same_file = self.neovim_file.as_ref().map(|f| f.0.as_path()) == Some(path.as_path());
        let seen = self.neovim_file.clone();
        let Some(neovim) = &mut self.neovim else {
            return false;
        };
        if !same_file {
            neovim.show(&path);
        }
        if !same_file || seen.as_ref().map(|f| f.1) != Some(revision) {
            neovim.set_text(&text);
        }
        if !same_file || seen.as_ref().map(|f| f.2) != Some(head) {
            neovim.set_cursor(line, column);
        }
        self.neovim_file = Some((path, revision, head));
        if let Some(neovim) = &mut self.neovim {
            neovim.input(&keys);
        }
        self.neovim_settle();
        true
    }

    /// Brings Cellix up to date with what the last key did.
    fn neovim_settle(&mut self) {
        let Some(state) = self.neovim.as_mut().and_then(Neovim::state) else {
            return;
        };
        self.neovim_mode = match state.mode.chars().next() {
            Some('i') => "INSERT",
            Some('R') => "REPLACE",
            Some('v' | 'V' | '\u{16}') => "VISUAL",
            Some('c') => "COMMAND",
            _ => "NORMAL",
        }
        .to_string();
        self.neovim_line = state.cmdline.clone();
        if state.cmdtype.is_empty() {
            self.neovim_line.clear();
        }
        let area = self.focused_area();
        let index = self.active();
        let Some(editor) = self.editors.get_mut(index) else {
            return;
        };
        if let Some(lines) = state.lines {
            let ended = editor.buffer.text().ends_with('\n');
            let text = lines.join("\n") + if ended { "\n" } else { "" };
            if text != editor.buffer.text() {
                let end = editor.buffer.rope().len_chars();
                editor
                    .buffer
                    .replace_range(&mut editor.selections, 0, end, &text);
            }
        }
        let rope = editor.buffer.rope();
        let char_at = |(line, byte): (usize, usize)| {
            let line = line.min(rope.len_lines().saturating_sub(1));
            let start = rope.line_to_char(line);
            let bytes = rope.line_to_byte(line) + byte;
            rope.byte_to_char(bytes.min(rope.len_bytes())).max(start)
        };
        let head = char_at(state.cursor);
        let selection = if state.mode.starts_with(['v', 'V']) {
            let anchor = char_at(state.anchor);
            // Vim's selection includes the character under the cursor.
            let (from, to) = if anchor <= head {
                (anchor, head + 1)
            } else {
                (anchor + 1, head)
            };
            Selection {
                anchor: from,
                head: to.min(rope.len_chars()),
                goal: None,
            }
        } else {
            Selection::cursor(head)
        };
        let revision = editor.buffer.revision();
        editor.selections.set_single(selection);
        editor.reveal(area);
        if let Some(file) = &mut self.neovim_file {
            file.1 = revision;
            file.2 = selection.head;
        }
    }

    /// Asks what to do about unsaved work before closing a tab or quitting.
    fn confirm_leaving(&mut self, leaving: Leaving) {
        let dirty: Vec<String> = match leaving {
            Leaving::CloseTab => self
                .editors
                .get(self.active())
                .into_iter()
                .map(Editor::title)
                .collect(),
            Leaving::Quit => {
                let unsaved = self.editors.iter().filter(|e| e.buffer.is_dirty());
                unsaved.map(Editor::title).collect()
            }
        };
        let what = match dirty.as_slice() {
            [one] => one.clone(),
            many => format!("{} files", many.len()),
        };
        self.overlay = Overlay::Confirm(Confirm {
            title: format!("Save changes to {what}?"),
            detail: "Your changes will be lost if you don't save them.".to_string(),
            buttons: vec![
                ("Save", Choice::Save(leaving)),
                ("Don't Save", Choice::Discard(leaving)),
                ("Cancel", Choice::Cancel),
            ],
            selected: 0,
        });
    }

    fn leave(&mut self, leaving: Leaving) {
        match leaving {
            Leaving::Quit => self.quit = true,
            Leaving::CloseTab => self.close_tab(),
        }
    }

    fn choose(&mut self, choice: Choice) {
        match choice {
            Choice::Cancel => {}
            Choice::Save(leaving) => {
                self.save_dirty(leaving == Leaving::CloseTab);
                self.leave(leaving);
            }
            Choice::Discard(leaving) => self.leave(leaving),
            Choice::Delete(path) => {
                if let Err(e) = self.workspace.delete(&path) {
                    self.say(format!("Could not delete it: {e}"));
                }
                self.refresh_tree();
            }
            Choice::Undo(commit) => {
                let root = self.workspace.root().to_path_buf();
                let result = restore(&root, &commit);
                self.say(match result {
                    Ok(count) => format!("Restored {count} files, and saved a point to redo"),
                    Err(e) => format!("Could not undo: {e}"),
                });
                if let Some(review) = &mut self.review {
                    review.refresh();
                }
            }
        }
    }

    fn confirm_key(&mut self, mut confirm: Confirm, key: KeyEvent) {
        let count = confirm.buttons.len();
        let picked = match key.key {
            Key::Esc => {
                self.choose(Choice::Cancel);
                return;
            }
            Key::Enter => Some(confirm.selected),
            Key::Right | Key::Down | Key::Tab => {
                confirm.selected = (confirm.selected + 1) % count;
                None
            }
            Key::Left | Key::Up | Key::BackTab => {
                confirm.selected = (confirm.selected + count - 1) % count;
                None
            }
            Key::Char('y' | 'Y') => Some(0),
            Key::Char('n' | 'N') => Some(1),
            Key::Char(c) => {
                let letter = c.to_ascii_lowercase();
                let starts = |label: &str| label.to_lowercase().starts_with(letter);
                confirm.buttons.iter().position(|(label, _)| starts(label))
            }
            _ => None,
        };
        match picked.and_then(|i| confirm.buttons.get(i)) {
            Some((_, choice)) => self.choose(choice.clone()),
            None => self.overlay = Overlay::Confirm(confirm),
        }
    }

    fn confirm_area(&self) -> Rect {
        let screen = Rect::new(0, 0, self.size.0, self.size.1);
        let width = CONFIRM_WIDTH.min(screen.w);
        let y = (screen.h / 3).min(screen.h.saturating_sub(CONFIRM_HEIGHT));
        Rect::new(screen.x + (screen.w - width) / 2, y, width, CONFIRM_HEIGHT)
    }

    /// Each button as (cells, x), laid out from the right edge.
    fn confirm_buttons(&self, confirm: &Confirm) -> Vec<(String, u16)> {
        let area = self.confirm_area();
        let mut x = area.right().saturating_sub(2);
        let mut placed = Vec::new();
        // The same padding on each, which only a measured width gives proportional text.
        let plain = self.measure.in_cells();
        let font = Font {
            size: 90,
            ..Font::default()
        };
        for (label, _) in confirm.buttons.iter().rev() {
            let cells = if plain {
                format!("[{label}]")
            } else {
                let width = self.measure.width(label, font).ceil() as usize + CONFIRM_PADDING;
                format!("{label:^width$}")
            };
            x = x.saturating_sub(cells.chars().count() as u16);
            placed.push((cells, x));
            x = x.saturating_sub(1);
        }
        placed.reverse();
        placed
    }

    fn draw_confirm(&self, grid: &mut Grid, confirm: &Confirm) {
        let theme = &self.theme;
        let area = self.confirm_area();
        popup(grid, area, theme);
        let base = Style {
            fg: theme.text.fg,
            bg: theme.popup,
            ..theme.bar
        };
        let strong = Style {
            bold: true,
            fg: theme.heading,
            ..base
        };
        let dim = Style {
            fg: theme.popup_dim,
            ..base
        };
        grid.label(
            area.x + 2,
            area.y + 1,
            &confirm.title,
            strong,
            area.right().saturating_sub(1),
        );
        grid.label(
            area.x + 2,
            area.y + 2,
            &confirm.detail,
            dim,
            area.right().saturating_sub(1),
        );
        let y = area.y + 4;
        for (i, (cells, x)) in self.confirm_buttons(confirm).iter().enumerate() {
            let on = i == confirm.selected;
            let width = cells.chars().count() as u16;
            let hovered = self
                .hover
                .is_some_and(|h| h.1 == y && h.0 >= *x && h.0 < x + width);
            let fill = match (on, hovered) {
                (true, _) => theme.button,
                (false, true) => theme.popup_hover,
                (false, false) => theme.quiet_button(theme.popup),
            };
            let style = Style {
                fg: if on { theme.button_text } else { theme.text.fg },
                bg: fill,
                ..base
            };
            grid.large_button(*x, y, cells, confirm.buttons[i].0, fill, style);
        }
    }

    fn overlay_key(&mut self, key: KeyEvent) {
        let overlay = std::mem::replace(&mut self.overlay, Overlay::None);
        match overlay {
            Overlay::Palette(mut palette) => match key.key {
                Key::Esc => {}
                Key::Enter => self.pick(palette.current().cloned()),
                Key::Up => {
                    palette.move_by(-1);
                    self.overlay = Overlay::Palette(palette);
                }
                Key::Down => {
                    palette.move_by(1);
                    self.overlay = Overlay::Palette(palette);
                }
                Key::Backspace => {
                    palette.query.pop();
                    palette.filter();
                    self.overlay = Overlay::Palette(palette);
                }
                Key::Char(c) if !key.mods.ctrl && !key.mods.alt => {
                    palette.query.push(c);
                    palette.filter();
                    self.overlay = Overlay::Palette(palette);
                }
                _ => self.overlay = Overlay::Palette(palette),
            },
            Overlay::Prompt(mut prompt) => match key.key {
                Key::Esc => {}
                Key::Enter => self.finish_prompt(prompt),
                Key::Backspace => {
                    prompt.input.pop();
                    self.overlay = Overlay::Prompt(prompt);
                }
                Key::Char(c) if !key.mods.ctrl && !key.mods.alt => {
                    prompt.input.push(c);
                    self.overlay = Overlay::Prompt(prompt);
                }
                _ => self.overlay = Overlay::Prompt(prompt),
            },
            Overlay::Find(find) => self.find_key(find, key),
            Overlay::Confirm(confirm) => self.confirm_key(confirm, key),
            Overlay::TablePicker(mut picker) => match key.key {
                Key::Esc => {}
                Key::Enter => self.insert_table(picker.size),
                other => {
                    picker.key(other);
                    self.overlay = Overlay::TablePicker(picker);
                }
            },
            Overlay::Math(mut panel) => match key.key {
                Key::Esc => {}
                Key::Enter => self.insert_math(panel.selected),
                other => {
                    panel.key(other);
                    self.overlay = Overlay::Math(panel);
                }
            },
            Overlay::Menu(mut menu) => match key.key {
                Key::Up | Key::BackTab => {
                    menu.step(-1);
                    self.overlay = Overlay::Menu(menu);
                }
                Key::Down | Key::Tab => {
                    menu.step(1);
                    self.overlay = Overlay::Menu(menu);
                }
                Key::Enter => {
                    if let Some(action) = menu.selected.and_then(|row| menu.action(row)) {
                        self.menu_action(action.clone());
                    }
                }
                Key::Esc => {}
                _ => self.overlay = Overlay::Menu(menu),
            },
            Overlay::None => {}
        }
    }

    fn find_key(&mut self, mut find: Find, key: KeyEvent) {
        match key.key {
            Key::Esc => {
                if let Some(editor) = self.editor_mut() {
                    editor.found.clear();
                }
                return;
            }
            Key::Enter if key.mods.shift => find.step(-1),
            Key::Enter | Key::Down => find.step(1),
            Key::Up => find.step(-1),
            Key::Backspace => {
                find.query.pop();
            }
            Key::Char(c) if !key.mods.ctrl && !key.mods.alt => find.query.push(c),
            _ => {}
        }
        self.overlay = Overlay::Find(find);
        if matches!(key.key, Key::Backspace | Key::Char(_)) {
            self.search_file();
        } else {
            self.show_match();
        }
    }

    /// Searches the open file for the query as it stands.
    fn search_file(&mut self) {
        let index = self.active();
        let Some(editor) = self.editors.get(index) else {
            return;
        };
        let from = editor.selections.primary().start();
        let text = editor.buffer.text();
        if let Overlay::Find(find) = &mut self.overlay {
            find.search(&text, from);
        }
        self.show_match();
    }

    fn show_match(&mut self) {
        let area = self.focused_area();
        let index = self.active();
        let Overlay::Find(find) = &self.overlay else {
            return;
        };
        let (matches, selected) = (find.matches.clone(), find.selected());
        let Some(editor) = self.editors.get_mut(index) else {
            return;
        };
        editor.found = matches;
        if let Some((start, end)) = selected {
            editor.selections.set_single(Selection {
                anchor: start,
                head: end,
                goal: None,
            });
            editor.reveal(area);
        }
    }

    fn open_find(&mut self) {
        if self.review.is_some() || self.active() >= self.editors.len() {
            self.show_search(Field::Query);
            return;
        }
        let query = self
            .selected_text()
            .filter(|text| !text.contains('\n'))
            .unwrap_or_default();
        self.overlay = Overlay::Find(Find {
            query,
            ..Find::default()
        });
        self.search_file();
    }

    fn find_bar(&self) -> Rect {
        let pane = self.focused_area();
        let width = FIND_WIDTH.min(pane.w);
        let x = pane.right().saturating_sub(width + 1).max(pane.x);
        Rect::new(x, pane.y, width, 3)
    }

    fn draw_find(&self, grid: &mut Grid, find: &Find) {
        let theme = &self.theme;
        let area = self.find_bar();
        popup(grid, area, theme);
        let count = find.count_label();
        let right = FIND_BUTTONS.len() as u16 * FIND_BUTTON + count.chars().count() as u16 + 2;
        let field = area.w.saturating_sub(right + 2);
        let line = Rect::new(area.x + 1, area.y + 1, field, 1);
        let text = Input {
            text: &find.query,
            hint: "Find",
            focused: true,
            behind: theme.popup,
        };
        input(grid, line, &text, &*self.measure, theme);
        let base = Style {
            fg: theme.gutter.fg,
            bg: theme.popup,
            ..theme.bar
        };
        grid.label(line.right() + 1, area.y + 1, &count, base, area.right());
        let mut x = area.right() - FIND_BUTTON * FIND_BUTTONS.len() as u16 - 1;
        for label in FIND_BUTTONS {
            let button = Rect::new(x, area.y + 1, FIND_BUTTON, 1);
            let on_it = self.hover.is_some_and(|h| button.contains(h.0, h.1));
            if on_it {
                let pill = Bounds::inset(button, 0.1, 0.055);
                grid.panel(pill, 18, Some(theme.popup_hover), None);
            }
            let style = if on_it {
                Style {
                    fg: theme.text.fg,
                    ..base
                }
            } else {
                base
            };
            grid.label(x + 1, area.y + 1, label, style, area.right());
            x += FIND_BUTTON;
        }
    }

    fn click_find(&mut self, mut find: Find, x: u16, y: u16) {
        let area = self.find_bar();
        let first = area.right() - FIND_BUTTON * FIND_BUTTONS.len() as u16 - 1;
        let key = if area.contains(x, y) && x >= first && y == area.y + 1 {
            match (x - first) / FIND_BUTTON {
                0 => Key::Up,
                1 => Key::Down,
                _ => Key::Esc,
            }
        } else {
            self.overlay = Overlay::Find(find);
            return;
        };
        find.current = find.current.min(find.matches.len());
        self.find_key(find, KeyEvent::plain(key));
    }

    fn pick(&mut self, item: Option<Item>) {
        match item {
            Some(Item::File(path, _)) => {
                if let Err(e) = self.open(&path) {
                    self.say(format!("Could not open: {e}"));
                }
            }
            Some(Item::Command(_, command)) => self.run(command),
            Some(Item::Branch(name, _)) => {
                let result = switch_branch(self.workspace.root(), &name);
                self.branch_changed(&name, result);
            }
            Some(Item::NewBranch) => self.ask("New branch", PromptAction::NewBranch),
            Some(Item::Gap) | None => {}
        }
    }

    /// Shows the new branch's files, or says why git wouldn't move.
    fn branch_changed(&mut self, name: &str, result: Result<(), String>) {
        match result {
            Ok(()) => {
                self.tick();
                self.refresh_tree();
                self.refresh_committed();
                if let Some(review) = &mut self.review {
                    review.refresh();
                }
                self.say(format!("On {name}"));
            }
            Err(reason) => {
                self.overlay = Overlay::Confirm(Confirm {
                    title: format!("Couldn't switch to {name}"),
                    detail: reason,
                    buttons: vec![("OK", Choice::Cancel)],
                    selected: 0,
                });
            }
        }
    }

    fn switch_branch_list(&mut self) {
        let root = self.workspace.root();
        let current = self.branch.clone();
        let mut items: Vec<Item> = branches(root)
            .into_iter()
            .map(|b| {
                let here = current.as_deref() == Some(b.name.as_str());
                let when = if here { "current".to_string() } else { b.when };
                Item::Branch(b.name, when)
            })
            .collect();
        // The branch you're on comes first, so the list opens on where you are.
        items.sort_by_key(|i| !matches!(i, Item::Branch(_, when) if when == "current"));
        items.push(Item::Gap);
        items.push(Item::NewBranch);
        self.overlay = Overlay::Palette(Palette::asking(items, "Switch branch"));
    }

    fn finish_prompt(&mut self, prompt: Prompt) {
        let name = prompt.input.trim();
        let result = match &prompt.action {
            PromptAction::Image => {
                let area = self.focused_area();
                let image = format!("![]({name})");
                if let Some(editor) = self.editor_mut() {
                    editor.prepare_edit(area);
                    editor.insert_block(&image, 2);
                }
                Ok(())
            }
            PromptAction::GoToLine => {
                let area = self.focused_area();
                if let (Ok(line), Some(editor)) = (name.trim().parse(), self.editor_mut()) {
                    editor.go_to_line(line, area);
                }
                Ok(())
            }
            PromptAction::SendBackPlan => {
                if let Some(path) = self.plan.as_ref().map(|p| p.path.clone()) {
                    let feedback = self.plan_feedback(&path, name);
                    self.finish_plan(Decision::SendBack(feedback));
                }
                Ok(())
            }
            PromptAction::Commit => {
                if let (Some(review), false) = (&mut self.review, name.is_empty()) {
                    let result = review.commit(name);
                    self.say(result.unwrap_or_else(|e| e));
                }
                Ok(())
            }
            PromptAction::RenameSymbol if !name.is_empty() => {
                self.poll();
                if self.pending_rename.is_some() {
                    self.say("A rename is already running");
                } else if let Some((language, path, line, column)) = self.lsp_target()
                    && let Some(client) = self.servers.get_mut(language)
                {
                    client.rename(&path, line, column, name);
                    let index = self.active();
                    let revision = self.editors.get(index).map(|e| e.buffer.revision());
                    self.pending_rename = revision.map(|r| (path, r));
                }
                Ok(())
            }
            PromptAction::RenameSymbol => Ok(()),
            PromptAction::Comment(file, line) => {
                if !name.is_empty() {
                    self.add_comment(file.clone(), *line, name.to_string());
                }
                Ok(())
            }
            PromptAction::Reply(id) => {
                if let Some(thread) = self.threads.iter_mut().find(|t| t.id == *id)
                    && !name.is_empty()
                {
                    thread.replies.push(Reply {
                        author: "You".to_string(),
                        text: name.to_string(),
                    });
                    self.save_threads();
                }
                Ok(())
            }
            _ if name.is_empty() => Ok(()),
            PromptAction::NewBranch => {
                let result = create_branch(self.workspace.root(), name);
                self.branch_changed(name, result);
                Ok(())
            }
            PromptAction::NewFile(dir) => self.workspace.create_file(&dir.join(name)),
            PromptAction::NewFolder(dir) => self.workspace.create_dir(&dir.join(name)),
            PromptAction::Rename(path) => match path.parent() {
                Some(parent) => self.workspace.rename(path, &parent.join(name)),
                None => Ok(()),
            },
        };
        if let Err(e) = result {
            self.say(format!("Could not {}: {e}", prompt.label.to_lowercase()));
        }
        if let PromptAction::NewFile(dir) | PromptAction::NewFolder(dir) = &prompt.action {
            if dir != self.workspace.root() {
                self.workspace.expand(dir);
            }
            if matches!(prompt.action, PromptAction::NewFile(_)) && !name.is_empty() {
                let path = dir.join(name);
                if let Err(e) = self.open(&path) {
                    self.say(format!("Could not open: {e}"));
                }
            }
        }
        self.explorer.refresh(&self.workspace);
    }

    pub fn run(&mut self, command: Command) {
        let area = self.focused_area();
        match command {
            Command::Quit if self.editors.iter().any(|e| e.buffer.is_dirty()) => {
                self.confirm_leaving(Leaving::Quit);
            }
            Command::Quit => self.quit = true,
            Command::CloseTab
                if self
                    .editors
                    .get(self.active())
                    .is_some_and(|e| e.buffer.is_dirty()) =>
            {
                self.confirm_leaving(Leaving::CloseTab);
            }
            Command::OpenDiff if self.proposal.is_some() => self.reject_proposal(),
            Command::MentionSelection => self.mention_selection(),
            Command::AcceptProposal => self.accept_proposal(),
            Command::RejectProposal => self.reject_proposal(),
            Command::OpenDiff => {
                self.review = match self.review {
                    Some(_) => None,
                    None => Some(self.new_review(self.workspace.root().to_path_buf())),
                };
            }
            Command::AddComment => self.prompt_comment_at_cursor(),
            Command::FindInFile => self.open_find(),
            Command::Format(format) => {
                let area = self.focused_area();
                if let Some(editor) = self.editor_mut() {
                    editor.format(format, area);
                }
            }
            Command::SendToAgent => self.send_to_agent(),
            Command::ApprovePlan => {
                let path = self.plan.as_ref().map(|p| p.path.as_path());
                let edited = self
                    .editors
                    .iter()
                    .find(|e| path.is_some() && e.buffer.path() == path && e.buffer.is_dirty())
                    .map(|e| e.buffer.text());
                self.finish_plan(Decision::Approve(edited));
            }
            Command::SendBackPlan => {
                if self.plan.is_some() {
                    self.ask(
                        "What should change? Comments go too",
                        PromptAction::SendBackPlan,
                    );
                } else {
                    self.say("No plan is waiting for review");
                }
            }
            Command::ShowAgents => self.toggle_dock(),
            Command::FocusAgent => {
                let wanted = !self.dock.focused;
                self.dock.open |= wanted;
                self.dock.focused = wanted && self.dock.session().is_some();
            }
            Command::OpenGemini => self.open_agent("gemini"),
            Command::OpenSettings => {
                let closing = self.settings_page.is_some();
                self.settings_page = (!closing).then(|| SettingsPage::new(Found::look()));
                if !closing {
                    self.review = None;
                    self.drawer = false;
                }
            }
            Command::SwitchBranch => self.switch_branch_list(),
            Command::ReviewPreviousTurn => {
                if self.review.is_none() {
                    self.review = Some(self.new_review(self.workspace.root().to_path_buf()));
                }
                if let Some(review) = &mut self.review {
                    review.previous_turn();
                }
            }
            Command::UndoTurn => self.prompt_undo_turn(),
            Command::Commit => self.ask("Commit message", PromptAction::Commit),
            Command::OpenInBrowser => self.open_in_browser(),
            Command::OpenClaude => self.open_agent("claude"),
            Command::OpenCodex => self.open_agent("codex"),
            Command::FindInProject => self.show_search(Field::Query),
            Command::ReplaceInProject => self.show_search(Field::Replace),
            Command::MarkdownPreview => {
                let area = self.focused_area();
                if let Some(editor) = self.editor_mut() {
                    editor.rendered = !editor.rendered;
                    editor.top = 0;
                    editor.reveal(area);
                }
            }
            Command::GoToDefinition | Command::GoToReferences => self.lsp_request(command),
            Command::FormatDocument => self.format_document(),
            Command::Complete => {
                self.suggest_asked = true;
                self.poll();
                self.refresh_suggestions();
            }
            Command::GoToLine => self.ask("Go to line", PromptAction::GoToLine),
            Command::RenameSymbol => self.ask("Rename to", PromptAction::RenameSymbol),
            Command::BlameLine => self.blame(),
            Command::ToggleSidebar if self.compact() => self.drawer = !self.drawer,
            Command::ToggleSidebar => self.show_sidebar = !self.show_sidebar,
            Command::SplitPane if self.compact() => self.say("Not enough room to split"),
            Command::Save => self.save(),
            Command::CloseTab => self.close_tab(),
            Command::NextTab => self.cycle_tab(1),
            Command::PreviousTab => self.cycle_tab(-1),
            Command::SplitPane => {
                self.panes.push(self.active());
                self.focus = self.panes.len() - 1;
            }
            Command::FocusNextPane => self.focus = (self.focus + 1) % self.panes.len(),
            Command::QuickOpen => {
                let root = self.workspace.root().to_path_buf();
                let mut files: Vec<PathBuf> = self.workspace.files();
                files.sort();
                let items = files
                    .into_iter()
                    .map(|path| {
                        let label = path
                            .strip_prefix(&root)
                            .unwrap_or(&path)
                            .to_string_lossy()
                            .into_owned();
                        Item::File(path, label)
                    })
                    .collect();
                self.overlay = Overlay::Palette(Palette::new(items));
            }
            Command::CommandPalette => {
                let items = COMMANDS
                    .iter()
                    .map(|(label, command)| Item::Command(label, *command))
                    .collect();
                self.overlay = Overlay::Palette(Palette::new(items));
            }
            Command::Copy => self.copy(),
            Command::PasteLine => self.paste_line(),
            Command::Cut => {
                self.copy();
                if let Some(editor) = self.editor_mut()
                    && editor.selected_text().is_some()
                {
                    editor.buffer.replace(&mut editor.selections, "");
                }
            }
            Command::Paste => {
                let text = system_clipboard().unwrap_or_else(|| self.clipboard.clone());
                self.paste(&text);
            }
            Command::Cancel if self.review.is_some() => self.review = None,
            Command::Cancel => {
                if let Some(editor) = self.editor_mut() {
                    let head = editor.selections.primary().head;
                    editor.selections.set_single(Selection::cursor(head));
                }
            }
            command => {
                if let Some(editor) = self.editor_mut() {
                    editor.run(command, area);
                }
            }
        }
    }

    /// A Markdown file as a page of its own, written beside nothing the user owns.
    fn write_web_page(&self, path: &Path) -> std::io::Result<PathBuf> {
        let editor = &self.editors[self.active()];
        let name = path.file_stem().map(|n| n.to_string_lossy().into_owned());
        let title = name.unwrap_or_else(|| "Document".to_string());
        let base = path.parent().map(|p| p.display().to_string());
        let page = cellix_doc::html::page(
            &title,
            &editor.buffer.text(),
            base.as_deref().unwrap_or_default(),
        );
        let folder = std::env::temp_dir().join("cellix-pages");
        std::fs::create_dir_all(&folder)?;
        let out = folder.join(format!("{title}.html"));
        std::fs::write(&out, page)?;
        Ok(out)
    }

    /// What "open in browser" means here: the link at the cursor, or this file as a page.
    fn browser_target(&self) -> Option<String> {
        let editor = self.editors.get(self.active())?;
        let rope = editor.buffer.rope();
        let head = editor.selections.primary().head;
        let line = rope.char_to_line(head);
        let text = rope.line(line).to_string();
        rendered::link_at(text.trim_end(), head - rope.line_to_char(line)).or_else(|| {
            let path = editor.buffer.path()?;
            match path.extension()?.to_str()? {
                "html" | "htm" => Some(path.display().to_string()),
                "md" | "markdown" | "mdx" => {
                    let page = self.write_web_page(path).ok()?;
                    Some(page.display().to_string())
                }
                _ => None,
            }
        })
    }

    fn open_in_browser(&mut self) {
        let Some(target) = self.browser_target() else {
            self.say("Nothing to open: put the cursor on a link, or open an HTML file");
            return;
        };
        let mut commands: Vec<Vec<String>> = self.settings.browser.iter().cloned().collect();
        commands.push(self.system_browser.clone());
        // A command can start and still fail, so only its exit says the page opened.
        let (done, result) = std::sync::mpsc::channel();
        let page = target.clone();
        std::thread::spawn(move || {
            let opened = commands.iter().any(|command| ran_well(command, &page));
            let _ = done.send(opened);
        });
        self.browser_result = Some((target, result));
    }

    /// Says how a page's opening went, once it's known.
    fn poll_browser(&mut self) -> bool {
        let opened = self.browser_result.as_ref().and_then(|(target, result)| {
            let opened = result.try_recv().ok()?;
            Some((target.clone(), opened))
        });
        let Some((target, opened)) = opened else {
            return false;
        };
        self.browser_result = None;
        let name = target.rsplit('/').next().unwrap_or(&target).to_string();
        self.say(if opened {
            format!("Opened {name} in the browser")
        } else {
            "No browser would open. Set `browser` in the settings file".to_string()
        });
        true
    }

    /// Runs the real agent beside the editor, in a split of whatever hosts Cellix. Hosting
    /// it ourselves would mean chasing every update, so this stays a launcher.
    /// The environment and command that start an agent. For Claude Code they connect it to
    /// this Cellix and load the plugin, with `cellix` on the path for the plugin's hooks.
    fn agent_launch(&self, agent: &str) -> (Vec<String>, Vec<String>) {
        let mut launch = vec![agent.to_string()];
        // Any agent can answer a comment with `cellix reply`, under its own name.
        let mut env = vec![format!(
            "CELLIX_AGENT={}",
            crate::agents::display_name(agent)
        )];
        let own_folder = std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(|folder| folder.display().to_string()));
        if let (Some(folder), Ok(path)) = (own_folder, std::env::var("PATH")) {
            env.push(format!("PATH={folder}:{path}"));
        }
        if agent != "claude" {
            return (env, launch);
        }
        if let Some(ide) = &self.ide {
            env.push(format!("CLAUDE_CODE_SSE_PORT={}", ide.port));
            env.push("ENABLE_IDE_INTEGRATION=true".to_string());
        }
        // Once installed, Claude Code already loads it, and loading it twice would run
        // every hook twice.
        let var = |name: &str| std::env::var(name).ok();
        if !crate::integrations::claude_installed(var)
            && let Some(plugin) = crate::plugin::install()
        {
            launch.extend(["--plugin-dir".to_string(), plugin.display().to_string()]);
        }
        (env, launch)
    }

    /// The side pane's rectangle, title row included.
    fn dock_area(&self) -> Option<Rect> {
        let left = self.main_left();
        let height = self.size.1.saturating_sub(1);
        let width = self.dock.width_in(self.size.0.saturating_sub(left));
        (width > 0).then(|| Rect::new(self.size.0 - width, 0, width, height))
    }

    /// Where a session's screen goes.
    fn dock_screen(&self) -> Option<Rect> {
        self.dock_area().map(|a| {
            Rect::new(
                a.x + 1,
                a.y + 1,
                a.w.saturating_sub(1),
                a.h.saturating_sub(1),
            )
        })
    }

    /// Runs the real program in the side pane, so it updates itself and works as it does
    /// anywhere else, with the file still in view.
    fn open_agent(&mut self, agent: &str) {
        let root = self.workspace.root().to_path_buf();
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "sh".to_string());
        let (name, env, launch) = if agent == crate::agents::SHELL.1 {
            (crate::agents::SHELL.0, Vec::new(), vec![shell])
        } else {
            let (env, launch) = self.agent_launch(agent);
            (crate::agents::display_name(agent), env, launch)
        };
        self.start_session(name, &launch, &env, &root);
    }

    fn start_session(&mut self, name: &str, argv: &[String], env: &[String], folder: &Path) {
        self.dock.open = true;
        let size = self.dock_screen().map_or((80, 24), |a| (a.w, a.h));
        match Session::start(name, argv, env, folder, size) {
            Ok(session) => {
                self.dock.sessions.push(session);
                self.dock.active = Some(self.dock.sessions.len() - 1);
                self.dock.focused = true;
                self.dock.comments = false;
            }
            Err(e) => self.say(format!("Could not start {name}: {e}")),
        }
    }

    /// Keeps each session as big as the pane it's in.
    fn fit_dock(&mut self) {
        if let Some(screen) = self.dock_screen() {
            for session in &mut self.dock.sessions {
                session.resize((screen.w, screen.h));
            }
        }
    }

    /// Opens or closes the side pane. With nothing running it shows what can be started.
    fn toggle_dock(&mut self) {
        self.dock.open = !self.dock.open;
        self.dock.focused = self.dock.open && self.dock.session().is_some();
        if self.dock.open {
            self.agents.worktrees = worktrees(self.workspace.root());
            self.agents.look_for_agents();
            self.agents.scan(self.workspace.root());
        }
    }

    /// Everything the pane does with the pointer. Returns true if it took the event.
    fn dock_mouse(&mut self, mouse: MouseEvent) -> bool {
        let Some(area) = self.dock_area() else {
            return false;
        };
        let (x, y) = (mouse.x, mouse.y);
        let dragging = matches!(self.drag, Drag::Dock | Drag::Session);
        if mouse.kind == MouseKind::Drag && self.drag == Drag::Dock {
            self.dock.width = Some(self.size.0.saturating_sub(x));
            self.fit_dock();
            return true;
        }
        if mouse.kind == MouseKind::Up && dragging {
            let was = std::mem::replace(&mut self.drag, Drag::None);
            if was == Drag::Session {
                self.dock_session_mouse(mouse);
            }
            return true;
        }
        if mouse.kind == MouseKind::Drag && self.drag == Drag::Session {
            self.dock_session_mouse(mouse);
            return true;
        }
        if !area.contains(x, y) {
            if mouse.kind == MouseKind::Left {
                self.dock.focused = false;
            }
            return false;
        }
        if y == area.y {
            if mouse.kind == MouseKind::Left {
                match self.dock.hit(area, x) {
                    Some(TabHit::Tab(index)) => {
                        self.dock.active = Some(index);
                        self.dock.focused = true;
                        self.dock.comments = false;
                    }
                    Some(TabHit::Close(index)) => self.dock.close(index),
                    Some(TabHit::New) => {
                        self.dock.active = None;
                        self.dock.focused = false;
                        self.dock.comments = false;
                    }
                    Some(TabHit::Comments) => {
                        self.dock.comments = !self.dock.comments;
                        self.dock.focused = false;
                    }
                    None => {}
                }
            }
            return true;
        }
        if x == area.x && matches!(mouse.kind, MouseKind::Left) {
            self.drag = Drag::Dock;
            return true;
        }
        if self.dock.comments {
            let screen = self.dock_screen().unwrap_or(area);
            match mouse.kind {
                MouseKind::Left => {
                    let threads = self.open_threads();
                    let spot = (x, y);
                    let hit = threads::hit(
                        screen,
                        &threads,
                        self.dock.comments_top,
                        spot,
                        &*self.measure,
                    );
                    if let Some(hit) = hit {
                        self.thread_hit(hit);
                    }
                }
                MouseKind::ScrollUp => {
                    self.dock.comments_top = self.dock.comments_top.saturating_sub(3);
                }
                MouseKind::ScrollDown => self.dock.comments_top += 3,
                _ => {}
            }
            return true;
        }
        if self.dock.session().is_none() {
            if mouse.kind == MouseKind::Left {
                let screen = self.dock_screen().unwrap_or(area);
                match self.agents.hit(screen, y) {
                    Some(AgentHit::Start(program)) => self.open_agent(program),
                    Some(AgentHit::Worktree(path)) => {
                        self.review = Some(self.new_review(path));
                    }
                    None => {}
                }
            }
            return true;
        }
        match mouse.kind {
            MouseKind::ScrollUp | MouseKind::ScrollDown => {
                let up = mouse.kind == MouseKind::ScrollUp;
                let spot = self.session_spot(x, y);
                if let Some(session) = self.dock.session_mut() {
                    session.wheel(up, spot);
                }
            }
            MouseKind::Left => {
                self.dock.focused = true;
                self.drag = Drag::Session;
                self.dock_session_mouse(mouse);
            }
            _ => {}
        }
        true
    }

    fn session_spot(&self, x: u16, y: u16) -> (u16, u16) {
        let screen = self.dock_screen().unwrap_or_default();
        (
            x.saturating_sub(screen.x).min(screen.w.saturating_sub(1)),
            y.saturating_sub(screen.y).min(screen.h.saturating_sub(1)),
        )
    }

    fn dock_session_mouse(&mut self, mouse: MouseEvent) {
        let spot = self.session_spot(mouse.x, mouse.y);
        let copied = self
            .dock
            .session_mut()
            .and_then(|session| session.mouse(mouse.kind, spot));
        if let Some(text) = copied {
            self.copy_text(text, "Copied");
        }
    }

    fn draw_dock(&self, grid: &mut Grid) {
        let (Some(area), Some(screen)) = (self.dock_area(), self.dock_screen()) else {
            return;
        };
        let theme = &self.theme;
        grid.fill(area, theme.text);
        let threads = self.open_threads();
        self.dock
            .draw_tabs(grid, area, (self.hover, threads.len()), theme);
        grid.hairline(
            Bounds::cells(area.x as f32, area.y as f32, 0.0, area.h as f32),
            theme.border,
        );
        if self.dock.comments {
            let look = (&*self.measure, theme);
            let top = self.dock.comments_top;
            threads::draw(grid, screen, &threads, top, self.hover, look);
            return;
        }
        match self.dock.session() {
            Some(session) => session.draw(grid, screen, theme, self.dock.focused),
            None => self.agents.draw(grid, screen, self.hover, theme),
        }
    }

    /// The comments on a plan and any edits to it, as an answer the agent can act on.
    fn plan_feedback(&self, path: &Path, note: &str) -> String {
        let file = self.relative(path);
        let mut comments = self
            .comments
            .lock()
            .map_or_else(|_| Vec::new(), |c| c.clone());
        comments.retain(|c| c.file == file);
        let mut text = String::new();
        if !note.is_empty() {
            text.push_str(note);
            text.push('\n');
        }
        if !comments.is_empty() {
            text.push_str(&comments_message(&comments));
        }
        let editor = self.editors.iter().find(|e| e.buffer.path() == Some(path));
        if let Some(editor) = editor.filter(|e| e.buffer.is_dirty()) {
            text.push_str("The reviewer edited the plan to read:\n");
            text.push_str(&editor.buffer.text());
        }
        text
    }

    /// Answers the waiting agent and closes the plan.
    fn finish_plan(&mut self, decision: Decision) {
        let Some(plan) = self.plan.take() else {
            self.say("No plan is waiting for review");
            return;
        };
        let _ = plan.reply.send(decision);
        self.agents.plan_answered();
        let file = self.relative(&plan.path);
        if let Ok(mut comments) = self.comments.lock() {
            comments.retain(|c| c.file != file);
        }
        if let Some(index) = self
            .editors
            .iter()
            .position(|e| e.buffer.path() == Some(plan.path.as_path()))
        {
            self.set_active(index);
            self.close_tab();
        }
    }

    /// Delivers comments now if Cellix knows the agent's pane. Otherwise they wait, and the
    /// hook adds them to the next prompt.
    fn send_to_agent(&mut self) {
        let comments = self
            .comments
            .lock()
            .map_or_else(|_| Vec::new(), |c| c.clone());
        if comments.is_empty() {
            self.say("No comments to send");
            return;
        }
        let Some(session) = self.dock.agent_mut() else {
            self.say("Comments will go with your next prompt to the agent");
            return;
        };
        // Sent as one line, because a newline in a prompt box would submit it early.
        session.send_line(&comments_message(&comments).replace('\n', " "));
        if let Ok(mut pending) = self.comments.lock() {
            pending.clear();
        }
        self.say(format!("Sent {} comments to the agent", comments.len()));
    }

    fn new_review(&self, root: PathBuf) -> Review {
        let mut review = Review::new(root, self.registry.clone());
        review.tests.clone_from(&self.tests);
        review
    }

    fn prompt_undo_turn(&mut self) {
        let root = self.workspace.root();
        let target = match &self.review {
            Some(review) => review.undo_target(),
            None => turns(root).last().cloned(),
        };
        match target {
            Some(turn) => {
                self.overlay = Overlay::Confirm(Confirm {
                    title: format!("Undo back to \u{201c}{}\u{201d}?", turn.label),
                    detail: "Files go back to that point. A checkpoint is kept, so you can redo."
                        .to_string(),
                    buttons: vec![
                        ("Undo Turn", Choice::Undo(turn.commit)),
                        ("Cancel", Choice::Cancel),
                    ],
                    selected: 0,
                });
            }
            None => self.say("No agent turns recorded here yet"),
        }
    }

    fn show_search(&mut self, field: Field) {
        self.review = None;
        self.sidebar_view = SidebarView::Search;
        self.show_sidebar = true;
        self.drawer = self.compact();
        self.search.focus = Some(field);
    }

    fn show_explorer(&mut self) {
        self.search.focus = None;
        self.review = None;
        self.sidebar_view = SidebarView::Explorer;
    }

    /// Keys for the search boxes. Returns false to let a shortcut through.
    fn search_key(&mut self, key: KeyEvent) -> bool {
        if key.mods.ctrl || key.mods.alt {
            return false;
        }
        match key.key {
            Key::Esc => {
                self.search.focus = None;
                self.drawer = false;
            }
            Key::Tab | Key::BackTab => {
                self.search.focus = Some(match self.search.focus {
                    Some(Field::Query) => Field::Replace,
                    _ => Field::Query,
                });
            }
            Key::Enter if self.search.focus == Some(Field::Replace) => self.replace_all(),
            Key::Enter => self.search.start(),
            Key::Backspace => {
                if let Some(field) = self.search.field_mut() {
                    field.pop();
                }
                self.search_edited();
            }
            Key::Char(c) => {
                if let Some(field) = self.search.field_mut() {
                    field.push(c);
                }
                self.search_edited();
            }
            _ => return false,
        }
        true
    }

    /// Results follow the query as it's typed, and don't depend on the replacement.
    fn search_edited(&mut self) {
        if self.search.focus == Some(Field::Query) {
            self.search.start();
        }
    }

    fn replace_all(&mut self) {
        let (query, with) = (self.search.query.clone(), self.search.replace.clone());
        let dirty: Vec<PathBuf> = self
            .editors
            .iter()
            .filter(|e| e.buffer.is_dirty())
            .filter_map(|e| e.buffer.path().map(Path::to_path_buf))
            .collect();
        match self.workspace.replace_all(&query, &with, &dirty) {
            Ok(count) => {
                let skipped = if dirty.is_empty() {
                    String::new()
                } else {
                    format!(", skipped {} with unsaved edits", dirty.len())
                };
                self.say(format!("Replaced in {count} files{skipped}"));
            }
            Err(e) => self.say(format!("Could not replace: {e}")),
        }
        self.search.start();
    }

    fn open_match(&mut self, found: &cellix_core::Match) {
        if let Err(e) = self.open(&found.path) {
            self.say(format!("Could not open: {e}"));
            return;
        }
        self.drawer = false;
        self.search.focus = None;
        let area = self.focused_area();
        let length = self.search.query.chars().count();
        if let Some(editor) = self.editor_mut() {
            let start = editor.buffer.position(found.line, found.column);
            editor.selections.set_single(Selection {
                anchor: start,
                head: start + length,
                goal: None,
            });
            editor.reveal(area);
        }
    }

    /// Opens a test run's output in a tab of its own.
    fn show_test_output(&mut self, index: usize) {
        let Some(run) = self.tests.get(index) else {
            return;
        };
        let text = format!("$ {}\n\n{}", run.command, run.output);
        let buffer = Buffer::from_text(&text);
        let editor = Editor::new(buffer, self.measure.clone(), self.registry.clone());
        self.editors.push(editor);
        self.set_active(self.editors.len() - 1);
        self.review = None;
    }

    fn click_search(&mut self, area: Rect, x: u16, y: u16) {
        match self.search.hit(area, x, y) {
            Some(search::Hit::Field(field)) => self.search.focus = Some(field),
            Some(search::Hit::ReplaceAll) => self.replace_all(),
            Some(search::Hit::File(path)) => self.search.toggle_file(path),
            Some(search::Hit::Match(found)) => self.open_match(&found),
            None => {}
        }
    }

    fn ask(&mut self, label: &str, action: PromptAction) {
        self.overlay = Overlay::Prompt(Prompt {
            label: label.to_string(),
            input: String::new(),
            action,
        });
    }

    fn save_dirty(&mut self, only_active: bool) {
        let active = self.active();
        for (i, editor) in self.editors.iter_mut().enumerate() {
            if editor.buffer.is_dirty() && (!only_active || i == active) {
                let _ = editor.buffer.save();
            }
        }
    }

    fn relative(&self, path: &Path) -> String {
        path.strip_prefix(self.workspace.root())
            .unwrap_or(path)
            .to_string_lossy()
            .into_owned()
    }

    fn add_comment(&mut self, file: String, line: usize, text: String) {
        // A plan's comments are its answer, and `--wait` writes them into the file.
        let answering = self.plan.is_some() || self.wait_file.is_some();
        let page = self.editors.get(self.active()).filter(|e| e.rendered);
        if let Some(editor) = page.filter(|_| !answering) {
            // A document's comments become conversations, kept beside it.
            let quote = editor
                .selected_text()
                .unwrap_or_else(|| editor.line_text(line.saturating_sub(1)));
            let quote = quote.lines().next().unwrap_or_default().trim().to_string();
            self.threads.push(Thread {
                id: cellix_review::threads::next_id(&self.threads),
                file,
                line,
                quote,
                text,
                replies: Vec::new(),
                resolved: false,
            });
            self.save_threads();
            self.dock.open = true;
            self.dock.comments = true;
            self.dock.focused = false;
            return;
        }
        if let Ok(mut comments) = self.comments.lock() {
            comments.push(Comment { file, line, text });
        }
    }

    fn save_threads(&mut self) {
        if self.threads_broken {
            self.say("The comments file can't be read, so new comments aren't being saved");
            return;
        }
        if let Err(e) = cellix_review::threads::save(self.workspace.root(), &self.threads) {
            self.say(format!("Could not save comments: {e}"));
        }
    }

    /// The open comments on the file in view, top to bottom.
    fn open_threads(&self) -> Vec<&Thread> {
        let file = self
            .editors
            .get(self.active())
            .and_then(|e| e.buffer.path())
            .map(|p| self.relative(p));
        let mut threads: Vec<&Thread> = self
            .threads
            .iter()
            .filter(|t| !t.resolved && file.as_ref().is_none_or(|f| *f == t.file))
            .collect();
        threads.sort_by_key(|t| t.line);
        threads
    }

    fn thread_hit(&mut self, hit: ThreadHit) {
        match hit {
            ThreadHit::Open(id) => {
                let line = self.threads.iter().find(|t| t.id == id).map(|t| t.line);
                let area = self.focused_area();
                if let (Some(line), Some(editor)) = (line, self.editor_mut()) {
                    editor.go_to_line(line, area);
                }
                self.dock.focused = false;
            }
            ThreadHit::Reply(id) => self.ask("Reply", PromptAction::Reply(id)),
            ThreadHit::Ask(id) => {
                let message = self
                    .threads
                    .iter()
                    .find(|t| t.id == id)
                    .map(Thread::message);
                match (message, self.dock.agent_mut()) {
                    (Some(message), Some(session)) => {
                        session.send_line(&message);
                        self.say("Sent to the agent. Its answer lands here");
                    }
                    _ => {
                        self.dock.comments = false;
                        self.dock.active = None;
                        self.say("Start an agent here first");
                    }
                }
            }
            ThreadHit::Resolve(id) => {
                if let Some(thread) = self.threads.iter_mut().find(|t| t.id == id) {
                    thread.resolved = true;
                }
                self.save_threads();
            }
        }
    }

    fn comment_lines(&self, file: &str) -> Vec<usize> {
        let waiting = self.comments.lock().map_or_else(
            |_| Vec::new(),
            |c| {
                c.iter()
                    .filter(|c| c.file == file)
                    .map(|c| c.line)
                    .collect()
            },
        );
        let threads = self
            .threads
            .iter()
            .filter(|t| !t.resolved && t.file == file);
        waiting.into_iter().chain(threads.map(|t| t.line)).collect()
    }

    fn cursor_line(&self) -> Option<(PathBuf, usize)> {
        let editor = self.editors.get(self.active())?;
        let path = editor.buffer.path()?.to_path_buf();
        let rope = editor.buffer.rope();
        Some((
            path,
            rope.char_to_line(editor.selections.primary().head) + 1,
        ))
    }

    fn prompt_comment_at_cursor(&mut self) {
        if let Some((path, line)) = self.cursor_line() {
            let file = self.relative(&path);
            self.ask(
                &format!("Comment on line {line}"),
                PromptAction::Comment(file, line),
            );
        }
    }

    fn blame(&mut self) {
        if let Some((path, line)) = self.cursor_line() {
            let file = self.relative(&path);
            let root = self.workspace.root().to_path_buf();
            let note = blame_line(&root, Path::new(&file), line);
            self.say(note.unwrap_or_else(|| "No blame for this line".to_string()));
        }
    }

    /// Runs requests from the socket thread.
    pub fn serve(&mut self, request: Request) {
        match request {
            Request::Open(path) => {
                if let Err(e) = self.open(&path) {
                    self.say(format!("Could not open {}: {e}", path.display()));
                }
            }
            Request::CommentsTaken => self.say("An agent took the review comments"),
            Request::Reply { id, author, text } => {
                match self.threads.iter_mut().find(|t| t.id == id) {
                    Some(thread) => {
                        thread.replies.push(Reply {
                            author: author.clone(),
                            text,
                        });
                        self.save_threads();
                        self.say(format!("{author} answered a comment"));
                    }
                    None => self.say(format!("{author} answered a comment that's gone")),
                }
            }
            Request::Threads(reply) => {
                let open: Vec<&Thread> = self.threads.iter().filter(|t| !t.resolved).collect();
                let json = serde_json::to_string(&open).unwrap_or_else(|_| "[]".to_string());
                let _ = reply.send(json);
            }
            // Only the pane loop reads these, and it handles them before they get here.
            Request::Pane(_) => {}
            Request::Agent(event) => self.agents.update(event),
            Request::Test(run) => {
                let text = format!(
                    "{} {}",
                    run.command,
                    if run.passed { "passed" } else { "failed" }
                );
                self.say(text);
                self.tests.push(run);
                if let Some(review) = &mut self.review {
                    review.tests.clone_from(&self.tests);
                }
            }
            Request::Plan { path, reply } => {
                self.review = None;
                match self.open(&path) {
                    Ok(()) => {
                        self.plan = Some(PendingPlan { path, reply });
                        self.attention = Some("plan");
                        self.say("An agent is waiting for you to review this plan");
                    }
                    // Dropping `reply` lets the agent ask in its own way.
                    Err(e) => self.say(format!("Could not open the plan: {e}")),
                }
            }
        }
    }

    /// Writes comments into the `--wait` file. Called once, as the app exits.
    pub fn finish(&self) -> io::Result<()> {
        let Some(path) = &self.wait_file else {
            return Ok(());
        };
        let file = self.relative(path);
        let comments: Vec<Comment> = self.comments.lock().map_or_else(
            |_| Vec::new(),
            |c| c.iter().filter(|c| c.file == file).cloned().collect(),
        );
        if comments.is_empty() {
            return Ok(());
        }
        let text = std::fs::read_to_string(path)?;
        std::fs::write(path, append_comments(&text, &comments))
    }

    fn click_review(&mut self, x: u16, y: u16) {
        let area = self.main_area();
        let Some(review) = &mut self.review else {
            return;
        };
        let comments = self.comments.lock().map(|c| c.clone()).unwrap_or_default();
        match review.hit(area, x, y, &comments) {
            Some(Hit::File(index)) => review.select(index),
            Some(Hit::AcceptProposal) => self.accept_proposal(),
            Some(Hit::RejectProposal) => self.reject_proposal(),
            Some(Hit::Accept(hunk)) if review.is_proposal() => {
                if review.keep_proposed(hunk) {
                    self.accept_proposal();
                }
            }
            Some(Hit::Reject(hunk)) if review.is_proposal() => {
                if review.drop_proposed(hunk) {
                    self.accept_proposal();
                }
            }
            Some(Hit::Accept(hunk)) => {
                let result = review.accept(hunk);
                if let Err(e) = result {
                    self.say(format!("Could not stage: {e}"));
                }
            }
            Some(Hit::Base(base)) => review.set_base(base),
            Some(Hit::Commit) => self.ask("Commit message", PromptAction::Commit),
            Some(Hit::Test(index)) => self.show_test_output(index),
            Some(Hit::Reject(hunk)) => self.reject(hunk),
            Some(Hit::PreviousFile) => review.step_file(-1),
            Some(Hit::NextFile) => review.step_file(1),
            Some(Hit::Line(file, line)) => {
                self.ask(
                    &format!("Comment on line {line}"),
                    PromptAction::Comment(file, line),
                );
            }
            None => {}
        }
    }

    /// Puts one hunk back on disk. Refuses if the file has unsaved edits, which the
    /// rejection would silently overwrite.
    fn reject(&mut self, hunk: usize) {
        let Some(review) = &self.review else {
            return;
        };
        let (Some(path), Some(text)) = (review.absolute_path(), review.rejected_text(hunk)) else {
            return;
        };
        let dirty = self
            .editors
            .iter()
            .any(|e| e.buffer.path() == Some(path.as_path()) && e.buffer.is_dirty());
        if dirty {
            self.say("Save or undo your edits to this file first");
            return;
        }
        let note = match std::fs::write(&path, text) {
            Ok(()) => "Rejected the hunk".to_string(),
            Err(e) => format!("Could not write the file: {e}"),
        };
        self.say(note);
        if let Some(review) = &mut self.review {
            review.refresh();
        }
    }

    /// Vim's `p`: a copied line goes below this one, and other text after the cursor.
    fn paste_line(&mut self) {
        let text = self.clipboard.clone();
        let area = self.focused_area();
        if text.ends_with('\n') {
            self.run(Command::Move(cellix_input::Direction::Down, false));
            self.run(Command::Move(
                cellix_input::Direction::LineColumnZero,
                false,
            ));
        } else {
            self.run(Command::Move(cellix_input::Direction::Right, false));
        }
        if let Some(editor) = self.editor_mut() {
            editor.buffer.replace(&mut editor.selections, &text);
            editor.reveal(area);
        }
    }

    fn copy(&mut self) {
        if let Some(text) = self.selected_text() {
            self.clipboard.clone_from(&text);
            self.outgoing_clipboard = Some(text);
        }
    }

    fn save(&mut self) {
        let Some(editor) = self.editor_mut() else {
            return;
        };
        let note = match editor.buffer.save() {
            Ok(()) => format!("Saved {}", editor.title()),
            Err(e) => format!("Could not save: {e}"),
        };
        self.say(note);
        // Sent after any pending edit, so the server checks what was saved.
        self.poll();
        if let Some((language, path, ..)) = self.lsp_target()
            && let Some(client) = self.servers.get(language)
        {
            client.saved(&path);
        }
    }

    fn cycle_tab(&mut self, delta: isize) {
        let count = self.editors.len() as isize;
        if count > 0 {
            self.set_active((self.active() as isize + delta).rem_euclid(count) as usize);
        }
    }

    fn close_tab(&mut self) {
        let index = self.active();
        if index >= self.editors.len() {
            return;
        }
        let closed = self.editors.remove(index);
        // Closing an unanswered plan hands the question back to the agent's own dialog.
        if let Some(plan) = &self.plan
            && closed.buffer.path() == Some(plan.path.as_path())
        {
            self.plan = None;
        }
        let last = self.editors.len().saturating_sub(1);
        for pane in &mut self.panes {
            if *pane > index {
                *pane -= 1;
            }
            *pane = (*pane).min(last);
        }
        if self.panes.len() > 1 {
            self.panes.remove(self.focus);
            self.focus = self.focus.min(self.panes.len() - 1);
        }
    }

    fn pane_at(&self, x: u16, y: u16) -> Option<usize> {
        (0..self.panes.len()).find(|i| self.pane_area(*i).contains(x, y))
    }

    fn mouse(&mut self, mouse: MouseEvent) {
        if mouse.kind == MouseKind::Move {
            self.hover = Some((mouse.x, mouse.y));
            return;
        }
        if !matches!(self.overlay, Overlay::None) {
            self.overlay_mouse(mouse);
            return;
        }
        if mouse.kind == MouseKind::Left && self.click_offer(mouse.x, mouse.y) {
            return;
        }
        if mouse.x != u16::MAX && self.dock_mouse(mouse) {
            return;
        }
        if let Some(area) = self.suggest_area() {
            let on_row = area.contains(mouse.x, mouse.y) && mouse.kind == MouseKind::Left;
            let row = mouse.y.saturating_sub(area.y + 1) as usize;
            if on_row {
                self.accept_suggestion(row);
                return;
            }
            self.close_suggestions();
        }
        // The wheel over a pane comes with no position, so it means the file in front.
        if mouse.x == u16::MAX {
            let delta = if mouse.kind == MouseKind::ScrollUp {
                -1
            } else {
                1
            };
            if let Some(review) = &mut self.review {
                review.scroll_by(delta);
            } else if let Some(editor) = self.editor_mut() {
                editor.scroll_by(delta);
            }
            return;
        }
        let (x, y, fine_x) = (mouse.x, mouse.y, mouse.fine_x);
        if self.compact() && self.drawer && !self.sidebar_area().is_some_and(|a| a.contains(x, y)) {
            self.drawer = false;
            return;
        }
        if self.activity_area().contains(x, y) {
            if mouse.kind == MouseKind::Left {
                self.click_activity(y);
            }
            return;
        }
        let status_row = self.size.1.saturating_sub(1);
        if y == status_row {
            let on_branch = self
                .status_branch()
                .is_some_and(|(at, end)| x >= at && x < end);
            if mouse.kind == MouseKind::Left && on_branch {
                self.run(Command::SwitchBranch);
            }
            return;
        }
        let in_sidebar = self.sidebar_area().is_some_and(|a| a.contains(x, y));
        if self.settings_page.is_some() && !in_sidebar {
            self.settings_mouse(mouse);
            return;
        }
        if self.review.is_some() && !in_sidebar && self.main_area().contains(x, y) {
            match mouse.kind {
                MouseKind::Left => self.click_review(x, y),
                MouseKind::ScrollUp | MouseKind::ScrollDown => {
                    let delta = if mouse.kind == MouseKind::ScrollUp {
                        -self.wheel_lines
                    } else {
                        self.wheel_lines
                    };
                    if let Some(review) = &mut self.review {
                        review.scroll_by(delta);
                    }
                }
                _ => {}
            }
            return;
        }
        match mouse.kind {
            MouseKind::Middle if y == self.main_area().y && !in_sidebar => {
                let tab = self
                    .visible_tabs()
                    .into_iter()
                    .find(|(_, at, width)| x >= *at && x < at + width);
                if let Some((index, _, _)) = tab {
                    self.set_active(index);
                    self.run(Command::CloseTab);
                }
            }
            MouseKind::ScrollUp | MouseKind::ScrollDown => {
                // A trackpad sends a stream of events, one per line of travel, and a wheel
                // sends a notch at a time. Treating them alike makes a trackpad lurch.
                let streaming = self
                    .last_wheel
                    .is_some_and(|at| at.elapsed() < WHEEL_STREAM_GAP);
                self.last_wheel = Some(std::time::Instant::now());
                let step = if streaming { 1 } else { self.wheel_lines };
                let delta = if mouse.kind == MouseKind::ScrollUp {
                    -step
                } else {
                    step
                };
                if in_sidebar && self.sidebar_view == SidebarView::Search {
                    self.search.scroll_by(delta);
                } else if in_sidebar {
                    self.explorer.scroll_by(delta);
                } else if self.pane_at(x, y).is_some() {
                    let area = self.focused_area();
                    if let Some(editor) = self.editor_mut() {
                        let before = editor.top() as i32;
                        editor.scroll_by(delta);
                        let rows = editor.top() as i32 - before;
                        let text = editor.scroll_region(area);
                        let total = self.scrolled.map_or(0, |(_, r)| r) + rows;
                        self.scrolled = Some((text, total));
                    }
                }
            }
            MouseKind::Left if in_sidebar && self.sidebar_view == SidebarView::Search => {
                if let Some(area) = self.sidebar_area() {
                    self.click_search(area, x, y);
                }
            }
            MouseKind::Right if in_sidebar && self.sidebar_view == SidebarView::Explorer => {
                self.open_explorer_menu(x, y);
            }
            MouseKind::Right if self.tabs_area().contains(x, y) => self.open_tab_menu(x, y),
            MouseKind::Right if self.pane_at(x, y).is_some() => {
                if let Some(pane) = self.pane_at(x, y) {
                    self.focus = pane;
                }
                let area = self.focused_area();
                if let Some(editor) = self.editor_mut() {
                    let at = editor.position_at(area, fine_x, y);
                    // A click on the selection is about the selection, so it stays.
                    let on_it = editor
                        .selections
                        .iter()
                        .any(|s| at >= s.start() && at < s.end());
                    if !on_it {
                        editor.selections.set_single(Selection::cursor(at));
                    }
                }
                self.open_editor_menu(x, y);
            }
            MouseKind::Left if in_sidebar && self.on_explorer_header(y) => {
                self.run(Command::SwitchBranch);
            }
            MouseKind::Left if in_sidebar => self.click_explorer(y),
            MouseKind::Left
                if y == 1
                    && !in_sidebar
                    && self.toolbar_row().is_some()
                    && self.click_toolbar(x) => {}
            MouseKind::Left if y == self.plan_bar().y + 1 && self.plan_in_view() && !in_sidebar => {
                self.click_plan_bar(x)
            }
            MouseKind::Left if self.compact() && y == 0 && x < MENU_BUTTON => {
                self.run(Command::ToggleSidebar);
            }
            MouseKind::Left if self.tabs_area().contains(x, y) => self.click_tab(x),
            MouseKind::Left => {
                let Some(pane) = self.pane_at(x, y) else {
                    return;
                };
                self.search.focus = None;
                self.focus = pane;
                let area = self.focused_area();
                let clicks = self.count_click(x, y);
                if let Some(editor) = self.editor_mut() {
                    if editor.on_scrollbar(area, x) {
                        editor.scroll_to_row(area, y);
                        self.drag = Drag::Scrollbar;
                        return;
                    }
                    if editor.click_checkbox(area, fine_x, y) {
                        return;
                    }
                    let at = editor.position_at(area, fine_x, y);
                    if clicks > 1 {
                        if clicks == 2 {
                            editor.select_word_at(at);
                        } else {
                            editor.select_line_at(at);
                        }
                        self.drag = Drag::Text;
                        return;
                    }
                    if mouse.mods.ctrl {
                        editor.selections.set_single(Selection::cursor(at));
                        self.open_in_browser();
                        return;
                    }
                    if mouse.mods.shift {
                        let anchor = editor.selections.primary().anchor;
                        editor.selections.set_single(Selection {
                            anchor,
                            head: at,
                            goal: None,
                        });
                    } else if mouse.mods.alt {
                        editor.selections.add(Selection::cursor(at));
                    } else {
                        editor.selections.set_single(Selection::cursor(at));
                    }
                    self.drag = Drag::Text;
                }
            }
            MouseKind::Drag if self.drag == Drag::Text => {
                let area = self.focused_area();
                if let Some(editor) = self.editor_mut() {
                    let at = editor.position_at(area, fine_x, y);
                    let anchor = editor.selections.primary().anchor;
                    editor.selections.set_single(Selection {
                        anchor,
                        head: at,
                        goal: None,
                    });
                    editor.reveal(area);
                }
            }
            MouseKind::Drag if self.drag == Drag::Scrollbar => {
                let area = self.focused_area();
                if let Some(editor) = self.editor_mut() {
                    editor.scroll_to_row(area, y);
                }
            }
            MouseKind::Up => self.drag = Drag::None,
            _ => {}
        }
    }

    /// One, two or three, for a press in this place soon after the last.
    fn count_click(&mut self, x: u16, y: u16) -> u8 {
        let again = self.last_click.filter(|(at, click_x, click_y, _)| {
            at.elapsed() < MULTI_CLICK && *click_y == y && click_x.abs_diff(x) <= 1
        });
        let count = again.map_or(1, |(.., count)| count % 3 + 1);
        self.last_click = Some((std::time::Instant::now(), x, y, count));
        count
    }

    fn on_explorer_header(&self, y: u16) -> bool {
        self.sidebar_view == SidebarView::Explorer
            && self.branch.is_some()
            && self.sidebar_area().is_some_and(|a| y == a.y)
    }

    fn overlay_mouse(&mut self, mouse: MouseEvent) {
        // Another right click moves the menu there, as it does everywhere else.
        if mouse.kind == MouseKind::Right && matches!(self.overlay, Overlay::Menu(_)) {
            self.overlay = Overlay::None;
            self.mouse(mouse);
            return;
        }
        if mouse.kind != MouseKind::Left {
            return;
        }
        let screen = Rect::new(0, 0, self.size.0, self.size.1);
        let overlay = std::mem::replace(&mut self.overlay, Overlay::None);
        match overlay {
            Overlay::Palette(mut palette) => {
                if let Some(row) = palette.row_at(screen, mouse.y) {
                    palette.selected = row;
                    self.pick(palette.current().cloned());
                }
            }
            Overlay::Menu(menu) => {
                // A greyed row stays open, so a slip doesn't dismiss the menu.
                if let Some(row) = menu.row_at(mouse.x, mouse.y) {
                    match menu.action(row) {
                        Some(action) => self.menu_action(action.clone()),
                        None => self.overlay = Overlay::Menu(menu),
                    }
                }
            }
            Overlay::Find(find) => self.click_find(find, mouse.x, mouse.y),
            Overlay::TablePicker(picker) => {
                if let Some(size) = picker.size_at(mouse.x, mouse.y) {
                    self.insert_table(size);
                }
            }
            Overlay::Math(panel) => {
                if let Some(index) = panel.symbol_at(mouse.x, mouse.y) {
                    self.insert_math(index);
                }
            }
            Overlay::Confirm(confirm) => {
                let y = self.confirm_area().y + 4;
                let spots = self.confirm_buttons(&confirm);
                let width = |cells: &String| cells.chars().count() as u16;
                let on = spots
                    .iter()
                    .position(|(c, x)| mouse.y == y && mouse.x >= *x && mouse.x < x + width(c));
                match on {
                    Some(index) => self.choose(confirm.buttons[index].1.clone()),
                    None => self.overlay = Overlay::Confirm(confirm),
                }
            }
            other => self.overlay = other,
        }
    }

    fn show_menu(&mut self, at: (u16, u16), items: Vec<MenuItem>) {
        let screen = Rect::new(0, 0, self.size.0, self.size.1);
        self.overlay = Overlay::Menu(Menu::new(at, items, &*self.measure, screen));
    }

    fn open_explorer_menu(&mut self, x: u16, y: u16) {
        let target = self.sidebar_area().and_then(|area| {
            let index = self.explorer.entry_at(area, y)?;
            self.explorer.selected = Some(index);
            Some(self.explorer.entries[index].path.clone())
        });
        let root = self.workspace.root().to_path_buf();
        let target = target.unwrap_or_else(|| root.clone());
        let dir = if target.is_dir() {
            target.clone()
        } else {
            target.parent().unwrap_or(&root).to_path_buf()
        };
        let entry = target != root;
        let items = vec![
            MenuItem::new("New File", MenuAction::NewFile(dir.clone())),
            MenuItem::new("New Folder", MenuAction::NewFolder(dir)),
            MenuItem::new("Rename", MenuAction::Rename(target.clone()))
                .after_rule()
                .when(entry),
            MenuItem::new("Delete", MenuAction::Delete(target.clone())).when(entry),
            MenuItem::new("Copy Path", MenuAction::CopyPath(target.clone())).after_rule(),
            MenuItem::new("Copy Relative Path", MenuAction::CopyRelativePath(target)),
        ];
        self.show_menu((x, y), items);
    }

    fn open_tab_menu(&mut self, x: u16, y: u16) {
        let tab = self
            .visible_tabs()
            .into_iter()
            .find(|(_, at, width)| x >= *at && x < at + width);
        let Some((index, ..)) = tab else {
            return;
        };
        // Below the breadcrumbs, so its edge runs through neither them nor the title.
        let y = y + 3;
        let path = self.editors[index].buffer.path().map(Path::to_path_buf);
        let others = self.editors.len() > 1;
        let on_file = |action: fn(PathBuf) -> MenuAction| {
            let fallback = PathBuf::new();
            action(path.clone().unwrap_or(fallback))
        };
        let items = vec![
            MenuItem::new("Close", MenuAction::CloseTab(index)),
            MenuItem::new("Close Others", MenuAction::CloseOtherTabs(index)).when(others),
            MenuItem::new("Close All", MenuAction::CloseAllTabs),
            MenuItem::new("Copy Path", on_file(MenuAction::CopyPath))
                .after_rule()
                .when(path.is_some()),
            MenuItem::new("Copy Relative Path", on_file(MenuAction::CopyRelativePath))
                .when(path.is_some()),
            MenuItem::new("Reveal in Sidebar", on_file(MenuAction::Reveal)).when(path.is_some()),
        ];
        self.show_menu((x, y), items);
    }

    /// What a right click on text offers. A page gets Xcode's formatting, and code gets
    /// what its language server can do, greyed out when there's no server to ask.
    fn editor_items(&self) -> Vec<MenuItem> {
        let Some(editor) = self.editors.get(self.active()) else {
            return Vec::new();
        };
        let selected = editor.selected_text().is_some();
        let path = editor.buffer.path();
        let language = path.and_then(Registry::language_name);
        let run = MenuItem::run;
        let edit = [
            run("Cut", Command::Cut).after_rule().when(selected),
            run("Copy", Command::Copy).when(selected),
            run("Paste", Command::Paste),
        ];
        let agent = [
            run("Add Comment", Command::AddComment).after_rule(),
            run("Send Selection to Agent", Command::MentionSelection).when(selected),
        ];
        let mut items = Vec::new();
        if editor.rendered {
            items.push(run("Show Source", Command::MarkdownPreview));
            items.extend(edit);
            let format = |name, format| run(name, Command::Format(format));
            if editor.table_at_caret().is_some() {
                let table = |name, edit| MenuItem::new(name, MenuAction::Table(edit));
                items.extend([
                    table("Insert Row Above", TableEdit::RowAbove).after_rule(),
                    table("Insert Row Below", TableEdit::RowBelow),
                    table("Insert Column Left", TableEdit::ColumnLeft),
                    table("Insert Column Right", TableEdit::ColumnRight),
                    table("Align Left", TableEdit::Align(Align::Left)).after_rule(),
                    table("Align Center", TableEdit::Align(Align::Centre)),
                    table("Align Right", TableEdit::Align(Align::Right)),
                    table("Delete Row", TableEdit::DeleteRow).after_rule(),
                    table("Delete Column", TableEdit::DeleteColumn),
                    table("Delete Table", TableEdit::Delete),
                ]);
                items.extend(agent);
                return items;
            }
            items.extend([
                format("Bold", Format::Bold).after_rule(),
                format("Italic", Format::Italic),
                format("Strikethrough", Format::Strikethrough),
                format("Code", Format::Code),
                format("Plain Text", Format::PlainText).after_rule(),
                format("Heading", Format::Heading),
                format("Code Block", Format::CodeBlock),
                MenuItem::new("Insert Table…", MenuAction::Tool(Tool::Table)),
                MenuItem::new("Insert Formula…", MenuAction::Tool(Tool::Math)),
                MenuItem::new("Insert Image…", MenuAction::Tool(Tool::Image)),
            ]);
            items.extend(agent);
        } else {
            let server = language.is_some_and(|l| self.servers.contains_key(l));
            items.extend([
                run("Go to Definition", Command::GoToDefinition).when(server),
                run("Go to References", Command::GoToReferences).when(server),
                run("Rename Symbol", Command::RenameSymbol).when(server),
                run("Format Document", Command::FormatDocument)
                    .when(server || language.is_some_and(|l| formatter(l).is_some())),
            ]);
            if language == Some("Markdown") {
                items.push(run("Show Page", Command::MarkdownPreview));
            }
            items.extend(edit);
            items.push(run("Toggle Comment", Command::ToggleComment).after_rule());
            items.extend(agent.into_iter().map(|i| MenuItem { rule: false, ..i }));
            items.push(run("Command Palette…", Command::CommandPalette).after_rule());
        }
        items
    }

    fn open_editor_menu(&mut self, x: u16, y: u16) {
        let items = self.editor_items();
        if !items.is_empty() {
            self.show_menu((x, y), items);
        }
    }

    fn copy_text(&mut self, text: String, note: &str) {
        self.clipboard.clone_from(&text);
        self.outgoing_clipboard = Some(text);
        self.say(note);
    }

    /// Closes every tab but `keep`, leaving any with unsaved edits open.
    fn close_tabs_except(&mut self, keep: Option<usize>) {
        let path = keep.and_then(|i| {
            self.editors
                .get(i)?
                .buffer
                .path()?
                .to_str()
                .map(str::to_string)
        });
        let plan = self.plan.as_ref().map(|p| p.path.clone());
        let before = self.editors.len();
        self.editors.retain(|e| {
            let kept = path.is_some() && e.buffer.path().and_then(Path::to_str) == path.as_deref();
            kept || e.buffer.is_dirty() || e.buffer.path() == plan.as_deref()
        });
        self.panes = vec![0];
        self.focus = 0;
        let unsaved = self
            .editors
            .len()
            .saturating_sub(usize::from(path.is_some()));
        if unsaved > 0 {
            self.say(format!("{unsaved} tabs with unsaved edits stayed open"));
        } else if before > self.editors.len() {
            self.say("Closed");
        }
    }

    fn reveal_in_sidebar(&mut self, path: &Path) {
        let root = self.workspace.root().to_path_buf();
        let parents: Vec<PathBuf> = path
            .ancestors()
            .skip(1)
            .take_while(|p| p.starts_with(&root) && *p != root)
            .map(Path::to_path_buf)
            .collect();
        self.explorer.refresh(&self.workspace);
        for parent in parents.iter().rev() {
            let closed = self
                .explorer
                .entries
                .iter()
                .any(|e| e.path == *parent && !e.expanded);
            if closed {
                self.workspace.toggle(parent);
                self.explorer.refresh(&self.workspace);
            }
        }
        self.show_explorer();
        self.show_sidebar = true;
        self.explorer.selected = self.explorer.entries.iter().position(|e| e.path == path);
    }

    fn menu_action(&mut self, action: MenuAction) {
        let name = |path: &Path| {
            path.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default()
        };
        let prompt = |label: &str, input: String, action| {
            Overlay::Prompt(Prompt {
                label: label.to_string(),
                input,
                action,
            })
        };
        match action {
            MenuAction::Run(command) => self.run(command),
            MenuAction::Tool(tool) => self.use_tool(tool, self.main_area().x),
            MenuAction::Table(edit) => {
                if let Some(editor) = self.editor_mut() {
                    editor.edit_table(edit);
                }
            }
            MenuAction::NewFile(dir) => {
                self.overlay = prompt("New file", String::new(), PromptAction::NewFile(dir));
            }
            MenuAction::NewFolder(dir) => {
                self.overlay = prompt("New folder", String::new(), PromptAction::NewFolder(dir));
            }
            MenuAction::Rename(target) => {
                let input = name(&target);
                self.overlay = prompt("Rename", input, PromptAction::Rename(target));
            }
            MenuAction::Delete(target) => {
                self.overlay = Overlay::Confirm(Confirm {
                    title: format!("Delete {}?", name(&target)),
                    detail: "This can't be undone.".to_string(),
                    buttons: vec![
                        ("Delete", Choice::Delete(target)),
                        ("Cancel", Choice::Cancel),
                    ],
                    selected: 1,
                });
            }
            MenuAction::CopyPath(path) => {
                self.copy_text(path.display().to_string(), "Copied the path");
            }
            MenuAction::CopyRelativePath(path) => {
                self.copy_text(self.relative(&path), "Copied the relative path");
            }
            MenuAction::Reveal(path) => self.reveal_in_sidebar(&path),
            MenuAction::CloseTab(index) => {
                self.set_active(index);
                self.run(Command::CloseTab);
            }
            MenuAction::CloseOtherTabs(index) => self.close_tabs_except(Some(index)),
            MenuAction::CloseAllTabs => self.close_tabs_except(None),
        }
    }

    fn settings_values(&self) -> (String, String, bool, bool) {
        let chosen = |value: &Option<String>, fallback: &str| {
            value.clone().unwrap_or_else(|| fallback.to_string())
        };
        (
            chosen(&self.settings.theme, "auto"),
            chosen(&self.settings.keymap, "simple"),
            self.settings.smooth_scroll != Some(false),
            self.settings.minimap != Some(false),
        )
    }

    fn settings_key(&mut self, key: KeyEvent) -> bool {
        let area = self.sheet_area();
        let Some(page) = &mut self.settings_page else {
            return false;
        };
        match key.key {
            Key::Esc => self.settings_page = None,
            Key::Down | Key::Right | Key::Tab => page.step(area, 1),
            Key::Up | Key::Left | Key::BackTab => page.step(area, -1),
            Key::Enter | Key::Char(' ') => {
                if let Some(control) = page.focused(area) {
                    self.use_control(control);
                }
            }
            // Ctrl keys and the like still reach the app, so Ctrl+P works from here.
            Key::Char(_) if !key.mods.ctrl && !key.mods.alt => {}
            _ => return false,
        }
        true
    }

    fn settings_mouse(&mut self, mouse: MouseEvent) {
        let area = self.sheet_area();
        let Some(page) = &self.settings_page else {
            return;
        };
        if matches!(mouse.kind, MouseKind::ScrollUp | MouseKind::ScrollDown) {
            let delta = if mouse.kind == MouseKind::ScrollUp {
                -1
            } else {
                1
            };
            if let Some(page) = &mut self.settings_page {
                page.scroll_by(delta);
            }
            return;
        }
        if mouse.kind != MouseKind::Left {
            return;
        }
        if self.tabs_area().contains(mouse.x, mouse.y) {
            self.settings_page = None;
        } else if let Some(control) = page.control_at(area, mouse.x, mouse.y) {
            self.use_control(control);
        }
    }

    /// Picks up a change to the theme, keeping the frame on the terminal's own colour.
    fn apply_theme(&mut self) {
        self.theme = Theme::named(self.settings.theme.as_deref());
        let (light, background) = self.terminal_look;
        self.follow_terminal_theme(light);
        self.match_terminal_background(background);
    }

    fn use_control(&mut self, control: Control) {
        let mut note = String::new();
        let var = |name: &str| std::env::var(name).ok();
        match control {
            Control::Section(section) => {
                if let Some(page) = &mut self.settings_page {
                    page.section = section;
                    page.focus = None;
                }
            }
            Control::Theme(name) => {
                self.settings.theme = Some(name.to_string());
                self.apply_theme();
                note = store("theme", name.into());
            }
            Control::SmoothScroll => {
                let on = self.settings.smooth_scroll == Some(false);
                self.settings.smooth_scroll = Some(on);
                self.smooth_change = Some(on);
                note = store("smooth_scroll", on.into());
            }
            Control::Minimap => {
                let on = self.settings.minimap == Some(false);
                self.settings.minimap = Some(on);
                for editor in &mut self.editors {
                    editor.minimap_on = on;
                }
                note = store("minimap", on.into());
            }
            Control::Keymap(name) => {
                self.vim = (name == "vim").then(Vim::default);
                self.neovim = (name == "neovim")
                    .then(|| Neovim::start(&[]).ok())
                    .flatten();
                if name == "neovim" && self.neovim.is_none() {
                    note = "Neovim wasn't found, so the keymap stayed as it was".to_string();
                } else {
                    self.settings.keymap = Some(name.to_string());
                    note = store("keymap", name.into());
                }
            }
            Control::ClaudeCode => {
                let installed = self
                    .settings_page
                    .as_ref()
                    .is_some_and(|p| p.found.claude_installed);
                let done = if installed {
                    crate::integrations::remove_claude(var)
                        .map(|()| "Removed. Restart Claude Code.")
                } else {
                    crate::integrations::install_claude(var)
                        .map(|()| "Installed. Restart Claude Code to load it.")
                };
                note = done.map_or_else(|e| format!("Couldn't: {e}"), str::to_string);
            }
            Control::CopyInstall(command) => {
                self.copy_text(command.to_string(), "");
                note = format!("Copied: {command}");
            }
            Control::Codex | Control::Gemini | Control::InstallServer(_) => {
                self.start_install(control);
                return;
            }
            Control::Ghostty => {
                note = crate::integrations::free_cmd_d(var).map_or_else(
                    |e| format!("Couldn't: {e}"),
                    |()| "Done. Reload Ghostty's config with Cmd+Shift+,".to_string(),
                );
            }
        }
        if let Some(page) = &mut self.settings_page {
            if matches!(control, Control::ClaudeCode | Control::Ghostty) {
                page.found = Found::look();
            }
            page.note = note;
        }
    }

    /// Starts an install off the main thread: package managers and agents' own commands
    /// can take minutes, and the app stays usable meanwhile.
    fn start_install(&mut self, control: Control) {
        let Some(page) = &mut self.settings_page else {
            return;
        };
        if page.installing.is_some() {
            page.note = "Another install is still running".to_string();
            return;
        }
        let found = (page.found.codex_installed, page.found.gemini_installed);
        let (sender, receiver) = std::sync::mpsc::channel();
        page.note = "Working…".to_string();
        page.installing = Some(receiver);
        self.say("Working…");
        std::thread::spawn(move || {
            let var = |name: &str| std::env::var(name).ok();
            let said = |done: io::Result<()>, note: &str| {
                done.map_or_else(|e| format!("Couldn't: {e}"), |()| note.to_string())
            };
            let note = match control {
                Control::Codex if found.0 => {
                    said(crate::integrations::remove_codex(), "Removed from Codex.")
                }
                Control::Codex => said(
                    crate::integrations::install_codex(var),
                    "Installed. Run /hooks in Codex once to trust its hooks.",
                ),
                Control::Gemini if found.1 => said(
                    crate::integrations::remove_gemini(),
                    "Removed from Gemini CLI.",
                ),
                Control::Gemini => said(
                    crate::integrations::install_gemini(var),
                    "Installed. Restart Gemini CLI to load it.",
                ),
                Control::InstallServer(index) => install_server(index),
                _ => String::new(),
            };
            let _ = sender.send(note);
        });
    }

    /// Takes in a finished install, and connects any file it can now help.
    fn poll_install(&mut self) -> bool {
        let Some(page) = &mut self.settings_page else {
            return false;
        };
        let done = page.installing.as_ref().and_then(|r| r.try_recv().ok());
        let Some(note) = done else {
            return false;
        };
        page.installing = None;
        page.note.clone_from(&note);
        page.found = Found::look();
        self.say(note);
        self.attach_open_files();
        true
    }

    /// The new smooth scrolling setting, once, for the terminal loop to act on.
    pub fn take_smooth_change(&mut self) -> Option<bool> {
        self.smooth_change.take()
    }

    fn draw_settings_tab(&self, grid: &mut Grid) {
        let main = self.main_area();
        let theme = &self.theme;
        grid.fill(Rect::new(main.x, main.y, main.w, 1), theme.tab);
        let area = self.tabs_area();
        let width = self.measure.width("Settings", Font::default()).ceil() as u16 + TAB_PADDING;
        let tab = Rect::new(area.x, area.y, width, 1);
        let style = theme.tab_active;
        grid.fill(tab, style);
        let close = tab.right().saturating_sub(2);
        grid.label_at(
            area.x as f32 + 1.5,
            area.x + 2,
            area.y,
            "Settings",
            style,
            close,
            100,
        );
        grid.glyph(close as f32, area.y, Icon::Close, theme.tab.fg, "×", style);
        let bounds = Bounds::inset(tab, 0.0, 0.0);
        grid.corners(bounds, TAB_RADIUS, theme.tab.bg, TOP_LEFT | TOP_RIGHT);
    }

    fn click_explorer(&mut self, y: u16) {
        let Some(area) = self.sidebar_area() else {
            return;
        };
        let Some(index) = self.explorer.entry_at(area, y) else {
            return;
        };
        self.explorer.selected = Some(index);
        let entry = self.explorer.entries[index].clone();
        if entry.is_dir {
            self.workspace.toggle(&entry.path);
            self.explorer.refresh(&self.workspace);
        } else if let Err(e) = self.open(&entry.path) {
            self.say(format!("Could not open {}: {e}", entry.name));
        } else {
            self.drawer = false;
        }
    }

    fn click_activity(&mut self, y: u16) {
        let area = self.activity_area();
        let index = (0..ACTIVITY.len()).find(|i| {
            let top = Self::rail_row(*i, area);
            y >= top && y < top + ACTIVITY_STEP
        });
        if let Some((_, command)) = index.and_then(|i| ACTIVITY.get(i)) {
            if *command != Command::OpenSettings {
                self.settings_page = None;
            }
            let on_explorer = self.sidebar_view == SidebarView::Explorer;
            match command {
                Command::ToggleSidebar if self.review.is_some() || !on_explorer => {
                    self.show_explorer();
                    self.show_sidebar = true;
                }
                // A second click closes the panel, as it does in VS Code.
                Command::FindInProject if !on_explorer && self.review.is_none() => {
                    self.show_explorer();
                    self.show_sidebar = false;
                }
                command => self.run(*command),
            }
        }
    }

    /// Columns a tab takes, which hug its name: proportional text is narrower than its cells.
    fn tab_width(&self, editor: &Editor) -> u16 {
        let name = self.measure.width(&editor.title(), Font::default());
        name.ceil() as u16 + TAB_PADDING
    }

    /// Tabs that fit, as (index, x, width), starting late enough to show the active one.
    /// Drawing and clicking share this, so a click always lands on what's shown.
    fn visible_tabs(&self) -> Vec<(usize, u16, u16)> {
        let area = self.tabs_area();
        let widths: Vec<u16> = self.editors.iter().map(|e| self.tab_width(e) + 1).collect();
        let active = self.active().min(widths.len().saturating_sub(1));
        let mut first = 0;
        while first < active && widths[first..=active].iter().sum::<u16>() > area.w {
            first += 1;
        }
        let mut tabs = Vec::new();
        let mut x = area.x;
        for (i, width) in widths.iter().enumerate().skip(first) {
            if x >= area.right() {
                break;
            }
            tabs.push((i, x, *width - 1));
            x += width;
        }
        tabs
    }

    fn click_tab(&mut self, x: u16) {
        for (i, at, width) in self.visible_tabs() {
            if x >= at && x < at + width {
                self.set_active(i);
                // A little wider than the glyph, so it isn't a pixel hunt.
                if x + 3 >= at + width {
                    self.run(Command::CloseTab);
                }
                return;
            }
        }
    }

    pub fn draw(&self, grid: &mut Grid) {
        grid.cursor = None;
        let theme = &self.theme;
        grid.fill(grid.area(), theme.tab);
        let sheet = self.sheet_area();
        grid.fill(sheet, theme.text);
        self.draw_activity(grid);
        let drawer_over_main = self.compact() && self.drawer;
        if drawer_over_main {
            self.draw_tabs(grid);
        }
        if let Some(area) = self.sidebar_area() {
            let name = self
                .workspace
                .root()
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            match self.sidebar_view {
                SidebarView::Explorer => {
                    let header = (name.as_str(), self.branch.as_deref());
                    let measure = &*self.measure;
                    self.explorer
                        .draw(grid, area, header, measure, self.hover, theme);
                }
                SidebarView::Search => {
                    self.search
                        .draw(grid, area, self.hover, &*self.measure, theme);
                }
            }
        }
        self.draw_dock(grid);
        if let Some(page) = &self.settings_page {
            self.draw_settings_tab(grid);
            let (chosen, keymap, smooth_scroll, minimap) = self.settings_values();
            let values = Values {
                theme: &chosen,
                keymap: &keymap,
                smooth_scroll,
                minimap,
            };
            let area = self.sheet_area();
            page.draw(grid, area, &values, (self.hover, &*self.measure), theme);
            self.draw_status(grid);
            self.draw_overlay(grid);
            return;
        }
        if let Some(review) = &self.review {
            let comments = self.comments.lock().map(|c| c.clone()).unwrap_or_default();
            review.draw(grid, self.main_area(), self.hover, theme, &comments);
            self.draw_status(grid);
            self.draw_overlay(grid);
            return;
        }
        if drawer_over_main {
            self.draw_status(grid);
            self.draw_overlay(grid);
            return;
        }
        self.draw_tabs(grid);
        if self.editors.is_empty() {
            self.draw_welcome(grid);
        }
        self.draw_breadcrumbs(grid);
        for (pane, index) in self.panes.iter().enumerate() {
            if let Some(editor) = self.editors.get(*index) {
                let area = self.pane_area(pane);
                if area.w == 0 {
                    continue;
                }
                let file = editor.buffer.path().map(|p| self.relative(p));
                let marks = Marks {
                    comments: file.map_or_else(Vec::new, |f| self.comment_lines(&f)),
                    errors: editor
                        .buffer
                        .path()
                        .and_then(|p| self.diagnostics.get(p))
                        .map_or_else(Vec::new, |d| d.iter().map(|d| d.line + 1).collect()),
                };
                editor.draw(
                    grid,
                    area,
                    theme,
                    pane == self.focus,
                    &self.registry,
                    &marks,
                );
                if pane + 1 < self.panes.len() {
                    let line = Rect::new(area.right(), area.y, 1, area.h);
                    grid.fill(line, theme.tab);
                }
            }
        }
        if self.plan_in_view() {
            self.draw_plan_bar(grid);
        }
        self.draw_status(grid);
        self.draw_suggest(grid);
        self.draw_offer(grid);
        self.draw_overlay(grid);
        self.draw_tooltip(grid);
    }

    /// Names what's under the pointer, for the buttons that are only an icon.
    fn draw_tooltip(&self, grid: &mut Grid) {
        let Some((x, y)) = self.hover else {
            return;
        };
        let rail = self.activity_area();
        let font = Font {
            size: TOOLTIP_SIZE,
            ..Font::default()
        };
        let wide = |name: &str| self.measure.width(name, font) + 3.0;
        // Where each one goes: its name, left edge and top, in cells and rows.
        let spot = match &self.overlay {
            Overlay::Find(_) => {
                let area = self.find_bar();
                let first = area.right() - FIND_BUTTON * FIND_BUTTONS.len() as u16 - 1;
                let tool = x.checked_sub(first).map(|at| at / FIND_BUTTON);
                let name = tool
                    .filter(|_| y == area.y + 1)
                    .and_then(|t| FIND_NAMES.get(t as usize));
                name.map(|name| {
                    let left = area.right() as f32 - wide(name);
                    (*name, left, area.y as f32 + 2.5)
                })
            }
            Overlay::None if rail.w > 0 && rail.contains(x, y) => {
                let icon = (0..ACTIVITY.len()).find(|i| {
                    let top = Self::rail_row(*i, rail);
                    y >= top && y < top + u16::from(ACTIVITY_ICON.1)
                });
                icon.map(|i| {
                    let top = Self::rail_row(i, rail) as f32;
                    // The middle of a two-row icon.
                    (
                        RAIL_NAMES[i],
                        rail.right() as f32 + 1.0,
                        top + 1.0 - TOOLTIP_HEIGHT / 2.0,
                    )
                })
            }
            Overlay::None => self
                .toolbar_row()
                .filter(|_| y == 1)
                .and_then(|row| toolbar::hit(row, x))
                .map(|button| {
                    let name = button.name;
                    let middle = button.x as f32 + button.width as f32 / 2.0;
                    let left =
                        (middle - wide(name) / 2.0).min(self.size.0 as f32 - wide(name) - 0.5);
                    (name, left.max(0.5), 2.2)
                }),
            _ => None,
        };
        let Some((name, left, top)) = spot else {
            return;
        };
        let theme = &self.theme;
        let bounds = Bounds::cells(left, top, wide(name), TOOLTIP_HEIGHT);
        grid.popup(bounds, 24, theme.popup, theme.popup_border);
        let text_top = top + (TOOLTIP_HEIGHT - 1.0) / 2.0;
        if self.measure.in_cells() {
            let style = Style {
                fg: theme.text.fg,
                bg: theme.popup,
                ..theme.bar
            };
            grid.put_str(
                left as u16 + 1,
                text_top.round() as u16,
                name,
                style,
                self.size.0,
            );
        } else {
            let end = left + wide(name);
            grid.text_run(
                (left + 1.5, text_top),
                end,
                name,
                (theme.text.fg, false),
                font,
            );
        }
    }

    fn draw_overlay(&self, grid: &mut Grid) {
        let theme = &self.theme;
        // The text's caret would stand on top of anything that floats over it.
        if matches!(
            self.overlay,
            Overlay::Menu(_) | Overlay::Confirm(_) | Overlay::TablePicker(_) | Overlay::Math(_)
        ) {
            grid.caret = None;
            grid.cursor = None;
        }
        match &self.overlay {
            Overlay::Palette(palette) => palette.draw(grid, self.hover, &*self.measure, theme),
            Overlay::Menu(menu) => menu.draw(grid, self.hover, theme),
            Overlay::Prompt(prompt) => self.draw_prompt(grid, prompt),
            Overlay::Find(find) => self.draw_find(grid, find),
            Overlay::Confirm(confirm) => self.draw_confirm(grid, confirm),
            Overlay::TablePicker(picker) => picker.draw(grid, self.hover, theme),
            Overlay::Math(panel) => panel.draw(grid, self.hover, theme),
            Overlay::None => {}
        }
    }

    /// Asks in a floating box at the top, as VS Code's quick input does.
    fn draw_prompt(&self, grid: &mut Grid, prompt: &Prompt) {
        let screen = Rect::new(0, 0, self.size.0, self.size.1);
        let top = Palette::area(screen);
        let area = Rect::new(top.x, top.y, top.w, 2);
        raised(grid, area, &self.theme);
        let base = Style {
            fg: self.theme.text.fg,
            bg: self.theme.popup,
            ..self.theme.bar
        };
        let dim = Style {
            fg: self.theme.gutter.fg,
            ..base
        };
        grid.label_at(
            area.x as f32 + 1.0,
            area.x + 1,
            area.y,
            &prompt.label,
            dim,
            area.right(),
            90,
        );
        let line = Rect::new(area.x + 1, area.y + 1, area.w.saturating_sub(2), 1);
        let field = Input {
            text: &prompt.input,
            hint: "",
            focused: true,
            behind: self.theme.popup,
        };
        input(grid, line, &field, &*self.measure, &self.theme);
    }

    /// Where an icon goes. The last one is Settings, which sits at the bottom given room.
    fn rail_row(index: usize, area: Rect) -> u16 {
        let stacked = 1 + index as u16 * ACTIVITY_STEP;
        if index + 1 == ACTIVITY.len() {
            stacked.max(area.bottom().saturating_sub(3))
        } else {
            stacked
        }
    }

    /// How many files differ from HEAD, for the badge on the rail.
    fn changed_count(&self) -> usize {
        let files = self.explorer.status.values();
        files.filter(|status| **status != DIRECTORY_MARK).count()
    }

    fn draw_activity(&self, grid: &mut Grid) {
        let area = self.activity_area();
        if area.w == 0 {
            return;
        }
        grid.fill(area, self.theme.activity);
        for (i, (icon, command)) in ACTIVITY.iter().enumerate() {
            let y = Self::rail_row(i, area);
            if y + 1 >= area.bottom() {
                break;
            }
            let on = match command {
                Command::ToggleSidebar => {
                    self.show_sidebar
                        && self.review.is_none()
                        && self.sidebar_view == SidebarView::Explorer
                }
                Command::FindInProject => {
                    self.show_sidebar
                        && self.review.is_none()
                        && self.sidebar_view == SidebarView::Search
                }
                Command::ShowAgents => self.dock.open,
                Command::OpenDiff => self.review.is_some(),
                Command::OpenSettings => self.settings_page.is_some(),
                _ => false,
            };
            let row = Rect::new(area.x, y, area.w, ACTIVITY_ICON.1 as u16);
            let hovered = self.hover.is_some_and(|h| row.contains(h.0, h.1));
            let style = Style {
                fg: if on {
                    self.theme.activity_active
                } else if hovered {
                    self.theme.text.fg
                } else {
                    self.theme.activity.fg
                },
                ..self.theme.activity
            };
            let wash = on.then_some(self.theme.tint);
            let wash = wash.or(hovered.then_some(self.theme.row_hover));
            let style = wash.map_or(style, |bg| Style { bg, ..style });
            if on {
                // Only the cell tier needs the bar, since pixels have the pill.
                let bar = Style {
                    fg: self.theme.accent,
                    painted: true,
                    ..self.theme.activity
                };
                for row in 0..u16::from(ACTIVITY_ICON.1) {
                    grid.put_str(area.x, y + row, "▌", bar, area.right());
                }
            }
            if wash.is_some() {
                // Icons are cells, so the pill is too: the rail's full width, with paint
                // bringing it in half a column at each side.
                let across = Rect::new(area.x, y, area.w, row.h);
                grid.fill(across, style);
                let ground = self.theme.activity.bg;
                let (h, top) = (row.h as f32, y as f32);
                let inset = 0.5;
                let right = area.right() as f32 - inset;
                let left = area.x as f32 + inset;
                grid.panel(
                    Bounds::cells(area.x as f32, top, inset, h),
                    0,
                    Some(ground),
                    None,
                );
                grid.panel(Bounds::cells(right, top, inset, h), 0, Some(ground), None);
                let bounds = Bounds::cells(left, top, right - left, h);
                grid.corners(bounds, PILL_RADIUS, ground, ALL_CORNERS);
            }
            grid.put_icon(area.x + 1, y, *icon, ACTIVITY_ICON, style);
            let changed = self.changed_count();
            if *command == Command::OpenDiff && changed > 0 {
                let color = self.theme.button_text;
                let badge = Bounds::cells(area.x as f32 + 3.0, y as f32 - 0.16, 1.3, 0.58);
                grid.panel(badge, 30, Some(self.theme.accent), None);
                let font = Font {
                    bold: true,
                    size: BADGE_SIZE,
                    ..Font::default()
                };
                let (x, top) = (area.x as f32 + 3.32, y as f32 - 0.36);
                grid.text_run(
                    (x, top),
                    area.right() as f32,
                    &changed.to_string(),
                    (color, false),
                    font,
                );
            }
            if *command == Command::ShowAgents && self.agents.needing_you() > 0 {
                let dot = Bounds::cells(area.x as f32 + 3.0, y as f32 + 0.1, 0.8, 0.4);
                grid.panel(dot, 22, Some(self.theme.attention), None);
            }
        }
    }

    /// The raised surface the work sits on, under the tabs.
    fn sheet_area(&self) -> Rect {
        let main = self.main_area();
        Rect::new(main.x, main.y + 1, main.w, main.h.saturating_sub(1))
    }

    fn draw_tabs(&self, grid: &mut Grid) {
        let main = self.main_area();
        let theme = &self.theme;
        grid.fill(Rect::new(main.x, main.y, main.w, 1), theme.tab);
        if self.compact() {
            grid.glyph(
                main.x as f32 + 0.5,
                main.y,
                Icon::Files,
                theme.tab.fg,
                "☰",
                theme.tab,
            );
        }
        if self.plan_in_view() {
            self.draw_plan_banner(grid);
            return;
        }
        let area = self.tabs_area();
        for (i, x, width) in self.visible_tabs() {
            let active = i == self.active();
            let hovered = self
                .hover
                .is_some_and(|h| h.1 == area.y && h.0 >= x && h.0 < x + width);
            let style = if active { theme.tab_active } else { theme.tab };
            let tab = Rect::new(x, area.y, width.min(area.right().saturating_sub(x)), 1);
            grid.fill(tab, style);
            let editor = &self.editors[i];
            let name = editor.title();
            let close_x = x + width.saturating_sub(2);
            grid.label_at(x as f32 + 1.5, x + 2, area.y, &name, style, close_x, 100);
            if close_x < area.right() {
                let dirty = editor.buffer.is_dirty();
                let shown = if dirty { "●" } else { "×" };
                let mark = Style {
                    painted: true,
                    ..style
                };
                if dirty && !hovered {
                    grid.put_str(close_x, area.y, shown, mark, area.right());
                    let dot =
                        Bounds::cells(close_x as f32 + 0.45, area.y as f32 + 0.36, 0.56, 0.28);
                    grid.panel(dot, 14, Some(theme.sidebar.fg), None);
                } else if active || hovered {
                    let on_close = self
                        .hover
                        .is_some_and(|h| h.1 == area.y && h.0 >= close_x && h.0 < x + width);
                    if on_close {
                        let chip = Bounds::cells(close_x as f32 - 0.05, 0.12, 1.75, 0.76);
                        grid.panel(chip, 16, Some(theme.row_hover), None);
                    }
                    let color = if hovered { theme.text.fg } else { theme.tab.fg };
                    grid.glyph(close_x as f32, area.y, Icon::Close, color, shown, style);
                } else {
                    let quiet = Style {
                        painted: true,
                        ..style
                    };
                    grid.put_str(close_x, area.y, shown, quiet, area.right());
                }
            }
            if active {
                let bounds = Bounds::inset(tab, 0.0, 0.0);
                grid.corners(bounds, TAB_RADIUS, theme.tab.bg, TOP_LEFT | TOP_RIGHT);
            }
        }
    }

    /// Whether the plan waiting for review is the tab in view.
    fn plan_in_view(&self) -> bool {
        let open = self
            .editors
            .get(self.active())
            .and_then(|e| e.buffer.path());
        self.plan
            .as_ref()
            .is_some_and(|p| open == Some(p.path.as_path()))
    }

    /// Where the plan's decision floats, low in the middle of the sheet.
    fn plan_bar(&self) -> Rect {
        let sheet = self.sheet_area();
        let width = PLAN_BAR_WIDTH.min(sheet.w);
        let x = sheet.x + (sheet.w - width) / 2;
        Rect::new(x, sheet.bottom().saturating_sub(4), width, 3)
    }

    /// Stands in for the tabs while a plan waits, because this is the one thing to do.
    fn draw_plan_banner(&self, grid: &mut Grid) {
        let theme = &self.theme;
        let main = self.main_area();
        let dot = Bounds::cells(main.x as f32 + 1.0, main.y as f32 + 0.34, 0.45, 0.32);
        grid.panel(dot, 16, Some(theme.attention), None);
        let strong = Style {
            bold: true,
            fg: theme.heading,
            ..theme.tab
        };
        let dim = Style {
            fg: theme.status.fg,
            ..theme.tab
        };
        let end = grid.label(
            main.x + 2,
            main.y,
            "Claude Code is waiting on this plan",
            strong,
            main.right(),
        );
        let hint = "Edit it directly, or comment on any line";
        grid.label(end + 2, main.y, hint, dim, main.right());
    }

    fn draw_plan_bar(&self, grid: &mut Grid) {
        let bar = self.plan_bar();
        let theme = &self.theme;
        popup(grid, bar, theme);
        let base = Style {
            fg: theme.status.fg,
            bg: theme.popup,
            ..theme.bar
        };
        let primary = Style {
            fg: theme.button_text,
            ..base
        };
        let secondary = Style {
            fg: theme.text.fg,
            ..base
        };
        let y = bar.y + 1;
        let file = self.plan.as_ref().map(|p| self.relative(&p.path));
        let noted = self.comments.lock().map_or(0, |c| {
            c.iter().filter(|c| Some(&c.file) == file.as_ref()).count()
        });
        let words = match noted {
            0 => "No comments".to_string(),
            1 => "1 comment".to_string(),
            count => format!("{count} comments"),
        };
        grid.label(bar.x + 2, y, &words, base, bar.right());
        let approve = bar.right().saturating_sub(APPROVE.len() as u16 + 2);
        let back = approve.saturating_sub(SEND_BACK.len() as u16 + 1);
        let lit = |x: u16, cells: &str, fill: Rgb| {
            let spot = Rect::new(x, y, cells.len() as u16, 1);
            if self.hover.is_some_and(|h| spot.contains(h.0, h.1)) {
                theme.quiet_button(fill)
            } else {
                fill
            }
        };
        let back_fill = lit(back, SEND_BACK, theme.quiet_button(theme.popup));
        let approve_fill = lit(approve, APPROVE, theme.button);
        grid.button(back, y, SEND_BACK, "Send back", back_fill, secondary);
        grid.button(approve, y, APPROVE, "Approve", approve_fill, primary);
    }

    fn click_plan_bar(&mut self, x: u16) {
        let bar = self.plan_bar();
        let approve = bar.right().saturating_sub(APPROVE.len() as u16 + 2);
        let back = approve.saturating_sub(SEND_BACK.len() as u16 + 1);
        if x >= approve && x < approve + APPROVE.len() as u16 {
            self.run(Command::ApprovePlan);
        } else if x >= back && x < back + SEND_BACK.len() as u16 {
            self.run(Command::SendBackPlan);
        }
    }

    fn draw_breadcrumbs(&self, grid: &mut Grid) {
        if self.chrome_rows() < 2 {
            return;
        }
        let Some(path) = self
            .editors
            .get(self.active())
            .and_then(|e| e.buffer.path())
        else {
            return;
        };
        let main = self.main_area();
        // A page reads as a document, so it carries no path.
        let page = self.editors.get(self.active()).is_some_and(|e| e.rendered);
        let text = if page {
            String::new()
        } else {
            self.relative(path).replace('/', " › ")
        };
        let style = self.theme.breadcrumb;
        grid.fill(Rect::new(main.x, 1, main.w, 1), style);
        if let Some(row) = self.toolbar_row() {
            let (paragraph, active) = self.editors[self.active()].caret_formats();
            toolbar::draw(grid, row, (paragraph, &active), self.hover, &self.theme);
            return;
        }
        grid.label_at(
            main.x as f32 + 1.5,
            main.x + 2,
            1,
            &text,
            style,
            main.right(),
            90,
        );
    }

    /// The row the document toolbar sits in, if the page in view has one.
    fn toolbar_row(&self) -> Option<Rect> {
        let main = self.main_area();
        let page = self.editors.get(self.active()).is_some_and(|e| e.rendered);
        let room = main.w >= toolbar::MIN_WIDTH;
        (page && room && self.chrome_rows() == 2 && !self.plan_in_view())
            .then(|| Rect::new(main.x, 1, main.w, 1))
    }

    fn click_toolbar(&mut self, x: u16) -> bool {
        let button = self.toolbar_row().and_then(|row| toolbar::hit(row, x));
        let Some(button) = button else {
            return false;
        };
        self.use_tool(button.tool, button.x);
        true
    }

    fn use_tool(&mut self, tool: Tool, x: u16) {
        let below = (x, 2);
        match tool {
            Tool::Format(format) => self.run(Command::Format(format)),
            Tool::Styles => {
                let level =
                    |name, level| MenuItem::run(name, Command::Format(Format::Level(level)));
                let items = vec![
                    level("Text", 0),
                    level("Heading 1", 1),
                    level("Heading 2", 2),
                    level("Heading 3", 3),
                    MenuItem::run("Code Block", Command::Format(Format::CodeBlock)).after_rule(),
                ];
                self.show_menu(below, items);
            }
            Tool::Table => self.overlay = Overlay::TablePicker(TablePicker::new((x, 3))),
            Tool::Math => {
                self.overlay = Overlay::Math(MathPanel::new((x, 3), &*self.measure));
            }
            Tool::Image => {
                self.overlay = Overlay::Prompt(Prompt {
                    label: "Image file".to_string(),
                    input: String::new(),
                    action: PromptAction::Image,
                });
            }
            Tool::Comment => self.run(Command::AddComment),
            Tool::Export => {
                let items = vec![MenuItem::run("Open in Browser", Command::OpenInBrowser)];
                self.show_menu(below, items);
            }
            Tool::More => {
                let row = self.toolbar_row().unwrap_or_default();
                let items = toolbar::overflow(row)
                    .into_iter()
                    .map(|(name, tool)| MenuItem::new(name, MenuAction::Tool(tool)))
                    .collect();
                self.show_menu(below, items);
            }
        }
    }

    fn insert_math(&mut self, index: usize) {
        let area = self.focused_area();
        if let (Some(symbol), Some(editor)) = (SYMBOLS.get(index), self.editor_mut()) {
            editor.prepare_edit(area);
            editor.insert_math(symbol);
        }
    }

    fn insert_table(&mut self, (rows, columns): (usize, usize)) {
        let area = self.focused_area();
        if let Some(editor) = self.editor_mut() {
            editor.prepare_edit(area);
            editor.insert_table(rows, columns);
        }
    }

    fn draw_welcome(&self, grid: &mut Grid) {
        const ROWS: [(&str, &str); 4] = [
            ("Ctrl+P", "Open a file"),
            ("Ctrl+Shift+P", "Commands"),
            ("Ctrl+G", "Review changes"),
            ("Ctrl+Shift+F", "Search the project"),
        ];
        /// Columns from a shortcut to what it does.
        const KEYS_WIDTH: u16 = 10;
        const WIDTH: u16 = 24;
        let main = self.main_area();
        if main.h < 12 {
            return;
        }
        let theme = &self.theme;
        let y = main.y + main.h / 3;
        let x = main.x + main.w.saturating_sub(WIDTH) / 2;
        let title = Style {
            bold: true,
            ..theme.text
        };
        grid.label_at(x as f32, x, y, "Cellix", title, main.right(), 180);
        let keys = Style {
            fg: theme.text.fg,
            ..theme.text
        };
        let description = Style {
            fg: theme.gutter.fg,
            ..theme.text
        };
        for (i, (key, what)) in ROWS.iter().enumerate() {
            let line = y + 2 + i as u16;
            let second = x + KEYS_WIDTH;
            grid.label_at(x as f32, x, line, key, keys, second, 95);
            grid.label_at(
                second as f32,
                second,
                line,
                what,
                description,
                main.right(),
                95,
            );
        }
    }

    fn draw_status(&self, grid: &mut Grid) {
        let y = self.size.1.saturating_sub(1);
        let width = self.size.0;
        let bar = self.theme.status;
        grid.fill(Rect::new(0, y, width, 1), bar);
        let message = self.message.borrow();
        let at_cursor = self.cursor_line().and_then(|(path, line)| {
            let items = self.diagnostics.get(&path)?;
            items
                .iter()
                .find(|d| d.line + 1 == line)
                .map(|d| d.message.clone())
        });
        let reviewing = self
            .review
            .as_ref()
            .filter(|r| !r.is_proposal())
            .map_or_else(String::new, |r| {
                let count = r.changes.len();
                let files = if count == 1 { "file" } else { "files" };
                format!("Reviewing {count} {files}")
            });
        let proposed = self
            .review
            .as_ref()
            .filter(|r| r.is_proposal())
            .map_or_else(String::new, |r| format!("{} changes", r.diff.hunks.len()));
        let waiting = self.agents.needing_you();
        let attention = format!("An agent needs you ({waiting})");
        let typed = format!(":{}", self.neovim_line);
        let left = if !self.neovim_line.is_empty() {
            typed.as_str()
        } else if !message.is_empty() {
            message.as_str()
        } else if waiting > 0 {
            attention.as_str()
        } else if let Some(problem) = &at_cursor {
            problem.as_str()
        } else if !proposed.is_empty() {
            proposed.as_str()
        } else if self.editors.is_empty() && self.review.is_none() {
            // Hints only help a window with nothing in it yet.
            if self.compact() {
                "Ctrl+P open"
            } else {
                "Ctrl+P open  Ctrl+G review  Ctrl+Q quit"
            }
        } else {
            reviewing.as_str()
        };
        // The cursor always shows, and the rest stays only while it leaves `left` room.
        let mut right = Vec::new();
        if let Some(editor) = self.editors.get(self.active()) {
            let head = editor.selections.primary().head;
            let rope = editor.buffer.rope();
            let line = rope.char_to_line(head);
            let column = head - rope.line_to_char(line) + 1;
            let language = editor
                .buffer
                .path()
                .and_then(Registry::language_name)
                .unwrap_or("Plain Text");
            // A final newline ends the last line rather than starting another.
            let ends_line = rope.len_chars() > 0 && rope.char(rope.len_chars() - 1) == '\n';
            let trailing = usize::from(ends_line);
            let mode = match self.vim.as_ref().map(|v| v.mode) {
                Some(VimMode::Normal) => "NORMAL",
                Some(VimMode::Insert) => "INSERT",
                Some(VimMode::Visual) => "VISUAL",
                None => self.neovim_mode.as_str(),
            };
            right = vec![
                format!("{mode}   Ln {}, Col {column}", line + 1)
                    .trim_start()
                    .to_string(),
                format!("{} lines", rope.len_lines().saturating_sub(trailing)),
                language.to_string(),
                "UTF-8".to_string(),
                "Spaces: 4".to_string(),
            ];
        }
        if self.in_pane && self.review.is_some() {
            right = vec!["Esc returns to Claude".to_string()];
        }
        if self.settings_page.is_some() {
            right.clear();
        }
        // A message outranks the details, and a hint gives way to them.
        let kept = if message.is_empty() { 24 } else { 64 };
        let start = self.status_branch().map_or(1, |(_, end)| end + 1);
        let problems: usize = self.diagnostics.values().map(Vec::len).sum();
        let problems = (problems > 0 && !self.compact()).then(|| format!("⊗ {problems}  "));
        let flagged = problems.as_ref().map_or(0, |p| p.chars().count() as u16);
        let used = left.chars().count().min(kept) as u16 + start + flagged + 3;
        let room = width.saturating_sub(used) as usize;
        let mut shown = right.clone();
        while shown.len() > 1 && shown.join("   ").chars().count() > room {
            shown.pop();
        }
        // Dropped from the far end, and read from the other, with the cursor last.
        shown.reverse();
        let right = shown.join("   ");
        let right_x = width.saturating_sub(right.chars().count() as u16 + 1);
        if let (Some((at, end)), Some(branch)) = (self.status_branch(), &self.branch) {
            let hovered = self
                .hover
                .is_some_and(|h| h.1 == y && h.0 >= at && h.0 < end);
            if hovered {
                let chip = Rect::new(at, y, end.saturating_sub(at), 1);
                grid.panel(
                    Bounds::inset(chip, -0.3, 0.12),
                    16,
                    Some(self.theme.row_hover),
                    None,
                );
            }
            let color = if hovered { self.theme.text.fg } else { bar.fg };
            let shown = Style { fg: color, ..bar };
            grid.glyph(at as f32, y, Icon::Git, color, "⎇", bar);
            grid.label_at(at as f32 + 1.8, at + 2, y, branch, shown, end, 90);
        }
        let mut x = start;
        if let Some(problems) = &problems {
            x = grid.label(x, y, problems, bar, right_x);
        }
        let needs_you = waiting > 0 && message.is_empty() && self.neovim_line.is_empty();
        let bar = if needs_you {
            let dot = Bounds::cells(x as f32, y as f32 + 0.34, 0.45, 0.32);
            grid.panel(dot, 16, Some(self.theme.attention), None);
            x += 1;
            Style {
                fg: self.theme.attention,
                ..bar
            }
        } else {
            bar
        };
        grid.label_at(x as f32, x, y, left, bar, right_x.saturating_sub(1), 90);
        grid.label_right(width.saturating_sub(1), y, &right, bar, 90);
    }

    /// Where the branch sits in the status bar, as (start, end) columns.
    fn status_branch(&self) -> Option<(u16, u16)> {
        let name = self.branch.as_ref()?.chars().count() as u16;
        Some((1, 1 + 2 + name.min(STATUS_BRANCH_WIDTH) + 1))
    }

    /// Whether something in the side pane may write at any moment.
    pub fn has_session(&self) -> bool {
        self.dock.running()
    }

    pub fn rename_running(&self) -> bool {
        self.pending_rename.is_some()
    }

    pub fn overlay_is_open(&self) -> bool {
        !matches!(self.overlay, Overlay::None) || self.suggest.is_some()
    }

    pub fn size(&self) -> (u16, u16) {
        self.size
    }

    pub fn background(&self) -> Style {
        self.theme.text
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cellix_input::{Mods, MouseEvent};
    use cellix_render::Paint;
    use std::fs;
    use tempfile::TempDir;

    fn project() -> (TempDir, App) {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("alpha.txt"), "first line\nsecond line\n").unwrap();
        fs::write(dir.path().join("beta.rs"), "fn main() {}\n").unwrap();
        fs::create_dir(dir.path().join("src")).unwrap();
        fs::write(dir.path().join("src/lib.rs"), "pub fn x() {}\n").unwrap();
        let app = App::new(dir.path().to_path_buf(), (100, 30));
        (dir, app)
    }

    fn screen(app: &App) -> Grid {
        let mut grid = Grid::new(app.size().0, app.size().1, app.background());
        app.draw(&mut grid);
        grid
    }

    fn mouse(kind: MouseKind, x: u16, y: u16) -> Event {
        Event::Mouse(MouseEvent {
            kind,
            x,
            y,
            fine_x: x as f32,
            mods: Mods::default(),
        })
    }

    fn typed(app: &mut App, text: &str) {
        for c in text.chars() {
            app.handle(Event::Key(KeyEvent::plain(Key::Char(c))));
        }
    }

    fn enter(app: &mut App) {
        app.handle(Event::Key(KeyEvent::plain(Key::Enter)));
    }

    /// Row of the first line containing `text`.
    fn find(grid: &Grid, text: &str) -> (u16, u16) {
        for y in 0..grid.height() {
            if let Some(i) = grid.row_text(y).find(text) {
                return (grid.row_text(y)[..i].chars().count() as u16, y);
            }
        }
        panic!("{text:?} not on screen:\n{}", grid.text());
    }

    /// Right-clicks a row in the explorer and picks a context menu entry.
    fn menu(app: &mut App, name: &str, item: &str) {
        let (x, y) = find(&screen(app), name);
        app.handle(mouse(MouseKind::Right, x, y));
        let (x, y) = find(&screen(app), item);
        app.handle(mouse(MouseKind::Left, x + 1, y));
    }

    #[test]
    fn click_in_explorer_opens_file() {
        let (_dir, mut app) = project();
        let (x, y) = find(&screen(&app), "beta.rs");
        app.handle(mouse(MouseKind::Left, x, y));
        assert!(screen(&app).text().contains("fn main() {}"));
    }

    #[test]
    fn click_on_folder_expands_it() {
        let (_dir, mut app) = project();
        let (x, y) = find(&screen(&app), "src");
        app.handle(mouse(MouseKind::Left, x, y));
        assert!(screen(&app).text().contains("lib.rs"));
        app.handle(mouse(MouseKind::Left, x, y));
        assert!(!screen(&app).text().contains("lib.rs"));
    }

    #[test]
    fn explorer_create_rename_delete() {
        let (dir, mut app) = project();
        menu(&mut app, "alpha.txt", "New File");
        typed(&mut app, "made.txt");
        enter(&mut app);
        assert!(dir.path().join("made.txt").exists());
        assert!(screen(&app).text().contains("made.txt"));

        menu(&mut app, "beta.rs", "Rename");
        for _ in 0.."beta.rs".len() {
            app.handle(Event::Key(KeyEvent::plain(Key::Backspace)));
        }
        typed(&mut app, "gamma.rs");
        enter(&mut app);
        assert!(dir.path().join("gamma.rs").exists());
        assert!(!dir.path().join("beta.rs").exists());

        menu(&mut app, "gamma.rs", "Delete");
        typed(&mut app, "n");
        enter(&mut app);
        assert!(dir.path().join("gamma.rs").exists(), "n must not delete");
        menu(&mut app, "gamma.rs", "Delete");
        typed(&mut app, "y");
        enter(&mut app);
        assert!(!dir.path().join("gamma.rs").exists());
        assert!(!screen(&app).text().contains("gamma.rs"));
    }

    #[test]
    fn drag_selects_text() {
        let (dir, mut app) = project();
        app.open(&dir.path().join("alpha.txt")).unwrap();
        let (x, y) = find(&screen(&app), "first line");
        app.handle(mouse(MouseKind::Left, x, y));
        app.handle(mouse(MouseKind::Drag, x + 5, y));
        app.handle(mouse(MouseKind::Up, x + 5, y));
        assert_eq!(app.selected_text().as_deref(), Some("first"));
        let grid = screen(&app);
        assert_eq!(grid.cell(x, y).style.bg, Theme::default().selection);
    }

    #[test]
    fn wheel_scrolls_without_moving_the_cursor() {
        let (dir, mut app) = project();
        let long: String = (0..200).map(|i| format!("line {i}\n")).collect();
        fs::write(dir.path().join("long.txt"), long).unwrap();
        app.open(&dir.path().join("long.txt")).unwrap();
        app.handle(mouse(MouseKind::ScrollDown, 60, 10));
        let text = screen(&app).text();
        assert!(text.contains("line 3"));
        assert!(!text.contains("line 2\n"));
    }

    #[test]
    fn type_and_save_writes_file() {
        let (dir, mut app) = project();
        let path = dir.path().join("alpha.txt");
        app.open(&path).unwrap();
        typed(&mut app, "hi ");
        assert!(screen(&app).text().contains("●"));
        app.handle(Event::Key(KeyEvent::ctrl('s')));
        assert_eq!(
            fs::read_to_string(path).unwrap(),
            "hi first line\nsecond line\n"
        );
        assert!(!screen(&app).text().contains("●"));
    }

    #[test]
    fn click_tab_switches_buffer() {
        let (dir, mut app) = project();
        app.open(&dir.path().join("alpha.txt")).unwrap();
        app.open(&dir.path().join("beta.rs")).unwrap();
        let (x, y) = find(&screen(&app), " alpha.txt ");
        app.handle(mouse(MouseKind::Left, x + 2, y));
        assert!(screen(&app).text().contains("first line"));
    }

    #[test]
    fn newline_keeps_indent() {
        let (dir, mut app) = project();
        fs::write(dir.path().join("i.txt"), "    a").unwrap();
        app.open(&dir.path().join("i.txt")).unwrap();
        app.run(Command::Move(cellix_input::Direction::LineEnd, false));
        app.run(Command::Newline);
        typed(&mut app, "b");
        assert_eq!(app.active_text().unwrap(), "    a\n    b");
    }

    #[test]
    fn bracketed_paste_is_one_undo() {
        let (dir, mut app) = project();
        app.open(&dir.path().join("alpha.txt")).unwrap();
        app.handle(Event::Paste("many\nlines\nof text".into()));
        assert!(
            app.active_text()
                .unwrap()
                .starts_with("many\nlines\nof text")
        );
        app.run(Command::Undo);
        assert_eq!(app.active_text().unwrap(), "first line\nsecond line\n");
    }

    #[test]
    fn copy_and_paste_round_trip_with_osc52() {
        let (dir, mut app) = project();
        app.open(&dir.path().join("alpha.txt")).unwrap();
        app.run(Command::SelectAll);
        app.run(Command::Copy);
        assert_eq!(
            app.outgoing_clipboard.take().as_deref(),
            Some("first line\nsecond line\n")
        );
        app.run(Command::Cut);
        assert_eq!(app.active_text().unwrap(), "");
        app.run(Command::Paste);
        assert_eq!(app.active_text().unwrap(), "first line\nsecond line\n");
    }

    #[test]
    fn split_shows_two_panes() {
        let (dir, mut app) = project();
        app.open(&dir.path().join("alpha.txt")).unwrap();
        app.run(Command::SplitPane);
        app.open(&dir.path().join("beta.rs")).unwrap();
        let text = screen(&app).text();
        assert!(text.contains("first line"), "{text}");
        assert!(text.contains("fn main()"), "{text}");
        let (x, y) = find(&screen(&app), "first line");
        app.handle(mouse(MouseKind::Left, x, y));
        typed(&mut app, "!");
        assert!(screen(&app).text().contains("!first line"));
        assert!(!screen(&app).text().contains("!fn main"));
    }

    #[test]
    fn closing_a_tab_keeps_panes_valid() {
        let (dir, mut app) = project();
        app.open(&dir.path().join("alpha.txt")).unwrap();
        app.run(Command::SplitPane);
        app.open(&dir.path().join("beta.rs")).unwrap();
        app.run(Command::CloseTab);
        assert!(screen(&app).text().contains("first line"));
        app.run(Command::CloseTab);
        app.run(Command::CloseTab);
        screen(&app);
    }

    #[test]
    fn quick_open_fuzzy_matches() {
        let (_dir, mut app) = project();
        app.run(Command::QuickOpen);
        typed(&mut app, "lbr");
        assert!(screen(&app).text().contains("src/lib.rs"));
        enter(&mut app);
        assert!(screen(&app).text().contains("pub fn x()"));
    }

    #[test]
    fn command_palette_runs_commands_by_mouse() {
        let (_dir, mut app) = project();
        app.run(Command::CommandPalette);
        typed(&mut app, "sidebar");
        let (x, y) = find(&screen(&app), "Toggle Sidebar");
        app.handle(mouse(MouseKind::Left, x, y));
        assert!(!screen(&app).text().contains("alpha.txt"));
    }

    #[test]
    fn rust_keywords_are_highlighted() {
        let (dir, mut app) = project();
        app.open(&dir.path().join("beta.rs")).unwrap();
        let grid = screen(&app);
        let (x, y) = find(&grid, "fn main");
        assert_eq!(grid.cell(x, y).style.fg, Theme::default().keyword);
        assert_eq!(grid.cell(x + 3, y).style.fg, Theme::default().function);
    }

    #[test]
    fn external_change_reloads_clean_buffer() {
        let (dir, mut app) = project();
        let path = dir.path().join("alpha.txt");
        app.open(&path).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        fs::write(&path, "written by an agent\n").unwrap();
        app.handle(Event::Tick);
        assert!(screen(&app).text().contains("written by an agent"));
        assert!(screen(&app).text().contains("Reloaded alpha.txt"));
        app.run(Command::Undo);
        assert!(screen(&app).text().contains("first line"));
    }

    #[test]
    fn external_change_keeps_unsaved_edits() {
        let (dir, mut app) = project();
        let path = dir.path().join("alpha.txt");
        app.open(&path).unwrap();
        typed(&mut app, "mine ");
        std::thread::sleep(std::time::Duration::from_millis(20));
        fs::write(&path, "theirs\n").unwrap();
        app.handle(Event::Tick);
        let text = screen(&app).text();
        assert!(text.contains("mine first line"), "{text}");
        assert!(text.contains("keeping your edits"), "{text}");
    }

    #[test]
    fn new_files_appear_in_the_explorer_by_themselves() {
        let (dir, mut app) = project();
        fs::write(dir.path().join("from-agent.txt"), "x").unwrap();
        for _ in 0..EXPLORER_REFRESH_TICKS {
            app.handle(Event::Tick);
        }
        assert!(screen(&app).text().contains("from-agent.txt"));
    }

    fn git(dir: &Path, args: &[&str]) {
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .unwrap()
            .status;
        assert!(status.success(), "git {args:?}");
    }

    /// A committed project with one agent edit to alpha.txt.
    fn reviewed() -> (TempDir, App) {
        let (dir, _) = project();
        git(dir.path(), &["init", "-q"]);
        git(dir.path(), &["config", "user.email", "t@example.com"]);
        git(dir.path(), &["config", "user.name", "Tester"]);
        git(dir.path(), &["add", "."]);
        git(dir.path(), &["commit", "-q", "-m", "base"]);
        fs::write(dir.path().join("alpha.txt"), "first line\nCHANGED\n").unwrap();
        let app = App::new(dir.path().to_path_buf(), (120, 30));
        (dir, app)
    }

    #[test]
    fn a_change_stacks_the_old_line_over_the_new_one() {
        let (_dir, mut app) = reviewed();
        app.run(Command::OpenDiff);
        let grid = screen(&app);
        let (_, y) = find(&grid, "second line");
        assert!(grid.row_text(y + 1).contains("CHANGED"), "{}", grid.text());
        assert_eq!(find(&grid, "first line").1 + 1, y);
        assert!(grid.text().contains("[Accept]") && grid.text().contains("[Reject]"));
        assert!(
            grid.text().contains("since the last commit"),
            "{}",
            grid.text()
        );
    }

    #[test]
    fn a_comment_sits_in_its_card() {
        let (dir, mut app) = reviewed();
        app.open(&dir.path().join("alpha.txt")).unwrap();
        app.run(Command::Move(cellix_input::Direction::Down, false));
        app.run(Command::AddComment);
        typed(&mut app, "Why this line");
        enter(&mut app);
        app.run(Command::OpenDiff);
        let grid = screen(&app);
        let (_, y) = find(&grid, "Why this line");
        assert!(grid.row_text(y).contains("Your comment"), "{}", grid.text());
    }

    #[test]
    fn reject_hunk_restores_file() {
        let (dir, mut app) = reviewed();
        app.run(Command::OpenDiff);
        let (x, y) = find(&screen(&app), "[Reject]");
        app.handle(mouse(MouseKind::Left, x + 1, y));
        assert_eq!(
            fs::read_to_string(dir.path().join("alpha.txt")).unwrap(),
            "first line\nsecond line\n"
        );
        assert!(screen(&app).text().contains("No changes"));
    }

    #[test]
    fn accept_stages_a_hunk_and_keeps_the_file() {
        let (dir, mut app) = reviewed();
        app.run(Command::OpenDiff);
        let (x, y) = find(&screen(&app), "[Accept]");
        app.handle(mouse(MouseKind::Left, x + 1, y));
        assert!(screen(&app).text().contains("staged"));
        let staged = cellix_review::index_text(dir.path(), Path::new("alpha.txt"));
        assert!(staged.contains("CHANGED"), "{staged}");
        assert!(
            fs::read_to_string(dir.path().join("alpha.txt"))
                .unwrap()
                .contains("CHANGED")
        );
    }

    #[test]
    fn commit_from_review() {
        let (dir, mut app) = reviewed();
        app.run(Command::OpenDiff);
        let (x, y) = find(&screen(&app), "[Accept]");
        app.handle(mouse(MouseKind::Left, x + 1, y));
        let (x, y) = find(&screen(&app), "[Commit 1 staged]");
        app.handle(mouse(MouseKind::Left, x + 1, y));
        typed(&mut app, "Change the second line");
        enter(&mut app);
        assert!(screen(&app).text().contains("Committed"));
        assert!(cellix_review::changed_files(dir.path()).is_empty());
    }

    #[test]
    fn review_compares_against_a_turn() {
        let (dir, mut app) = reviewed();
        cellix_review::checkpoint(dir.path(), "Add the limit").unwrap();
        fs::write(dir.path().join("beta.rs"), "fn turn() {}\n").unwrap();
        app.run(Command::OpenDiff);
        let text = screen(&app).text();
        assert!(text.contains("alpha.txt"), "against HEAD: {text}");
        let (x, y) = find(&screen(&app), "Add the limit");
        app.handle(mouse(MouseKind::Left, x, y));
        assert!(
            app.review.as_ref().unwrap().base.is_some(),
            "the click lands at once, before git has answered"
        );
        settle_review(&mut app);
        let text = screen(&app).text();
        assert!(text.contains("fn turn()"), "{text}");
        assert!(
            !text.contains("alpha.txt"),
            "that changed before the turn: {text}"
        );
        app.run(Command::ReviewPreviousTurn);
        settle_review(&mut app);
        assert!(
            screen(&app).text().contains("alpha.txt"),
            "back to the last commit"
        );
    }

    #[test]
    fn a_turn_can_be_undone_from_review() {
        let (dir, mut app) = reviewed();
        cellix_review::checkpoint(dir.path(), "Prompt").unwrap();
        fs::write(dir.path().join("alpha.txt"), "agent\n").unwrap();
        app.run(Command::OpenDiff);
        app.run(Command::UndoTurn);
        assert!(screen(&app).text().contains("Undo back to"));
        typed(&mut app, "y");
        enter(&mut app);
        let text = fs::read_to_string(dir.path().join("alpha.txt")).unwrap();
        assert_eq!(text, "first line\nCHANGED\n");
    }

    #[test]
    fn diffs_have_syntax_colours() {
        let (dir, _) = reviewed();
        fs::write(dir.path().join("beta.rs"), "fn other() {}\n").unwrap();
        let mut app = App::new(dir.path().to_path_buf(), (120, 30));
        app.run(Command::OpenDiff);
        let list = (0..30)
            .find(|y| screen(&app).row_text(*y).contains("beta.rs"))
            .unwrap();
        app.handle(mouse(MouseKind::Left, 6, list));
        let grid = screen(&app);
        let (x, y) = find(&grid, "fn other");
        assert_eq!(grid.cell(x, y).style.fg, Theme::default().keyword);
    }

    #[test]
    fn reject_refuses_to_overwrite_unsaved_edits() {
        let (dir, mut app) = reviewed();
        app.open(&dir.path().join("alpha.txt")).unwrap();
        typed(&mut app, "mine");
        app.run(Command::OpenDiff);
        let (x, y) = find(&screen(&app), "[Reject]");
        app.handle(mouse(MouseKind::Left, x + 1, y));
        assert!(
            fs::read_to_string(dir.path().join("alpha.txt"))
                .unwrap()
                .contains("CHANGED")
        );
        assert!(screen(&app).text().contains("Save or undo"));
    }

    #[test]
    fn comment_is_attached_to_line() {
        let (dir, mut app) = reviewed();
        app.run(Command::OpenDiff);
        let (x, y) = find(&screen(&app), "CHANGED");
        app.handle(mouse(MouseKind::Left, x, y));
        typed(&mut app, "why this");
        enter(&mut app);
        let comment = app.comments.lock().unwrap()[0].clone();
        assert_eq!(
            comment,
            Comment {
                file: "alpha.txt".into(),
                line: 2,
                text: "why this".into()
            }
        );
        app.run(Command::OpenDiff);
        app.open(&dir.path().join("alpha.txt")).unwrap();
        assert!(screen(&app).text().contains("◆"));
    }

    #[test]
    fn plan_review_returns_comments() {
        let (dir, mut app) = project();
        let plan = dir.path().join("plan.md");
        fs::write(&plan, "# Plan\nstep one\n").unwrap();
        app.wait_file = Some(plan.clone());
        app.open(&plan).unwrap();
        app.handle(Event::Mouse(MouseEvent {
            kind: MouseKind::Right,
            x: find(&screen(&app), "step one").0,
            y: find(&screen(&app), "step one").1,
            fine_x: find(&screen(&app), "step one").0 as f32,
            mods: Mods::default(),
        }));
        let (x, y) = find(&screen(&app), "Add Comment");
        app.handle(mouse(MouseKind::Left, x + 1, y));
        typed(&mut app, "split this step");
        enter(&mut app);
        app.finish().unwrap();
        assert_eq!(
            fs::read_to_string(plan).unwrap(),
            "# Plan\nstep one\n\n## Review comments\n\n- Line 2: split this step\n"
        );
    }

    #[test]
    fn explorer_shows_git_status() {
        let (dir, mut app) = reviewed();
        let (_, y) = find(&screen(&app), "alpha.txt");
        assert!(
            screen(&app).row_text(y).contains(" M"),
            "{}",
            screen(&app).row_text(y)
        );
        fs::write(dir.path().join("beta.rs"), "changed\n").unwrap();
        for _ in 0..EXPLORER_REFRESH_TICKS {
            app.handle(Event::Tick);
        }
        let (_, y) = find(&screen(&app), "beta.rs");
        assert!(screen(&app).row_text(y).contains("M"));
    }

    #[test]
    fn blame_reports_the_commit() {
        let (dir, mut app) = reviewed();
        app.open(&dir.path().join("beta.rs")).unwrap();
        app.run(Command::BlameLine);
        assert!(screen(&app).text().contains("Tester: base"));
    }

    #[test]
    fn quit_with_unsaved_changes_asks_first() {
        let (dir, mut app) = project();
        app.open(&dir.path().join("alpha.txt")).unwrap();
        typed(&mut app, "x");
        app.run(Command::Quit);
        assert!(!app.quit);
        assert!(screen(&app).text().contains("Save changes to"));
        app.handle(Event::Key(KeyEvent::plain(Key::Esc)));
        assert!(!app.quit, "Esc must cancel");
        app.run(Command::Quit);
        typed(&mut app, "y");
        enter(&mut app);
        assert!(app.quit);
        assert!(
            fs::read_to_string(dir.path().join("alpha.txt"))
                .unwrap()
                .starts_with("xfirst")
        );
    }

    #[test]
    fn quit_can_discard_on_request() {
        let (dir, mut app) = project();
        app.open(&dir.path().join("alpha.txt")).unwrap();
        typed(&mut app, "x");
        app.run(Command::Quit);
        typed(&mut app, "n");
        enter(&mut app);
        assert!(app.quit);
        assert!(
            fs::read_to_string(dir.path().join("alpha.txt"))
                .unwrap()
                .starts_with("first")
        );
    }

    fn dirty(dir: &TempDir, app: &mut App) {
        app.open(&dir.path().join("alpha.txt")).unwrap();
        typed(app, "x");
    }

    #[test]
    fn closing_unsaved_work_offers_buttons() {
        let (dir, mut app) = project();
        dirty(&dir, &mut app);
        app.run(Command::CloseTab);
        let text = screen(&app).text();
        for label in ["[Save]", "[Don't Save]", "[Cancel]"] {
            assert!(text.contains(label), "{text}");
        }
        assert!(text.contains("Save changes to alpha.txt"), "{text}");
    }

    #[test]
    fn the_keyboard_works_the_buttons() {
        let (dir, mut app) = project();
        dirty(&dir, &mut app);
        app.run(Command::CloseTab);
        key(&mut app, Key::Right);
        enter(&mut app);
        assert!(
            !screen(&app).text().contains("xfirst line"),
            "Don't Save closes it"
        );
        let on_disk = fs::read_to_string(dir.path().join("alpha.txt")).unwrap();
        assert!(
            on_disk.starts_with("first line"),
            "and leaves the file alone"
        );

        dirty(&dir, &mut app);
        app.run(Command::CloseTab);
        enter(&mut app);
        let on_disk = fs::read_to_string(dir.path().join("alpha.txt")).unwrap();
        assert!(on_disk.starts_with("xfirst line"), "Save is the default");
    }

    #[test]
    fn the_mouse_works_the_buttons() {
        let (dir, mut app) = project();
        dirty(&dir, &mut app);
        app.run(Command::CloseTab);
        let (x, y) = find(&screen(&app), "[Cancel]");
        app.handle(mouse(MouseKind::Left, x + 1, y));
        assert!(
            screen(&app).text().contains("xfirst line"),
            "Cancel keeps the tab"
        );
        assert!(!app.overlay_is_open());
        app.run(Command::CloseTab);
        let (x, y) = find(&screen(&app), "[Don't Save]");
        app.handle(mouse(MouseKind::Left, x + 1, y));
        assert!(!screen(&app).text().contains("xfirst line"));
    }

    #[test]
    fn deleting_defaults_to_cancel() {
        let (dir, mut app) = project();
        menu(&mut app, "beta.rs", "Delete");
        enter(&mut app);
        assert!(
            dir.path().join("beta.rs").exists(),
            "Enter alone keeps the file"
        );
        menu(&mut app, "beta.rs", "Delete");
        let (x, y) = find(&screen(&app), "[Delete]");
        app.handle(mouse(MouseKind::Left, x + 1, y));
        assert!(!dir.path().join("beta.rs").exists());
    }

    #[test]
    fn closing_a_dirty_tab_asks_first() {
        let (dir, mut app) = project();
        app.open(&dir.path().join("alpha.txt")).unwrap();
        typed(&mut app, "x");
        app.run(Command::CloseTab);
        assert!(screen(&app).text().contains("Save changes to"));
        typed(&mut app, "n");
        enter(&mut app);
        assert!(!screen(&app).text().contains("xfirst line"));
    }

    #[test]
    fn messages_outlive_background_ticks() {
        let (dir, mut app) = project();
        app.open(&dir.path().join("alpha.txt")).unwrap();
        app.run(Command::Save);
        app.handle(Event::Tick);
        assert!(screen(&app).text().contains("Saved alpha.txt"));
    }

    fn vim_app(text: &str) -> (TempDir, App) {
        let (dir, _) = project();
        fs::create_dir(dir.path().join(".cellix")).unwrap();
        fs::write(
            dir.path().join(".cellix/settings.json"),
            r#"{"keymap": "vim"}"#,
        )
        .unwrap();
        fs::write(dir.path().join("v.txt"), text).unwrap();
        let mut app = App::new(dir.path().to_path_buf(), (100, 30));
        app.open(&dir.path().join("v.txt")).unwrap();
        (dir, app)
    }

    #[test]
    fn vim_motions_and_operators() {
        let (_dir, mut app) = vim_app("one two three\nsecond\nthird\n");
        assert!(screen(&app).text().contains("NORMAL"));
        typed(&mut app, "wdw");
        assert_eq!(app.active_text().unwrap(), "one three\nsecond\nthird\n");
        typed(&mut app, "jdd");
        assert_eq!(app.active_text().unwrap(), "one three\nthird\n");
        typed(&mut app, "ggyyGp");
        assert!(app.active_text().unwrap().contains("one three\n"));
        typed(&mut app, "u");
        typed(&mut app, "ggx");
        assert_eq!(app.active_text().unwrap(), "ne three\nthird\n");
        typed(&mut app, "u");
        typed(&mut app, "A!");
        assert!(screen(&app).text().contains("INSERT"));
        app.handle(Event::Key(KeyEvent::plain(Key::Esc)));
        assert_eq!(app.active_text().unwrap(), "one three!\nthird\n");
    }

    #[test]
    fn vim_counts_and_change() {
        let (_dir, mut app) = vim_app("a\nb\nc\nd\n");
        typed(&mut app, "2dd");
        assert_eq!(app.active_text().unwrap(), "c\nd\n");
        typed(&mut app, "cwZ");
        assert_eq!(app.active_text().unwrap(), "Z\nd\n");
    }

    #[test]
    fn vim_visual_deletes_the_selection() {
        let (_dir, mut app) = vim_app("hello world\n");
        typed(&mut app, "vlld");
        assert_eq!(app.active_text().unwrap(), "lo world\n");
    }

    #[test]
    fn simple_is_the_default_keymap() {
        let (dir, mut app) = project();
        app.open(&dir.path().join("alpha.txt")).unwrap();
        typed(&mut app, "dd");
        assert!(app.active_text().unwrap().starts_with("ddfirst"));
        assert!(!screen(&app).text().contains("NORMAL"));
    }

    /// Sends text to the search box and waits for the search to finish.
    fn search_for(app: &mut App, query: &str) {
        app.run(Command::FindInProject);
        typed(app, query);
        wait(app, |a| {
            let text = screen(a).text();
            !text.contains("Searching") && (text.contains(" result") || text.contains("No results"))
        });
    }

    #[test]
    fn search_replaces_the_explorer_in_the_sidebar() {
        let (_dir, mut app) = project();
        app.run(Command::FindInProject);
        let text = screen(&app).text();
        assert!(text.contains("SEARCH"), "{text}");
        assert!(!text.contains("alpha.txt"), "{text}");
    }

    #[test]
    fn project_search_groups_results_by_file() {
        let (dir, mut app) = project();
        fs::write(dir.path().join("found.txt"), "needle one\nneedle two\n").unwrap();
        fs::write(dir.path().join("src/more.txt"), "a needle\n").unwrap();
        search_for(&mut app, "needle");
        let text = screen(&app).text();
        assert!(text.contains("3 results in 2 files"), "{text}");
        assert!(text.contains("found.txt"), "{text}");
        assert!(text.contains("more.txt"), "{text}");
    }

    #[test]
    fn search_highlights_the_match() {
        let (dir, mut app) = project();
        fs::write(dir.path().join("found.txt"), "    a needle here\n").unwrap();
        search_for(&mut app, "needle");
        let grid = screen(&app);
        let (x, y) = find(&grid, "a needle here");
        assert_eq!(grid.cell(x + 2, y).style.bg, Theme::default().find_match);
        assert_ne!(grid.cell(x, y).style.bg, Theme::default().find_match);
    }

    #[test]
    fn clicking_a_result_opens_it_with_the_match_selected() {
        let (dir, mut app) = project();
        fs::write(dir.path().join("found.txt"), "a\nb\nsome needle here\n").unwrap();
        search_for(&mut app, "needle");
        let (x, y) = find(&screen(&app), "some needle here");
        app.handle(mouse(MouseKind::Left, x, y));
        assert_eq!(app.selected_text().as_deref(), Some("needle"));
        assert!(screen(&app).text().contains("Ln 3, Col"));
    }

    #[test]
    fn clicking_a_file_collapses_its_results() {
        let (dir, mut app) = project();
        fs::write(dir.path().join("found.txt"), "needle one\n").unwrap();
        search_for(&mut app, "needle");
        let (x, y) = find(&screen(&app), "found.txt");
        app.handle(mouse(MouseKind::Left, x, y));
        assert!(!screen(&app).text().contains("needle one"));
    }

    #[test]
    fn search_follows_typing_and_drops_stale_results() {
        let (dir, mut app) = project();
        fs::write(dir.path().join("found.txt"), "zqneedle\nzqneat\n").unwrap();
        search_for(&mut app, "zqne");
        assert!(screen(&app).text().contains("2 results"));
        typed(&mut app, "at");
        wait(&mut app, |a| screen(a).text().contains("1 result in"));
    }

    #[test]
    fn typing_in_search_does_not_touch_the_open_file() {
        let (dir, mut app) = project();
        app.open(&dir.path().join("alpha.txt")).unwrap();
        search_for(&mut app, "first");
        assert_eq!(app.active_text().unwrap(), "first line\nsecond line\n");
    }

    #[test]
    fn escape_returns_the_keyboard_to_the_editor() {
        let (dir, mut app) = project();
        app.open(&dir.path().join("alpha.txt")).unwrap();
        app.run(Command::FindInProject);
        app.handle(Event::Key(KeyEvent::plain(Key::Esc)));
        typed(&mut app, "x");
        assert!(app.active_text().unwrap().starts_with("xfirst"));
    }

    #[test]
    fn shortcuts_still_work_from_the_search_box() {
        let (_dir, mut app) = project();
        app.run(Command::FindInProject);
        app.handle(Event::Key(KeyEvent::ctrl('p')));
        assert!(screen(&app).text().contains("src/lib.rs"));
    }

    #[test]
    fn project_replace_changes_all_matches() {
        let (dir, mut app) = project();
        fs::write(dir.path().join("a.txt"), "old old\n").unwrap();
        fs::write(dir.path().join("src/b.txt"), "old\n").unwrap();
        fs::write(dir.path().join("keep.txt"), "old\n").unwrap();
        app.open(&dir.path().join("keep.txt")).unwrap();
        typed(&mut app, "x");
        search_for(&mut app, "old");
        app.handle(Event::Key(KeyEvent::plain(Key::Tab)));
        typed(&mut app, "new");
        enter(&mut app);
        let read = |p: &str| fs::read_to_string(dir.path().join(p)).unwrap();
        assert_eq!(read("a.txt"), "new new\n");
        assert_eq!(read("src/b.txt"), "new\n");
        assert_eq!(read("keep.txt"), "old\n", "unsaved files are skipped");
        assert!(screen(&app).text().contains("Replaced in 2 files"));
    }

    #[test]
    fn replace_all_works_by_mouse() {
        let (dir, mut app) = project();
        fs::write(dir.path().join("a.txt"), "old\n").unwrap();
        search_for(&mut app, "old");
        app.handle(Event::Key(KeyEvent::plain(Key::Tab)));
        typed(&mut app, "new");
        let (x, y) = find(&screen(&app), " All");
        app.handle(mouse(MouseKind::Left, x + 2, y));
        assert_eq!(
            fs::read_to_string(dir.path().join("a.txt")).unwrap(),
            "new\n"
        );
    }

    #[test]
    fn activity_bar_switches_between_explorer_and_search() {
        let (_dir, mut app) = project();
        app.handle(mouse(MouseKind::Left, 2, 1 + ACTIVITY_STEP));
        assert!(screen(&app).text().contains("SEARCH"));
        app.handle(mouse(MouseKind::Left, 2, 1));
        assert!(screen(&app).text().contains("alpha.txt"));
    }

    #[test]
    fn narrow_search_opens_in_the_drawer_and_closes_on_a_pick() {
        let (dir, mut app) = narrow();
        fs::write(dir.path().join("found.txt"), "needle\n").unwrap();
        search_for(&mut app, "needle");
        let (x, y) = find(&screen(&app), "needle");
        let row = (y..30)
            .find(|y| screen(&app).row_text(*y).contains("    needle"))
            .unwrap();
        let _ = (x, y);
        app.handle(mouse(MouseKind::Left, 6, row));
        assert!(!screen(&app).text().contains("SEARCH"));
        assert_eq!(app.selected_text().as_deref(), Some("needle"));
    }

    fn page(text: &str) -> (TempDir, App) {
        let (dir, mut app) = project();
        fs::write(dir.path().join("n.md"), text).unwrap();
        app.open(&dir.path().join("n.md")).unwrap();
        (dir, app)
    }

    fn key(app: &mut App, key: Key) {
        app.handle(Event::Key(KeyEvent::plain(key)));
    }

    #[test]
    fn markdown_opens_rendered_and_editable() {
        let (dir, mut app) = page("# Title\n\nsome **bold** text\n");
        let grid = screen(&app);
        assert!(!grid.text().contains("# Title"));
        assert!(!grid.text().contains("**"));
        let (x, y) = find(&grid, "Title");
        assert!(grid.cell(x, y).style.bold);
        assert!(grid.text().contains("some bold text"));

        // Clicking on the page and typing edits the source at that spot.
        let (x, y) = find(&grid, "bold text");
        app.handle(mouse(MouseKind::Left, x + 1, y));
        typed(&mut app, "X");
        app.run(Command::Save);
        assert_eq!(
            fs::read_to_string(dir.path().join("n.md")).unwrap(),
            "# Title\n\nsome **bXold** text\n"
        );
    }

    #[test]
    fn there_is_no_second_pane_or_gutter() {
        let (_dir, app) = page("# Title\n");
        let grid = screen(&app);
        assert_eq!(grid.text().matches("Title").count(), 1);
        let (x, _) = find(&grid, "Title");
        assert_eq!(x, app.focused_area().x + 4);
    }

    #[test]
    fn typing_starts_after_a_hidden_marker() {
        let (_dir, mut app) = page("# Title\n");
        typed(&mut app, "My ");
        assert_eq!(app.active_text().unwrap(), "# My Title\n");
    }

    #[test]
    fn markers_stay_hidden_on_the_line_being_edited() {
        let (_dir, mut app) = page("# Title\n");
        typed(&mut app, "x");
        assert!(!screen(&app).text().contains("# "));
    }

    #[test]
    fn typing_a_marker_formats_the_line() {
        let (_dir, mut app) = page("plain\n");
        let grid = screen(&app);
        let before = find(&grid, "plain");
        typed(&mut app, "# ");
        let grid = screen(&app);
        let (x, y) = find(&grid, "plain");
        assert!(grid.cell(x, y).style.bold);
        assert_eq!((x, y), before);
    }

    #[test]
    fn enter_continues_a_list_and_an_empty_item_ends_it() {
        let (_dir, mut app) = page("- one\n");
        app.run(Command::Move(cellix_input::Direction::LineEnd, false));
        enter(&mut app);
        typed(&mut app, "two");
        assert_eq!(app.active_text().unwrap(), "- one\n- two\n");
        enter(&mut app);
        enter(&mut app);
        assert_eq!(app.active_text().unwrap(), "- one\n- two\n\n");
    }

    #[test]
    fn enter_counts_ordered_lists() {
        let (_dir, mut app) = page("1. one\n");
        app.run(Command::Move(cellix_input::Direction::LineEnd, false));
        enter(&mut app);
        typed(&mut app, "two");
        assert_eq!(app.active_text().unwrap(), "1. one\n2. two\n");
    }

    #[test]
    fn backspace_at_the_start_removes_the_marker() {
        let (_dir, mut app) = page("## Title\n");
        app.run(Command::Move(cellix_input::Direction::LineStart, false));
        key(&mut app, Key::Backspace);
        assert_eq!(app.active_text().unwrap(), "Title\n");
        app.run(Command::Undo);
        assert_eq!(app.active_text().unwrap(), "## Title\n");
    }

    #[test]
    fn spaces_type_on_a_page() {
        let (_dir, mut app) = page("one\n");
        app.run(Command::Move(cellix_input::Direction::LineEnd, false));
        typed(&mut app, " two three");
        assert_eq!(app.active_text().unwrap(), "one two three\n");
    }

    fn pixel_page(text: &str) -> (TempDir, App) {
        let (dir, mut app) = page(text);
        app.set_measure(Rc::new(cellix_render::FontMeasure::new(16, 36)));
        screen(&app);
        (dir, app)
    }

    #[test]
    fn spaces_type_on_a_page_of_real_fonts() {
        let (_dir, mut app) = pixel_page("one\n");
        app.run(Command::Move(cellix_input::Direction::LineEnd, false));
        typed(&mut app, " two  three");
        assert_eq!(app.active_text().unwrap(), "one two  three\n");
    }

    #[test]
    fn spaces_type_where_a_line_wraps() {
        let long = "word ".repeat(50);
        let (_dir, mut app) = pixel_page(&format!("{}\n", long.trim_end()));
        app.run(Command::Move(cellix_input::Direction::Top, false));
        app.run(Command::Move(cellix_input::Direction::LineEnd, false));
        app.run(Command::Move(cellix_input::Direction::Bottom, false));
        app.run(Command::Move(cellix_input::Direction::Left, false));
        typed(&mut app, " end of it");
        assert!(
            app.active_text().unwrap().contains("word end of it"),
            "{:?}",
            app.active_text()
        );
        // In the middle of the line too, at a wrapped row's edge.
        for _ in 0..40 {
            app.run(Command::Move(cellix_input::Direction::WordLeft, false));
            typed(&mut app, " ");
            app.run(Command::Move(cellix_input::Direction::Left, false));
        }
        let text = app.active_text().unwrap();
        assert!(text.matches(' ').count() > 50, "{text}");
    }

    #[test]
    fn a_space_at_the_end_of_a_wrapped_row_moves_the_caret() {
        let width = 60;
        let (_dir, mut app) = pixel_page("x\n");
        for i in 0..width {
            typed(&mut app, if i % 7 == 6 { " " } else { "a" });
        }
        let text = app.active_text().unwrap();
        assert_eq!(
            text.chars().filter(|c| *c == ' ').count(),
            width / 7,
            "{text}"
        );
    }

    #[test]
    fn typing_in_a_flowed_paragraph_keeps_the_files_lines() {
        let (_dir, mut app) = pixel_page("one\ntwo\n");
        for _ in 0..3 {
            key(&mut app, Key::Right);
        }
        typed(&mut app, " more");
        assert_eq!(app.active_text().unwrap(), "one more\ntwo\n");
    }

    #[test]
    fn backspace_joins_lines_in_a_flowed_paragraph() {
        let (_dir, mut app) = pixel_page("one\ntwo\n");
        for _ in 0..4 {
            key(&mut app, Key::Right);
        }
        key(&mut app, Key::Backspace);
        assert_eq!(app.active_text().unwrap(), "onetwo\n");
    }

    #[test]
    fn two_spaces_stay_two_on_a_page() {
        let (_dir, mut app) = page("one\n");
        app.run(Command::Move(cellix_input::Direction::LineEnd, false));
        typed(&mut app, "  x");
        assert_eq!(app.active_text().unwrap(), "one  x\n");
    }

    #[test]
    fn spaces_type_in_a_heading_and_a_list() {
        let (_dir, mut app) = page("# One\n\n- item\n");
        app.run(Command::Move(cellix_input::Direction::LineEnd, false));
        typed(&mut app, " more");
        assert!(app.active_text().unwrap().starts_with("# One more\n"));
        app.run(Command::Move(cellix_input::Direction::Bottom, false));
        app.run(Command::Move(cellix_input::Direction::Up, false));
        app.run(Command::Move(cellix_input::Direction::LineEnd, false));
        typed(&mut app, " two");
        assert!(app.active_text().unwrap().contains("- item two"));
    }

    #[test]
    fn the_wheel_moves_only_the_text() {
        let (dir, mut app) = project();
        app.set_measure(Rc::new(cellix_render::FontMeasure::new(10, 20)));
        app.handle(Event::Resize(140, 30));
        let path = dir.path().join("long.rs");
        fs::write(&path, "fn name() {}\n".repeat(300)).unwrap();
        app.open(&path).unwrap();
        app.handle(mouse(MouseKind::ScrollDown, 100, 10));
        let (area, _) = app.take_scroll().unwrap();
        let pane = app.focused_area();
        assert!(
            area.right() < pane.right().saturating_sub(10),
            "not the minimap: {area:?}"
        );
        assert!(area.x > pane.x, "not the sheet's rounded corner: {area:?}");
    }

    #[test]
    fn scrolling_stops_at_the_end_of_the_file() {
        let (dir, mut app) = project();
        let path = dir.path().join("short.rs");
        fs::write(&path, "x\n".repeat(40)).unwrap();
        app.open(&path).unwrap();
        screen(&app);
        for _ in 0..200 {
            app.handle(mouse(MouseKind::ScrollDown, 60, 10));
        }
        assert!(
            screen(&app).text().contains("41"),
            "the last line stays in view"
        );
        assert!(
            screen(&app).text().contains(" 20 "),
            "with a full page above it"
        );
    }

    #[test]
    fn clicking_a_checkbox_toggles_the_source() {
        let (_dir, mut app) = page("- [ ] task\n");
        let (x, y) = find(&screen(&app), "☐ task");
        app.handle(mouse(MouseKind::Left, x, y));
        assert_eq!(app.active_text().unwrap(), "- [x] task\n");
        assert!(screen(&app).text().contains("☑ task"));
    }

    #[test]
    fn tables_edit_in_their_cells() {
        let source = "| Name | Size |\n|---|---|\n| apple | b |\n";
        let (_dir, mut app) = page(source);
        let grid = screen(&app);
        assert!(!grid.text().contains('|'));
        // A word no temporary folder's name will hold, since the sidebar shows that name.
        let (x, y) = find(&grid, "apple");
        app.handle(mouse(MouseKind::Left, x + 1, y));
        typed(&mut app, "long");
        assert_eq!(
            app.active_text().unwrap(),
            "| Name | Size |\n|---|---|\n| alongpple | b |\n"
        );
        let text = screen(&app).text();
        assert!(text.contains("alongpple") && text.contains('│'), "{text}");
    }

    #[test]
    fn tab_moves_between_table_cells() {
        let (_dir, mut app) = page("| a | b |\n|---|---|\n| c | d |\n");
        key(&mut app, Key::Tab);
        typed(&mut app, "x");
        assert_eq!(
            app.active_text().unwrap(),
            "| a | xb |\n|---|---|\n| c | d |\n"
        );
    }

    #[test]
    fn arrows_walk_the_page_and_skip_hidden_text() {
        let (_dir, mut app) = page("**bold**\n\nnext\n");
        app.run(Command::Move(cellix_input::Direction::Down, false));
        app.run(Command::Move(cellix_input::Direction::Down, false));
        typed(&mut app, "x");
        assert_eq!(app.active_text().unwrap(), "**bold**\n\nxnext\n");
    }

    #[test]
    fn selecting_on_the_page_selects_source() {
        let (_dir, mut app) = page("# Title\n");
        for _ in 0..3 {
            app.run(Command::Move(cellix_input::Direction::Right, true));
        }
        assert_eq!(app.selected_text().as_deref(), Some("Tit"));
    }

    #[test]
    fn code_fences_hide_and_their_code_stays_editable() {
        let (_dir, mut app) = page("```rust\nlet x;\n```\n");
        let grid = screen(&app);
        assert!(!grid.text().contains("```"));
        assert!(grid.text().contains("rust"));
        let (x, y) = find(&grid, "let x;");
        assert_eq!(grid.cell(x, y).style.bg, Theme::default().code_bg);
        app.handle(mouse(MouseKind::Left, x, y));
        typed(&mut app, "y");
        assert_eq!(app.active_text().unwrap(), "```rust\nylet x;\n```\n");
    }

    #[test]
    fn the_page_wraps_in_a_narrow_pane() {
        let long = "word ".repeat(30);
        let (_dir, mut app) = page(&long);
        app.handle(Event::Resize(40, 30));
        let grid = screen(&app);
        assert!(grid.row_text(3).contains("word"), "{}", grid.text());
        app.run(Command::Move(cellix_input::Direction::LineEnd, false));
        app.run(Command::Move(cellix_input::Direction::Down, false));
        typed(&mut app, "x");
        assert_eq!(app.active_text().unwrap().matches('x').count(), 1);
    }

    #[test]
    fn the_source_is_one_command_away() {
        let (_dir, mut app) = page("# Title\n");
        app.run(Command::MarkdownPreview);
        assert!(screen(&app).text().contains("# Title"));
        app.run(Command::MarkdownPreview);
        assert!(!screen(&app).text().contains("# Title"));
    }

    #[test]
    fn long_pages_scroll_and_keep_the_caret_visible() {
        let long: String = (0..200).map(|i| format!("line {i}\n\n")).collect();
        let (_dir, mut app) = page(&long);
        app.run(Command::Move(cellix_input::Direction::Bottom, false));
        app.handle(mouse(MouseKind::ScrollUp, 60, 10));
        for _ in 0..80 {
            app.run(Command::Move(cellix_input::Direction::Down, false));
        }
        let cursor = screen(&app).cursor;
        assert!(cursor.is_some(), "the caret is off the page");
    }

    #[test]
    fn comments_show_on_rendered_pages() {
        let (_dir, mut app) = page("# Title\n");
        app.run(Command::AddComment);
        typed(&mut app, "note");
        enter(&mut app);
        assert!(screen(&app).text().contains("◆"));
    }

    fn browser_app(text: &str, name: &str) -> (TempDir, App, PathBuf) {
        let (dir, _) = project();
        let log = dir.path().join("opened.txt");
        fs::create_dir(dir.path().join(".cellix")).unwrap();
        let script = format!("printf '%s' \"$1\" > {}", log.display());
        let settings = serde_json::json!({"browser": ["sh", "-c", script, "browser"]});
        fs::write(
            dir.path().join(".cellix/settings.json"),
            settings.to_string(),
        )
        .unwrap();
        fs::write(dir.path().join(name), text).unwrap();
        let mut app = App::new(dir.path().to_path_buf(), (100, 30));
        app.open(&dir.path().join(name)).unwrap();
        (dir, app, log)
    }

    fn wait_for_file(path: &Path) -> String {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !path.exists() {
            assert!(
                std::time::Instant::now() < deadline,
                "the browser never ran"
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
        fs::read_to_string(path).unwrap()
    }

    #[test]
    fn the_link_at_the_cursor_opens_in_the_browser() {
        let (_dir, mut app, log) = browser_app("see [docs](https://a.b/c) now\n", "n.md");
        let (x, y) = find(&screen(&app), "docs");
        app.handle(Event::Mouse(MouseEvent {
            kind: MouseKind::Left,
            x: x + 1,
            y,
            fine_x: (x + 1) as f32,
            mods: Mods {
                ctrl: true,
                ..Mods::default()
            },
        }));
        assert_eq!(wait_for_file(&log), "https://a.b/c");
    }

    #[test]
    fn a_markdown_file_opens_in_the_browser_as_a_page() {
        let (_dir, mut app, log) = browser_app("# Title\n\nA formula: $x^2$\n", "notes.md");
        app.run(Command::OpenInBrowser);
        let opened = wait_for_file(&log);
        assert!(opened.ends_with("notes.html"), "{opened}");
        let page = fs::read_to_string(opened).unwrap();
        assert!(page.contains("<h1>Title</h1>"));
        assert!(page.contains("<svg"), "the formula is typeset");
    }

    #[test]
    fn a_failed_browser_falls_back_to_the_system_one() {
        let (dir, mut app, log) = browser_app("<h1>hi</h1>\n", "page.html");
        let script = format!("printf %s \"$1\" > {}", log.display());
        app.settings.browser = Some(vec!["false".to_string()]);
        app.system_browser = vec![
            "sh".to_string(),
            "-c".to_string(),
            script,
            "browser".to_string(),
        ];
        app.run(Command::OpenInBrowser);
        let opened = wait_for_file(&log);
        assert_eq!(opened, dir.path().join("page.html").display().to_string());
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !app.poll_browser() {
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(screen(&app).text().contains("Opened page.html"));
    }

    #[test]
    fn a_browser_that_never_works_says_so() {
        let (_dir, mut app, _log) = browser_app("<h1>hi</h1>\n", "page.html");
        app.settings.browser = Some(vec!["false".to_string()]);
        app.system_browser = vec!["false".to_string()];
        app.run(Command::OpenInBrowser);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !app.poll_browser() {
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(screen(&app).text().contains("No browser would open"));
    }

    fn java_project() -> (TempDir, App) {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("Main.java"), "class Main {}\n").unwrap();
        let mut app = App::new(dir.path().to_path_buf(), (120, 30));
        app.server_install = Some("exit 0".to_string());
        app.open(&dir.path().join("Main.java")).unwrap();
        // Whether jdtls is really here doesn't change what happens without it.
        app.servers.clear();
        app.note_missing("Java");
        (dir, app)
    }

    #[test]
    fn opening_a_file_offers_its_language_support() {
        let (_dir, mut app) = java_project();
        let grid = screen(&app);
        assert!(grid.text().contains("Install Java support?"));
        assert!(grid.text().contains("jdtls"));
        let (x, y) = find(&grid, "Not now");
        app.handle(mouse(MouseKind::Left, x, y));
        assert!(!screen(&app).text().contains("Install Java"), "asked once");
    }

    #[test]
    fn the_offer_installs_with_a_click() {
        let (_dir, mut app) = java_project();
        let at = app.offer_buttons()[0].2;
        let y = app.offer_area().y + OFFER_BUTTON_ROW;
        app.handle(mouse(MouseKind::Left, at + 1, y));
        assert!(screen(&app).text().contains("Installing Java support"));
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !app.poll_offer() {
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let grid = screen(&app);
        assert!(grid.text().contains("Installed jdtls"));
        assert!(!grid.text().contains("Install Java"));
    }

    #[test]
    fn a_language_can_be_declined_for_good() {
        let (_dir, mut app) = java_project();
        let (x, y) = find(&screen(&app), "Never");
        app.handle(mouse(MouseKind::Left, x, y));
        assert_eq!(app.settings.declined_languages, ["Java"]);
        assert!(app.offer().is_none());
    }

    #[test]
    fn html_files_open_in_the_browser() {
        let (dir, mut app, log) = browser_app("<h1>hi</h1>\n", "page.html");
        app.run(Command::OpenInBrowser);
        let opened = wait_for_file(&log);
        assert_eq!(opened, dir.path().join("page.html").display().to_string());
    }

    #[test]
    fn hover_highlights_a_row_without_clearing_messages() {
        let (_dir, mut app) = project();
        let grid = screen(&app);
        let before = grid.paint.len();
        let (x, y) = find(&grid, "beta.rs");
        app.say("still here");
        app.handle(mouse(MouseKind::Move, x, y));
        let after = screen(&app);
        assert!(after.paint.len() > before, "the row gains a highlight");
        assert!(after.text().contains("still here"));
    }

    #[test]
    fn chrome_is_painted_and_the_cells_keep_the_text() {
        let (dir, mut app) = project();
        app.open(&dir.path().join("alpha.txt")).unwrap();
        let grid = screen(&app);
        let has_text = |want: &str| {
            grid.paint
                .iter()
                .any(|p| matches!(p, cellix_render::Paint::Text { text, .. } if text == want))
        };
        assert!(has_text("beta.rs"), "explorer rows use the interface font");
        assert!(has_text("alpha.txt"), "tabs use the interface font");
        assert!(
            grid.text().contains("beta.rs"),
            "the cell tier still has it"
        );
        let (x, y) = find(&grid, "beta.rs");
        assert!(grid.cell(x, y).style.painted);
    }

    /// Starts a stand-in for an agent, and waits until it has drawn `ready`.
    fn session(app: &mut App, script: &str) {
        let argv = ["sh".to_string(), "-c".to_string(), script.to_string()];
        let root = app.workspace.root().to_path_buf();
        app.start_session("Stand-in", &argv, &[], &root);
        shows(app, "ready");
    }

    fn shows(app: &mut App, want: &str) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            app.poll();
            app.fit_dock();
            let text = screen(app).text();
            if text.contains(want) {
                return;
            }
            assert!(std::time::Instant::now() < deadline, "no {want}: {text}");
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    #[test]
    fn an_agent_runs_beside_the_editor() {
        let (dir, mut app) = project();
        app.open(&dir.path().join("alpha.txt")).unwrap();
        session(&mut app, "echo ready; read line; echo got:$line; sleep 9");
        let text = screen(&app).text();
        assert!(
            text.contains("first line"),
            "the file stays in view: {text}"
        );
        assert!(text.contains("Stand-in"), "{text}");
        typed(&mut app, "hello");
        enter(&mut app);
        shows(&mut app, "got:hello");
        let (x, y) = find(&screen(&app), "first line");
        app.handle(mouse(MouseKind::Left, x, y));
        typed(&mut app, "Z");
        assert!(
            screen(&app).text().contains("Z"),
            "the file has the keys again"
        );
        assert!(app.editors[0].buffer.text().contains('Z'));
        app.run(Command::FocusAgent);
        assert!(app.dock.focused);
    }

    #[test]
    fn the_rail_opens_the_pane_and_it_starts_a_shell() {
        let (_dir, mut app) = project();
        app.run(Command::ShowAgents);
        let grid = screen(&app);
        assert!(grid.text().contains("Terminal"), "{}", grid.text());
        assert!(app.main_area().w < 80, "the file makes room");
        let (x, y) = find(&grid, "Terminal");
        app.handle(mouse(MouseKind::Left, x, y));
        assert_eq!(app.dock.sessions.len(), 1);
        assert!(app.dock.focused);
        app.run(Command::ShowAgents);
        assert_eq!(app.main_area().right(), 100, "closing gives the room back");
        assert_eq!(app.dock.sessions.len(), 1, "and leaves it running");
    }

    #[test]
    fn a_pane_tab_closes_and_stops_its_program() {
        let (_dir, mut app) = project();
        session(&mut app, "echo ready; sleep 9");
        let area = app.dock_area().unwrap();
        let (_, end) = (area.x + 1, area.x + 1 + 8 + 6);
        app.handle(mouse(MouseKind::Left, end - 2, 0));
        assert!(app.dock.sessions.is_empty());
    }

    #[test]
    fn text_selected_in_a_session_is_copied() {
        let (_dir, mut app) = project();
        session(&mut app, "echo ready; sleep 9");
        let (x, y) = find(&screen(&app), "ready");
        app.handle(mouse(MouseKind::Left, x, y));
        app.handle(mouse(MouseKind::Drag, x + 4, y));
        app.handle(mouse(MouseKind::Up, x + 4, y));
        assert_eq!(app.clipboard, "ready");
    }

    #[test]
    fn auto_theme_follows_the_terminal() {
        let (_dir, mut app) = project();
        app.follow_terminal_theme(Some(true));
        assert_eq!(app.background(), Theme::named(Some("light")).text);
        app.follow_terminal_theme(Some(false));
        assert_eq!(app.background(), Theme::default().text);
        app.follow_terminal_theme(None);
        assert_eq!(app.background(), Theme::default().text);
    }

    #[test]
    fn a_chosen_theme_ignores_the_terminal() {
        let (dir, _) = project();
        fs::create_dir(dir.path().join(".cellix")).unwrap();
        fs::write(
            dir.path().join(".cellix/settings.json"),
            r#"{"theme": "dark"}"#,
        )
        .unwrap();
        let mut app = App::new(dir.path().to_path_buf(), (100, 30));
        app.follow_terminal_theme(Some(true));
        assert_eq!(app.background(), Theme::default().text);
    }

    #[test]
    fn the_caret_stays_inside_the_search_box() {
        let (_dir, mut app) = project();
        app.run(Command::FindInProject);
        typed(&mut app, &"x".repeat(200));
        let grid = screen(&app);
        let area = app.sidebar_area().unwrap();
        let caret = grid.caret.unwrap();
        let right = area.right() as i32 * cellix_render::units(1.0);
        assert!(caret.x < right, "the caret left the box");
        assert!(
            caret.h < cellix_render::units(1.0),
            "the caret is taller than the box"
        );
    }

    /// A page laid out with real fonts, as on the pixel tiers, where headings are tall.
    fn typeset(text: &str) -> (TempDir, App) {
        let (dir, mut app) = page(text);
        app.set_measure(Rc::new(cellix_render::FontMeasure::new(17, 37)));
        (dir, app)
    }

    #[test]
    fn clicking_either_half_of_a_tall_heading_lands_on_it() {
        for lower in [0, 1] {
            let (_dir, mut app) = typeset("## Start of a session\n\nbody\n");
            let grid = screen(&app);
            let (x, y) = grid
                .paint
                .iter()
                .find_map(|p| match p {
                    cellix_render::Paint::Text { text, x, y, .. } if text.starts_with("Start") => {
                        Some((
                            *x / cellix_render::units(1.0),
                            *y / cellix_render::units(1.0),
                        ))
                    }
                    _ => None,
                })
                .unwrap();
            app.handle(mouse(MouseKind::Left, x as u16 + 2, y as u16 + lower));
            typed(&mut app, "Q");
            let text = app.active_text().unwrap();
            assert!(text.starts_with("## "), "{text}");
            assert!(text.lines().next().unwrap().contains('Q'), "{text}");
        }
    }

    #[test]
    fn the_heading_mark_sits_on_the_headings_row() {
        let (_dir, app) = typeset("## Title\n");
        let grid = screen(&app);
        let row = |want: &str| {
            grid.paint.iter().find_map(|p| match p {
                cellix_render::Paint::Text { text, y, .. } if text.as_str() == want => Some(*y),
                _ => None,
            })
        };
        let (mark, title) = (row("#").unwrap(), row("Title").unwrap());
        assert!(mark >= title, "the mark isn't above the title");
        assert!(
            mark - title < cellix_render::units(0.3),
            "the mark is a row away"
        );
    }

    #[test]
    fn close_and_quit_work_with_a_palette_open() {
        let (dir, mut app) = project();
        app.open(&dir.path().join("alpha.txt")).unwrap();
        app.run(Command::QuickOpen);
        app.handle(Event::Key(KeyEvent::ctrl('w')));
        assert!(!screen(&app).text().contains("first line"));
        app.run(Command::QuickOpen);
        app.handle(Event::Key(KeyEvent::ctrl('q')));
        assert!(app.quit);
    }

    #[test]
    fn close_and_quit_work_from_the_search_box() {
        let (dir, mut app) = project();
        app.open(&dir.path().join("alpha.txt")).unwrap();
        app.run(Command::FindInProject);
        app.handle(Event::Key(KeyEvent::ctrl('w')));
        assert!(!screen(&app).text().contains("first line"));
        app.handle(Event::Key(KeyEvent::ctrl('q')));
        assert!(app.quit);
    }

    fn agent(state: cellix_agent::AgentState, detail: &str) -> cellix_agent::AgentEvent {
        cellix_agent::AgentEvent {
            session: "s1".into(),
            agent: "Claude Code".into(),
            state,
            detail: detail.into(),
            folder: "/w".into(),
        }
    }

    #[test]
    fn agents_panel_shows_who_needs_you() {
        use cellix_agent::AgentState::{Ended, NeedsYou, Working};
        let (_dir, mut app) = project();
        app.serve(Request::Agent(agent(NeedsYou, "Allow Bash?")));
        assert!(screen(&app).text().contains("An agent needs you"));
        app.run(Command::ShowAgents);
        let text = screen(&app).text();
        assert!(
            text.contains("Claude Code") && text.contains("needs you"),
            "{text}"
        );
        assert!(text.contains("Allow Bash?"), "{text}");
        app.serve(Request::Agent(agent(Working, "Working")));
        assert!(!screen(&app).text().contains("needs you"));
        app.serve(Request::Agent(agent(Ended, "Ended")));
        assert!(screen(&app).text().contains("No agents running"));
    }

    #[test]
    fn test_results_show_in_review() {
        let (_dir, mut app) = reviewed();
        app.serve(Request::Test(TestRun {
            command: "cargo test".into(),
            passed: false,
            output: "boom at line 3".into(),
        }));
        app.run(Command::OpenDiff);
        let grid = screen(&app);
        let (x, y) = find(&grid, "TESTS");
        assert!(grid.row_text(y + 1).contains("cargo test"));
        assert!(grid.row_text(y + 1).contains("failed"));
        app.handle(mouse(MouseKind::Left, x, y + 1));
        let text = screen(&app).text();
        assert!(text.contains("boom at line 3"), "{text}");
    }

    fn pending_plan(dir: &TempDir, app: &mut App) -> std::sync::mpsc::Receiver<Decision> {
        let path = dir.path().join("plan.md");
        fs::write(&path, "# Plan\n\nstep one\n").unwrap();
        let (reply, decision) = std::sync::mpsc::channel();
        app.serve(Request::Plan { path, reply });
        decision
    }

    #[test]
    fn approving_a_plan_answers_the_agent() {
        let (dir, mut app) = project();
        let decision = pending_plan(&dir, &mut app);
        let (x, y) = find(&screen(&app), APPROVE);
        app.handle(mouse(MouseKind::Left, x + 1, y));
        assert_eq!(decision.try_recv(), Ok(Decision::Approve(None)));
        assert!(!screen(&app).text().contains("step one"), "the plan closes");
    }

    #[test]
    fn approving_an_edited_plan_sends_the_edits() {
        let (dir, mut app) = project();
        let decision = pending_plan(&dir, &mut app);
        typed(&mut app, "My ");
        app.run(Command::ApprovePlan);
        let Ok(Decision::Approve(Some(text))) = decision.try_recv() else {
            panic!("no answer");
        };
        assert!(text.contains("# My Plan"), "{text}");
    }

    #[test]
    fn closing_a_plan_hands_it_back_to_the_agent() {
        let (dir, mut app) = project();
        let decision = pending_plan(&dir, &mut app);
        app.run(Command::CloseTab);
        assert!(matches!(
            decision.try_recv(),
            Err(std::sync::mpsc::TryRecvError::Disconnected)
        ));
    }

    #[test]
    fn sending_a_plan_back_carries_comments_and_edits() {
        let (dir, mut app) = project();
        let decision = pending_plan(&dir, &mut app);
        app.run(Command::AddComment);
        typed(&mut app, "Too big");
        enter(&mut app);
        typed(&mut app, "My ");
        let (x, y) = find(&screen(&app), SEND_BACK);
        app.handle(mouse(MouseKind::Left, x + 1, y));
        typed(&mut app, "Split it");
        enter(&mut app);
        let Ok(Decision::SendBack(text)) = decision.try_recv() else {
            panic!("no answer");
        };
        assert!(text.contains("Split it"), "{text}");
        assert!(text.contains("plan.md:1: Too big"), "{text}");
        assert!(text.contains("# My Plan"), "{text}");
    }

    #[test]
    fn comments_are_sent_to_the_agent_pane() {
        let (dir, mut app) = project();
        app.open(&dir.path().join("alpha.txt")).unwrap();
        app.run(Command::AddComment);
        typed(&mut app, "Why this");
        enter(&mut app);
        session(&mut app, "echo ready; read line; echo got:$line; sleep 9");
        app.run(Command::SendToAgent);
        shows(&mut app, "got:");
        assert!(screen(&app).text().contains("Why this"));
        assert!(app.comments.lock().unwrap().is_empty());
    }

    #[test]
    fn with_no_pane_comments_wait_for_the_next_prompt() {
        let (dir, mut app) = project();
        app.open(&dir.path().join("alpha.txt")).unwrap();
        app.run(Command::AddComment);
        typed(&mut app, "Later");
        enter(&mut app);
        app.run(Command::SendToAgent);
        assert!(screen(&app).text().contains("next prompt"));
        assert_eq!(app.comments.lock().unwrap().len(), 1);
    }

    fn ide(tool: &str, args: Value) -> (IdeCall, std::sync::mpsc::Receiver<Vec<String>>) {
        let (reply, answer) = std::sync::mpsc::channel();
        let tool = tool.to_string();
        (IdeCall { tool, args, reply }, answer)
    }

    fn diff_args(dir: &TempDir, tab: &str, new: &str) -> Value {
        let path = dir.path().join("alpha.txt");
        json!({
            "old_file_path": path,
            "new_file_path": path,
            "new_file_contents": new,
            "tab_name": tab,
        })
    }

    fn propose(app: &mut App, args: Value) -> std::sync::mpsc::Receiver<Vec<String>> {
        let (call, answer) = ide("openDiff", args);
        app.ide_call(call);
        answer
    }

    #[test]
    fn find_in_file_jumps_between_matches() {
        let (dir, mut app) = project();
        let path = dir.path().join("many.txt");
        fs::write(&path, "one two\ntwo three\nTWO\n").unwrap();
        app.open(&path).unwrap();
        app.handle(Event::Key(KeyEvent::ctrl('f')));
        typed(&mut app, "two");
        assert!(
            screen(&app).text().contains("1 of 3"),
            "{}",
            screen(&app).text()
        );
        assert_eq!(app.selected_text().as_deref(), Some("two"));
        enter(&mut app);
        assert!(screen(&app).text().contains("2 of 3"));
        enter(&mut app);
        assert_eq!(app.selected_text().as_deref(), Some("TWO"));
        let (x, y) = find(&screen(&app), "↑");
        app.handle(mouse(MouseKind::Left, x, y));
        assert!(screen(&app).text().contains("2 of 3"));
        app.handle(Event::Key(KeyEvent::plain(Key::Esc)));
        assert!(!screen(&app).text().contains("2 of 3"));
        assert_eq!(
            app.selected_text().as_deref(),
            Some("two"),
            "the match stays selected"
        );
        assert!(app.editors[0..].iter().all(|e| e.found.is_empty()));
    }

    #[test]
    fn find_starts_from_the_selection() {
        let (dir, mut app) = project();
        app.open(&dir.path().join("alpha.txt")).unwrap();
        app.run(Command::Move(cellix_input::Direction::WordRight, true));
        app.run(Command::FindInFile);
        assert!(
            screen(&app).text().contains("1 of 1"),
            "{}",
            screen(&app).text()
        );
    }

    #[test]
    fn toolbar_bolds_the_selection() {
        let (dir, mut app) = project();
        let path = dir.path().join("note.md");
        fs::write(&path, "one two\n").unwrap();
        app.open(&path).unwrap();
        app.run(Command::Move(cellix_input::Direction::WordRight, true));
        let grid = screen(&app);
        let row = app.toolbar_row().unwrap();
        let first = toolbar::layout(row)[1].x;
        assert_eq!(grid.row_text(1).chars().nth(first as usize + 1), Some('B'));
        app.handle(mouse(MouseKind::Left, first + 1, 1));
        assert_eq!(app.active_text().as_deref(), Some("**one** two\n"));
        assert_eq!(app.selected_text().as_deref(), Some("one"));
        app.handle(mouse(MouseKind::Left, first + 1, 1));
        assert_eq!(
            app.active_text().as_deref(),
            Some("one two\n"),
            "a second click undoes it"
        );
    }

    fn click_tool(app: &mut App, name: &str) {
        let row = app.toolbar_row().unwrap();
        let button = toolbar::layout(row).into_iter().find(|b| b.name == name);
        let button = button.unwrap_or_else(|| panic!("no {name}"));
        app.handle(mouse(MouseKind::Left, button.x + 1, 1));
    }

    fn page_app(text: &str) -> (TempDir, App) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("note.md");
        fs::write(&path, text).unwrap();
        let mut app = App::new(dir.path().to_path_buf(), (150, 40));
        app.open(&path).unwrap();
        (dir, app)
    }

    #[test]
    fn the_toolbar_writes_every_format() {
        for (tool, want) in [
            ("Bold", "**word** tail\n"),
            ("Italic", "*word* tail\n"),
            ("Strikethrough", "~~word~~ tail\n"),
            ("Code", "`word` tail\n"),
            ("Link", "[word](https://) tail\n"),
            ("Bullet List", "- word tail\n"),
            ("Numbered List", "1. word tail\n"),
            ("Task List", "- [ ] word tail\n"),
            ("Quote", "> word tail\n"),
        ] {
            let (_dir, mut app) = page_app("word tail\n");
            click_tool(&mut app, tool);
            assert_eq!(app.active_text().as_deref(), Some(want), "{tool}");
        }
        let (_dir, mut app) = page_app("word\n");
        click_tool(&mut app, "Divider");
        assert_eq!(app.active_text().as_deref(), Some("word\n\n---\n\n"));
    }

    #[test]
    fn the_style_menu_sets_a_paragraph() {
        let (_dir, mut app) = page_app("word\n");
        let grid = screen(&app);
        assert!(grid.row_text(1).contains("Text"));
        click_tool(&mut app, "Paragraph Style");
        let (x, y) = find(&screen(&app), "Heading 3");
        app.handle(mouse(MouseKind::Left, x, y));
        assert_eq!(app.active_text().as_deref(), Some("### word\n"));
        assert!(screen(&app).row_text(1).contains("Heading 3"));
    }

    #[test]
    fn a_narrow_pane_keeps_every_tool_in_a_menu() {
        let (_dir, mut app) = page_app("word\n");
        app.run(Command::ShowAgents);
        click_tool(&mut app, "More");
        let text = screen(&app).text();
        assert!(text.contains("Comment"), "{text}");
    }

    #[test]
    fn a_new_table_comes_from_the_size_picker() {
        let (_dir, mut app) = page_app("before\n");
        click_tool(&mut app, "Table");
        assert!(matches!(app.overlay, Overlay::TablePicker(_)));
        let Overlay::TablePicker(picker) = &app.overlay else {
            unreachable!();
        };
        let area = picker.area();
        app.handle(mouse(MouseKind::Left, area.x + 1 + 3 * 3, area.y + 1));
        let text = app.active_text().unwrap();
        assert!(text.contains("Column 4"), "{text}");
        assert_eq!(text.matches('\n').count(), 6, "{text}");
        assert_eq!(app.selected_text().as_deref(), Some("Column 1"));
    }

    fn cell_of(app: &App, text: &str) -> (u16, u16) {
        find(&screen(app), text)
    }

    #[test]
    fn tables_grow_and_shrink_from_the_menu() {
        let source = "| Alpha | Beta |\n|---|---|\n| one | two |\n";
        let pick = |app: &mut App, name: &str| {
            let (x, y) = cell_of(app, "two");
            app.handle(mouse(MouseKind::Right, x + 1, y));
            let (x, y) = find(&screen(app), name);
            app.handle(mouse(MouseKind::Left, x, y));
        };
        let (_dir, mut app) = page_app(source);
        pick(&mut app, "Insert Row Below");
        let text = app.active_text().unwrap();
        assert_eq!(text.lines().count(), 4, "{text}");
        pick(&mut app, "Insert Column Right");
        let text = app.active_text().unwrap();
        assert_eq!(
            text.lines().next().unwrap().matches('|').count(),
            4,
            "{text}"
        );
        pick(&mut app, "Align Right");
        assert!(app.active_text().unwrap().contains("--:"));
        pick(&mut app, "Delete Row");
        let text = app.active_text().unwrap();
        assert_eq!(text.lines().count(), 3, "{text}");
        assert!(!text.contains("two"), "{text}");
        let at_header = |app: &mut App, cell: &str, name: &str| {
            let (x, y) = cell_of(app, cell);
            app.handle(mouse(MouseKind::Right, x, y));
            let (x, y) = find(&screen(app), name);
            app.handle(mouse(MouseKind::Left, x, y));
        };
        at_header(&mut app, "Beta", "Delete Column");
        let text = app.active_text().unwrap();
        assert!(!text.contains("Beta"), "{text}");
        at_header(&mut app, "Alpha", "Delete Table");
        assert_eq!(app.active_text().as_deref(), Some(""));
    }

    #[test]
    fn formulas_show_typeset_and_open_on_the_caret() {
        let (_dir, mut app) = page_app("See $x^2$ here\n");
        let area = app.focused_area();
        app.set_measure(Rc::new(cellix_render::FontMeasure::new(17, 37)));
        let pictures = |app: &App| {
            screen(app)
                .paint
                .iter()
                .filter(|p| matches!(p, cellix_render::Paint::Image { .. }))
                .count()
        };
        assert_eq!(pictures(&app), 1, "typeset, with the caret away");
        let editor = app.editor_mut().unwrap();
        editor.selections.set_single(Selection::cursor(6));
        assert_eq!(pictures(&app), 0, "open, as source");
        let source = |p: &cellix_render::Paint| matches!(p, cellix_render::Paint::Text { text, .. } if text == "$x^2$");
        assert!(screen(&app).paint.iter().any(source));
        typed(&mut app, "+1");
        assert_eq!(app.active_text().as_deref(), Some("See $x+1^2$ here\n"));
        let editor = app.editor_mut().unwrap();
        editor.selections.set_single(Selection::cursor(0));
        assert_eq!(pictures(&app), 1, "and it closes on leaving");
        assert!(area.w > 0);
    }

    #[test]
    fn clicking_anywhere_on_a_block_formula_opens_it() {
        let (_dir, mut app) = page_app("$$\n\\frac{a}{b} + \\frac{\\frac{c}{d}}{e}\n$$\n\nnext\n");
        app.set_measure(Rc::new(cellix_render::FontMeasure::new(17, 37)));
        let area = app.focused_area();
        screen(&app);
        let at = app.editors[0].position_at(area, area.x as f32 + 30.0, area.y + 2);
        assert!((1..30).contains(&at), "inside the formula: {at}");
    }

    fn comment_on(app: &mut App, text: &str) {
        app.run(Command::AddComment);
        typed(app, text);
        enter(app);
    }

    #[test]
    fn comments_are_threads_beside_the_page() {
        let (dir, mut app) = page_app("# Plan\n\nThe cache goes first.\n");
        app.handle(mouse(MouseKind::Left, 40, 5));
        let (x, y) = find(&screen(&app), "The cache");
        app.handle(mouse(MouseKind::Left, x + 1, y));
        comment_on(&mut app, "Why first");
        let text = screen(&app).text();
        assert!(text.contains("Why first"), "{text}");
        assert!(
            text.contains("The cache goes first."),
            "the line it's about"
        );
        assert!(text.contains("Comments 1"), "{text}");
        let again = App::new(dir.path().to_path_buf(), (150, 40));
        assert_eq!(again.threads.len(), 1, "kept with the project");
        assert_eq!(again.threads[0].text, "Why first");
    }

    #[test]
    fn an_agent_replies_to_a_comment() {
        let (_dir, mut app) = page_app("The cache goes first.\n");
        comment_on(&mut app, "Why first");
        session(&mut app, "echo ready; read line; echo got:$line; sleep 9");
        app.dock.comments = true;
        let id = app.threads[0].id;
        app.thread_hit(ThreadHit::Ask(id));
        app.dock.comments = false;
        shows(&mut app, "cellix reply 1");
        app.dock.comments = true;
        app.serve(Request::Reply {
            id,
            author: "Claude Code".into(),
            text: "It saves a read.".into(),
        });
        let text = screen(&app).text();
        assert!(text.contains("It saves a read."), "{text}");
        assert!(text.contains("Claude Code"), "{text}");
        let (sender, answer) = std::sync::mpsc::channel();
        app.serve(Request::Threads(sender));
        assert!(answer.recv().unwrap().contains("Why first"));
    }

    #[test]
    fn a_thread_can_be_answered_and_resolved_with_its_buttons() {
        let (_dir, mut app) = page_app("line\n");
        comment_on(&mut app, "Note");
        let (x, y) = find(&screen(&app), "Reply");
        app.handle(mouse(MouseKind::Left, x, y));
        typed(&mut app, "Noted");
        enter(&mut app);
        assert_eq!(app.threads[0].replies[0].text, "Noted");
        assert_eq!(app.threads[0].replies[0].author, "You");
        let (x, y) = find(&screen(&app), "Resolve");
        app.handle(mouse(MouseKind::Left, x, y));
        assert!(app.threads[0].resolved);
        assert!(
            screen(&app)
                .text()
                .contains("Select text and press Comment")
        );
    }

    #[test]
    fn asking_with_no_agent_points_at_starting_one() {
        let (_dir, mut app) = page_app("line\n");
        comment_on(&mut app, "Note");
        let (x, y) = find(&screen(&app), "Ask agent");
        app.handle(mouse(MouseKind::Left, x, y));
        assert!(screen(&app).text().contains("Start an agent here first"));
    }

    #[test]
    fn the_math_panel_inserts_templates() {
        let (_dir, mut app) = page_app("x\n");
        app.run(Command::Move(cellix_input::Direction::LineEnd, false));
        click_tool(&mut app, "Formula");
        let Overlay::Math(panel) = &app.overlay else {
            panic!("no panel");
        };
        let area = panel.area();
        app.handle(mouse(MouseKind::Left, area.x + 2, area.y));
        assert_eq!(app.active_text().as_deref(), Some("x$\\frac{}{}$\n"));
        typed(&mut app, "a");
        assert_eq!(app.active_text().as_deref(), Some("x$\\frac{a}{}$\n"));
        app.run(Command::Move(cellix_input::Direction::Left, false));
        click_tool(&mut app, "Formula");
        enter(&mut app);
        let text = app.active_text().unwrap();
        assert_eq!(text.matches('$').count(), 2, "inside a formula: {text}");
    }

    #[test]
    fn toolbar_steps_lines_through_headings_and_lists() {
        let (dir, mut app) = project();
        let path = dir.path().join("note.md");
        fs::write(&path, "one\n").unwrap();
        app.open(&path).unwrap();
        for wanted in ["# one\n", "## one\n", "### one\n", "one\n"] {
            app.run(Command::Format(Format::Heading));
            assert_eq!(app.active_text().as_deref(), Some(wanted));
        }
        app.run(Command::Format(Format::Bullet));
        assert_eq!(app.active_text().as_deref(), Some("- one\n"));
        app.run(Command::Format(Format::Bullet));
        app.run(Command::Format(Format::Task));
        assert_eq!(app.active_text().as_deref(), Some("- [ ] one\n"));
    }

    #[test]
    fn the_wheel_reports_what_it_moved() {
        let (dir, mut app) = project();
        let path = dir.path().join("long.txt");
        fs::write(&path, "line\n".repeat(200)).unwrap();
        app.open(&path).unwrap();
        let (x, y) = (40, 10);
        app.handle(mouse(MouseKind::ScrollDown, x, y));
        app.handle(mouse(MouseKind::ScrollDown, x, y));
        let (_, rows) = app.take_scroll().unwrap();
        assert_eq!(
            rows,
            WHEEL_LINES as i32 + 1,
            "a notch, then a stream's single line"
        );
        assert_eq!(app.take_scroll(), None);
        app.handle(mouse(MouseKind::ScrollDown, x, y));
        typed(&mut app, "x");
        assert_eq!(
            app.take_scroll(),
            None,
            "an edit in between can't be animated"
        );
    }

    /// A cold rust-analyzer on a busy machine has taken over a minute to index std.
    const RUST_ANALYZER_WAIT: std::time::Duration = std::time::Duration::from_secs(200);

    /// Neovim's own config stays out of it, so this tests Cellix and not the machine.
    fn neovim_app() -> Option<(TempDir, App, PathBuf)> {
        let neovim = Neovim::start(&["--clean"]).ok()?;
        let (dir, mut app) = project();
        app.neovim = Some(neovim);
        let path = dir.path().join("note.txt");
        fs::write(&path, "one\ntwo\nthree\n").unwrap();
        app.open(&path).unwrap();
        Some((dir, app, path))
    }

    #[test]
    fn neovim_mode_edits_buffer() {
        let Some((_dir, mut app, path)) = neovim_app() else {
            return;
        };
        typed(&mut app, "dd");
        assert_eq!(app.active_text().unwrap(), "two\nthree\n");
        typed(&mut app, "A!");
        assert!(screen(&app).text().contains("INSERT"));
        key(&mut app, Key::Esc);
        assert_eq!(app.active_text().unwrap(), "two!\nthree\n");
        assert!(screen(&app).text().contains("NORMAL"));
        typed(&mut app, "u");
        assert_eq!(app.active_text().unwrap(), "two\nthree\n");
        typed(&mut app, ".");
        typed(&mut app, ":w");
        assert!(screen(&app).text().contains(":w"));
        enter(&mut app);
        assert_eq!(
            fs::read_to_string(path).unwrap(),
            app.active_text().unwrap()
        );
    }

    #[test]
    fn neovim_sees_edits_made_outside_it() {
        let Some((_dir, mut app, _path)) = neovim_app() else {
            return;
        };
        typed(&mut app, "x");
        assert_eq!(app.active_text().unwrap(), "ne\ntwo\nthree\n");
        let area = app.focused_area();
        let editor = app.editor_mut().unwrap();
        editor.prepare_edit(area);
        editor.buffer.type_text(&mut editor.selections, "Z");
        typed(&mut app, "x");
        // The cursor follows the Z, so x takes the letter after it.
        assert_eq!(app.active_text().unwrap(), "Ze\ntwo\nthree\n");
    }

    #[test]
    fn neovim_visual_mode_selects() {
        let Some((_dir, mut app, _path)) = neovim_app() else {
            return;
        };
        typed(&mut app, "vl");
        assert_eq!(app.selected_text().as_deref(), Some("on"));
        assert!(screen(&app).text().contains("VISUAL"));
    }

    #[test]
    fn middle_click_closes_a_tab() {
        let (dir, mut app) = project();
        fs::write(dir.path().join("other.txt"), "x\n").unwrap();
        app.open(&dir.path().join("alpha.txt")).unwrap();
        app.open(&dir.path().join("other.txt")).unwrap();
        let tab = |app: &App, name| screen(app).row_text(0).find(name);
        let x = tab(&app, "alpha.txt").unwrap();
        let x = screen(&app).row_text(0)[..x].chars().count() as u16;
        app.handle(mouse(MouseKind::Middle, x + 1, 0));
        assert_eq!(tab(&app, "alpha.txt"), None);
        assert!(tab(&app, "other.txt").is_some());
    }

    #[test]
    fn option_backspace_deletes_a_word() {
        let (dir, mut app) = project();
        let path = dir.path().join("words.txt");
        fs::write(&path, "one two\n").unwrap();
        app.open(&path).unwrap();
        app.run(Command::Move(cellix_input::Direction::LineEnd, false));
        app.run(Command::DeleteWordLeft);
        assert_eq!(app.active_text().unwrap(), "one \n");
        app.run(Command::Undo);
        assert_eq!(app.active_text().unwrap(), "one two\n");
        app.run(Command::DeleteToLineStart);
        assert_eq!(app.active_text().unwrap(), "\n");
    }

    #[test]
    fn typing_offers_words_from_the_file() {
        let (dir, mut app) = project();
        let path = dir.path().join("notes.txt");
        fs::write(&path, "deployment\n\n").unwrap();
        app.open(&path).unwrap();
        app.run(Command::Move(cellix_input::Direction::Bottom, false));
        typed(&mut app, "dep");
        assert!(
            screen(&app).text().contains("word"),
            "{}",
            screen(&app).text()
        );
        key(&mut app, Key::Tab);
        assert_eq!(app.active_text().unwrap(), "deployment\n\ndeployment");
    }

    #[test]
    fn large_files_get_their_colours_from_the_background() {
        let (dir, mut app) = project();
        let path = dir.path().join("large.rs");
        fs::write(&path, "fn name() {}\n".repeat(4000)).unwrap();
        app.open(&path).unwrap();
        let start = std::time::Instant::now();
        let plain = screen(&app);
        let keyword = app.theme.keyword;
        let code = app.main_area().x;
        let colored =
            |grid: &Grid| (code..grid.width()).any(|x| grid.cell(x, 2).style.fg == keyword);
        assert!(
            !colored(&plain),
            "the first frame doesn't wait for the parse"
        );
        while !colored(&screen(&app)) {
            app.poll();
            assert!(start.elapsed().as_secs() < 20, "no colours arrived");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        typed(&mut app, "x");
        assert!(colored(&screen(&app)), "an edit keeps the colours it had");
    }

    #[test]
    fn changed_lines_are_marked_in_the_margin() {
        let (dir, mut app) = reviewed();
        app.open(&dir.path().join("alpha.txt")).unwrap();
        app.handle(Event::Tick);
        for _ in 0..EXPLORER_REFRESH_TICKS {
            app.handle(Event::Tick);
        }
        assert!(screen(&app).text().contains('▎'), "{}", screen(&app).text());
    }

    #[test]
    fn a_new_file_has_no_margin_marks() {
        let (dir, mut app) = reviewed();
        fs::write(dir.path().join("new.txt"), "one\ntwo\n").unwrap();
        app.open(&dir.path().join("new.txt")).unwrap();
        assert!(
            !screen(&app).text().contains('▎'),
            "{}",
            screen(&app).text()
        );
    }

    #[test]
    fn the_status_line_shows_the_files_length() {
        let (_dir, app) = code("one\ntwo\nthree\n");
        assert!(
            screen(&app).text().contains("3 lines"),
            "{}",
            screen(&app).text()
        );
    }

    #[test]
    fn a_minimap_shows_on_pixel_tiers_with_room() {
        let (dir, mut app) = project();
        app.set_measure(Rc::new(cellix_render::FontMeasure::new(10, 20)));
        let path = dir.path().join("long.rs");
        fs::write(&path, "fn name() {}\n".repeat(300)).unwrap();
        app.open(&path).unwrap();
        let maps = |app: &App| {
            let grid = screen(app);
            let strips = |p: &&Paint| matches!(p, Paint::Strips { .. });
            grid.paint.iter().filter(strips).count()
        };
        app.handle(Event::Resize(140, 30));
        assert!(maps(&app) > 10);
        // Beside an agent, the code keeps the room.
        app.handle(Event::Resize(66, 30));
        assert_eq!(maps(&app), 0);
    }

    fn minimap_marker(app: &App) -> cellix_render::Bounds {
        let grid = screen(app);
        let code = Rect::new(grid.width() - 16, 0, 16, grid.height());
        let marker = grid.paint.iter().find_map(|p| match p {
            Paint::Panel {
                bounds,
                fill: Some(fill),
                ..
            } if *fill == app.theme.minimap_view
                && bounds.x >= code.x as i32 * cellix_render::UNIT =>
            {
                Some(*bounds)
            }
            _ => None,
        });
        marker.expect("a marker for the view")
    }

    fn long_file(app: &mut App, dir: &TempDir, name: &str, text: &str) {
        app.set_measure(Rc::new(cellix_render::FontMeasure::new(10, 20)));
        app.handle(Event::Resize(140, 30));
        let path = dir.path().join(name);
        fs::write(&path, text.repeat(600)).unwrap();
        app.open(&path).unwrap();
        screen(app);
    }

    #[test]
    fn the_minimap_marker_follows_the_view_to_the_end() {
        for (name, text) in [
            ("long.rs", "fn name() {}\n"),
            ("long.md", "A line of prose.\n\n"),
        ] {
            let (dir, mut app) = project();
            long_file(&mut app, &dir, name, text);
            let start = minimap_marker(&app);
            assert_eq!(
                start.y,
                app.focused_area().y as i32 * cellix_render::UNIT,
                "{name}"
            );
            for _ in 0..2000 {
                app.handle(mouse(MouseKind::ScrollDown, 80, 10));
            }
            let end = minimap_marker(&app);
            let pane = app.focused_area();
            let bottom = pane.bottom() as i32 * cellix_render::UNIT;
            assert!(
                end.y > start.y + 10 * cellix_render::UNIT,
                "{name}: it moved"
            );
            assert!(
                (bottom - (end.y + end.h)).abs() < cellix_render::UNIT,
                "{name}: {end:?}"
            );
        }
    }

    #[test]
    fn clicking_the_minimap_brings_that_part_of_the_file_to_the_middle() {
        let (dir, mut app) = project();
        long_file(&mut app, &dir, "long.rs", "fn name() {}\n");
        let pane = app.focused_area();
        let (x, y) = (pane.right() - 5, pane.y + pane.h / 2);
        app.handle(mouse(MouseKind::Left, x, y));
        let marker = minimap_marker(&app);
        let middle = marker.y + marker.h / 2;
        let expected = y as i32 * cellix_render::UNIT;
        assert!(
            (middle - expected).abs() < 2 * cellix_render::UNIT,
            "{marker:?} near row {y}"
        );
    }

    #[test]
    fn a_page_uses_the_whole_width() {
        let (dir, mut app) = project();
        app.handle(Event::Resize(140, 30));
        let path = dir.path().join("wide.md");
        fs::write(&path, "word ".repeat(100)).unwrap();
        app.open(&path).unwrap();
        let pane = app.focused_area();
        let grid = screen(&app);
        let (x, y) = find(&grid, "word");
        assert!(x <= pane.x + 4, "it starts at the left: {x}");
        let end = grid.row_text(y).trim_end().chars().count() as u16;
        assert!(end > pane.right() - 10, "it runs across the pane: {end}");
    }

    #[test]
    fn terraform_offers_its_keywords() {
        let (dir, mut app) = project();
        let path = dir.path().join("main.tf");
        fs::write(&path, "").unwrap();
        app.open(&path).unwrap();
        typed(&mut app, "reso");
        assert!(screen(&app).text().contains("keyword"));
        enter(&mut app);
        assert_eq!(app.active_text().unwrap(), "resource");
    }

    #[test]
    fn env_files_offer_keys_from_their_siblings() {
        let (dir, mut app) = project();
        fs::write(dir.path().join(".env.example"), "DATABASE_URL=\n").unwrap();
        let path = dir.path().join(".env");
        fs::write(&path, "").unwrap();
        app.open(&path).unwrap();
        typed(&mut app, "DAT");
        assert!(screen(&app).text().contains("DATABASE_URL"));
        key(&mut app, Key::Tab);
        assert_eq!(app.active_text().unwrap(), "DATABASE_URL");
    }

    #[test]
    fn enter_stays_a_new_line_in_prose() {
        let (dir, mut app) = project();
        let path = dir.path().join("note.md");
        fs::write(&path, "deployment\n\n").unwrap();
        app.open(&path).unwrap();
        app.run(Command::Move(cellix_input::Direction::Bottom, false));
        typed(&mut app, "dep");
        enter(&mut app);
        assert!(
            app.active_text().unwrap().contains("dep\n"),
            "{:?}",
            app.active_text()
        );
    }

    #[test]
    fn esc_and_moving_close_suggestions() {
        let (dir, mut app) = project();
        let path = dir.path().join("notes.txt");
        fs::write(&path, "deployment\n\n").unwrap();
        app.open(&path).unwrap();
        app.run(Command::Move(cellix_input::Direction::Bottom, false));
        typed(&mut app, "dep");
        key(&mut app, Key::Esc);
        assert!(!app.overlay_is_open());
        typed(&mut app, "l");
        key(&mut app, Key::Left);
        assert!(!app.overlay_is_open());
    }

    fn code(text: &str) -> (TempDir, App) {
        let (dir, mut app) = project();
        let path = dir.path().join("code.rs");
        fs::write(&path, text).unwrap();
        app.open(&path).unwrap();
        (dir, app)
    }

    #[test]
    fn brackets_close_and_step_over() {
        let (_dir, mut app) = code("");
        typed(&mut app, "(");
        assert_eq!(app.active_text().unwrap(), "()");
        typed(&mut app, ")x");
        assert_eq!(app.active_text().unwrap(), "()x");
        key(&mut app, Key::Left);
        key(&mut app, Key::Left);
        key(&mut app, Key::Backspace);
        assert_eq!(
            app.active_text().unwrap(),
            "x",
            "the empty pair goes together"
        );
    }

    #[test]
    fn backspace_removes_an_empty_pair() {
        let (_dir, mut app) = code("");
        typed(&mut app, "{");
        key(&mut app, Key::Backspace);
        assert_eq!(app.active_text().unwrap(), "");
    }

    #[test]
    fn a_selection_is_wrapped_in_quotes() {
        let (_dir, mut app) = code("word\n");
        app.run(Command::Move(cellix_input::Direction::WordRight, true));
        typed(&mut app, "\"");
        assert_eq!(app.active_text().unwrap(), "\"word\"\n");
    }

    #[test]
    fn enter_inside_braces_opens_a_block() {
        let (_dir, mut app) = code("");
        typed(&mut app, "fn a() {");
        enter(&mut app);
        typed(&mut app, "x");
        assert_eq!(app.active_text().unwrap(), "fn a() {\n    x\n}");
    }

    #[test]
    fn comments_toggle_on_selected_lines() {
        let (_dir, mut app) = code("    let a = 1;\n");
        app.run(Command::ToggleComment);
        assert_eq!(app.active_text().unwrap(), "    // let a = 1;\n");
        app.run(Command::ToggleComment);
        assert_eq!(app.active_text().unwrap(), "    let a = 1;\n");
    }

    #[test]
    fn lines_move_and_duplicate() {
        let (_dir, mut app) = code("one\ntwo\n");
        app.run(Command::MoveLineDown);
        assert_eq!(app.active_text().unwrap(), "two\none\n");
        app.run(Command::MoveLineUp);
        assert_eq!(app.active_text().unwrap(), "one\ntwo\n");
        app.run(Command::DuplicateLine);
        assert_eq!(app.active_text().unwrap(), "one\none\ntwo\n");
    }

    #[test]
    fn go_to_line_moves_the_cursor() {
        let (_dir, mut app) = code("one\ntwo\nthree\n");
        app.run(Command::GoToLine);
        typed(&mut app, "3");
        enter(&mut app);
        assert!(screen(&app).text().contains("Ln 3"));
    }

    #[test]
    fn a_proposed_change_is_shown_and_accepted() {
        let (dir, mut app) = project();
        let answer = propose(&mut app, diff_args(&dir, "t", "first line\nsecond\n"));
        let grid = screen(&app);
        assert!(grid.text().contains("PROPOSED CHANGE"), "{}", grid.text());
        let on_disk = fs::read_to_string(dir.path().join("alpha.txt")).unwrap();
        assert!(
            !on_disk.contains("second\n"),
            "nothing is written until accepted"
        );
        let (x, y) = find(&grid, "[Accept]");
        app.handle(mouse(MouseKind::Left, x + 1, y));
        assert_eq!(answer.try_recv().unwrap()[0], "FILE_SAVED");
        assert!(!screen(&app).text().contains("PROPOSED CHANGE"));
    }

    #[test]
    fn rejecting_every_hunk_rejects_the_proposal() {
        let (dir, mut app) = project();
        let answer = propose(&mut app, diff_args(&dir, "t", "first line\nsecond\n"));
        app.run(Command::RejectProposal);
        assert_eq!(answer.try_recv().unwrap(), vec!["DIFF_REJECTED", "t"]);
    }

    #[test]
    fn a_rejected_hunk_leaves_the_rest_of_the_proposal() {
        let (dir, mut app) = project();
        let old: String = (0..40).map(|i| format!("line {i}\n")).collect();
        fs::write(dir.path().join("alpha.txt"), &old).unwrap();
        let new = old
            .replace("line 2\n", "two\n")
            .replace("line 30\n", "thirty\n");
        let answer = propose(&mut app, diff_args(&dir, "t", &new));
        let review = app.review.as_mut().unwrap();
        assert_eq!(review.diff.hunks.len(), 2);
        assert!(!review.drop_proposed(0), "one hunk is still undecided");
        assert!(review.keep_proposed(0));
        app.run(Command::AcceptProposal);
        let answer = answer.try_recv().unwrap();
        assert_eq!(answer[0], "FILE_SAVED");
        assert!(
            answer[1].contains("line 2\n") && answer[1].contains("thirty"),
            "{answer:?}"
        );
    }

    #[test]
    fn a_diff_answered_in_the_agent_closes_here() {
        let (dir, mut app) = project();
        let answer = propose(&mut app, diff_args(&dir, "t", "new\n"));
        let queued = propose(&mut app, diff_args(&dir, "u", "other\n"));
        let (call, closed) = ide("close_tab", json!({"tab_name": "t"}));
        app.ide_call(call);
        assert_eq!(closed.try_recv().unwrap(), vec!["TAB_CLOSED"]);
        assert_eq!(answer.try_recv().unwrap()[0], "DIFF_REJECTED");
        assert!(
            screen(&app).text().contains("PROPOSED CHANGE"),
            "the next one shows"
        );
        app.run(Command::AcceptProposal);
        assert_eq!(queued.try_recv().unwrap()[1], "other\n");
    }

    #[test]
    fn selection_reaches_the_ide_client() {
        let (dir, mut app) = project();
        app.open(&dir.path().join("alpha.txt")).unwrap();
        app.run(Command::SelectAll);
        let selection = app.ide_selection().unwrap();
        assert_eq!(selection["selection"]["start"]["line"], 0);
        assert_eq!(selection["selection"]["isEmpty"], false);
        assert!(selection["text"].as_str().unwrap().contains("first line"));
        assert!(
            selection["fileUrl"]
                .as_str()
                .unwrap()
                .starts_with("file:///")
        );
        let (call, answer) = ide("getLatestSelection", json!({}));
        app.ide_call(call);
        assert!(answer.try_recv().unwrap()[0].contains("first line"));
    }

    #[test]
    fn open_file_selects_the_text_claude_points_at() {
        let (dir, mut app) = project();
        let path = dir.path().join("alpha.txt");
        let args = json!({"filePath": path, "startText": "first", "endText": "line"});
        let (call, answer) = ide("openFile", args);
        app.ide_call(call);
        assert!(answer.try_recv().unwrap()[0].starts_with("Opened file:"));
        assert_eq!(app.selected_text().as_deref(), Some("first line"));
    }

    #[test]
    fn claude_starts_with_cellix_on_its_path() {
        let (_dir, app) = project();
        let (env, launch) = app.agent_launch("claude");
        assert_eq!(launch[0], "claude");
        assert!(env.iter().any(|e| e.starts_with("PATH=")), "{env:?}");
        let (env, _) = app.agent_launch("codex");
        assert!(env.iter().any(|e| e == "CELLIX_AGENT=Codex"), "{env:?}");
    }

    #[test]
    fn clicking_x_closes_a_tab() {
        let (dir, mut app) = project();
        app.open(&dir.path().join("alpha.txt")).unwrap();
        app.open(&dir.path().join("beta.rs")).unwrap();
        let (x, y) = find(&screen(&app), "alpha.txt ×");
        app.handle(mouse(MouseKind::Left, x + 11, y));
        let text = screen(&app).text();
        assert!(!text.contains("alpha.txt ×"), "{text}");
        assert!(text.contains("fn main"), "{text}");
    }

    #[test]
    fn clicking_a_tab_name_does_not_close_it() {
        let (dir, mut app) = project();
        app.open(&dir.path().join("alpha.txt")).unwrap();
        let (x, y) = find(&screen(&app), "alpha.txt ×");
        app.handle(mouse(MouseKind::Left, x + 2, y));
        assert!(screen(&app).text().contains("first line"));
    }

    #[test]
    fn dirty_tab_shows_a_dot_and_asks_before_closing() {
        let (dir, mut app) = project();
        app.open(&dir.path().join("alpha.txt")).unwrap();
        typed(&mut app, "x");
        let (x, y) = find(&screen(&app), "●");
        assert!(!screen(&app).row_text(y).contains('×'));
        app.handle(mouse(MouseKind::Left, x, y));
        assert!(screen(&app).text().contains("Save changes to"));
    }

    #[test]
    fn activity_bar_opens_review_by_mouse() {
        let (dir, mut app) = project();
        git(dir.path(), &["init", "-q"]);
        app.handle(mouse(MouseKind::Left, 2, 1 + ACTIVITY_STEP * 2));
        assert!(app.review.is_some());
        app.handle(mouse(MouseKind::Left, 2, 1));
        assert!(app.review.is_none());
    }

    #[test]
    fn activity_icons_span_several_cells() {
        let (_dir, app) = project();
        let grid = screen(&app);
        let pieces = (0..ACTIVITY_WIDTH)
            .flat_map(|x| (0..3).map(move |y| (x, y)))
            .filter(|(x, y)| cellix_render::Piece::decode(&grid.cell(*x, *y).symbol).is_some())
            .count();
        assert_eq!(pieces, 6, "a 3 by 2 icon");
    }

    /// A project in a side pane the size of an agent's mod panel.
    fn narrow() -> (TempDir, App) {
        let (dir, mut app) = project();
        app.handle(Event::Resize(45, 30));
        app.open(&dir.path().join("alpha.txt")).unwrap();
        (dir, app)
    }

    #[test]
    fn a_narrow_pane_gives_the_code_nearly_all_its_width() {
        let (_dir, app) = narrow();
        let grid = screen(&app);
        let (x, _) = find(&grid, "first line");
        assert!(x <= 4, "code starts at column {x}");
        assert!(app.sidebar_area().is_none());
    }

    #[test]
    fn the_drawer_opens_over_the_code_and_closes_on_a_pick() {
        let (_dir, mut app) = narrow();
        app.handle(mouse(MouseKind::Left, 1, 0));
        let grid = screen(&app);
        assert!(grid.text().contains("alpha.txt"));
        assert!(!grid.text().contains("first line"));
        let (x, y) = find(&grid, "beta.rs");
        app.handle(mouse(MouseKind::Left, x, y));
        let text = screen(&app).text();
        assert!(text.contains("fn main"), "{text}");
        assert!(app.sidebar_area().is_none(), "the drawer closes on a pick");
    }

    #[test]
    fn the_active_tab_stays_visible_when_tabs_overflow() {
        let (dir, mut app) = project();
        app.handle(Event::Resize(40, 30));
        for name in ["one", "two", "three", "four", "five"] {
            let path = dir.path().join(format!("{name}-long-name.txt"));
            fs::write(&path, name).unwrap();
            app.open(&path).unwrap();
        }
        assert!(screen(&app).row_text(0).contains("five-long-name"));
        let (x, y) = find(&screen(&app), "×");
        app.handle(mouse(MouseKind::Left, x, y));
        assert!(!screen(&app).text().contains("five-long-name"));
    }

    #[test]
    fn narrow_status_keeps_the_cursor_and_drops_the_rest() {
        let (_dir, app) = narrow();
        let grid = screen(&app);
        let status = grid.row_text(29);
        assert!(status.contains("Ln 1, Col 1"), "{status}");
        assert!(!status.contains("Spaces"), "{status}");
    }

    #[test]
    fn narrow_panes_refuse_to_split() {
        let (_dir, mut app) = narrow();
        app.run(Command::SplitPane);
        assert!(screen(&app).text().contains("Not enough room"));
    }

    #[test]
    fn narrow_review_stacks_changes_in_one_column() {
        let (_dir, mut app) = reviewed();
        app.handle(Event::Resize(50, 30));
        app.run(Command::OpenDiff);
        let grid = screen(&app);
        assert_eq!(find(&grid, "second line").1 + 1, find(&grid, "CHANGED").1);
        let (x, y) = find(&grid, "[Reject]");
        app.handle(mouse(MouseKind::Left, x + 1, y));
        assert!(screen(&app).text().contains("No changes"));
    }

    #[test]
    fn a_split_stays_usable_after_shrinking() {
        let (dir, mut app) = project();
        app.open(&dir.path().join("alpha.txt")).unwrap();
        app.run(Command::SplitPane);
        app.open(&dir.path().join("beta.rs")).unwrap();
        app.handle(Event::Resize(45, 30));
        let text = screen(&app).text();
        assert!(text.contains("fn main"), "{text}");
        assert!(!text.contains("first line"), "{text}");
    }

    #[test]
    fn status_bar_shows_branch_and_language() {
        let (dir, mut app) = reviewed();
        app.open(&dir.path().join("beta.rs")).unwrap();
        let text = screen(&app).text();
        assert!(text.contains("master") || text.contains("main"), "{text}");
        assert!(text.contains("Rust"), "{text}");
    }

    #[test]
    fn breadcrumbs_show_the_path() {
        let (dir, mut app) = project();
        app.open(&dir.path().join("src/lib.rs")).unwrap();
        assert!(screen(&app).text().contains("src › lib.rs"));
    }

    #[test]
    fn scrollbar_click_jumps_through_the_file() {
        let (dir, mut app) = project();
        let long: String = (0..300).map(|i| format!("line {i}\n")).collect();
        fs::write(dir.path().join("long.txt"), long).unwrap();
        app.open(&dir.path().join("long.txt")).unwrap();
        let area = app.focused_area();
        app.handle(mouse(MouseKind::Left, area.right() - 1, area.bottom() - 1));
        assert!(
            screen(&app).text().contains("line 29"),
            "{}",
            screen(&app).text()
        );
    }

    #[test]
    fn indented_lines_show_guides() {
        let (dir, mut app) = project();
        fs::write(dir.path().join("i.txt"), "        deep\n").unwrap();
        app.open(&dir.path().join("i.txt")).unwrap();
        let grid = screen(&app);
        let (x, y) = find(&grid, "deep");
        assert_eq!(grid.cell(x - 8, y).symbol, "│");
        assert_eq!(grid.cell(x - 4, y).symbol, "│");
        assert_eq!(app.active_text().unwrap(), "        deep\n");
    }

    #[test]
    fn the_cursor_line_is_highlighted() {
        let (dir, mut app) = project();
        app.open(&dir.path().join("alpha.txt")).unwrap();
        let grid = screen(&app);
        let (x, y) = find(&grid, "first line");
        assert_eq!(grid.cell(x, y).style.bg, Theme::default().line);
        let (x, y) = find(&grid, "second line");
        assert_eq!(grid.cell(x, y).style.bg, Theme::default().text.bg);
    }

    #[test]
    fn settings_change_theme() {
        let (dir, _) = project();
        fs::create_dir(dir.path().join(".cellix")).unwrap();
        fs::write(
            dir.path().join(".cellix/settings.json"),
            r#"{"theme": "light"}"#,
        )
        .unwrap();
        let light = App::new(dir.path().to_path_buf(), (100, 30));
        assert_eq!(light.background(), Theme::named(Some("light")).text);
        assert_ne!(light.background(), Theme::default().text);
    }

    /// Scripted language server: answers the handshake, reports one diagnostic and
    /// jumps every definition to line 2.
    const FAKE_SERVER: &str = r#"
import json, sys
def read():
    n = 0
    while True:
        h = sys.stdin.buffer.readline()
        if not h: sys.exit()
        h = h.strip()
        if not h: break
        if h.lower().startswith(b"content-length:"): n = int(h.split(b":")[1])
    return json.loads(sys.stdin.buffer.read(n))
def send(m):
    b = json.dumps(m).encode()
    sys.stdout.buffer.write(b"Content-Length: %d\r\n\r\n" % len(b) + b)
    sys.stdout.buffer.flush()
while True:
    m = read()
    method, i = m.get("method"), m.get("id")
    if method == "initialize":
        send({"jsonrpc": "2.0", "id": i, "result": {"capabilities": {}}})
    elif method == "textDocument/didOpen":
        uri = m["params"]["textDocument"]["uri"]
        send({"jsonrpc": "2.0", "method": "textDocument/publishDiagnostics", "params": {"uri": uri,
            "diagnostics": [{"range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 1}},
            "message": "fake problem here", "severity": 1}]}})
    elif method == "textDocument/definition":
        uri = m["params"]["textDocument"]["uri"]
        send({"jsonrpc": "2.0", "id": i, "result": {"uri": uri,
            "range": {"start": {"line": 1, "character": 3}, "end": {"line": 1, "character": 4}}}})
    elif method == "textDocument/completion":
        send({"jsonrpc": "2.0", "id": i, "result": [{"label": "completed_name"}]})
    elif method == "textDocument/rename":
        uri = m["params"]["textDocument"]["uri"]
        send({"jsonrpc": "2.0", "id": i, "result": {"changes": {uri: [{"newText": m["params"]["newName"],
            "range": {"start": {"line": 0, "character": 3}, "end": {"line": 0, "character": 7}}}]}}})
    elif i is not None:
        send({"jsonrpc": "2.0", "id": i, "result": None})
"#;

    fn with_server() -> (TempDir, App) {
        let (dir, _) = project();
        fs::write(dir.path().join("server.py"), FAKE_SERVER).unwrap();
        fs::create_dir(dir.path().join(".cellix")).unwrap();
        let settings = serde_json::json!({"language_servers": {
            "Rust": ["python3", "-I", dir.path().join("server.py")]
        }});
        fs::write(
            dir.path().join(".cellix/settings.json"),
            settings.to_string(),
        )
        .unwrap();
        let mut app = App::new(dir.path().to_path_buf(), (100, 30));
        fs::write(dir.path().join("beta.rs"), "fn main() {}\nfn other() {}\n").unwrap();
        app.open(&dir.path().join("beta.rs")).unwrap();
        (dir, app)
    }

    fn wait(app: &mut App, done: impl Fn(&App) -> bool) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !done(app) {
            assert!(
                std::time::Instant::now() < deadline,
                "{}",
                screen(app).text()
            );
            app.poll();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    #[test]
    fn lsp_diagnostics_are_shown() {
        let (_dir, mut app) = with_server();
        wait(&mut app, |a| screen(a).text().contains("fake problem here"));
        let grid = screen(&app);
        let (x, y) = find(&grid, "fn main");
        assert_eq!(grid.cell(x - 2, y).style.fg, Theme::default().error);
    }

    #[test]
    fn lsp_goto_definition_jumps() {
        let (_dir, mut app) = with_server();
        app.run(Command::GoToDefinition);
        wait(&mut app, |a| screen(a).text().contains("Ln 2, Col 4"));
    }

    #[test]
    fn lsp_completion_replaces_the_word() {
        let (_dir, mut app) = with_server();
        typed(&mut app, "");
        app.run(Command::Complete);
        wait(&mut app, |a| screen(a).text().contains("completed_name"));
        enter(&mut app);
        assert!(
            app.active_text()
                .unwrap()
                .starts_with("completed_namefn main")
        );
    }

    #[test]
    fn lsp_rename_edits_the_open_buffer() {
        let (_dir, mut app) = with_server();
        app.run(Command::RenameSymbol);
        typed(&mut app, "start");
        enter(&mut app);
        wait(&mut app, |a| {
            a.active_text().unwrap().starts_with("fn start()")
        });
    }

    /// Needs rust-analyzer. Run with `harness/x test real_rust_analyzer -- --ignored`.
    #[test]
    #[ignore = "needs a real language server"]
    fn real_rust_analyzer_finds_a_definition() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("src")).unwrap();
        fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        fs::write(
            dir.path().join("src/main.rs"),
            "fn helper() -> i32 {\n    1\n}\n\nfn main() {\n    helper();\n}\n",
        )
        .unwrap();
        let mut app = App::new(dir.path().to_path_buf(), (100, 30));
        app.open(&dir.path().join("src/main.rs")).unwrap();
        for _ in 0..5 {
            app.run(Command::Move(cellix_input::Direction::Down, false));
        }
        for _ in 0..5 {
            app.run(Command::Move(cellix_input::Direction::Right, false));
        }
        assert!(screen(&app).text().contains("Ln 6, Col 6"));
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
        while !screen(&app).text().contains("Ln 1, Col 4") {
            assert!(
                std::time::Instant::now() < deadline,
                "{}",
                screen(&app).text()
            );
            app.run(Command::GoToDefinition);
            std::thread::sleep(std::time::Duration::from_millis(500));
            app.poll();
        }
    }

    /// Needs rust-analyzer. Covers what the scripted server can't: real indexing,
    /// real edits and real compiler output.
    #[test]
    #[ignore = "needs a real language server"]
    fn real_rust_analyzer_completes_renames_and_reports_errors() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("src")).unwrap();
        fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        fs::write(
            dir.path().join("src/main.rs"),
            "fn helper() -> i32 {\n    1\n}\n\nfn main() {\n    helper();\n}\n",
        )
        .unwrap();
        // Servers report canonical paths, as the real app uses from startup.
        let root = dir.path().canonicalize().unwrap();
        let path = root.join("src/main.rs");
        let mut app = App::new(root, (100, 30));
        app.open(&path).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
        let wait_for =
            |app: &mut App, done: &dyn Fn(&App) -> bool, retry: &mut dyn FnMut(&mut App)| {
                while !done(app) {
                    assert!(
                        std::time::Instant::now() < deadline,
                        "{}",
                        screen(app).text()
                    );
                    retry(app);
                    std::thread::sleep(std::time::Duration::from_millis(300));
                    app.poll();
                }
            };

        // Rename from the call site, retrying until the server has indexed the project.
        for _ in 0..5 {
            app.run(Command::Move(cellix_input::Direction::Down, false));
        }
        for _ in 0..5 {
            app.run(Command::Move(cellix_input::Direction::Right, false));
        }
        wait_for(
            &mut app,
            &|a| a.active_text().unwrap().contains("fn renamed()"),
            &mut |a| {
                if !a.rename_running() {
                    a.run(Command::RenameSymbol);
                    typed(a, "renamed");
                    enter(a);
                }
            },
        );
        let text = app.active_text().unwrap();
        assert!(text.contains("    renamed();"), "{text}");

        // Complete a prefix of the new name on a fresh line in main.
        app.run(Command::Move(cellix_input::Direction::LineEnd, false));
        app.run(Command::Newline);
        typed(&mut app, "rena");
        wait_for(
            &mut app,
            &|a| a.overlay_is_open() && screen(a).text().contains("renamed"),
            &mut |a| {
                if a.overlay_is_open() {
                    a.handle(Event::Key(KeyEvent::plain(Key::Esc)));
                }
                a.run(Command::Complete);
            },
        );
        app.handle(Event::Key(KeyEvent::plain(Key::Esc)));
        app.run(Command::Undo);
    }

    /// Needs rust-analyzer. The error is already on disk when the server starts.
    #[test]
    #[ignore = "needs a real language server"]
    fn real_rust_analyzer_reports_compiler_errors() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("src")).unwrap();
        fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        let root = dir.path().canonicalize().unwrap();
        let path = root.join("src/main.rs");
        fs::write(
            &path,
            "fn main() {\n    let x: i32 = \"text\";\n    println!(\"{x}\");\n}\n",
        )
        .unwrap();
        let mut app = App::new(root, (100, 30));
        app.open(&path).unwrap();
        app.run(Command::Move(cellix_input::Direction::Down, false));
        app.run(Command::Save);
        let deadline = std::time::Instant::now() + RUST_ANALYZER_WAIT;
        while !screen(&app).text().contains("mismatched types") {
            assert!(
                std::time::Instant::now() < deadline,
                "{}",
                screen(&app).text()
            );
            app.handle(Event::Key(KeyEvent::plain(Key::Esc)));
            app.poll();
            std::thread::sleep(std::time::Duration::from_millis(300));
        }
    }

    /// Needs rust-analyzer. The main case: an agent rewrites a file that's open.
    #[test]
    #[ignore = "needs a real language server"]
    fn real_rust_analyzer_reports_an_error_an_agent_wrote() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("src")).unwrap();
        fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        let root = dir.path().canonicalize().unwrap();
        let path = root.join("src/main.rs");
        fs::write(
            &path,
            "fn main() {\n    let x: i32 = 1;\n    println!(\"{x}\");\n}\n",
        )
        .unwrap();
        let mut app = App::new(root, (100, 30));
        app.open(&path).unwrap();
        let settle = std::time::Instant::now();
        while settle.elapsed() < std::time::Duration::from_secs(6) {
            app.poll();
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        fs::write(
            &path,
            "fn main() {\n    let x: i32 = \"text\";\n    println!(\"{x}\");\n}\n",
        )
        .unwrap();
        app.handle(Event::Tick);
        app.run(Command::Move(cellix_input::Direction::Top, false));
        app.run(Command::Move(cellix_input::Direction::Down, false));
        let deadline = std::time::Instant::now() + RUST_ANALYZER_WAIT;
        while !screen(&app).text().contains("mismatched types") {
            assert!(
                std::time::Instant::now() < deadline,
                "{}",
                screen(&app).text()
            );
            app.handle(Event::Key(KeyEvent::plain(Key::Esc)));
            app.handle(Event::Tick);
            app.poll();
            std::thread::sleep(std::time::Duration::from_millis(300));
        }
    }

    /// Needs rust-analyzer. An error typed into a clean file, as opposed to one already
    /// on disk when the server started.
    #[test]
    #[ignore = "needs a real language server"]
    fn real_rust_analyzer_reports_an_error_typed_after_opening() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("src")).unwrap();
        fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        let root = dir.path().canonicalize().unwrap();
        let path = root.join("src/main.rs");
        fs::write(
            &path,
            "fn main() {\n    let x: i32 = 1;\n    println!(\"{x}\");\n}\n",
        )
        .unwrap();
        let mut app = App::new(root, (100, 30));
        app.open(&path).unwrap();
        // Long enough for the first check to finish, so this tests a later one.
        let settle = std::time::Instant::now();
        while settle.elapsed() < std::time::Duration::from_secs(6) {
            app.poll();
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        app.run(Command::Move(cellix_input::Direction::Down, false));
        app.run(Command::Move(cellix_input::Direction::LineEnd, false));
        app.run(Command::Move(cellix_input::Direction::Left, false));
        app.run(Command::DeleteBackward);
        typed(&mut app, "\"text\"");
        app.run(Command::Save);
        let deadline = std::time::Instant::now() + RUST_ANALYZER_WAIT;
        while !screen(&app).text().contains("mismatched types") {
            assert!(
                std::time::Instant::now() < deadline,
                "{}",
                screen(&app).text()
            );
            app.handle(Event::Key(KeyEvent::plain(Key::Esc)));
            app.poll();
            std::thread::sleep(std::time::Duration::from_millis(300));
        }
    }

    #[test]
    fn a_rename_is_dropped_if_the_file_changed_while_waiting() {
        let (_dir, mut app) = with_server();
        app.run(Command::RenameSymbol);
        typed(&mut app, "start");
        enter(&mut app);
        assert!(app.rename_running());
        app.run(Command::RenameSymbol);
        typed(&mut app, "again");
        enter(&mut app);
        assert!(screen(&app).text().contains("already running"));
        typed(&mut app, "x");
        wait(&mut app, |a| !a.rename_running());
        let text = screen(&app).text();
        assert!(text.contains("rename was dropped"), "{text}");
        assert!(app.active_text().unwrap().starts_with("xfn main"));
    }

    #[test]
    fn server_errors_are_shown_in_its_own_words() {
        let (_dir, mut app) = with_server();
        app.lsp_event(LspEvent::Failed("No references found".into()));
        assert!(screen(&app).text().contains("No references found"));
    }

    #[test]
    fn a_missing_server_is_not_an_error() {
        let (dir, _) = project();
        fs::create_dir(dir.path().join(".cellix")).unwrap();
        let settings = r#"{"language_servers": {"Rust": ["cellix-no-such-server"]}}"#;
        fs::write(dir.path().join(".cellix/settings.json"), settings).unwrap();
        let mut app = App::new(dir.path().to_path_buf(), (100, 30));
        app.open(&dir.path().join("beta.rs")).unwrap();
        app.run(Command::GoToDefinition);
        assert!(screen(&app).text().contains("No language server for Rust"));
    }

    #[test]
    fn small_windows_do_not_panic() {
        let (dir, mut app) = project();
        app.open(&dir.path().join("alpha.txt")).unwrap();
        for size in [(1, 1), (5, 2), (28, 3), (29, 2), (40, 1)] {
            app.handle(Event::Resize(size.0, size.1));
            screen(&app);
        }
    }

    /// Lets a background read of git land.
    fn settle_review(app: &mut App) {
        while app.review.as_ref().is_some_and(Review::is_reading) {
            app.poll();
        }
    }

    /// The word at a place on screen, by a fast double click.
    fn double_click(app: &mut App, x: u16, y: u16) {
        for _ in 0..2 {
            app.handle(mouse(MouseKind::Left, x, y));
            app.handle(mouse(MouseKind::Up, x, y));
        }
    }

    #[test]
    fn a_double_click_selects_the_word_and_a_third_the_line() {
        let (dir, mut app) = project();
        app.open(&dir.path().join("alpha.txt")).unwrap();
        let (x, y) = find(&screen(&app), "second");
        double_click(&mut app, x + 2, y);
        assert_eq!(app.selected_text().as_deref(), Some("second"));
        app.handle(mouse(MouseKind::Left, x + 2, y));
        assert_eq!(app.selected_text().as_deref(), Some("second line\n"));
    }

    #[test]
    fn a_double_click_works_on_a_page() {
        let (_dir, mut app) = page("One **bold** word\n");
        let (x, y) = find(&screen(&app), "bold");
        double_click(&mut app, x + 1, y);
        assert_eq!(app.selected_text().as_deref(), Some("bold"));
    }

    #[test]
    fn clicks_apart_are_not_a_double_click() {
        let (dir, mut app) = project();
        app.open(&dir.path().join("alpha.txt")).unwrap();
        let (x, y) = find(&screen(&app), "second");
        app.handle(mouse(MouseKind::Left, x, y));
        app.handle(mouse(MouseKind::Left, x + 7, y));
        assert_eq!(app.selected_text(), None);
    }

    #[test]
    fn ctrl_d_gathers_the_same_word_and_typing_replaces_them_all() {
        let (_dir, mut app) = page("cat and cat and cat\n");
        let (x, y) = find(&screen(&app), "cat");
        double_click(&mut app, x + 1, y);
        for _ in 0..2 {
            app.run(Command::SelectNextMatch);
        }
        assert_eq!(app.editors[0].selections.len(), 3);
        typed(&mut app, "dog");
        assert_eq!(app.active_text().unwrap(), "dog and dog and dog\n");
    }

    #[test]
    fn right_clicking_a_page_offers_formatting() {
        let (_dir, mut app) = page("# Title\n\nsome words here\n");
        let (x, y) = find(&screen(&app), "words");
        double_click(&mut app, x + 1, y);
        app.handle(mouse(MouseKind::Right, x + 1, y));
        let text = screen(&app).text();
        for item in [
            "Show Source",
            "Cut",
            "Bold",
            "Strikethrough",
            "Code Block",
            "Insert Table",
        ] {
            assert!(text.contains(item), "{item}\n{text}");
        }
        assert_eq!(
            app.selected_text().as_deref(),
            Some("words"),
            "it stays selected"
        );
        let (x, y) = find(&screen(&app), "Strikethrough");
        app.handle(mouse(MouseKind::Left, x + 1, y));
        assert_eq!(
            app.active_text().unwrap(),
            "# Title\n\nsome ~~words~~ here\n"
        );
    }

    #[test]
    fn right_clicking_code_offers_what_the_language_can_do() {
        let (dir, mut app) = project();
        app.open(&dir.path().join("alpha.txt")).unwrap();
        let (x, y) = find(&screen(&app), "first");
        app.handle(mouse(MouseKind::Right, x, y));
        let text = screen(&app).text();
        assert!(
            text.contains("Go to Definition") && text.contains("Paste"),
            "{text}"
        );
        assert!(!text.contains("Bold"), "formatting is for pages");
        // With no language server this row can't do anything, so a click leaves it open.
        let (rx, ry) = find(&screen(&app), "Go to Definition");
        app.handle(mouse(MouseKind::Left, rx + 1, ry));
        assert!(screen(&app).text().contains("Go to Definition"));
    }

    #[test]
    fn a_menu_works_from_the_keyboard() {
        let (dir, mut app) = project();
        app.open(&dir.path().join("alpha.txt")).unwrap();
        let (x, y) = find(&screen(&app), "first");
        app.handle(mouse(MouseKind::Right, x, y));
        key(&mut app, Key::Esc);
        assert!(!app.overlay_is_open());
        app.handle(mouse(MouseKind::Right, x, y));
        key(&mut app, Key::Down);
        enter(&mut app);
        assert!(!app.overlay_is_open(), "Paste ran and the menu closed");
    }

    #[test]
    fn page_formats_change_the_source() {
        let cases = [
            (Format::PlainText, "## Title\n", "Title\n"),
            (Format::PlainText, "- [ ] task\n", "task\n"),
            (Format::CodeBlock, "code\n", "```\ncode\n```\n"),
        ];
        for (format, before, after) in cases {
            let (_dir, mut app) = page(before);
            app.run(Command::Format(format));
            assert_eq!(app.active_text().unwrap(), after, "{format:?}");
        }
        let (_dir, mut app) = page("words\n");
        app.run(Command::Format(Format::Table));
        assert_eq!(
            app.active_text().unwrap(),
            format!("words\n\n{}\n\n", cellix_doc::table::new_table(3, 3))
        );
    }

    #[test]
    fn a_tab_has_its_own_menu() {
        let (dir, mut app) = project();
        for name in ["alpha.txt", "beta.rs"] {
            app.open(&dir.path().join(name)).unwrap();
        }
        let (x, y) = find(&screen(&app), "alpha.txt");
        let row = (0..y + 3)
            .find(|r| screen(&app).row_text(*r).contains("beta.rs"))
            .unwrap();
        assert!(row <= y);
        app.handle(mouse(MouseKind::Right, app.visible_tabs()[0].1 + 2, 0));
        assert!(screen(&app).text().contains("Close Others"));
        let (x2, y2) = find(&screen(&app), "Close Others");
        app.handle(mouse(MouseKind::Left, x2 + 1, y2));
        assert_eq!(app.editors.len(), 1);
        assert!(app.editors[0].title().contains("alpha"), "{x}");
    }

    #[test]
    fn clicking_the_branch_switches_it() {
        let (dir, mut app) = reviewed();
        let run = |args: &[&str]| {
            let status = std::process::Command::new("git")
                .current_dir(dir.path())
                .args(args)
                .output()
                .unwrap();
            assert!(status.status.success());
        };
        run(&["branch", "feature"]);
        app.handle(Event::Tick);
        app.refresh_tree();
        let branch = app.branch.clone().unwrap();
        let (x, y) = find(&screen(&app), &branch);
        app.handle(mouse(MouseKind::Left, x, y));
        assert!(screen(&app).text().contains("feature"), "the list shows it");
        typed(&mut app, "feat");
        enter(&mut app);
        assert_eq!(app.branch.as_deref(), Some("feature"));
        assert!(screen(&app).text().contains("feature"));
    }

    #[test]
    fn a_new_branch_can_be_made_from_the_list() {
        let (_dir, mut app) = reviewed();
        app.run(Command::SwitchBranch);
        typed(&mut app, "New");
        enter(&mut app);
        typed(&mut app, "topic");
        enter(&mut app);
        assert_eq!(app.branch.as_deref(), Some("topic"));
    }

    #[test]
    fn a_refused_switch_says_why_in_a_dialog() {
        let (dir, mut app) = reviewed();
        let result = switch_branch(dir.path(), "no-such-branch");
        app.branch_changed("no-such-branch", result);
        assert!(screen(&app).text().contains("Couldn't switch"));
        enter(&mut app);
        assert!(!app.overlay_is_open());
    }

    #[test]
    fn the_gear_opens_settings_and_they_apply_at_once() {
        let (_dir, mut app) = project();
        let area = app.activity_area();
        app.handle(mouse(MouseKind::Left, 2, App::rail_row(4, area)));
        assert!(screen(&app).text().contains("Appearance"));
        let (x, y) = find(&screen(&app), "Light");
        app.handle(mouse(MouseKind::Left, x, y));
        assert_eq!(app.settings.theme.as_deref(), Some("light"));
        assert_eq!(
            app.theme.text.bg,
            Theme::named(Some("light")).text.bg,
            "at once"
        );
        key(&mut app, Key::Esc);
        assert!(!screen(&app).text().contains("Appearance"));
    }

    #[test]
    fn settings_reach_the_editors_and_work_by_keyboard() {
        let (dir, mut app) = project();
        app.open(&dir.path().join("alpha.txt")).unwrap();
        app.run(Command::OpenSettings);
        let area = app.sheet_area();
        let page = app.settings_page.as_ref().unwrap();
        let at = page
            .controls(area)
            .iter()
            .position(|(_, c)| *c == Control::Minimap)
            .unwrap();
        for _ in 0..=at {
            key(&mut app, Key::Tab);
        }
        key(&mut app, Key::Enter);
        assert_eq!(app.settings.minimap, Some(false));
        assert!(!app.editors[0].minimap_on);
        app.run(Command::QuickOpen);
        typed(&mut app, "beta");
        enter(&mut app);
        assert!(
            app.settings_page.is_none(),
            "opening a file leaves the page"
        );
        assert!(
            !app.editors[1].minimap_on,
            "and new files follow the setting"
        );
    }

    #[test]
    fn the_keymap_changes_without_a_restart() {
        let (_dir, mut app) = project();
        app.run(Command::OpenSettings);
        let area = app.sheet_area();
        app.settings_page.as_mut().unwrap().section = crate::settings_page::Section::Editor;
        let page = app.settings_page.as_ref().unwrap();
        let (rect, _) = page
            .controls(area)
            .into_iter()
            .find(|(_, c)| *c == Control::Keymap("vim"))
            .unwrap();
        app.handle(mouse(MouseKind::Left, rect.x + 1, rect.y));
        assert!(app.vim.is_some());
    }

    #[test]
    fn the_agents_panel_offers_something_to_do() {
        let (_dir, mut app) = project();
        app.run(Command::ShowAgents);
        let text = screen(&app).text();
        assert!(text.contains("No agents running"), "{text}");
    }

    #[test]
    fn references_land_in_the_search_panel_and_open_where_they_are() {
        let (dir, mut app) = project();
        let path = dir.path().join("beta.rs");
        app.open(&path).unwrap();
        app.show_references(vec![
            (path.clone(), 0, 3),
            (dir.path().join("src/lib.rs"), 0, 7),
        ]);
        assert!(app.sidebar_view == SidebarView::Search);
        let text = screen(&app).text();
        assert!(text.contains("2 results in 2 files"), "{text}");
        assert!(text.contains("pub fn x"), "read from disk: {text}");
    }

    #[test]
    fn no_references_says_so() {
        let (_dir, mut app) = project();
        app.show_references(Vec::new());
        assert!(screen(&app).text().contains("No references found"));
        assert!(app.sidebar_view != SidebarView::Search);
    }

    #[test]
    fn a_formatter_reads_the_file_and_gives_it_back() {
        assert_eq!(format_with(&["cat"], "a.rs", "x\n").unwrap(), "x\n");
        let missing = format_with(&["no-such-formatter"], "a.rs", "x").unwrap_err();
        assert!(missing.contains("Install no-such-formatter"), "{missing}");
        let failed = format_with(&["false"], "a.rs", "x").unwrap_err();
        assert!(failed.starts_with("false:"), "{failed}");
    }

    #[test]
    fn code_menus_offer_references_and_formatting() {
        let (dir, mut app) = project();
        app.open(&dir.path().join("beta.rs")).unwrap();
        let (x, y) = find(&screen(&app), "main");
        app.handle(mouse(MouseKind::Right, x, y));
        let text = screen(&app).text();
        assert!(text.contains("Go to References"), "{text}");
        assert!(text.contains("Format Document"), "{text}");
    }

    /// Pointing at this place changes what's painted.
    fn hover_shows(app: &mut App, x: u16, y: u16) -> bool {
        let before = screen(app).paint;
        app.handle(mouse(MouseKind::Move, x, y));
        screen(app).paint != before
    }

    #[test]
    fn everything_clickable_answers_the_pointer() {
        let (dir, mut app) = reviewed();
        app.handle(Event::Resize(140, 40));
        app.open(&dir.path().join("alpha.txt")).unwrap();
        let grid = screen(&app);
        let (x, y) = find(&grid, "beta.rs");
        assert!(hover_shows(&mut app, x, y), "a file in the tree");
        assert!(hover_shows(&mut app, 2, 1), "a rail icon");
        let status = app.size.1 - 1;
        assert!(hover_shows(&mut app, 3, status), "the status bar's branch");
        app.handle(mouse(MouseKind::Move, 40, 12));

        fs::write(dir.path().join("beta.rs"), "fn changed() {}\n").unwrap();
        app.run(Command::OpenDiff);
        settle_review(&mut app);
        let (x, y) = find(&screen(&app), "beta.rs");
        app.handle(mouse(MouseKind::Move, 40, 20));
        assert!(hover_shows(&mut app, x, y), "a row in review");
        app.run(Command::Cancel);

        app.run(Command::QuickOpen);
        let (x, y) = find(&screen(&app), "beta.rs");
        assert!(hover_shows(&mut app, x, y + 1), "a palette row");
    }

    #[test]
    fn search_results_answer_the_pointer() {
        let (_dir, mut app) = project();
        search_for(&mut app, "first");
        let (x, y) = find(&screen(&app), "alpha.txt");
        assert!(hover_shows(&mut app, x, y), "a file in the results");
    }

    #[test]
    fn icons_say_what_they_are() {
        let (_dir, mut app) = project();
        app.handle(mouse(MouseKind::Move, 2, 4));
        assert!(screen(&app).text().contains("Search"), "the rail's names");
        app.handle(mouse(MouseKind::Move, 40, 12));
        assert!(!screen(&app).text().contains("Review Changes"));
    }

    #[test]
    fn letting_go_of_the_mouse_keeps_a_message_up() {
        let (_dir, mut app) = project();
        app.say("Working…");
        app.handle(mouse(MouseKind::Up, 40, 12));
        assert!(screen(&app).text().contains("Working…"));
    }
}
