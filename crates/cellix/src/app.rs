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
    ALL_CORNERS, BOTTOM_LEFT, BOTTOM_RIGHT, Bounds, CellMeasure, Font, Grid, Icon, Measure, Rect,
    Style, TOP_LEFT, TOP_RIGHT,
};
use cellix_review::{
    Comment, append_comments, blame_line, changed_files, current_branch, head_text, restore, turns,
    worktrees,
};

use cellix_agent::hook::comments_message;
use cellix_agent::ide::{IdeCall, IdeServer};
use cellix_agent::{Decision, Request, SharedComments, TestRun};

use serde_json::{Value, json};

use crate::agents::Agents;
use crate::editor::{Editor, Marks};
use crate::explorer::Explorer;
use crate::find::Find;
use crate::neovim::{self, Neovim};
use crate::palette::{COMMANDS, Input, Item, Palette, input, popup};
use crate::rendered;
use crate::review::{Hit, Review};
use crate::search::{self, Field, Search};
use crate::settings::{Settings, default_server};
use crate::theme::Theme;

const ACTIVITY_WIDTH: u16 = 5;
/// Activity bar buttons, with what each one runs.
const ACTIVITY: [(Icon, Command); 5] = [
    (Icon::Files, Command::ToggleSidebar),
    (Icon::Search, Command::FindInProject),
    (Icon::Git, Command::OpenDiff),
    (Icon::Agents, Command::ShowAgents),
    (Icon::Settings, Command::CommandPalette),
];
/// Cells each activity icon spans.
const ACTIVITY_ICON: (u8, u8) = (3, 2);
/// Rows from one activity icon to the next.
const ACTIVITY_STEP: u16 = 4;
const APPROVE: &str = "[Approve]";
const FIND_WIDTH: u16 = 46;
const PLAN_BAR_WIDTH: u16 = 60;
/// Corners as percentages of a row's height.
const SHEET_RADIUS: u8 = 34;
const TAB_RADIUS: u8 = 22;
const PILL_RADIUS: u8 = 30;
const SUGGEST_WIDTH: u16 = 42;
/// Room kept for the path before the toolbar is left out.
const MIN_BREADCRUMB: u16 = 12;
const FIND_BUTTON: u16 = 3;
const FIND_BUTTONS: [&str; 3] = ["↑", "↓", "×"];
/// One cell either side of each mark.
const TOOL_WIDTH: u16 = 3;
const TOOLS: [(&str, Format); 7] = [
    ("B", Format::Bold),
    ("I", Format::Italic),
    ("<>", Format::Code),
    ("H", Format::Heading),
    ("•", Format::Bullet),
    ("☐", Format::Task),
    ("❝", Format::Quote),
];
const SEND_BACK: &str = "[Send back]";
const SIDEBAR_WIDTH: u16 = 28;
const SIDEBAR_MIN_WIDTH: u16 = 20;
/// Below this width the window is a side pane, as beside an agent: chrome gives way to
/// the code, and the sidebar becomes a drawer that opens over it.
const COMPACT_WIDTH: u16 = 70;
/// Columns for the menu button at the start of the tab bar, in the compact layout.
const MENU_BUTTON: u16 = 3;
const WHEEL_LINES: isize = 3;
const MENU_ITEMS: [&str; 4] = ["New File", "New Folder", "Rename", "Delete"];
const MENU_WIDTH: u16 = 16;
/// Ticks between explorer refreshes, so files an agent creates appear on their own.
const EXPLORER_REFRESH_TICKS: u32 = 4;

/// Which part of the screen a press started in, so a drag stays with it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Drag {
    None,
    Text,
    Scrollbar,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum PromptAction {
    NewFile(PathBuf),
    NewFolder(PathBuf),
    Rename(PathBuf),
    Delete(PathBuf),
    Comment(String, usize),
    RenameSymbol,
    Quit,
    CloseTab,
    Commit,
    GoToLine,
    SendBackPlan,
    /// Puts the tree back to this checkpoint, after a yes.
    UndoTurn(String),
}

struct Prompt {
    label: String,
    input: String,
    action: PromptAction,
}

struct Menu {
    x: u16,
    y: u16,
    target: PathBuf,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SidebarView {
    Explorer,
    Search,
    Agents,
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
}

/// What can open a split for an agent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SplitHost {
    Tmux,
    Zellij,
}

impl SplitHost {
    fn command(self, agent: &[String], root: &str, env: &[String]) -> Vec<String> {
        let start: &[&str] = match self {
            Self::Tmux => &["tmux", "split-window", "-h", "-c", root],
            Self::Zellij => &["zellij", "run", "-d", "right", "--cwd", root, "--"],
        };
        let mut words: Vec<String> = start.iter().map(ToString::to_string).collect();
        if !env.is_empty() {
            words.push("env".to_string());
            words.extend_from_slice(env);
        }
        words.extend_from_slice(agent);
        words
    }
}

fn agent_split(var: impl Fn(&str) -> Option<String>) -> Option<SplitHost> {
    if var("TMUX").is_some_and(|v| !v.is_empty()) {
        Some(SplitHost::Tmux)
    } else {
        var("ZELLIJ").map(|_| SplitHost::Zellij)
    }
}

/// Runs a program and waits for it, for short commands whose outcome matters.
fn run_to_end(program: &str, args: &[&str]) -> bool {
    std::process::Command::new(program)
        .args(args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// Starts a program detached from the terminal, and reports whether it could start.
fn spawn(command: &[String], args: &[&str]) -> bool {
    let Some((program, fixed)) = command.split_first() else {
        return false;
    };
    let child = std::process::Command::new(program)
        .args(fixed)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
    match child {
        Ok(mut child) => {
            // Reaped on a thread, so it never lingers as a zombie or blocks the interface.
            std::thread::spawn(move || child.wait());
            true
        }
        Err(_) => false,
    }
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
    /// Lines a wheel notch moves. Set by the terminal loop for the tier in use.
    pub wheel_lines: isize,
    suggest: Option<Suggest>,
    /// The language server's latest answer, kept to rank against each new letter.
    server_words: Vec<String>,
    /// Set when suggestions were asked for, so an empty prefix still gets them.
    suggest_asked: bool,
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
    /// The tmux pane of an agent Cellix started, with the program to reach it.
    agent_pane: Option<(String, String)>,
    tests: Vec<TestRun>,
    /// Where the pointer is, for hover highlights.
    hover: Option<(u16, u16)>,
    /// What the sidebar shows, as in VS Code's activity bar.
    sidebar_view: SidebarView,
    search: Search,
    /// In the compact layout, whether the sidebar is open over the code.
    drawer: bool,
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
            compact_width: COMPACT_WIDTH,
            attention: None,
            suggest: None,
            server_words: Vec::new(),
            suggest_asked: false,
            wheel_lines: WHEEL_LINES,
            scrolled: None,
            ide: None,
            proposal: None,
            queued_diffs: VecDeque::new(),
            told_selection: None,
            plan: None,
            agent_pane: None,
            tests: Vec::new(),
            hover: None,
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
        let chosen = self.settings.theme.as_deref();
        if chosen.is_none() || chosen == Some("auto") {
            let name = if light == Some(true) { "light" } else { "dark" };
            self.theme = Theme::named(Some(name));
        }
    }

    /// Lines the theme up with the terminal, so no seam shows around the app.
    pub fn match_terminal_background(&mut self, background: Option<(u8, u8, u8)>) {
        let chosen = self.settings.theme.as_deref();
        if let Some((r, g, b)) = background.filter(|_| chosen.is_none() || chosen == Some("auto")) {
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
        // A file that isn't in git has nothing to compare with, so it shows no marks.
        let tracked = !text.is_empty() || self.branch.is_some();
        editor.committed = tracked.then_some(text);
        editor.refresh_changes();
    }

    pub fn open(&mut self, path: &Path) -> io::Result<()> {
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
            match command.map(|c| Client::start(&c, &root)) {
                Some(Ok(client)) => {
                    self.servers.insert(language, client);
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
        let searched = self.search.poll();
        let mut events = Vec::new();
        for client in self.servers.values() {
            events.extend(client.events.try_iter());
        }
        let changed = highlighted || called || searched || !events.is_empty();
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
        // Review uses the explorer's width: side-by-side code needs it more.
        (self.show_sidebar && self.review.is_none()).then(|| {
            let width = (self.size.0 / 4).clamp(SIDEBAR_MIN_WIDTH, SIDEBAR_WIDTH);
            Rect::new(activity.right(), 0, width, activity.h)
        })
    }

    fn main_area(&self) -> Rect {
        let x = match self.sidebar_area() {
            Some(area) if !self.compact() => area.right(),
            _ => self.activity_area().right(),
        };
        // A column of ground on the right, where the sheet has room to be one.
        let edge = u16::from(!self.compact());
        Rect::new(
            x,
            0,
            self.size.0.saturating_sub(x + edge),
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
                    kind: MouseKind::Move,
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
    }

    fn paste(&mut self, text: &str) {
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
            if let Some(review) = &mut self.review {
                review.refresh();
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
        // These two work from anywhere, even with a palette or prompt open.
        let leaving = matches!(simple_keymap(key), Some(Command::Quit | Command::CloseTab));
        if !matches!(self.overlay, Overlay::None) && !leaving {
            self.overlay_key(key);
            return;
        }
        if leaving {
            self.overlay = Overlay::None;
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
            Overlay::Menu(_) | Overlay::None => {}
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
            grid.label(x + 1, area.y + 1, label, base, area.right());
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
            None => {}
        }
    }

    fn finish_prompt(&mut self, prompt: Prompt) {
        let name = prompt.input.trim();
        let result = match &prompt.action {
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
            PromptAction::UndoTurn(commit) => {
                if name.eq_ignore_ascii_case("y") {
                    let root = self.workspace.root().to_path_buf();
                    let result = restore(&root, commit);
                    self.say(match result {
                        Ok(count) => format!("Restored {count} files, and saved a point to redo"),
                        Err(e) => format!("Could not undo: {e}"),
                    });
                    if let Some(review) = &mut self.review {
                        review.refresh();
                    }
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
            PromptAction::Quit | PromptAction::CloseTab => {
                if name.eq_ignore_ascii_case("y") {
                    self.save_dirty(matches!(prompt.action, PromptAction::CloseTab));
                }
                if !name.is_empty() {
                    match prompt.action {
                        PromptAction::Quit => self.quit = true,
                        _ => self.close_tab(),
                    }
                }
                Ok(())
            }
            PromptAction::Delete(path) if name.eq_ignore_ascii_case("y") => {
                self.workspace.delete(path)
            }
            PromptAction::Delete(_) => Ok(()),
            _ if name.is_empty() => Ok(()),
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
                self.ask("Save changes first? y/n", PromptAction::Quit);
            }
            Command::Quit => self.quit = true,
            Command::CloseTab
                if self
                    .editors
                    .get(self.active())
                    .is_some_and(|e| e.buffer.is_dirty()) =>
            {
                self.ask("Save changes first? y/n", PromptAction::CloseTab);
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
            Command::ShowAgents => {
                self.review = None;
                self.search.focus = None;
                self.sidebar_view = SidebarView::Agents;
                self.show_sidebar = true;
                self.drawer = self.compact();
                self.agents.worktrees = worktrees(self.workspace.root());
            }
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
            Command::GoToDefinition => self.lsp_request(command),
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
                let text = self.clipboard.clone();
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

    /// What "open in browser" means here: the link at the cursor, or this file if it's
    /// HTML.
    fn browser_target(&self) -> Option<String> {
        let editor = self.editors.get(self.active())?;
        let rope = editor.buffer.rope();
        let head = editor.selections.primary().head;
        let line = rope.char_to_line(head);
        let text = rope.line(line).to_string();
        rendered::link_at(text.trim_end(), head - rope.line_to_char(line)).or_else(|| {
            let path = editor.buffer.path()?;
            let extension = path.extension()?.to_str()?;
            matches!(extension, "html" | "htm").then(|| path.display().to_string())
        })
    }

    fn open_in_browser(&mut self) {
        let Some(target) = self.browser_target() else {
            self.say("Nothing to open: put the cursor on a link, or open an HTML file");
            return;
        };
        let opened = match &self.settings.browser {
            Some(command) => spawn(command, &[&target]),
            None => spawn(
                &["terminal-browser".to_string(), "open".to_string()],
                &[&target, "--split", "right"],
            ),
        };
        // Without terminal-browser, the system's browser still gets the page.
        let fallback = if cfg!(target_os = "macos") {
            "open"
        } else {
            "xdg-open"
        };
        if opened || spawn(&[fallback.to_string()], &[&target]) {
            self.say(format!("Opened {target}"));
        } else {
            self.say("Could not open a browser");
        }
    }

    /// Runs the real agent beside the editor, in a split of whatever hosts Cellix. Hosting
    /// it ourselves would mean chasing every update, so this stays a launcher.
    /// The environment and command that start an agent. For Claude Code they connect it to
    /// this Cellix and load the plugin, with `cellix` on the path for the plugin's hooks.
    fn agent_launch(&self, agent: &str) -> (Vec<String>, Vec<String>) {
        let mut launch = vec![agent.to_string()];
        if agent != "claude" {
            return (Vec::new(), launch);
        }
        let mut env = Vec::new();
        if let Some(ide) = &self.ide {
            env.push(format!("CLAUDE_CODE_SSE_PORT={}", ide.port));
            env.push("ENABLE_IDE_INTEGRATION=true".to_string());
        }
        let own_folder = std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(|folder| folder.display().to_string()));
        if let (Some(folder), Ok(path)) = (own_folder, std::env::var("PATH")) {
            env.push(format!("PATH={folder}:{path}"));
        }
        if let Some(plugin) = crate::plugin::install() {
            launch.extend(["--plugin-dir".to_string(), plugin.display().to_string()]);
        }
        (env, launch)
    }

    fn open_agent(&mut self, agent: &str) {
        let root = self.workspace.root().to_string_lossy().into_owned();
        let host = agent_split(|name| std::env::var(name).ok());
        let (env, launch) = self.agent_launch(agent);
        let Some(command) = host.map(|host| host.command(&launch, &root, &env)) else {
            self.say(format!(
                "Run {agent} in a split: Cellix opens it for you inside tmux or Zellij"
            ));
            return;
        };
        if host == Some(SplitHost::Tmux) {
            // Asking tmux for the new pane's id is what lets comments be sent to it later.
            let (program, args) = command.split_first().expect("commands aren't empty");
            let output = std::process::Command::new(program)
                .args(["split-window", "-P", "-F", "#{pane_id}"])
                .args(&args[1..])
                .output();
            match output {
                Ok(done) if done.status.success() => {
                    let pane = String::from_utf8_lossy(&done.stdout).trim().to_string();
                    self.agent_pane = Some((program.clone(), pane));
                    self.say(format!("Opened {agent}"));
                }
                _ => self.say(format!("Could not start {agent}")),
            }
        } else if spawn(&command, &[]) {
            self.say(format!("Opened {agent}"));
        } else {
            self.say(format!("Could not start {agent}"));
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
        let Some((program, pane)) = self.agent_pane.clone() else {
            self.say("Comments will go with your next prompt to the agent");
            return;
        };
        // Sent as one line, because a newline in a prompt box would submit it early.
        let message = comments_message(&comments).replace('\n', " ");
        let sent = run_to_end(&program, &["send-keys", "-t", &pane, "-l", &message])
            && run_to_end(&program, &["send-keys", "-t", &pane, "Enter"]);
        if sent {
            if let Ok(mut pending) = self.comments.lock() {
                pending.clear();
            }
            self.say(format!("Sent {} comments to the agent", comments.len()));
        } else {
            self.agent_pane = None;
            self.say("The agent's pane is gone, so comments will wait for your next prompt");
        }
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
                let label = format!("Undo back to \"{}\"? y/n", turn.label);
                self.ask(&label, PromptAction::UndoTurn(turn.commit));
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
        if let Ok(mut comments) = self.comments.lock() {
            comments.push(Comment { file, line, text });
        }
    }

    fn comment_lines(&self, file: &str) -> Vec<usize> {
        self.comments.lock().map_or_else(
            |_| Vec::new(),
            |c| {
                c.iter()
                    .filter(|c| c.file == file)
                    .map(|c| c.line)
                    .collect()
            },
        )
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
        match review.hit(area, x, y) {
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
        let in_sidebar = self.sidebar_area().is_some_and(|a| a.contains(x, y));
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
                let delta = if mouse.kind == MouseKind::ScrollUp {
                    -self.wheel_lines
                } else {
                    self.wheel_lines
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
                        let text = Rect::new(area.x, area.y, area.w.saturating_sub(1), area.h);
                        let total = self.scrolled.map_or(0, |(_, r)| r) + rows;
                        self.scrolled = Some((text, total));
                    }
                }
            }
            MouseKind::Left if in_sidebar && self.sidebar_view == SidebarView::Agents => {
                let tree = self
                    .sidebar_area()
                    .and_then(|a| self.agents.worktree_at(a, y));
                if let Some(path) = tree {
                    self.review = Some(self.new_review(path));
                    self.drawer = false;
                }
            }
            MouseKind::Left if in_sidebar && self.sidebar_view == SidebarView::Search => {
                if let Some(area) = self.sidebar_area() {
                    self.click_search(area, x, y);
                }
            }
            MouseKind::Right if in_sidebar && self.sidebar_view == SidebarView::Explorer => {
                self.open_menu(x, y);
            }
            MouseKind::Right if self.pane_at(x, y).is_some() => {
                if let Some(pane) = self.pane_at(x, y) {
                    self.focus = pane;
                }
                let area = self.focused_area();
                if let Some(editor) = self.editor_mut() {
                    let at = editor.position_at(area, fine_x, y);
                    editor.selections.set_single(Selection::cursor(at));
                }
                self.prompt_comment_at_cursor();
            }
            MouseKind::Left if in_sidebar => self.click_explorer(y),
            MouseKind::Left
                if y == 1 && !in_sidebar && self.toolbar_x().is_some() && self.click_toolbar(x) => {
            }
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

    fn overlay_mouse(&mut self, mouse: MouseEvent) {
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
                let row = mouse.y.checked_sub(menu.y).map(usize::from);
                let inside = mouse.x >= menu.x && mouse.x < menu.x + MENU_WIDTH;
                if let Some(row) = row.filter(|r| inside && *r < MENU_ITEMS.len()) {
                    self.menu_choice(row, &menu.target);
                }
            }
            Overlay::Find(find) => self.click_find(find, mouse.x, mouse.y),
            other => self.overlay = other,
        }
    }

    fn open_menu(&mut self, x: u16, y: u16) {
        let target = self.sidebar_area().and_then(|area| {
            let index = self.explorer.entry_at(area, y)?;
            self.explorer.selected = Some(index);
            Some(self.explorer.entries[index].path.clone())
        });
        let target = target.unwrap_or_else(|| self.workspace.root().to_path_buf());
        self.overlay = Overlay::Menu(Menu { x, y, target });
    }

    fn menu_choice(&mut self, row: usize, target: &Path) {
        let dir = if target.is_dir() {
            target.to_path_buf()
        } else {
            target
                .parent()
                .unwrap_or(self.workspace.root())
                .to_path_buf()
        };
        let name = target
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let (label, input, action) = match row {
            0 => (
                "New file".to_string(),
                String::new(),
                PromptAction::NewFile(dir),
            ),
            1 => (
                "New folder".to_string(),
                String::new(),
                PromptAction::NewFolder(dir),
            ),
            2 => (
                "Rename".to_string(),
                name,
                PromptAction::Rename(target.to_path_buf()),
            ),
            _ => (
                format!("Delete {name}? y/n"),
                String::new(),
                PromptAction::Delete(target.to_path_buf()),
            ),
        };
        self.overlay = Overlay::Prompt(Prompt {
            label,
            input,
            action,
        });
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
        let index = y.saturating_sub(1) / ACTIVITY_STEP;
        if let Some((_, command)) = ACTIVITY.get(index as usize) {
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

    /// The close button is the last glyph: `×`, or `●` while the file has unsaved edits.
    fn tab_label(editor: &Editor) -> String {
        let close = if editor.buffer.is_dirty() {
            "●"
        } else {
            "×"
        };
        format!("  {}  {close} ", editor.title())
    }

    /// Tabs that fit, as (index, x, width), starting late enough to show the active one.
    /// Drawing and clicking share this, so a click always lands on what's shown.
    fn visible_tabs(&self) -> Vec<(usize, u16, u16)> {
        let area = self.tabs_area();
        let widths: Vec<u16> = self
            .editors
            .iter()
            .map(|e| Self::tab_label(e).chars().count() as u16 + 1)
            .collect();
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
                    self.explorer.draw(grid, area, &name, self.hover, theme);
                }
                SidebarView::Search => self.search.draw(grid, area, &*self.measure, theme),
                SidebarView::Agents => self.agents.draw(grid, area, theme),
            }
        }
        if let Some(review) = &self.review {
            let comments = self.comments.lock().map(|c| c.clone()).unwrap_or_default();
            let commented =
                |file: &str, line: usize| comments.iter().any(|c| c.file == file && c.line == line);
            review.draw(grid, self.main_area(), theme, commented);
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
        self.round_sheet(grid);
        if self.plan_in_view() {
            self.draw_plan_bar(grid);
        }
        self.draw_status(grid);
        self.draw_suggest(grid);
        self.draw_overlay(grid);
    }

    fn draw_overlay(&self, grid: &mut Grid) {
        let theme = &self.theme;
        match &self.overlay {
            Overlay::Palette(palette) => palette.draw(grid, self.hover, &*self.measure, theme),
            Overlay::Menu(menu) => self.draw_menu(grid, menu),
            Overlay::Prompt(prompt) => self.draw_prompt(grid, prompt),
            Overlay::Find(find) => self.draw_find(grid, find),
            Overlay::None => {}
        }
    }

    fn draw_menu(&self, grid: &mut Grid, menu: &Menu) {
        let height = MENU_ITEMS.len() as u16;
        popup(
            grid,
            Rect::new(menu.x, menu.y, MENU_WIDTH, height),
            &self.theme,
        );
        let base = Style {
            fg: self.theme.text.fg,
            bg: self.theme.popup,
            ..self.theme.bar
        };
        for (i, item) in MENU_ITEMS.iter().enumerate() {
            let y = menu.y + i as u16;
            let row = Rect::new(menu.x, y, MENU_WIDTH, 1);
            if self.hover.is_some_and(|h| row.contains(h.0, h.1)) {
                grid.panel(
                    Bounds::inset(row, 0.3, 0.05),
                    16,
                    Some(self.theme.pick),
                    None,
                );
            }
            let end = menu.x + MENU_WIDTH;
            grid.label_at(menu.x as f32 + 1.0, menu.x + 1, y, item, base, end, 100);
        }
    }

    /// Asks in a floating box at the top, as VS Code's quick input does.
    fn draw_prompt(&self, grid: &mut Grid, prompt: &Prompt) {
        let screen = Rect::new(0, 0, self.size.0, self.size.1);
        let top = Palette::area(screen);
        let area = Rect::new(top.x, top.y, top.w, 3);
        popup(grid, area, &self.theme);
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

    fn draw_activity(&self, grid: &mut Grid) {
        let area = self.activity_area();
        if area.w == 0 {
            return;
        }
        grid.fill(area, self.theme.activity);
        for (i, (icon, command)) in ACTIVITY.iter().enumerate() {
            let y = 1 + i as u16 * ACTIVITY_STEP;
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
                Command::ShowAgents => {
                    self.show_sidebar
                        && self.review.is_none()
                        && self.sidebar_view == SidebarView::Agents
                }
                Command::OpenDiff => self.review.is_some(),
                _ => false,
            };
            let row = Rect::new(area.x, y, area.w, ACTIVITY_ICON.1 as u16);
            let hovered = self.hover.is_some_and(|h| row.contains(h.0, h.1));
            let style = Style {
                fg: if on || hovered {
                    self.theme.activity_active
                } else {
                    self.theme.activity.fg
                },
                ..self.theme.activity
            };
            let pill = Rect::new(area.x + 1, y, u16::from(ACTIVITY_ICON.0), row.h);
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
                grid.fill(pill, style);
                let bounds = Bounds::inset(pill, 0.0, 0.0);
                grid.corners(bounds, PILL_RADIUS, self.theme.activity.bg, ALL_CORNERS);
            }
            grid.put_icon(area.x + 1, y, *icon, ACTIVITY_ICON, style);
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

    fn round_sheet(&self, grid: &mut Grid) {
        if !self.compact() {
            let which = TOP_RIGHT | BOTTOM_RIGHT | BOTTOM_LEFT;
            let bounds = Bounds::inset(self.sheet_area(), 0.0, 0.0);
            grid.corners(bounds, SHEET_RADIUS, self.theme.tab.bg, which);
        }
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
            let close_x = x + name.chars().count() as u16 + 4;
            grid.label_at(x as f32 + 1.5, x + 2, area.y, &name, style, close_x, 100);
            if close_x < area.right() {
                let dirty = editor.buffer.is_dirty();
                let shown = if dirty { "●" } else { "×" };
                if dirty && !hovered {
                    grid.put_str(close_x, area.y, shown, style, area.right());
                } else if active || hovered {
                    grid.glyph(close_x as f32, area.y, Icon::Close, style.fg, shown, style);
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
        grid.label(bar.x + 2, y, "Claude Code is waiting", base, bar.right());
        let approve = bar.right().saturating_sub(APPROVE.len() as u16 + 2);
        let back = approve.saturating_sub(SEND_BACK.len() as u16 + 1);
        grid.button(back, y, SEND_BACK, "Send back", theme.input, secondary);
        grid.button(approve, y, APPROVE, "Approve", theme.button, primary);
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
        let text = self.relative(path).replace('/', " › ");
        let style = self.theme.breadcrumb;
        grid.fill(Rect::new(main.x, 1, main.w, 1), style);
        let toolbar = self.toolbar_x();
        grid.label_at(
            main.x as f32 + 1.5,
            main.x + 2,
            1,
            &text,
            style,
            toolbar.unwrap_or(main.right()),
            90,
        );
        let Some(mut x) = toolbar else {
            return;
        };
        for (label, format) in TOOLS {
            let tool = Style {
                bold: format == Format::Bold,
                italic: format == Format::Italic,
                ..style
            };
            let button = Rect::new(x, 1, TOOL_WIDTH, 1);
            if self.hover.is_some_and(|h| button.contains(h.0, h.1)) {
                let pill = Bounds::inset(button, 0.1, 0.08);
                grid.panel(pill, 12, Some(self.theme.pick), None);
            }
            grid.put_str(x + 1, 1, label, tool, main.right());
            x += TOOL_WIDTH;
        }
    }

    /// Where the Markdown toolbar starts, if the page in view has one.
    fn toolbar_x(&self) -> Option<u16> {
        let width = TOOL_WIDTH * TOOLS.len() as u16;
        let main = self.main_area();
        let page = self.editors.get(self.active()).is_some_and(|e| e.rendered);
        let room = main.w > width + MIN_BREADCRUMB;
        (page && room && self.chrome_rows() == 2 && !self.plan_in_view())
            .then(|| main.right() - width - 1)
    }

    fn click_toolbar(&mut self, x: u16) -> bool {
        let Some(first) = self.toolbar_x() else {
            return false;
        };
        let tool = x.checked_sub(first).map(|at| at / TOOL_WIDTH);
        let Some((_, format)) = tool.and_then(|tool| TOOLS.get(tool as usize)) else {
            return false;
        };
        self.run(Command::Format(*format));
        true
    }

    fn draw_welcome(&self, grid: &mut Grid) {
        const ROWS: [(&str, &str); 4] = [
            ("Ctrl+P", "Open a file"),
            ("Ctrl+Shift+P", "Commands"),
            ("Ctrl+G", "Review changes"),
            ("Ctrl+Shift+F", "Search the project"),
        ];
        const WIDTH: u16 = 32;
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
            let line = y + 3 + i as u16;
            let second = x + 15;
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
        let hint = if self.compact() {
            "Ctrl+P open"
        } else {
            "Ctrl+P open  Ctrl+S save  Ctrl+G review  Ctrl+Q quit"
        };
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
        } else {
            hint
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
        // A message outranks the details, and a hint gives way to them.
        let kept = if message.is_empty() { 24 } else { 64 };
        let room = width.saturating_sub(left.chars().count().min(kept) as u16 + 4) as usize;
        let mut shown = right.clone();
        while shown.len() > 1 && shown.join("   ").chars().count() > room {
            shown.pop();
        }
        // Dropped from the far end, and read from the other, with the cursor last.
        shown.reverse();
        let right = shown.join("   ");
        let right_x = width.saturating_sub(right.chars().count() as u16 + 1);
        let mut x = 1;
        if !self.compact() {
            if let Some(branch) = &self.branch {
                let piece = (Icon::Git, (1, 1));
                grid.put_icon(x, y, piece.0, piece.1, bar);
                let font = Font {
                    size: 90,
                    ..Font::default()
                };
                grid.label_at(x as f32 + 1.6, x + 2, y, branch, bar, right_x, 90);
                x = (x as f32 + 3.0 + self.measure.width(branch, font)).ceil() as u16;
            }
            let problems: usize = self.diagnostics.values().map(Vec::len).sum();
            if problems > 0 {
                x = grid.label(x, y, &format!("⊗ {problems}  "), bar, right_x);
            }
        }
        grid.label_at(x as f32, x, y, left, bar, right_x.saturating_sub(1), 90);
        grid.label_right(width.saturating_sub(1), y, &right, bar, 90);
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
    fn diff_view_shows_both_versions_side_by_side() {
        let (_dir, mut app) = reviewed();
        app.run(Command::OpenDiff);
        let grid = screen(&app);
        let (_, y) = find(&grid, "second line");
        assert!(grid.row_text(y).contains("CHANGED"), "{}", grid.text());
        assert_eq!(find(&grid, "first line").1 + 1, y);
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
        let text = screen(&app).text();
        assert!(text.contains("fn turn()"), "{text}");
        assert!(
            !text.contains("alpha.txt"),
            "that changed before the turn: {text}"
        );
        app.run(Command::ReviewPreviousTurn);
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
        assert!(screen(&app).text().contains("Save changes first"));
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

    #[test]
    fn closing_a_dirty_tab_asks_first() {
        let (dir, mut app) = project();
        app.open(&dir.path().join("alpha.txt")).unwrap();
        typed(&mut app, "x");
        app.run(Command::CloseTab);
        assert!(screen(&app).text().contains("Save changes first"));
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
        assert_eq!(x, app.focused_area().x + 2);
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
    fn clicking_a_checkbox_toggles_the_source() {
        let (_dir, mut app) = page("- [ ] task\n");
        let (x, y) = find(&screen(&app), "☐ task");
        app.handle(mouse(MouseKind::Left, x, y));
        assert_eq!(app.active_text().unwrap(), "- [x] task\n");
        assert!(screen(&app).text().contains("☑ task"));
    }

    #[test]
    fn tables_edit_in_their_cells() {
        let source = "| Name | Size |\n|---|---|\n| a | b |\n";
        let (_dir, mut app) = page(source);
        let grid = screen(&app);
        assert!(!grid.text().contains('|'));
        let (x, y) = find(&grid, "a ");
        app.handle(mouse(MouseKind::Left, x + 1, y));
        typed(&mut app, "long");
        assert_eq!(
            app.active_text().unwrap(),
            "| Name | Size |\n|---|---|\n| along | b |\n"
        );
        let text = screen(&app).text();
        assert!(text.contains("along") && text.contains('│'), "{text}");
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
        let (_dir, mut app) = page("**bold**\nnext\n");
        app.run(Command::Move(cellix_input::Direction::Down, false));
        typed(&mut app, "x");
        assert_eq!(app.active_text().unwrap(), "**bold**\nxnext\n");
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

    #[test]
    fn agents_open_in_a_split_of_the_host() {
        let tmux = |k: &str| (k == "TMUX").then(|| "/tmp/t,1,0".to_string());
        assert_eq!(agent_split(tmux), Some(SplitHost::Tmux));
        assert_eq!(
            SplitHost::Tmux.command(&["claude".to_string()], "/w", &[]),
            ["tmux", "split-window", "-h", "-c", "/w", "claude"]
        );
        let zellij = |k: &str| (k == "ZELLIJ").then(|| "0".to_string());
        assert_eq!(agent_split(zellij), Some(SplitHost::Zellij));
        assert_eq!(agent_split(|_| None), None);
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
        assert!(screen(&app).text().contains("None running"));
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
        let log = dir.path().join("sent.txt");
        let script = dir.path().join("fake-tmux");
        fs::write(
            &script,
            format!("#!/bin/sh\necho \"$@\" >> {}\n", log.display()),
        )
        .unwrap();
        fs::set_permissions(&script, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        app.agent_pane = Some((script.display().to_string(), "%7".into()));
        app.open(&dir.path().join("alpha.txt")).unwrap();
        app.run(Command::AddComment);
        typed(&mut app, "Why this");
        enter(&mut app);
        app.run(Command::SendToAgent);
        let sent = fs::read_to_string(&log).unwrap();
        assert!(sent.contains("-t %7 -l"), "{sent}");
        assert!(sent.contains("alpha.txt:1: Why this"), "{sent}");
        assert!(sent.trim_end().ends_with("Enter"), "{sent}");
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
        let first = app.toolbar_x().unwrap();
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
        let (area, rows) = app.take_scroll().unwrap();
        assert_eq!(rows, WHEEL_LINES as i32 * 2);
        assert_eq!(area, {
            let pane = app.focused_area();
            Rect::new(pane.x, pane.y, pane.w - 1, pane.h)
        });
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
    fn claude_starts_with_the_port_to_connect_to() {
        let agent = ["claude".to_string()];
        let command = SplitHost::Tmux.command(&agent, "/w", &["P=1".to_string()]);
        assert_eq!(command[5..], ["env", "P=1", "claude"]);
    }

    #[test]
    fn clicking_x_closes_a_tab() {
        let (dir, mut app) = project();
        app.open(&dir.path().join("alpha.txt")).unwrap();
        app.open(&dir.path().join("beta.rs")).unwrap();
        let (x, y) = find(&screen(&app), "alpha.txt  ×");
        app.handle(mouse(MouseKind::Left, x + 11, y));
        let text = screen(&app).text();
        assert!(!text.contains("alpha.txt  ×"), "{text}");
        assert!(text.contains("fn main"), "{text}");
    }

    #[test]
    fn clicking_a_tab_name_does_not_close_it() {
        let (dir, mut app) = project();
        app.open(&dir.path().join("alpha.txt")).unwrap();
        let (x, y) = find(&screen(&app), "alpha.txt  ×");
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
        assert!(screen(&app).text().contains("Save changes first"));
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
            .flat_map(|x| (0..5).map(move |y| (x, y)))
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
        assert!(status.contains("Ctrl+P open"), "{status}");
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
        assert!(screen(&app).text().contains("rename was dropped"));
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
}
