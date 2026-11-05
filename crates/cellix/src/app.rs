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
    Comment, Reply, Thread, YOU, append_comments, blame_line, branches, changed_files,
    create_branch, current_branch, head_text, restore, switch_branch, turns, worktrees,
};

use cellix_agent::hook::comments_message;
use cellix_agent::ide::{IdeCall, IdeServer};
use cellix_agent::{Decision, Request, SharedComments, TestRun};
use cellix_doc::latex::ENGINES;
use cellix_doc::notebook::{Marker, Notebook};
use cellix_doc::table::{Align, Edit as TableEdit};

use serde_json::{Value, json};

use crate::agents::{Agents, Hit as AgentHit};
use crate::dock::{BROWSER, CellPixels, Dock, Front, Session, TabHit};
use crate::editor::{Editor, Marks};
use crate::explorer::Explorer;
use crate::find::Find;
use crate::kernel::{self, Event as KernelEvent, Kernel};
use crate::menu::{Action as MenuAction, Item as MenuItem, Menu};
use crate::neovim::{self, Neovim};
use crate::outline::Outline;
use crate::palette::{COMMANDS, Input, Item, Palette, input, popup, raised};
use crate::panels::{MathPanel, SYMBOLS, TablePicker};
use crate::pdfview::{Hit as PdfHit, PdfView};
use crate::rendered::{self, Dialect, Target};
use crate::review::{Hit, Review};
use crate::search::{self, Field, Search};
use crate::settings::{SERVERS, Settings, default_server, formatter};
use crate::settings_page::{Control, Found, SettingsPage, Values};
use crate::snippets::Snippet;
use crate::source::{Hit as SourceHit, Source as SourceControl};
use crate::theme::Theme;
use crate::threads::{self, Draft, Hit as ThreadHit};
use crate::toolbar::{self, Bar, Tool};

const ACTIVITY_WIDTH: u16 = 4;
/// The buttons that stand by a selection, and the one on a picture.
const COMMENT_WIDTH: u16 = 10;
const RUN_WIDTH: u16 = 6;
/// Columns kept clear of the end of a notebook's bar, for its export button.
const KERNEL_STATUS_END: u16 = 6;
/// The row the Comment button stands on over a PDF, clear of its zoom buttons.
const VIEW_COMMENT_ROW: u16 = 2;
const ASK_WIDTH: u16 = 11;
/// The list an empty window shows, its width, the rows above it, and the height it needs.
const WELCOME_WIDTH: u16 = 44;
const WELCOME_LEAD_ROWS: u16 = 4;
const WELCOME_MIN_HEIGHT: u16 = 16;
/// Activity bar buttons, with what each one runs.
const ACTIVITY: [(Icon, Command); 5] = [
    (Icon::Files, Command::ToggleSidebar),
    (Icon::Search, Command::FindInProject),
    (Icon::Outline, Command::ShowOutline),
    (Icon::Git, Command::ShowSourceControl),
    (Icon::Settings, Command::OpenSettings),
];
/// Cells each activity icon spans.
const ACTIVITY_ICON: (u8, u8) = (2, 1);
/// Rows from one activity icon to the next: room between them, in VS Code's proportions.
/// Cells can't place half a row, so there it's two.
const ACTIVITY_STEP: f32 = 1.5;
const ACTIVITY_STEP_CELLS: f32 = 2.0;
/// What's in reach at the top right, as in a desktop app, whatever the pane is showing.
const PANE_ACTIONS: [(Icon, Command, &str); 5] = [
    (Icon::Comment, Command::ShowComments, "Comments"),
    (Icon::Terminal, Command::ToggleTerminal, "Terminal (Ctrl+J)"),
    (Icon::Diff, Command::OpenDiff, "Review Changes (Ctrl+G)"),
    (Icon::Globe, Command::ShowBrowser, "Browser"),
    (Icon::Agents, Command::ListAgents, "Agents"),
];
/// Columns each of those takes, and rows tall each icon is.
const ACTION_WIDTH: u16 = 4;
const ACTION_ICON: f32 = 0.62;
/// Columns kept clear on each side of the whole bar.
const ACTION_MARGIN: u16 = 1;
/// Rows tall a rail icon is on the pixel tiers. Text cells keep their block art.
/// Larger than the top right icons' box, since line icons fill less of theirs, so the two
/// sets look the same size.
const RAIL_ICON: f32 = 0.91;
/// The tile behind the rail icon that's showing, in columns and rows: square, a full row, so
/// the large rail icons keep air inside it. Pixels only.
const RAIL_PILL: (f32, f32) = (2.15, 1.0);
/// The same tile for the smaller icons at the top right.
const ACTION_PILL: (f32, f32) = (1.95, 0.89);
/// Where the pane's own close button stands, which they keep clear of.
const PANE_CLOSE: u16 = 3;
/// Columns kept for tabs beside them, in the pane and over the file. A window with less goes
/// without them, sooner than move them somewhere else.
const PANE_TABS_ROOM: u16 = 14;
const FILE_TABS_ROOM: u16 = 20;
/// Rows the window needs before the pane can stand under the file, and its share of them.
const STACK_HEIGHT: u16 = 16;
const STACK_SHARE: u16 = 45;
/// Rows each side keeps, however far the line between them is dragged.
const STACK_LEAST: u16 = 6;
const APPROVE: &str = "[Approve]";
const FIND_WIDTH: u16 = 58;
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
const BADGE_SIZE: u8 = 56;
const SUGGEST_WIDTH: u16 = 42;
const FIND_BUTTON: u16 = 2;
const FIND_BUTTONS: [Icon; 3] = [Icon::ArrowUp, Icon::ArrowDown, Icon::Close];
/// Columns for the chevron that shows the replace row.
const FIND_CHEVRON: u16 = 2;
/// Columns kept for the Find bar's count, "12 of 340" or "No results".
const FIND_COUNT: u16 = 11;
/// The three search switches, three columns each.
const SWITCHES_WIDTH: u16 = 9;
const SEND_BACK: &str = "[Send back]";
/// The card that offers a language's support.
const OFFER_WIDTH: u16 = 46;
const OFFER_HEIGHT: u16 = 4;
const OFFER_BUTTON_ROW: u16 = 3;
const OFFER_BUTTONS: [&str; 3] = ["Install", "Not now", "Never"];
/// Columns at the end of the pane's title that close it.
const PANE_CLOSE_REACH: u16 = 3;
/// A tooltip's height in rows, and the size of its text.
const TOOLTIP_HEIGHT: f32 = 1.22;
const TOOLTIP_SIZE: u8 = 92;
/// What each button is called, in order, since they're only icons and marks.
const FIND_NAMES: [&str; 6] = [
    "Previous Match (Shift+Enter)",
    "Next Match (Enter)",
    "Close (Escape)",
    "Toggle Replace (Ctrl+R)",
    "Replace (Enter)",
    "Replace All (Alt+Enter)",
];
const RAIL_NAMES: [&str; 5] = ["Files", "Search", "Outline", "Source Control", "Settings"];
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
/// How close together presses must be to count as one double or triple click.
const MULTI_CLICK: std::time::Duration = std::time::Duration::from_millis(400);
/// Ticks between explorer refreshes, so files an agent creates appear on their own.
const EXPLORER_REFRESH_TICKS: u32 = 4;

/// Which part of the screen a press started in, so a drag stays with it.
#[derive(Clone, Copy, PartialEq)]
enum Drag {
    None,
    Text,
    Scrollbar,
    /// The side pane's edge.
    Dock,
    /// A selection in a session.
    Session,
    /// An image's edge, on its source line.
    Image(usize),
    /// A press on the Preview's pages, which pans once it moves.
    Preview,
    /// One of the Preview's scrollbars.
    PreviewBar(PdfHit),
}

/// Where a press in the Preview started, and where the pages stood then. It pans only once
/// it has moved this far, so a click still goes to the source.
#[derive(Clone, Copy)]
struct PreviewPress {
    x: u16,
    y: u16,
    fine_x: f32,
    left: f32,
    top: f32,
    panning: bool,
}

/// How far a press must move before it pans, in columns. Rows are whole, so any row counts.
const PAN_START: f32 = 0.75;
/// How long the hint on moving a wide page stays, unless a pan sends it away first.
const PAN_HINT: std::time::Duration = std::time::Duration::from_secs(5);
const PAN_HINT_TEXT: &str = "Drag to move around · Shift+scroll goes sideways";

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
    /// Turn down a hunk in Review, with the reason going to the agent.
    RejectHunk(usize),
    /// Turn down the change Claude Code proposed, with the reason going to it.
    RejectProposal,
    NewBranch,
    Image,
    Link,
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
    Outline,
    Source,
}

/// Suggestions showing at the caret while typing.
struct Suggest {
    items: Vec<Candidate>,
    selected: usize,
    /// Where an accepted suggestion starts replacing, when that isn't the word's start.
    from: Option<usize>,
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

/// What takes the window while the rest waits, until it's restored. Kept while running only.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Maximized {
    /// The side pane, right of the activity rail.
    Pane,
    /// The editor in front, without the sidebar, the other panes or the side pane.
    Editor,
}

/// What shows, compared before and after something happens, so a maximized part gives the
/// window back when what's asked for is hidden behind it.
#[derive(Clone, PartialEq)]
struct InView {
    dock: (bool, Front),
    sidebar: (bool, SidebarView, bool),
    editor: (usize, usize),
    panes: usize,
    review: bool,
    settings: bool,
}

/// A tab being dragged.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Dragged {
    Editor(usize),
    View(Front),
    Session(usize),
}

/// Where a dragged tab would land if it were let go now.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Landing {
    /// Among the editor's tabs, before the one at this index.
    Tabs(usize),
    /// Beside a pane, on its right when true, as a new pane.
    Split(usize, bool),
    /// In another pane, in place of what it shows.
    Show(usize),
    /// Among the side pane's tabs.
    PaneTabs(usize),
    /// The side pane under the file, or back on the right.
    Under,
    Right,
    Nowhere,
}

/// A press on a tab, and where the pointer has taken it. It's a drag once it moves far enough,
/// so a click and a double-click stay what they were.
#[derive(Clone, Copy, Debug)]
struct TabDrag {
    what: Dragged,
    from: (u16, u16),
    at: (u16, u16),
    fine_x: f32,
    moving: bool,
}

/// Columns a press on a tab moves before it's a drag.
const DRAG_START: u16 = 2;
/// The narrowest a pane may be split to.
const LEAST_PANE: u16 = 20;
/// The wash where something would land: the accent at about 14%.
const DROP_WASH: u8 = 36;

/// What the tooltip on the pane's maximize button says.
const MAXIMIZE_TIP: &str = "Maximize or double-click the tab";
const RESTORE_TIP: &str = "Restore or double-click the tab";

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
/// A row of the Problems tab.
enum ProblemRow {
    File(String),
    Item(PathBuf, Diagnostic),
}

#[derive(Clone)]
enum Choice {
    Cancel,
    Save(Leaving),
    Discard(Leaving),
    Delete(PathBuf),
    /// Save over a change made on disk.
    Overwrite,
    /// Take the change made on disk, and lose the unsaved edits.
    Reload,
    /// Quit, stopping what's running in the side pane.
    Stop,
    /// Quit, leaving it running for next time.
    KeepRunning,
    /// Throw away a file's changes, deleting it if git never knew it.
    DiscardFile(PathBuf, bool),
    /// Export to PDF, with suggestions accepted or as they stand.
    ExportPdf(bool),
    CloseSession(usize),
    Undo(String),
}

/// The engines behind a keymap. `vim` runs the real Neovim when it's installed, since the
/// built-in mode is only a subset, and falls back to that subset when it isn't.
fn keymap_engines(name: Option<&str>) -> (Option<Vim>, Option<Neovim>) {
    // Tests of the built-in mode mustn't depend on whether this machine has Neovim.
    engines_for(name, !cfg!(test))
}

fn engines_for(name: Option<&str>, vim_may_use_neovim: bool) -> (Option<Vim>, Option<Neovim>) {
    let wanted = match name {
        Some("neovim") => true,
        Some("vim") => vim_may_use_neovim,
        _ => false,
    };
    let neovim = wanted.then(|| Neovim::start(&[]).ok()).flatten();
    let vim = (name == Some("vim") && neovim.is_none()).then(Vim::default);
    (vim, neovim)
}

/// Asked before a change is turned down: Enter turns it down, Esc keeps it.
const REJECT_ASK: &str = "Why reject it? Enter rejects, Esc keeps it";

/// The browser Cellix runs in the pane, and its own installer, from its releases page, for
/// macOS and Linux alike.
const BROWSER_TOOL: &str = "terminal-browser";
const BROWSER_INSTALL: &str = "curl -fsSL https://terminal-browser.sh/install | bash";

/// Whether terminal-browser is on the path. Tests never offer it, whatever this machine has.
fn browser_installed() -> bool {
    let var = |name: &str| std::env::var(name).ok();
    cfg!(test) || crate::integrations::on_path(BROWSER_TOOL, var)
}

/// What Ask Agent puts in Claude Code's prompt: the file as a mention, so the agent reads
/// it, the line and the words selected, then room for the question.
fn ask_in_prompt(file: &str, line: usize, quote: &str) -> String {
    const LONGEST: usize = 200;
    let mut short: String = quote.chars().take(LONGEST).collect();
    if quote.chars().count() > LONGEST {
        short.push('…');
    }
    format!("@{file} line {line}, \"{short}\": ")
}

/// What Cellix says once Ask Agent has put the selection in Claude Code's prompt.
fn asked_in_prompt(quote: &str, line: usize) -> String {
    const LONGEST: usize = 24;
    let what = if quote.is_empty() {
        format!("Line {line}")
    } else if quote.chars().count() > LONGEST {
        let short: String = quote.chars().take(LONGEST).collect();
        format!("\"{}…\"", short.trim_end())
    } else {
        format!("\"{quote}\"")
    };
    format!("{what} is in Claude Code's prompt. Press Esc to ask there.")
}

/// What an agent is asked when the reviewer presses Check my review.
fn review_prompt(file: &str, mine: &[&Thread]) -> String {
    let mut text = format!("I'm reviewing {file} in Cellix. Read it, then help with my review.\n");
    if mine.is_empty() {
        text.push_str("I haven't commented yet.\n");
    } else {
        text.push_str("For each of my comments, answer with `cellix reply <id> \"Agree (<kind>): <why>\"` or `cellix reply <id> \"Disagree (<kind>): <why>\"`, where <kind> is Clarity, Correctness, Evidence, Structure or Reproducibility:\n");
        for thread in mine {
            text.push_str(&format!(
                "- {} line {}, on \"{}\": {}\n",
                thread.id, thread.line, thread.quote, thread.text
            ));
        }
    }
    text.push_str(&format!(
        "Then suggest up to five comments a careful reviewer would add that I haven't, favouring kinds I haven't covered, each with `cellix suggest {file} <line> <kind> \"<exact words it's about>\" \"<comment>\"`. Don't edit the document."
    ));
    text
}

/// Whether two LaTeX files build as one paper, as their files on disk say.
fn same_paper(a: &Path, b: &Path) -> bool {
    let root = |path: &Path| {
        let tex = path.extension().is_some_and(|e| e == "tex");
        let text = std::fs::read_to_string(path).ok().filter(|_| tex)?;
        Some(cellix_doc::latex::root_of(path, &text))
    };
    root(a).is_some_and(|r| Some(r) == root(b))
}

/// A plan's earlier versions, `Plan v1.md` and on beside `Plan.md`, oldest first.
fn plan_versions(path: &Path) -> Vec<PathBuf> {
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut versions = Vec::new();
    for version in 1.. {
        let earlier = path.with_file_name(format!("{stem} v{version}.md"));
        if !earlier.exists() {
            break;
        }
        versions.push(earlier);
    }
    versions
}

/// Where an export builds and where its PDF ends up: built beside the document, so pictures
/// resolve, under a name of its own whose build files all go afterwards. Not a hidden name: TeX
/// Live refuses to write a file whose name starts with a dot (`openout_any = p`), so pdflatex
/// and latexmk couldn't write the log, and the export failed everywhere but tectonic.
fn export_names(path: &Path) -> (PathBuf, PathBuf) {
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let source = path.with_file_name(format!("{stem}.cellix-export.tex"));
    (source, path.with_extension("pdf"))
}

/// What the periodic look at the disk found.
struct DiskLook {
    entries: Vec<cellix_core::Entry>,
    branch: Option<String>,
    changes: Vec<cellix_review::Change>,
    /// Each open file as it was last committed.
    committed: Vec<(PathBuf, String)>,
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
    let mut shell = std::process::Command::new("sh");
    shell
        .args(["-c", command])
        .stdin(std::process::Stdio::null());
    // In a session of its own, with no terminal. An installer that asks on /dev/tty, as
    // terminal-browser's `setup` does, would otherwise ask on Cellix's own screen, where
    // Enter arrives as \r and never answers it, and the install would never end. With no
    // terminal it takes its default answer, and sudo fails instead of waiting for a password.
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // SAFETY: setsid is async-signal-safe, and this runs in the child before it execs.
        unsafe {
            shell.pre_exec(|| {
                if libc::setsid() < 0 {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }
    let done = shell.output();
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
/// Where pictures added to a document go, beside it.
const IMAGE_FOLDER: &str = "images";

/// Saves the clipboard's picture to the file named last, on this platform.
fn clipboard_image_command() -> Vec<String> {
    let script = if cfg!(target_os = "macos") {
        "osascript -e 'on run {path}' -e 'set out to open for access POSIX file path with write permission' \
         -e 'write (the clipboard as «class PNGf») to out' -e 'close access out' -e 'end run' \"$1\""
    } else {
        "wl-paste --type image/png > \"$1\" 2>/dev/null || xclip -selection clipboard -t image/png -o > \"$1\""
    };
    vec![
        "sh".to_string(),
        "-c".to_string(),
        script.to_string(),
        "clipboard".to_string(),
    ]
}

/// A document with its SVG images swapped for pictures LaTeX can read, made beside it.
fn svg_as_pictures(text: &str, folder: &Path, made: &mut Vec<PathBuf>) -> String {
    let lines = text.split('\n').map(|line| {
        let image = cellix_doc::figure::image_line(line).filter(|i| i.path.ends_with(".svg"));
        let picture = image.and_then(|image| {
            let source = std::fs::read_to_string(folder.join(image.path)).ok()?;
            let name = format!(".diagram-{}.png", made.len());
            write_svg(&source, &folder.join(&name))?;
            made.push(folder.join(&name));
            cellix_doc::figure::set_path(line, &name)
        });
        picture.unwrap_or_else(|| line.to_string())
    });
    lines.collect::<Vec<_>>().join("\n")
}

/// What's been built for a block of LaTeX, as a picture a page can hold in itself.
fn built_as_html(folder: &Path, body: &str) -> Option<String> {
    use base64::Engine;
    const WIDTH: u32 = 1200;
    const SCALE: f32 = 2.5;
    let snippet = Snippet::new(cellix_doc::latex::standalone(&[], body));
    let crate::snippets::State::Ready(path) = crate::snippets::state(folder, snippet.key) else {
        return None;
    };
    let document = cellix_doc::pdf::Document::open(std::fs::read(path).ok()?).ok()?;
    let page = document.render_at(0, SCALE, WIDTH)?;
    let image = image::RgbaImage::from_raw(page.width, page.height, page.rgba.as_ref().clone())?;
    let mut bytes = std::io::Cursor::new(Vec::new());
    image.write_to(&mut bytes, image::ImageFormat::Png).ok()?;
    let encoded = base64::engine::general_purpose::STANDARD.encode(bytes.into_inner());
    Some(format!("<img src=\"data:image/png;base64,{encoded}\">"))
}

/// Draws a diagram for print, on white and sharp enough for paper.
fn write_diagram(kind: cellix_doc::diagram::Kind, source: &str, file: &Path) -> Option<()> {
    let svg = cellix_doc::diagram::svg(kind, source, false).ok()?;
    write_svg(&svg, file)
}

/// Draws SVG to a picture for print.
fn write_svg(svg: &str, file: &Path) -> Option<()> {
    const PRINT_SCALE: f32 = 3.0;
    const PRINT_WIDTH: u32 = 2400;
    let raster = cellix_doc::diagram::rasterize(svg, PRINT_SCALE, PRINT_WIDTH).ok()?;
    let image = image::RgbaImage::from_raw(raster.width, raster.height, raster.rgba)?;
    image.save(file).ok()
}

/// Whether a file is a picture a document can show.
fn is_image(path: &Path) -> bool {
    let extension = path.extension().and_then(|e| e.to_str());
    matches!(
        extension.map(str::to_ascii_lowercase).as_deref(),
        Some("png" | "jpg" | "jpeg" | "gif" | "webp" | "svg")
    )
}

/// The picture a pasted path names. Terminals quote a dropped path, or escape its spaces.
fn dropped_image(text: &str) -> Option<PathBuf> {
    let text = text.trim();
    let unquoted = text
        .strip_prefix('\'')
        .and_then(|t| t.strip_suffix('\''))
        .or_else(|| text.strip_prefix('"').and_then(|t| t.strip_suffix('"')));
    let path = match unquoted {
        Some(path) => path.to_string(),
        None => text.replace("\\ ", " "),
    };
    let path = PathBuf::from(path);
    (path.is_absolute() && is_image(&path) && path.is_file()).then_some(path)
}

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

/// How a build of a paper went.
struct Build {
    file: PathBuf,
    worked: bool,
    said: String,
    took: std::time::Duration,
    /// BibTeX or Biber failing, and why: the PDF is made all the same, without references.
    bibliography: Option<(cellix_doc::latex::Bibliography, String)>,
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
    wheel: crate::wheel::Wheel,
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
    /// The user agreed to stop what's running, for this quit.
    stop_confirmed: bool,
    /// A build under way.
    build: Option<std::sync::mpsc::Receiver<Build>>,
    /// A file made only for a build, to remove once it's done.
    remove_after_build: Vec<PathBuf>,
    /// The comments on this project, kept with it.
    threads: Vec<Thread>,
    /// The editor and revision comments were last placed for.
    threads_seen: Option<(usize, u64)>,
    /// The file couldn't be read, so saving would lose what's in it.
    threads_broken: bool,
    tests: Vec<TestRun>,
    /// Where the pointer is, for hover highlights.
    hover: Option<(u16, u16)>,
    /// The side pane or the editor taking the window.
    maximized: Option<Maximized>,
    /// A tab pressed, and perhaps being dragged.
    tab_drag: Option<TabDrag>,
    /// A press in the Preview, until it's let go.
    preview_press: Option<PreviewPress>,
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
    outline: Outline,
    source: SourceControl,
    /// Ticks since Source Control last read the repository.
    source_age: u32,
    /// The periodic look at the tree and git, on its way.
    disk_look: Option<std::sync::mpsc::Receiver<DiskLook>>,
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
    /// What saves the clipboard's picture to the file named last.
    clipboard_image: Vec<String>,
    /// The answer to quitting was to leave things running.
    keep_running: bool,
    /// Whether how the two sides work together has been said yet.
    pane_explained: bool,
    /// A comment waiting for an agent to be started.
    pending_ask: Option<u32>,
    /// A `Z` typed in Neovim's normal mode, waiting to see if it's `ZZ` or `ZQ`.
    neovim_z: bool,
    /// A review check waiting for an agent to be started.
    pending_review: Option<String>,
    /// A prompt for the agent this Cellix is a pane in, for the mod to submit.
    outgoing_prompt: Option<String>,
    /// Words for the mod to put in Claude Code's prompt box, unsent, once.
    outgoing_fill: Option<String>,
    /// The changed line a menu was opened on in Review, for its Comment choice: its file,
    /// number, text, and whether it was taken out.
    review_line: Option<(String, usize, String, bool)>,
    /// The fixes the language server last offered, for the menu that shows them.
    code_actions: Vec<cellix_lang::lsp::Fix>,
    /// Code's symbols from its language server, for the Outline, and the file and revision
    /// last asked about, so it's asked once a change.
    symbols: HashMap<PathBuf, Vec<crate::outline::Heading>>,
    symbols_asked: Option<(PathBuf, u64)>,
    /// Where the pointer rests, since when, and whether the server has been asked about it.
    rest: Option<((u16, u16), std::time::Instant, bool)>,
    /// What the server said about the symbol there, and the cell it's about.
    hover_info: Option<(String, (u16, u16))>,
    /// The cell a hover was asked for, so its answer lands there.
    hover_at: Option<(u16, u16)>,
    /// Places left by jumps, for Go Back, and the ones Go Back left, for Go Forward.
    back: Vec<(PathBuf, usize)>,
    forward: Vec<(PathBuf, usize)>,
    /// What a comment from Review's line numbers is about, and whether that line was removed.
    comment_quote: Option<(String, bool)>,
    /// Where an exported PDF goes once its hidden build is done.
    export_to: Option<PathBuf>,
    /// A push or pull on its way.
    sync_result: Option<std::sync::mpsc::Receiver<Result<String, String>>>,
    /// What runs sessions so they can outlive Cellix: Cellix itself, away from tests.
    host_program: Option<PathBuf>,
    /// A kernel for each notebook that has run something, by its file.
    kernels: HashMap<PathBuf, Kernel>,
    /// Stands in for the kernel's runner in tests.
    kernel_command: Option<Vec<String>>,
    /// Checks spelling and grammar, started when a page first needs it.
    spell: Option<crate::spell::Spell>,
    spell_on: bool,
    /// When the text last changed, so checks wait for a pause.
    last_edit: (u64, std::time::Instant),
    /// Where pieces of LaTeX built for pages are kept, and what builds them.
    built_folder: PathBuf,
    snippets: Option<crate::snippets::Worker>,
    /// A page on its way to the browser, and whether it got there.
    browser_result: Option<(String, std::sync::mpsc::Receiver<bool>)>,
    /// No browser would open and terminal-browser isn't here, so the corner card offers it.
    browser_offer: bool,
    /// terminal-browser's installer, running, and what it said.
    browser_install: Option<std::sync::mpsc::Receiver<String>>,
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
        let (vim, neovim) = keymap_engines(settings.keymap.as_deref());
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
            clipboard_image: clipboard_image_command(),
            pending_ask: None,
            pending_review: None,
            neovim_z: false,
            outgoing_prompt: None,
            outgoing_fill: None,
            export_to: None,
            comment_quote: None,
            back: Vec::new(),
            forward: Vec::new(),
            review_line: None,
            code_actions: Vec::new(),
            symbols: HashMap::new(),
            symbols_asked: None,
            rest: None,
            hover_info: None,
            hover_at: None,
            sync_result: None,
            keep_running: false,
            kernels: HashMap::new(),
            kernel_command: None,
            pane_explained: false,
            host_program: std::env::current_exe().ok().filter(|_| !cfg!(test)),
            spell: None,
            spell_on: cfg!(not(test)) && settings.spell_check != Some(false),
            last_edit: (0, std::time::Instant::now()),
            built_folder: crate::snippets::default_folder(),
            snippets: None,
            system_browser: crate::agents::opener(),
            browser_result: None,
            browser_offer: false,
            browser_install: None,
            missing_servers: HashMap::new(),
            not_now: Vec::new(),
            offer_install: None,
            server_install: None,
            comments: SharedComments::default(),
            wait_file: None,
            vim,
            neovim,
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
            wheel: crate::wheel::Wheel::default(),
            wheel_lines: WHEEL_LINES,
            scrolled: None,
            ide: None,
            proposal: None,
            queued_diffs: VecDeque::new(),
            told_selection: None,
            plan: None,
            dock: Dock::default(),
            stop_confirmed: false,
            build: None,
            remove_after_build: Vec::new(),
            threads,
            threads_seen: None,
            threads_broken,
            tests: Vec::new(),
            hover: None,
            last_click: None,
            preview_press: None,
            maximized: None,
            tab_drag: None,
            settings_page: None,
            terminal_look: (None, None),
            smooth_change: None,
            sidebar_view: SidebarView::Explorer,
            outline: Outline::default(),
            source: SourceControl::default(),
            source_age: 0,
            disk_look: None,
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

    pub(crate) fn say(&self, text: impl Into<String>) {
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
        let picture = ["pdf", "png", "jpg", "jpeg", "gif", "webp"];
        let extension = path.extension().and_then(|e| e.to_str());
        if extension.is_some_and(|e| picture.contains(&e.to_lowercase().as_str())) {
            self.show_pdf(path);
            return Ok(());
        }
        self.settings_page = None;
        if let Some(i) = self
            .editors
            .iter()
            .position(|e| e.buffer.path() == Some(path))
        {
            self.set_active(i);
            return Ok(());
        }
        let buffer = Buffer::open(path).map_err(|e| {
            if e.kind() != io::ErrorKind::InvalidData {
                return e;
            }
            let size = std::fs::metadata(path).map_or(0, |m| m.len());
            let megabytes = size as f64 / 1_000_000.0;
            io::Error::other(format!("it's a binary file of {megabytes:.1} MB"))
        })?;
        let book = path.extension().is_some_and(|e| e == "ipynb");
        let book = book.then(|| Notebook::read(&buffer.text()).ok()).flatten();
        let buffer = match &book {
            Some(book) => Buffer::open_as(path, &book.page())?,
            None => buffer,
        };
        let mut editor = Editor::new(buffer, self.measure.clone(), self.registry.clone());
        editor.minimap_on = self.settings.minimap != Some(false);
        editor.built = Some(self.built_folder.clone());
        editor.notebook = book;
        // Track Changes stays as it was left for this document.
        let relative = self.relative(path);
        editor.tracking = editor.rendered && self.tracked_files().contains(&relative);
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
        // A paper needs an engine to build, which the Build button asks about when it does.
        if language == "LaTeX" && self.build.is_none() && self.latex_engine().is_some() {
            return;
        }
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
        // The window's corner, so opening the side pane doesn't move it over the writing.
        Rect::new(
            self.size.0.saturating_sub(OFFER_WIDTH + 2),
            self.size.1.saturating_sub(OFFER_HEIGHT + 2),
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

    /// Whether the corner card is offering terminal-browser.
    fn offering_browser(&self) -> bool {
        let busy = self.settings_page.is_some() || self.review.is_some() || self.overlay_is_open();
        self.browser_offer && !busy
    }

    /// Handles a press on the offer. Returns true if the card took it.
    fn click_offer(&mut self, x: u16, y: u16) -> bool {
        if self.offering_browser() {
            return self.click_browser_offer(x, y);
        }
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

    fn click_browser_offer(&mut self, x: u16, y: u16) -> bool {
        let area = self.offer_area();
        if !area.contains(x, y) {
            return false;
        }
        if self.browser_install.is_some() || y != area.y + OFFER_BUTTON_ROW {
            return true;
        }
        let hit = self
            .offer_buttons()
            .into_iter()
            .find(|(_, cells, at)| x >= *at && x < at + cells.chars().count() as u16);
        match hit.map(|(label, _, _)| label) {
            Some("Install") => self.install_browser(),
            Some("Not now") => self.browser_offer = false,
            Some("Never") => {
                self.browser_offer = false;
                self.settings
                    .declined_languages
                    .push(BROWSER_TOOL.to_string());
                let declined = serde_json::json!(self.settings.declined_languages);
                let _ = Settings::store("declined_languages", declined);
            }
            _ => {}
        }
        true
    }

    /// With no browser to open, offers terminal-browser on the same card as a language server,
    /// unless it's here or was turned down for good. Returns whether it offered.
    fn offer_browser(&mut self, installed: bool) -> bool {
        let declined = self
            .settings
            .declined_languages
            .iter()
            .any(|l| l == BROWSER_TOOL);
        self.browser_offer = !installed && !declined;
        self.browser_offer
    }

    fn install_browser(&mut self) {
        if self.browser_install.is_some() {
            return;
        }
        let (done, result) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = done.send(install_with(BROWSER_INSTALL, BROWSER_TOOL));
        });
        self.browser_install = Some(result);
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
        // terminal-browser's card is a language's card with its own words.
        let words = if self.offering_browser() {
            Some((
                format!("Install {BROWSER_TOOL}?"),
                "It runs a real browser in the side pane.".to_string(),
                format!("Installing {BROWSER_TOOL}…"),
                "You can keep working while it installs.".to_string(),
                self.browser_install.is_some(),
            ))
        } else {
            self.offer().map(|(language, server)| {
                let tool = SERVERS[server].command[0];
                (
                    format!("Install {language} support?"),
                    format!("{tool} adds completion, errors and rename."),
                    format!("Installing {language} support…"),
                    format!("You can keep working while {tool} installs."),
                    self.offer_install.is_some(),
                )
            })
        };
        let Some((title, detail, busy_title, busy_detail, busy)) = words else {
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
        if busy {
            grid.label(area.x + 2, area.y + 1, &busy_title, strong, right);
            grid.label(area.x + 2, area.y + 2, &busy_detail, dim, right);
            return;
        }
        grid.label(area.x + 2, area.y, &title, strong, right);
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
            grid.button(*x, y, cells, label, fill, style);
        }
    }

    /// Sends edits to servers and handles their replies. Returns true if anything changed.
    /// Hands a waiting comment to the agent that was started for it, once it's ready to hear.
    fn poll_pending_ask(&mut self) -> bool {
        const QUIET: std::time::Duration = std::time::Duration::from_millis(1200);
        if self.pending_review.is_some() && self.dock.agent_mut().is_some_and(|s| s.settled(QUIET))
        {
            let text = self.pending_review.take().unwrap_or_default();
            if let Some(session) = self.dock.agent_mut() {
                session.send_line(&text.replace('\n', " "));
            }
            return true;
        }
        let Some(id) = self.pending_ask else {
            return false;
        };
        let ready = self.dock.agent_mut().is_some_and(|s| s.settled(QUIET));
        if ready {
            self.pending_ask = None;
            self.thread_hit(ThreadHit::Ask(id));
        }
        ready
    }

    /// Sends the page in view to be checked once typing pauses, and takes in what comes back.
    fn poll_spell(&mut self) -> bool {
        const PAUSE: std::time::Duration = std::time::Duration::from_millis(500);
        if !self.spell_on {
            return false;
        }
        let mut changed = false;
        while let Some(found) = self.spell.as_ref().and_then(crate::spell::Spell::poll) {
            let same = |e: &&mut Editor| e.buffer.path() == Some(found.file.as_path());
            if let Some(editor) = self.editors.iter_mut().find(same) {
                editor.issues = (found.revision, found.issues);
                changed = true;
            }
        }
        let Some(editor) = self.editors.get(self.active()) else {
            return changed;
        };
        let revision = editor.buffer.revision();
        if self.last_edit.0 != revision {
            self.last_edit = (revision, std::time::Instant::now());
        }
        let due = editor.rendered
            && editor.dialect().is_writing()
            && editor.checked != revision + 1
            && self.last_edit.1.elapsed()
                >= if revision == 0 {
                    Default::default()
                } else {
                    PAUSE
                };
        let Some(file) = editor.buffer.path().filter(|_| due).map(Path::to_path_buf) else {
            return changed;
        };
        let (text, source) = (editor.buffer.text(), editor.prose_source());
        let spell = self
            .spell
            .get_or_insert_with(|| crate::spell::Spell::start(Settings::words_file()));
        spell.check(file, revision, text, source);
        let active = self.active();
        self.editors[active].checked = revision + 1;
        changed
    }

    fn active_file(&self) -> Option<PathBuf> {
        let editor = self.editors.get(self.active())?;
        editor.buffer.path().map(Path::to_path_buf)
    }

    /// This notebook's kernel, started if it needs to be.
    fn kernel_for_notebook(&mut self) -> Option<PathBuf> {
        let file = self.active_file()?;
        if !self.kernels.contains_key(&file) {
            let folder = file.parent().map(Path::to_path_buf).unwrap_or_default();
            let editor = self.editors.get(self.active())?;
            let name = editor.notebook.as_ref()?.kernel();
            let started = match &self.kernel_command {
                Some(command) => Kernel::start_command(command, &folder),
                None => {
                    let python = kernel::python_for(&folder, self.workspace.root());
                    Kernel::start(&python, &folder, &name)
                }
            };
            match started {
                Ok(kernel) => {
                    self.kernels.insert(file.clone(), kernel);
                }
                Err(e) => {
                    self.say(format!("Could not start Python: {e}"));
                    return None;
                }
            }
        }
        Some(file)
    }

    fn add_cell(&mut self, code: bool) {
        let area = self.focused_area();
        if let Some(editor) = self.editor_mut() {
            editor.prepare_edit(area);
            editor.insert_cell(code);
        }
        self.reveal();
    }

    /// Runs the cell and moves on to the next, as Shift+Enter does in Jupyter.
    fn run_and_advance(&mut self) {
        self.run_cell();
        let area = self.focused_area();
        if let Some(editor) = self.editor_mut() {
            editor.go_to_next_cell(area);
        }
    }

    fn clear_all_outputs(&mut self) {
        let Some(editor) = self.editor_mut() else {
            return;
        };
        if let Some(book) = editor.notebook.as_mut() {
            book.clear_all_outputs();
        }
        if !editor.buffer.is_dirty() {
            let _ = editor.save();
        }
        editor.refresh_page();
    }

    /// Sends cells to the kernel, marking them as running.
    fn send_cells(&mut self, cells: Vec<(Marker, String)>) {
        let Some(file) = self.kernel_for_notebook() else {
            return;
        };
        for (mark, code) in cells {
            let Some(number) = mark.cell else {
                continue;
            };
            if let Some(book) = self.editor_mut().and_then(|e| e.notebook.as_mut()) {
                book.clear_outputs(number);
                book.running.push(number);
            }
            let Some(kernel) = self.kernels.get_mut(&file) else {
                return;
            };
            if let Err(e) = kernel.run(number, &code) {
                self.say(format!("The kernel has gone: {e}"));
                self.kernels.remove(&file);
                return;
            }
        }
        if let Some(editor) = self.editor_mut() {
            editor.refresh_page();
        }
    }

    /// Gives cells added since the file was read a number, so they can hold their outputs.
    fn number_cells(&mut self) -> bool {
        let Some(editor) = self.editor_mut() else {
            return false;
        };
        if editor
            .code_cells()
            .iter()
            .all(|(mark, _)| mark.cell.is_some())
        {
            return true;
        }
        match editor.number_new_cells() {
            Ok(()) => true,
            Err(e) => {
                self.say(format!("Could not save the notebook: {e}"));
                false
            }
        }
    }

    fn run_cell(&mut self) {
        if !self.number_cells() {
            return;
        }
        let line = self
            .cursor_line()
            .map_or(0, |(_, line)| line.saturating_sub(1));
        let cell = self
            .editors
            .get(self.active())
            .and_then(|e| e.code_cell_at(line));
        match cell {
            Some((_, mark, code)) => self.send_cells(vec![(mark, code)]),
            None => self.say("Put the caret in a code cell to run it"),
        }
    }

    fn run_all_cells(&mut self) {
        if !self.number_cells() {
            return;
        }
        let cells = self.editors.get(self.active()).map(Editor::code_cells);
        match cells.filter(|cells| !cells.is_empty()) {
            Some(cells) => self.send_cells(cells),
            None => self.say("This notebook has no code cells"),
        }
    }

    /// Takes in what kernels have to say.
    fn poll_kernels(&mut self) -> bool {
        let files: Vec<PathBuf> = self.kernels.keys().cloned().collect();
        let mut changed = false;
        for file in files {
            let events: Vec<KernelEvent> = self.kernels[&file].events.try_iter().collect();
            for event in events {
                changed = true;
                self.kernel_event(&file, event);
            }
        }
        changed
    }

    fn kernel_event(&mut self, file: &Path, event: KernelEvent) {
        let editor = self
            .editors
            .iter_mut()
            .find(|e| e.buffer.path() == Some(file));
        let Some(editor) = editor else {
            return;
        };
        let Some(book) = editor.notebook.as_mut() else {
            return;
        };
        match event {
            KernelEvent::Ready => {}
            KernelEvent::Output(cell, output) => book.add_output(cell, output),
            KernelEvent::Clear(cell) => book.clear_outputs(cell),
            KernelEvent::Done(cell, count) => {
                book.set_count(cell, count);
                book.running.retain(|n| *n != cell);
                // Outputs aren't in the text, so nothing else would ever write them.
                if book.running.is_empty() && !editor.buffer.is_dirty() {
                    let _ = editor.save();
                }
            }
            KernelEvent::Missing | KernelEvent::Failed(_) => {
                book.running.clear();
                editor.refresh_page();
                self.kernels.remove(file);
                if let KernelEvent::Failed(reason) = event {
                    self.say(format!("The kernel wouldn't start: {reason}"));
                } else {
                    self.offer_jupyter(file);
                }
                return;
            }
        }
        editor.refresh_page();
    }

    /// Running cells needs Jupyter, which is the project's to install, so it says what it
    /// would run and does it where it can be watched.
    fn offer_jupyter(&mut self, file: &Path) {
        let folder = file.parent().unwrap_or(Path::new("."));
        let python = kernel::python_for(folder, self.workspace.root());
        let install = vec![
            python.display().to_string(),
            "-m".to_string(),
            "pip".to_string(),
            "install".to_string(),
            "ipykernel".to_string(),
            "jupyter_client".to_string(),
        ];
        let items = vec![
            MenuItem::new(
                "Install Jupyter in the Side Pane",
                MenuAction::InPane("Install Jupyter".to_string(), install),
            ),
            MenuItem::run("Show Agents", Command::ShowAgents),
        ];
        self.say(format!(
            "Running cells needs Jupyter, which {} doesn't have",
            python.display()
        ));
        let main = self.main_area();
        self.show_menu((main.x + 4, main.y + 3), items);
    }

    /// The Run button on the code cell under the pointer, and a line in that cell.
    fn run_button(&self) -> Option<(Rect, usize)> {
        let (x, y) = self.hover?;
        if !matches!(self.overlay, Overlay::None) {
            return None;
        }
        let pane = self.pane_at(x, y)?;
        let editor = self.editors.get(*self.panes.get(pane)?)?;
        editor.notebook.as_ref()?;
        let area = self.pane_area(pane);
        let (line, top) = editor.code_block_on_row(area, y)?;
        editor.code_cell_at(line)?;
        let right = area.right().saturating_sub(RUN_WIDTH + 4);
        Some((Rect::new(right, top, RUN_WIDTH, 1), line))
    }

    /// Keeps Source Control up to date while it shows, and takes in what a push or pull did.
    fn poll_source(&mut self) -> bool {
        const EVERY_TICKS: u32 = 4;
        let mut changed = self.source.poll();
        let shown = self.review.as_ref().filter(|r| r.beside_panel);
        self.source.shown = shown.and_then(|r| r.current_path()).map(Path::to_path_buf);
        if let Some(result) = &self.sync_result
            && let Ok(result) = result.try_recv()
        {
            self.sync_result = None;
            self.source.busy = None;
            match result {
                Ok(done) => self.say(done),
                Err(why) => self.say(format!("Git refused: {why}")),
            }
            self.source.refresh(self.workspace.root());
            changed = true;
        }
        if self.sidebar_view == SidebarView::Source && self.show_sidebar {
            self.source_age += 1;
            if self.source_age >= EVERY_TICKS {
                self.source_age = 0;
                self.source.refresh(self.workspace.root());
            }
        }
        changed
    }

    fn click_source(&mut self, x: u16, y: u16) {
        let Some(area) = self.sidebar_area() else {
            return;
        };
        let hit = self.source.hit(area, x, y);
        self.source.focused = hit == Some(SourceHit::Message);
        let root = self.workspace.root().to_path_buf();
        let result = match hit {
            None | Some(SourceHit::Message) => Ok(()),
            Some(SourceHit::Commit) => {
                self.commit_message();
                return;
            }
            Some(SourceHit::Sync) => {
                self.sync_remote();
                return;
            }
            Some(SourceHit::Open(path)) => {
                self.show_change(path);
                return;
            }
            Some(SourceHit::Discard(path, untracked)) => {
                let name = path.display().to_string();
                self.overlay = Overlay::Confirm(Confirm {
                    title: format!("Discard changes to {name}?"),
                    detail: if untracked {
                        "This file isn't in git, so it will be deleted.".to_string()
                    } else {
                        "This can't be undone.".to_string()
                    },
                    buttons: vec![
                        ("Cancel", Choice::Cancel),
                        ("Discard", Choice::DiscardFile(path, untracked)),
                    ],
                    selected: 0,
                });
                return;
            }
            Some(SourceHit::Stage(path)) => cellix_review::stage(&root, &[path]),
            Some(SourceHit::Unstage(path)) => cellix_review::unstage(&root, &[path]),
            Some(SourceHit::StageAll) => cellix_review::stage(&root, &self.source.unstaged()),
            Some(SourceHit::UnstageAll) => cellix_review::unstage(&root, &self.source.staged()),
        };
        if let Err(why) = result {
            self.say(format!("Git refused: {why}"));
        }
        self.source_settled();
    }

    /// Reads the repository now, since something just changed it.
    fn source_settled(&mut self) {
        self.source.entries = cellix_review::status_entries(self.workspace.root());
        self.source.reread(self.workspace.root());
        if let Some(review) = &mut self.review {
            review.refresh();
        }
    }

    /// Shows a file's changes beside the panel.
    fn show_change(&mut self, path: PathBuf) {
        if self.review.is_none() {
            self.review = Some(self.new_review(self.workspace.root().to_path_buf()));
        }
        if let Some(review) = &mut self.review {
            review.beside_panel = true;
            review.show_path(path);
        }
    }

    /// Commits with the panel's message, staging everything first if nothing's staged.
    fn commit_message(&mut self) {
        let message = self.source.message.trim().to_string();
        if self.source.entries.is_empty() {
            self.say("Nothing to commit");
            return;
        }
        if message.is_empty() {
            self.source.focused = true;
            self.say("Write a message first");
            return;
        }
        let root = self.workspace.root().to_path_buf();
        if self.source.staged().is_empty()
            && let Err(why) = cellix_review::stage(&root, &self.source.unstaged())
        {
            self.say(format!("Git refused: {why}"));
            return;
        }
        match cellix_review::commit(&root, &message) {
            Ok(done) => {
                self.say(done);
                self.source.message.clear();
                self.source.focused = false;
            }
            Err(why) => self.say(format!("Git refused: {why}")),
        }
        self.source_settled();
        self.refresh_tree();
    }

    /// Pushes, pulls or publishes, off the main thread: a network can take its time.
    fn sync_remote(&mut self) {
        if self.source.busy.is_some() {
            return;
        }
        let root = self.workspace.root().to_path_buf();
        let counts = self.source.remote.counts;
        let branch = self.branch.clone().unwrap_or_default();
        self.source.busy = Some(match counts {
            Some((_, 0)) => "Pushing…",
            Some((0, _)) => "Pulling…",
            Some(_) => "Syncing…",
            None => "Publishing…",
        });
        let (done, result) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let outcome = match counts {
                Some((_, 0)) => cellix_review::push(&root).map(|()| "Pushed"),
                Some((0, _)) => cellix_review::pull(&root).map(|()| "Pulled"),
                Some(_) => cellix_review::pull(&root)
                    .and_then(|()| cellix_review::push(&root))
                    .map(|()| "Synced"),
                None => cellix_review::publish(&root, &branch).map(|()| "Published"),
            };
            let _ = done.send(outcome.map(ToString::to_string));
        });
        self.sync_result = Some(result);
    }

    /// Keys for the commit message. Returns true if it took the key.
    fn source_key(&mut self, key: KeyEvent) -> bool {
        if !self.source.focused || !matches!(self.overlay, Overlay::None) {
            return false;
        }
        match key.key {
            Key::Esc => self.source.focused = false,
            Key::Enter if key.mods.shift || key.mods.alt => self.source.message.push('\n'),
            Key::Enter => self.commit_message(),
            Key::Backspace => {
                self.source.message.pop();
            }
            Key::Char(c) if !key.mods.ctrl => self.source.message.push(c),
            _ => return false,
        }
        true
    }

    /// Passes on what pages are waiting for, and sets them again as it comes.
    fn poll_snippets(&mut self) -> bool {
        let wanted: Vec<_> = self.editors.iter().flat_map(Editor::take_wanted).collect();
        if !wanted.is_empty() && self.snippets.is_none() {
            let folder = self.built_folder.clone();
            self.snippets = self
                .latex_engine()
                .map(|engine| crate::snippets::Worker::start(engine, folder));
        }
        let Some(worker) = &mut self.snippets else {
            return false;
        };
        for snippet in wanted {
            worker.ask(snippet);
        }
        let finished = worker.poll();
        if finished {
            for editor in &mut self.editors {
                editor.refresh_page();
            }
        }
        finished
    }

    pub fn poll(&mut self) -> bool {
        if self.sidebar_view == SidebarView::Outline {
            self.refresh_outline();
        }
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
        let shown = self.maximized.map(|_| self.in_view());
        for call in calls {
            self.ide_call(call);
        }
        let built = self.poll_build();
        self.keep_in_view(shown);
        self.tell_selection();
        let terminal = self.dock.poll(&self.theme);
        let searched = self.search.poll();
        let reviewed = self.review.as_mut().is_some_and(Review::poll);
        let installed = self.poll_install()
            | self.poll_browser()
            | self.poll_offer()
            | built
            | self.poll_snippets()
            | self.poll_spell()
            | self.poll_kernels()
            | self.poll_source()
            | self.poll_disk()
            | self.poll_pending_ask()
            | self.poll_pan_hint();
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
        cellix_agent::ide::file_url(path)
    }

    /// A path from Claude Code, which may be absolute, a file URI, or relative to the root.
    fn ide_path(&self, text: &str) -> PathBuf {
        self.workspace
            .root()
            .join(cellix_agent::ide::url_path(text))
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
                self.mark_jump();
                if self.open(&path).is_ok() {
                    let area = self.focused_area();
                    if let Some(editor) = self.editor_mut() {
                        let at = editor.buffer.position(line, column);
                        editor.selections.set_single(Selection::cursor(at));
                        editor.reveal(area);
                    }
                }
            }
            LspEvent::Actions(found) => {
                if found.is_empty() {
                    self.say("No quick fixes here");
                    return;
                }
                let area = self.focused_area();
                let at = self
                    .editors
                    .get(self.active())
                    .and_then(|e| e.caret_cell(area))
                    .map_or((area.x, area.y), |(x, y)| (x as u16, y + 1));
                let items = found
                    .iter()
                    .enumerate()
                    .map(|(n, (title, _))| MenuItem::new(title.clone(), MenuAction::ServerFix(n)))
                    .collect();
                self.code_actions = found;
                self.show_menu(at, items);
                // Asked for from the keyboard, so Enter takes the first, as in VS Code.
                if let Overlay::Menu(menu) = &mut self.overlay {
                    menu.selected = Some(0);
                }
            }
            LspEvent::Symbols(path, found) => {
                let headings = found
                    .into_iter()
                    .map(|(title, depth, line)| crate::outline::Heading {
                        level: (depth + 1).min(6) as u8,
                        title,
                        line,
                        file: None,
                    })
                    .collect();
                self.symbols.insert(path, headings);
                self.refresh_outline();
            }
            LspEvent::Hover(text) => {
                if let Some(at) = self.hover_at.take() {
                    self.hover_info = Some((text, at));
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
                len: 0,
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
            Command::ShowHover => client.hover(&path, line, column),
            Command::GoToReferences => client.references(&path, line, column),
            Command::QuickFix => {
                let problems: Vec<(usize, usize, String, u64)> = self
                    .diagnostics
                    .get(&path)
                    .into_iter()
                    .flatten()
                    .filter(|d| d.line == line)
                    .map(|d| (d.line, d.column, d.message.clone(), d.severity))
                    .collect();
                client.code_actions(&path, (line, column), &problems);
            }
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

    /// A citation or label key being typed in a paper: which kind, what's typed of it, and where
    /// it starts.
    fn reference_slot(&self) -> Option<(cellix_doc::latex::Slot, String, usize)> {
        let editor = self.editors.get(self.active())?;
        if editor.dialect() != Dialect::Latex {
            return None;
        }
        let primary = editor.selections.primary();
        if !primary.is_empty() {
            return None;
        }
        let rope = editor.buffer.rope();
        let line_start = rope.line_to_char(rope.char_to_line(primary.head));
        let before = rope.slice(line_start..primary.head).to_string();
        let (slot, key) = cellix_doc::latex::reference_slot(&before)?;
        let typed = before.chars().skip(key).collect();
        Some((slot, typed, line_start + key))
    }

    /// Recomputes what shows from what's known, without asking any server.
    fn rank_suggestions(&mut self) {
        // Inside `\cite{` and `\ref{` the keys are what's wanted, on a page or not.
        if let Some((slot, typed, from)) = self.reference_slot() {
            let project = self
                .editors
                .get(self.active())
                .map(Editor::latex_project)
                .unwrap_or_default();
            let keys = match slot {
                cellix_doc::latex::Slot::Cite => &project.citations,
                cellix_doc::latex::Slot::Ref => &project.labels,
            };
            let mut keys: Vec<String> = keys.keys().cloned().collect();
            keys.sort();
            let all = keys.into_iter().map(|label| Candidate {
                label,
                source: Source::Key,
            });
            let items = complete::rank(&typed, all.collect());
            self.suggest = (!items.is_empty()).then_some(Suggest {
                items,
                selected: 0,
                from: Some(from),
            });
            return;
        }
        // Prose isn't code: a word that completes itself gets in a writer's way.
        if self.on_page() && !self.suggest_asked {
            self.close_suggestions();
            return;
        }
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
        self.suggest = (!items.is_empty()).then_some(Suggest {
            items,
            selected: 0,
            from: None,
        });
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
        let from = self.suggest.as_ref().and_then(|s| s.from);
        let label = self
            .suggest
            .as_ref()
            .and_then(|s| s.items.get(index))
            .map(|c| c.label.clone());
        self.close_suggestions();
        if let Some(label) = label {
            self.insert_completion(&label, from);
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
        let height = suggest.items.len() as u16;
        let bottom = self.size.1.saturating_sub(1);
        // A row clear of the caret, so the panel's paint stays off the line being typed.
        let y = if y + 2 + height <= bottom {
            y + 2
        } else {
            y.saturating_sub(height + 1)
        };
        let x = (x as u16).min(self.size.0.saturating_sub(SUGGEST_WIDTH + 1));
        Some(Rect::new(x, y, SUGGEST_WIDTH.min(self.size.0), height))
    }

    fn draw_suggest(&self, grid: &mut Grid) {
        let (Some(suggest), Some(area)) = (&self.suggest, self.suggest_area()) else {
            return;
        };
        let theme = &self.theme;
        crate::palette::raised(grid, area, theme);
        for (i, item) in suggest.items.iter().enumerate() {
            let y = area.y + i as u16;
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

    /// Replaces the word before the cursor with a completion, or what's from `from` on.
    fn insert_completion(&mut self, text: &str, from: Option<usize>) {
        if let Some(editor) = self.editor_mut() {
            let head = editor.selections.primary().head;
            let start = from
                .unwrap_or_else(|| editor.buffer.word_at(head).0)
                .min(head);
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
        if self.maximized.is_some() {
            return None;
        }
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
        // Review uses the explorer's width: side-by-side code needs it more. Source Control
        // stays, since it's what the diff was opened from.
        let beside = self.review.is_none() || self.sidebar_view == SidebarView::Source;
        (self.show_sidebar && beside).then(|| {
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
        let height = self.size.1.saturating_sub(1);
        if self.maximized == Some(Maximized::Pane) && self.dock.open {
            return Rect::new(x, 0, 0, height);
        }
        match self.dock_area() {
            Some(dock) if self.dock_under() => {
                Rect::new(x, 0, self.size.0.saturating_sub(x), dock.y)
            }
            dock => Rect::new(
                x,
                0,
                self.size.0.saturating_sub(x + dock.map_or(0, |a| a.w)),
                height,
            ),
        }
    }

    /// Whether the pane stands under the file, for a window too narrow to have them side by
    /// side, as when it's sharing a terminal with an agent. A short one has no room for that
    /// either, so there the pane takes the window.
    fn dock_under(&self) -> bool {
        if self.maximized.is_some() {
            return false;
        }
        let room = self.size.0.saturating_sub(self.main_left());
        let chosen = self.dock.open && self.dock.under;
        (chosen || self.dock.too_narrow(room)) && self.size.1 > STACK_HEIGHT
    }

    fn tabs_area(&self) -> Rect {
        let main = self.main_area();
        let inset = if self.compact() { MENU_BUTTON } else { 0 };
        let actions = self.pane_actions();
        let shared = actions.first().filter(|(cell, ..)| cell.x < main.right());
        let end = shared.map_or(main.right(), |(cell, ..)| cell.x - ACTION_MARGIN);
        let width = end.saturating_sub(main.x + inset);
        Rect::new(main.x + inset, main.y, width, 1)
    }

    /// The icons at the top right, each with where it stands.
    fn pane_actions(&self) -> Vec<(Rect, Icon, Command, &'static str)> {
        // The same place whatever's open, so a second click finds the same icon.
        let wide = ACTION_WIDTH * PANE_ACTIONS.len() as u16 + PANE_CLOSE + ACTION_MARGIN;
        let pane = self.dock_area().filter(|_| !self.dock_under());
        let least = pane.map_or(self.main_area().x + FILE_TABS_ROOM, |pane| {
            pane.x + PANE_TABS_ROOM
        });
        let Some(start) = self.size.0.checked_sub(wide).filter(|x| *x >= least) else {
            return Vec::new();
        };
        let cell = |i: usize| Rect::new(start + ACTION_WIDTH * i as u16, 0, ACTION_WIDTH, 1);
        let actions = PANE_ACTIONS.iter().enumerate();
        actions
            .map(|(i, (icon, command, name))| (cell(i), *icon, *command, *name))
            .collect()
    }

    fn action_at(&self, x: u16, y: u16) -> Option<Command> {
        let actions = self.pane_actions();
        let found = actions.into_iter().find(|(cell, ..)| cell.contains(x, y));
        found.map(|(_, _, command, _)| command)
    }

    /// Whether what a rail icon stands for is showing. Review is Source Control at full size.
    fn rail_on(&self, command: Command) -> bool {
        let shown = self.sidebar_area().is_some();
        match command {
            Command::ToggleSidebar => shown && self.sidebar_view == SidebarView::Explorer,
            Command::FindInProject => shown && self.sidebar_view == SidebarView::Search,
            Command::ShowOutline => shown && self.sidebar_view == SidebarView::Outline,
            Command::ShowSourceControl => {
                self.review.is_some() || (shown && self.sidebar_view == SidebarView::Source)
            }
            Command::OpenSettings => self.settings_page.is_some(),
            _ => false,
        }
    }

    /// Whether what an icon stands for is showing.
    fn action_on(&self, command: Command) -> bool {
        let session = self
            .dock
            .session()
            .filter(|_| self.dock.showing(Front::Session));
        match command {
            Command::ShowComments => self.dock.showing(Front::Comments),
            Command::ToggleTerminal => session.is_some_and(|s| s.shell),
            Command::OpenDiff => self.review.is_some(),
            Command::ShowBrowser => session.is_some_and(|s| s.browser),
            Command::ListAgents => self.dock.showing(Front::Agents),
            _ => false,
        }
    }

    fn draw_pane_actions(&self, grid: &mut Grid) {
        let theme = &self.theme;
        for (cell, icon, command, _) in self.pane_actions() {
            let hovered = self.hover.is_some_and(|h| cell.contains(h.0, h.1));
            let on = self.action_on(command);
            grid.fill(cell, theme.tab);
            if on || hovered {
                // The rail's chip, so the two bars of icons read as one set.
                let fill = if on { theme.tint } else { theme.row_hover };
                let (wide, high) = ACTION_PILL;
                let left = cell.x as f32 + (cell.w as f32 - wide) / 2.0;
                let top = cell.y as f32 + (1.0 - high) / 2.0;
                grid.panel(
                    Bounds::cells(left, top, wide, high),
                    PILL_RADIUS,
                    Some(fill),
                    None,
                );
            }
            let color = if on || hovered {
                theme.text.fg
            } else {
                theme.activity.fg
            };
            toolbar::draw_icon(grid, (icon, ACTION_ICON), cell, color, theme.tab);
            // With no tab to say so, the icon does.
            let waiting = match command {
                Command::ListAgents => self.agents.needing_you() > 0,
                Command::ShowComments => self.dock.comment_count.get() > 0 && !on,
                _ => false,
            };
            if waiting {
                let dot = Bounds::cells(cell.right() as f32 - 1.2, 0.1, 0.8, 0.4);
                grid.panel(dot, 22, Some(theme.attention), None);
            }
        }
    }

    /// Columns of the file's row the icons stand in.
    fn actions_over_main(&self) -> u16 {
        let main = self.main_area();
        let actions = self.pane_actions();
        let first = actions.first().filter(|(cell, ..)| cell.x < main.right());
        first.map_or(0, |(cell, ..)| main.right() - cell.x)
    }

    /// Brings up the terminal, or starts one. Pressed again, it takes the keyboard, and then
    /// puts itself away, as VS Code's does.
    fn toggle_terminal(&mut self) {
        let shell = self.dock.sessions.iter().position(|s| s.shell && !s.exited);
        let Some(index) = shell else {
            self.open_agent(crate::agents::SHELL.1);
            return;
        };
        let showing = self.dock.showing(Front::Session) && self.dock.active == Some(index);
        if showing && self.dock.focused {
            self.dock.open = false;
            self.dock.focused = false;
            return;
        }
        self.dock.active = Some(index);
        self.dock.open = true;
        self.dock.front = Front::Session;
        self.dock.focused = true;
    }

    /// Brings up the pane's browser, on the page in view if it hasn't got one yet.
    fn show_browser(&mut self) {
        let browser = self.dock.sessions.iter().position(|s| s.browser);
        let showing = self.dock.showing(Front::Session) && self.dock.active == browser;
        match browser {
            Some(_) if showing => self.run(Command::CloseDock),
            Some(index) => {
                self.dock.active = Some(index);
                self.dock.open = true;
                self.dock.front = Front::Session;
                self.dock.focused = true;
            }
            None if self.browser_target().is_some() => self.open_in_browser(),
            None if self.open_in_pane("about:blank") => {}
            None if self.offer_browser(browser_installed()) => {}
            None => self.say("There's no browser that can run in the pane here"),
        }
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
        if self.compact() || self.maximized == Some(Maximized::Editor) {
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
                    kind: MouseKind::Move
                        | MouseKind::Up
                        | MouseKind::ScrollLeft
                        | MouseKind::ScrollRight,
                    ..
                })
        );
        if !passive {
            self.message.borrow_mut().clear();
        }
        self.follow_rest(&event);
        let shown = self.maximized.map(|_| self.in_view());
        let wheel = matches!(
            event,
            Event::Mouse(MouseEvent {
                kind: MouseKind::ScrollUp
                    | MouseKind::ScrollDown
                    | MouseKind::ScrollLeft
                    | MouseKind::ScrollRight,
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
        self.poll_pan_hint();
        self.keep_in_view(shown);
        self.fit_dock();
        self.follow_threads();
    }

    fn paste(&mut self, text: &str) {
        if let Some(draft) = &mut self.dock.draft {
            draft.text.push_str(text);
            return;
        }
        if self.source.focused {
            self.source.message.push_str(text);
            return;
        }
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
                find.field_mut().push_str(text);
                if !find.in_replace {
                    self.search_file();
                }
            }
            _ => {
                // A file dropped on a terminal arrives as its path.
                if self.on_page()
                    && let Some(path) = dropped_image(text)
                {
                    self.place_image(&path, false);
                    return;
                }
                let area = self.focused_area();
                if let Some(editor) = self.editor_mut() {
                    editor.prepare_edit(area);
                    if editor.tracking && editor.rendered {
                        editor.tracked_paste(text);
                    } else {
                        editor.buffer.replace(&mut editor.selections, text);
                    }
                }
                self.reveal();
            }
        }
    }

    fn tick(&mut self) {
        if let Some(view) = &mut self.dock.pdf {
            view.refresh();
        }
        // The pane opens from many places, and this list mustn't depend on which.
        if self.dock.open && self.dock.listing() && !self.agents.looked() {
            self.refresh_agents();
        }
        let mut note = None;
        let mut reloaded = Vec::new();
        for editor in &mut self.editors {
            if !editor.buffer.changed_on_disk() {
                continue;
            }
            // The agent again, with its last edits still standing untouched as suggestions.
            // Untouched, it's marked against the text from before the turn. Once some are
            // accepted or rejected, against those decisions: the pending ones go back to
            // what they replace, and the agent's text is marked against that again.
            let untouched = editor.agent_review.as_ref().and_then(|(base, marked)| {
                let now = editor.buffer.text();
                if *marked == now {
                    return Some(base.clone());
                }
                let syntax = editor.change_syntax();
                let theirs: Vec<(String, String)> = cellix_doc::changes::find(marked, syntax)
                    .into_iter()
                    .map(|c| (c.old, c.new))
                    .collect();
                let only_decided = cellix_doc::changes::find(&now, syntax)
                    .iter()
                    .all(|c| theirs.contains(&(c.old.clone(), c.new.clone())));
                only_decided.then(|| cellix_doc::changes::settle_all(&now, syntax, false))
            });
            let disk = editor
                .buffer
                .path()
                .and_then(|path| std::fs::read_to_string(path).ok());
            if let (Some(base), Some(disk), true) = (untouched, disk, editor.tracking) {
                let marked = cellix_doc::changes::suggest(&base, &disk, editor.change_syntax());
                editor.set_all_text(&marked);
                editor.buffer.acknowledge_disk();
                editor.agent_review = Some((base, marked));
                note = Some(format!(
                    "{} changed again: its edits so far stand as suggestions",
                    editor.title()
                ));
                continue;
            }
            if editor.buffer.is_dirty() {
                note = Some(format!(
                    "{} changed on disk, keeping your edits",
                    editor.title()
                ));
                editor.buffer.acknowledge_disk();
                editor.conflict = true;
            } else if editor.tracking && editor.rendered {
                // With changes tracked, an agent's edit is a suggestion like anyone's.
                let before = editor.buffer.text();
                note = Some(match editor.reload() {
                    Ok(()) => {
                        let after = editor.buffer.text();
                        let marked =
                            cellix_doc::changes::suggest(&before, &after, editor.change_syntax());
                        if marked != after {
                            editor.set_all_text(&marked);
                            editor.agent_review = Some((before, marked));
                        }
                        format!(
                            "{} changed outside: accept or reject each suggestion, then save",
                            editor.title()
                        )
                    }
                    Err(e) => format!("Could not reload {}: {e}", editor.title()),
                });
            } else {
                note = Some(match editor.reload() {
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
            for editor in &self.editors {
                editor.recount();
            }
        }
        // And a paper an agent wrote rebuilds, so the PDF beside it stays true: the file in
        // front, or another file of the same paper.
        let active = self
            .editors
            .get(self.active())
            .and_then(|e| e.buffer.path());
        if active.is_some_and(|path| reloaded.iter().any(|p| p == path || same_paper(p, path))) {
            self.rebuild_on_save();
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
            self.look_at_disk();
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

    /// Reads the tree and what git knows, away from the screen. This runs every couple of
    /// seconds, and on the main thread it stopped typing and scrolling each time.
    fn look_at_disk(&mut self) {
        if self.disk_look.is_some() {
            return;
        }
        let workspace = self.workspace.clone();
        let opened = self.editors.iter().filter_map(|e| e.buffer.path());
        let opened: Vec<PathBuf> = opened.map(Path::to_path_buf).collect();
        let (done, look) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let root = workspace.root();
            let inside = |path: &PathBuf| path.strip_prefix(root).unwrap_or(path).to_path_buf();
            let _ = done.send(DiskLook {
                entries: workspace.entries(),
                branch: current_branch(root),
                changes: changed_files(root),
                committed: opened
                    .iter()
                    .map(|path| (path.clone(), head_text(root, &inside(path))))
                    .collect(),
            });
        });
        self.disk_look = Some(look);
    }

    /// Takes in what the disk said. Nothing else is touched unless something really changed,
    /// so an idle window stays idle. Returns true if it did.
    fn poll_disk(&mut self) -> bool {
        let Some(look) = self
            .disk_look
            .as_ref()
            .and_then(|look| look.try_recv().ok())
        else {
            return false;
        };
        self.disk_look = None;
        let status = self.status_of(&look.changes);
        let tree = self.explorer.entries != look.entries;
        let marks = self.explorer.status != status;
        let branch = self.branch != look.branch;
        if tree {
            self.explorer.show(look.entries);
        }
        self.explorer.status = status;
        self.branch = look.branch;
        let mut committed = false;
        for (path, text) in look.committed {
            let same = |e: &&mut Editor| e.buffer.path() == Some(path.as_path());
            let text = (!text.is_empty()).then_some(text);
            if let Some(editor) = self.editors.iter_mut().find(same)
                && editor.committed != text
            {
                editor.committed = text;
                editor.refresh_changes();
                committed = true;
            }
        }
        tree || marks || branch || committed
    }

    /// The marks for the explorer: each changed file's letter, and a dot on its folders.
    fn status_of(&self, changes: &[cellix_review::Change]) -> HashMap<PathBuf, char> {
        let root = self.workspace.root();
        let mut status = HashMap::new();
        for change in changes {
            let path = root.join(&change.path);
            for parent in path
                .ancestors()
                .skip(1)
                .take_while(|p| p.starts_with(root) && *p != root)
            {
                status.entry(parent.to_path_buf()).or_insert('•');
            }
            status.insert(path, change.status);
        }
        status
    }

    fn refresh_tree(&mut self) {
        // A look already on its way saw the tree before whatever brought this on.
        self.disk_look = None;
        self.explorer.refresh(&self.workspace);
        let root = self.workspace.root().to_path_buf();
        self.branch = current_branch(&root);
        self.explorer.status = self.status_of(&changed_files(&root));
    }

    fn reveal(&mut self) {
        let area = self.focused_area();
        if let Some(editor) = self.editor_mut() {
            editor.reveal(area);
        }
    }

    /// Types into the comment being written. Returns true if there is one.
    fn draft_key(&mut self, key: KeyEvent) -> bool {
        let Some(draft) = &mut self.dock.draft else {
            return false;
        };
        match key.key {
            Key::Esc => self.dock.draft = None,
            Key::Enter if key.mods.shift || key.mods.alt => draft.text.push('\n'),
            Key::Enter => {
                let draft = self.dock.draft.take().expect("checked above");
                self.finish_draft(draft);
            }
            Key::Backspace => {
                draft.text.pop();
            }
            Key::Char(c) if !key.mods.ctrl => draft.text.push(c),
            _ => {}
        }
        true
    }

    fn finish_draft(&mut self, draft: Draft) {
        let text = draft.text.trim().to_string();
        if text.is_empty() {
            return;
        }
        let mut asked = None;
        match draft.thread {
            Some(id) => {
                if let Some(thread) = self.threads.iter_mut().find(|t| t.id == id) {
                    thread.replies.push(Reply {
                        author: YOU.to_string(),
                        text,
                    });
                }
            }
            None => {
                let id = cellix_review::threads::next_id(&self.threads);
                if draft.code
                    && !draft.ask
                    && let Ok(mut comments) = self.comments.lock()
                {
                    comments.push(Comment {
                        file: draft.file.clone(),
                        line: draft.line,
                        text: text.clone(),
                        quote: draft.quote.clone(),
                        ..Comment::default()
                    });
                }
                self.threads.push(Thread {
                    id,
                    file: draft.file,
                    line: draft.line,
                    quote: draft.quote,
                    text,
                    replies: Vec::new(),
                    resolved: false,
                    told: draft.code.then_some(0),
                    asked: false,
                    kind: String::new(),
                    suggested_by: String::new(),
                    copy: (0, 0),
                    earlier: None,
                    reworded: false,
                });
                asked = draft.ask.then_some(id);
            }
        }
        self.threads_seen = None;
        self.save_threads();
        if let Some(id) = asked {
            self.thread_hit(ThreadHit::Ask(id));
        }
    }

    /// Where the buttons by a selection stand: under it, or over it on the last row.
    fn selection_bar(&self) -> Option<(Rect, Rect)> {
        let idle = matches!(self.overlay, Overlay::None) && self.drag != Drag::Text;
        let plain = self.review.is_none() && self.settings_page.is_none();
        if !idle || !plain || self.dock.draft.is_some() {
            return None;
        }
        let editor = self.editors.get(self.active())?;
        // A comment is on one place, so several cursors get none.
        if editor.selections.len() > 1 {
            return None;
        }
        editor.selected_text().filter(|t| !t.trim().is_empty())?;
        let area = self.focused_area();
        let primary = editor.selections.primary();
        let (start, end) = (
            primary.anchor.min(primary.head),
            primary.anchor.max(primary.head),
        );
        // Above the first selected line, so it never covers what it's about; below the last
        // when the first is at the top or out of view.
        let (x, y) = match editor.cell_of(area, start) {
            Some((x, y)) if y > area.y => (x, y - 1),
            _ => {
                let (x, y) = editor.cell_of(area, end)?;
                (x, (y + 1).min(area.bottom().saturating_sub(1)))
            }
        };
        let width = COMMENT_WIDTH + ASK_WIDTH;
        let x = (x as u16)
            .min(area.right().saturating_sub(width + 1))
            .max(area.x);
        Some((
            Rect::new(x, y, COMMENT_WIDTH, 1),
            Rect::new(x + COMMENT_WIDTH, y, ASK_WIDTH, 1),
        ))
    }

    /// The button on the picture under the pointer, and the line that picture is on.
    fn picture_comment(&self) -> Option<(Rect, usize)> {
        let (x, y) = self.hover?;
        if !matches!(self.overlay, Overlay::None) || self.review.is_some() {
            return None;
        }
        let pane = self.pane_at(x, y)?;
        let editor = self
            .editors
            .get(*self.panes.get(pane)?)
            .filter(|e| e.rendered)?;
        if self.measure.in_cells() {
            return None;
        }
        let area = self.pane_area(pane);
        let (line, [left, right, top, _]) = editor.figure_on_row(area, y)?;
        if (x as f32) < left - 1.0 || (x as f32) > right + COMMENT_WIDTH as f32 {
            return None;
        }
        let beside = (right as u16 + 1).min(area.right().saturating_sub(COMMENT_WIDTH + 2));
        Some((Rect::new(beside, top as u16, COMMENT_WIDTH, 1), line))
    }

    fn draw_chip(&self, grid: &mut Grid, chip: Rect, name: &str) {
        /// Its gap from the words it stands over, and from the button beside it, in cells.
        const LIFT: f32 = 0.2;
        const SIDE: f32 = 0.24;
        let theme = &self.theme;
        let on_it = self.hover.is_some_and(|h| chip.contains(h.0, h.1));
        let style = Style {
            fg: theme.text.fg,
            bold: false,
            painted: !self.measure.in_cells(),
            bg: theme.popup,
            ..theme.bar
        };
        if self.measure.in_cells() {
            grid.fill(chip, style);
        } else {
            let fill = if on_it {
                theme.popup_hover
            } else {
                theme.popup
            };
            let bounds = Bounds::cells(
                chip.x as f32 + SIDE,
                chip.y as f32 + 0.02 - LIFT,
                chip.w as f32 - SIDE * 2.0,
                0.96,
            );
            grid.popup(bounds, 20, fill, theme.popup_border);
            let font = Font::default();
            let width = self.measure.width(name, font);
            let x = chip.x as f32 + (chip.w as f32 - width) / 2.0;
            let top = chip.y as f32 - LIFT;
            grid.text_run(
                (x, top),
                chip.right() as f32,
                name,
                (theme.text.fg, false),
                font,
            );
        }
        grid.put_str(chip.x + 1, chip.y, name, style, chip.right());
    }

    /// The button on a PDF or picture in the side pane, once the pointer has been on it.
    fn view_comment(&self) -> Option<Rect> {
        let screen = self.dock_screen()?;
        let showing = self.dock.front == Front::Pdf && self.dock.pdf.is_some();
        self.hover.filter(|h| screen.contains(h.0, h.1))?;
        let ready = showing && self.dock.pdf_spot.is_some();
        (ready && matches!(self.overlay, Overlay::None)).then(|| {
            let left = screen.right().saturating_sub(COMMENT_WIDTH + 2);
            Rect::new(left, screen.y + VIEW_COMMENT_ROW, COMMENT_WIDTH, 1)
        })
    }

    /// Starts a comment on the place last pointed at. A paper's page leads back to its
    /// source, where a comment can stay put through every rebuild.
    fn comment_on_view(&mut self) {
        let Some(view) = &self.dock.pdf else {
            return;
        };
        let name = view
            .path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let is_pdf = view.path.extension().is_some_and(|e| e == "pdf");
        let spot = self.dock.pdf_spot.unwrap_or_default();
        let source = view.source_at(spot.0, spot.1, spot.2);
        let shown = view.path.clone();
        let (file, line, quote) = match source {
            Some((path, line)) => {
                let text = std::fs::read_to_string(&path).unwrap_or_default();
                let quote = text.lines().nth(line.saturating_sub(1)).unwrap_or_default();
                (self.relative(&path), line, quote.trim().to_string())
            }
            None if is_pdf => {
                let page = spot.0 + 1;
                (
                    self.relative(&shown),
                    page,
                    format!("Page {page} of {name}"),
                )
            }
            None => (self.relative(&shown), 1, format!("Image {name}")),
        };
        self.write_comment(Draft {
            thread: None,
            file,
            line,
            quote,
            text: String::new(),
            ask: false,
            code: false,
        });
    }

    /// The buttons that stand by what's selected, and on the picture under the pointer.
    fn draw_companion(&self, grid: &mut Grid) {
        if let Some(chip) = self.view_comment() {
            self.draw_chip(grid, chip, "Comment");
        }
        if let Some((chip, _)) = self.run_button() {
            self.draw_chip(grid, chip, "Run");
        }
        if let Some((comment, ask)) = self.selection_bar() {
            self.draw_chip(grid, comment, "Comment");
            self.draw_chip(grid, ask, "Ask Agent");
        } else if let Some((chip, _)) = self.picture_comment() {
            self.draw_chip(grid, chip, "Comment");
        }
    }

    /// Takes a click on those buttons.
    fn click_companion(&mut self, x: u16, y: u16) -> bool {
        if self.view_comment().is_some_and(|chip| chip.contains(x, y)) {
            self.comment_on_view();
            return true;
        }
        if let Some((chip, line)) = self.run_button().filter(|(chip, _)| chip.contains(x, y)) {
            let _ = chip;
            let area = self.focused_area();
            if let Some(editor) = self.editor_mut() {
                editor.go_to_line(line + 1, area);
            }
            self.run_cell();
            return true;
        }
        if let Some((comment, ask)) = self.selection_bar() {
            if comment.contains(x, y) {
                self.prompt_comment_at_cursor(false);
                return true;
            }
            if ask.contains(x, y) {
                self.prompt_comment_at_cursor(true);
                return true;
            }
        }
        match self.picture_comment() {
            Some((chip, line)) if chip.contains(x, y) => {
                self.comment_on_picture(line);
                true
            }
            _ => false,
        }
    }

    /// Remembers where on a page the pointer is, unless it's on the button itself.
    fn note_view_spot(&mut self, x: u16, y: u16) {
        if self.view_comment().is_some_and(|chip| chip.contains(x, y)) {
            return;
        }
        let screen = self.dock_screen().unwrap_or_default();
        let hit = self.dock.pdf.as_ref().and_then(|v| v.hit(screen, x, y));
        if let Some(PdfHit::Page(page, across, down)) = hit {
            self.dock.pdf_spot = Some((page, across, down));
        }
    }

    /// Starts a comment on a picture, which has no text to select.
    fn comment_on_picture(&mut self, line: usize) {
        let Some(editor) = self.editors.get(self.active()) else {
            return;
        };
        let quote = editor.figure_quote(line);
        let Some((path, _)) = self.cursor_line() else {
            return;
        };
        let file = self.relative(&path);
        self.write_comment(Draft {
            thread: None,
            file,
            line: line + 1,
            quote,
            text: String::new(),
            ask: false,
            code: false,
        });
    }

    /// Opens the comments, with one being written.
    fn write_comment(&mut self, draft: Draft) {
        self.dock.open = true;
        self.dock.show_comments();
        self.dock.focused = false;
        self.dock.comments_top = 0;
        self.dock.draft = Some(draft);
    }

    fn key(&mut self, key: KeyEvent) {
        if key.key == Key::Esc && self.cancel_drag() {
            return;
        }
        if matches!(self.overlay, Overlay::None) && self.draft_key(key) {
            return;
        }
        if self.dock.focused
            && self.dock.showing(Front::Pdf)
            && matches!(self.overlay, Overlay::None)
            && self.preview_key(key)
        {
            return;
        }
        if self.source_key(key) {
            return;
        }
        // Jupyter's keys, for a notebook: Shift+Enter runs and moves on, Ctrl+Enter stays.
        let in_notebook = !self.dock.focused
            && matches!(self.overlay, Overlay::None)
            && self
                .editors
                .get(self.active())
                .is_some_and(|e| e.notebook.is_some());
        if in_notebook && key.key == Key::Enter && (key.mods.shift || key.mods.ctrl) {
            if key.mods.shift {
                self.run_and_advance();
            } else {
                self.run_cell();
            }
            return;
        }
        if self.dock.focused && matches!(self.overlay, Overlay::None) {
            // The session gets everything but what moves away from it.
            let command = simple_keymap(key);
            let leaves = matches!(
                command,
                Some(
                    Command::FocusAgent
                        | Command::CloseDock
                        | Command::CommandPalette
                        | Command::Quit
                )
            );
            // Agents take Ctrl+J for a new line, so it's only ever the terminal's own.
            let shell = self.dock.session().is_some_and(|s| s.shell);
            let leaves = leaves || (shell && command == Some(Command::ToggleTerminal));
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
                    self.neovim_line = vim.command_line().unwrap_or_default().to_string();
                    for command in commands {
                        self.run(command);
                    }
                    return;
                }
                VimAction::Pending => {
                    self.neovim_line = vim.command_line().unwrap_or_default().to_string();
                    return;
                }
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
            if c.is_alphanumeric() || c == '_' || self.reference_slot().is_some() {
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
                "w" | "w!" | "up" | "update" | "wa" | "wall" => Some(vec![Command::Save]),
                "q" | "q!" | "close" | "bd" | "bd!" | "bw" | "bw!" | "bdelete" | "bwipeout" => {
                    Some(vec![Command::CloseTab])
                }
                "wq" | "wq!" | "x" | "x!" | "xit" | "exit" => {
                    Some(vec![Command::Save, Command::CloseTab])
                }
                "qa" | "qa!" | "qall" | "quitall" | "wqa" | "xa" => Some(vec![Command::Quit]),
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
        // `ZZ` and `ZQ` would end Neovim: they save and close, or close, the tab instead.
        if self.neovim_mode == "NORMAL" && self.neovim_line.is_empty() {
            let z = self.neovim_z;
            self.neovim_z = false;
            match (z, key.key) {
                (false, Key::Char('Z')) if !key.mods.ctrl && !key.mods.alt => {
                    self.neovim_z = true;
                    return true;
                }
                (true, Key::Char('Z')) => {
                    self.run(Command::Save);
                    self.run(Command::CloseTab);
                    return true;
                }
                (true, Key::Char('Q')) => {
                    self.run(Command::CloseTab);
                    return true;
                }
                (true, _) => {
                    if let Some(neovim) = &mut self.neovim {
                        neovim.input("Z");
                    }
                }
                _ => {}
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
        if state.strayed {
            self.neovim_mode = "NORMAL".to_string();
            self.neovim_line.clear();
            // Neovim may have emptied or wiped the buffer on its way out, so the next key
            // shows the file again and sends its text.
            self.neovim_file = None;
            self.say(
                "Open files with Ctrl+P and split with Ctrl+\\: Neovim here edits the file in view",
            );
            return;
        }
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
            Leaving::Quit => self.quit_now(),
            Leaving::CloseTab => self.close_tab(),
        }
    }

    fn choose(&mut self, choice: Choice) {
        match choice {
            Choice::Cancel => {}
            Choice::Overwrite => self.write_file(),
            Choice::Reload => {
                if let Some(editor) = self.editor_mut() {
                    editor.conflict = false;
                    let reloaded = editor.reload();
                    if let Err(e) = reloaded {
                        self.say(format!("Could not reload: {e}"));
                    }
                }
            }
            Choice::Stop => {
                self.stop_confirmed = true;
                self.run(Command::Quit);
            }
            Choice::ExportPdf(accepted) => self.export_pdf_with(accepted),
            Choice::DiscardFile(path, untracked) => {
                let root = self.workspace.root().to_path_buf();
                if let Err(why) = cellix_review::discard(&root, &path, untracked) {
                    self.say(format!("Git refused: {why}"));
                }
                self.source_settled();
            }
            Choice::KeepRunning => {
                self.keep_running = true;
                self.stop_confirmed = true;
                self.run(Command::Quit);
            }
            Choice::CloseSession(index) => self.dock.close(index),
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
        let mut searched = false;
        match key.key {
            Key::Esc => {
                if let Some(editor) = self.editor_mut() {
                    editor.found.clear();
                }
                return;
            }
            Key::Char(c) if key.mods.alt && find.options.flip_key(c) => searched = true,
            Key::Char('r' | 'R') if key.mods.ctrl => {
                find.replace = Some(find.replace.take().unwrap_or_default());
                find.in_replace = !find.query.is_empty();
            }
            Key::Tab | Key::BackTab if find.replace.is_some() => find.in_replace = !find.in_replace,
            Key::Enter if find.in_replace && key.mods.alt => {
                self.overlay = Overlay::Find(find);
                self.replace_in_file(true);
                return;
            }
            Key::Enter if find.in_replace => {
                self.overlay = Overlay::Find(find);
                self.replace_in_file(false);
                return;
            }
            Key::Enter if key.mods.shift => find.step(-1),
            Key::Enter | Key::Down => find.step(1),
            Key::Up => find.step(-1),
            Key::Backspace => {
                find.field_mut().pop();
                searched = !find.in_replace;
            }
            Key::Char(c) if !key.mods.ctrl && !key.mods.alt => {
                find.field_mut().push(c);
                searched = !find.in_replace;
            }
            _ => {}
        }
        self.overlay = Overlay::Find(find);
        if searched {
            self.search_file();
        } else {
            self.show_match();
        }
    }

    /// Replaces the match in view and moves to the next, or replaces every one, as one edit.
    fn replace_in_file(&mut self, all: bool) {
        let Overlay::Find(find) = &self.overlay else {
            return;
        };
        let with = find.replace.clone().unwrap_or_default();
        let (options, selected, count) = (find.options, find.selected(), find.matches.len());
        let Ok(matcher) = options.matcher(&find.query) else {
            return;
        };
        let area = self.focused_area();
        let Some(editor) = self.editor_mut() else {
            return;
        };
        editor.prepare_edit(area);
        let text = editor.buffer.text();
        // With changes tracked, each replacement is a suggestion like any edit.
        let tracking = editor.tracking.then(|| editor.change_syntax());
        let from = if all {
            let replaced = match tracking {
                Some(syntax) => {
                    let mut out = String::with_capacity(text.len());
                    let mut last = 0;
                    for hit in matcher.find_iter(&text) {
                        let found = hit.as_str();
                        let new = options.replacement(&matcher, found, &with);
                        out.push_str(&text[last..hit.start()]);
                        out.push_str(&cellix_doc::changes::mark(found, &new, syntax));
                        last = hit.end();
                    }
                    out.push_str(&text[last..]);
                    out
                }
                None => options.replace_all(&matcher, &text, &with),
            };
            if replaced != text {
                editor.selections.set_single(Selection {
                    anchor: 0,
                    head: editor.buffer.rope().len_chars(),
                    goal: None,
                });
                editor.buffer.replace(&mut editor.selections, &replaced);
            }
            0
        } else {
            let Some((start, end)) = selected else {
                return;
            };
            let found = editor.buffer.rope().slice(start..end).to_string();
            let replacement = options.replacement(&matcher, &found, &with);
            let replacement = match tracking {
                Some(syntax) => cellix_doc::changes::mark(&found, &replacement, syntax),
                None => replacement,
            };
            editor.selections.set_single(Selection {
                anchor: start,
                head: end,
                goal: None,
            });
            editor.buffer.replace(&mut editor.selections, &replacement);
            start + replacement.chars().count()
        };
        let text = editor.buffer.text();
        if let Overlay::Find(find) = &mut self.overlay {
            find.search(&text, from);
        }
        if all {
            self.say(format!("Replaced {count}"));
        }
        self.show_match();
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
        let replacing = matches!(&self.overlay, Overlay::Find(f) if f.replace.is_some());
        self.find_bar_for(replacing)
    }

    /// The Find bar, a row taller with the replace row.
    fn find_bar_for(&self, replacing: bool) -> Rect {
        let pane = self.focused_area();
        let width = FIND_WIDTH.min(pane.w);
        let x = pane.right().saturating_sub(width + 1).max(pane.x);
        // Just its rows: the panel is painted around them, so it's no taller than it needs.
        Rect::new(x, pane.y + 1, width, if replacing { 2 } else { 1 })
    }

    /// The Find bar's parts: the field, where the switches start inside it, and where the
    /// count and then the buttons start. A chevron stands before the field, and the replace
    /// row's field is as wide, with its two buttons straight after.
    fn find_layout(area: Rect) -> (Rect, u16, u16, u16) {
        let first = area.right() - FIND_BUTTON * FIND_BUTTONS.len() as u16 - 1;
        let count = first.saturating_sub(FIND_COUNT);
        let left = area.x + 1 + FIND_CHEVRON;
        let field = Rect::new(left, area.y, count.saturating_sub(left + 1), 1);
        let switches = field.right().saturating_sub(SWITCHES_WIDTH + 1);
        (field, switches, count, first)
    }

    /// One of the Find bar's icon buttons, with a wash under the pointer and a lit chip when on.
    fn find_button(&self, grid: &mut Grid, cell: Rect, icon: Icon, lit: bool) {
        let theme = &self.theme;
        let hovered = self.hover.is_some_and(|h| cell.contains(h.0, h.1));
        let chip = Bounds::inset(cell, 0.2, 0.15);
        if lit {
            grid.panel(chip, 14, Some(theme.pick), Some(theme.focus));
        } else if hovered {
            grid.panel(chip, 14, Some(theme.popup_hover), None);
        }
        let base = Style {
            fg: theme.text.fg,
            bg: if lit { theme.pick } else { theme.popup },
            ..theme.bar
        };
        toolbar::draw_icon(grid, (icon, 0.5), cell, theme.text.fg, base);
    }

    fn draw_find(&self, grid: &mut Grid, find: &Find) {
        let theme = &self.theme;
        let area = self.find_bar_for(find.replace.is_some());
        crate::palette::raised(grid, area, theme);
        let (line, switches, count_x, first) = Self::find_layout(area);
        let chevron = if find.replace.is_some() {
            Icon::ChevronDown
        } else {
            Icon::ChevronRight
        };
        let toggle = Rect::new(area.x + 1, area.y, FIND_CHEVRON, 1);
        self.find_button(grid, toggle, chevron, false);
        let text = Input {
            text: &find.query,
            hint: "Find",
            focused: !find.in_replace,
            behind: theme.popup,
        };
        input(grid, line, &text, &*self.measure, theme);
        let base = Style {
            fg: theme.gutter.fg,
            bg: theme.popup,
            ..theme.bar
        };
        // Inside the field's end, as VS Code has them.
        crate::search::draw_switches(
            grid,
            (switches, area.y),
            find.options,
            self.hover,
            Style {
                bg: theme.input,
                ..base
            },
            theme,
        );
        grid.label(count_x + 1, area.y, &find.count_label(), base, first);
        let mut x = first;
        for icon in FIND_BUTTONS {
            self.find_button(grid, Rect::new(x, area.y, FIND_BUTTON, 1), icon, false);
            x += FIND_BUTTON;
        }
        if let Some(with) = &find.replace {
            let row = Rect::new(line.x, area.y + 1, line.w, 1);
            let text = Input {
                text: with,
                hint: "Replace",
                focused: find.in_replace,
                behind: theme.popup,
            };
            input(grid, row, &text, &*self.measure, theme);
            let mut x = row.right() + 1;
            for icon in [Icon::Replace, Icon::ReplaceAll] {
                self.find_button(grid, Rect::new(x, area.y + 1, FIND_BUTTON, 1), icon, false);
                x += FIND_BUTTON;
            }
        }
    }

    fn click_find(&mut self, mut find: Find, x: u16, y: u16) {
        let area = self.find_bar_for(find.replace.is_some());
        let (field, switches, _, first) = Self::find_layout(area);
        let toggle = Rect::new(area.x + 1, area.y, FIND_CHEVRON, 1);
        if toggle.contains(x, y) {
            find.replace = match find.replace {
                Some(_) => None,
                None => Some(String::new()),
            };
            find.in_replace = find.replace.is_some() && !find.query.is_empty();
            self.overlay = Overlay::Find(find);
            return;
        }
        if find.replace.is_some() && y == area.y + 1 && area.contains(x, y) {
            let buttons = field.right() + 1;
            find.in_replace = true;
            self.overlay = Overlay::Find(find);
            if x >= buttons && x < buttons + 2 * FIND_BUTTON {
                self.replace_in_file(x >= buttons + FIND_BUTTON);
            }
            return;
        }
        if y == area.y && x >= switches && x < switches + SWITCHES_WIDTH {
            find.options.flip(((x - switches) / 3) as usize);
            self.overlay = Overlay::Find(find);
            self.search_file();
            return;
        }
        let key = if area.contains(x, y) && x >= first && y == area.y {
            match (x - first) / FIND_BUTTON {
                0 => Key::Up,
                1 => Key::Down,
                _ => Key::Esc,
            }
        } else {
            if area.contains(x, y) && y == area.y {
                find.in_replace = false;
            }
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
            Some(Item::Image(path, _, replacing)) => self.place_image(&path, replacing),
            Some(Item::Citation(key, _)) => {
                let area = self.focused_area();
                if let Some(editor) = self.editor_mut() {
                    editor.prepare_edit(area);
                    editor.insert_text(&format!("\\cite{{{key}}}"));
                }
            }
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
            PromptAction::Link => {
                let area = self.focused_area();
                if let Some(editor) = self.editor_mut().filter(|_| !name.is_empty()) {
                    editor.prepare_edit(area);
                    editor.make_link(name);
                }
                Ok(())
            }
            PromptAction::Image => {
                let area = self.focused_area();
                if let Some(editor) = self.editor_mut() {
                    editor.prepare_edit(area);
                    editor.insert_image(name);
                }
                Ok(())
            }
            PromptAction::GoToLine => {
                self.mark_jump();
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
            PromptAction::RejectHunk(hunk) => {
                let hunk = *hunk;
                self.note_rejection(hunk, name);
                self.reject(hunk);
                Ok(())
            }
            PromptAction::RejectProposal => {
                self.note_rejection(0, name);
                self.reject_proposal();
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
        // The sidebar stands behind anything maximized, so asking for it restores first.
        let sidebar = matches!(
            command,
            Command::ToggleSidebar
                | Command::FindInProject
                | Command::ReplaceInProject
                | Command::ShowOutline
                | Command::ShowSourceControl
        );
        if sidebar && self.maximized.take().is_some() {
            self.fit_dock();
            if command == Command::ToggleSidebar {
                self.show_sidebar = true;
                return;
            }
        }
        let area = self.focused_area();
        match command {
            Command::Quit if !self.stop_confirmed && !self.dock.busy().is_empty() => {
                let names = self.dock.busy().join(", ");
                let can_keep = self.dock.sessions.iter().any(Session::can_outlive);
                let mut buttons = vec![("Cancel", Choice::Cancel)];
                if can_keep {
                    buttons.push(("Keep Running", Choice::KeepRunning));
                }
                buttons.push(("Stop and Quit", Choice::Stop));
                self.overlay = Overlay::Confirm(Confirm {
                    title: "Stop what's running?".to_string(),
                    detail: if can_keep {
                        format!("{names} can keep running, and be here next time you open this.")
                    } else {
                        format!("{names} will stop if you quit.")
                    },
                    buttons,
                    selected: 0,
                });
            }
            Command::Quit if self.editors.iter().any(|e| e.buffer.is_dirty()) => {
                self.confirm_leaving(Leaving::Quit);
            }
            Command::Quit => self.quit_now(),
            // Settings shows as the only tab, so closing a tab closes it, not the file behind.
            Command::CloseTab if self.settings_page.is_some() => self.settings_page = None,
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
                    None => {
                        let mut review = self.new_review(self.workspace.root().to_path_buf());
                        review.beside_panel =
                            self.show_sidebar && self.sidebar_view == SidebarView::Source;
                        Some(review)
                    }
                };
            }
            Command::AddComment => self.prompt_comment_at_cursor(false),
            Command::FindInFile => self.open_find(),
            Command::NextChange => self.step_change(true),
            Command::PreviousChange => self.step_change(false),
            Command::NextProblem => self.step_problem(true),
            Command::PreviousProblem => self.step_problem(false),
            Command::GoBack => self.travel(true),
            Command::GoForward => self.travel(false),
            Command::ReplaceInFile => {
                self.open_find();
                if let Overlay::Find(find) = &mut self.overlay {
                    find.in_replace = !find.query.is_empty();
                    find.replace = Some(String::new());
                }
            }
            Command::Format(Format::Link) => self.ask("Link address", PromptAction::Link),
            Command::Format(format) => {
                let area = self.focused_area();
                if let Some(editor) = self.editor_mut() {
                    editor.format(format, area);
                }
            }
            // On a page, with words picked, it means what it means in any writing tool.
            Command::ToggleSidebar if self.on_page() && self.selected_text().is_some() => {
                self.run(Command::Format(Format::Bold));
            }
            Command::SendToAgent => self.send_to_agent(),
            Command::ApprovePlan => {
                let path = self.plan.as_ref().map(|p| p.path.as_path());
                let edited = self
                    .editors
                    .iter()
                    .find(|e| path.is_some() && e.buffer.path() == path && e.buffer.is_dirty())
                    .map(|e| e.buffer.text());
                // Comments made before approving go with the plan, so the agent has them
                // before the work, not after it.
                let notes = path.map(|p| self.plan_notes(p)).unwrap_or_default();
                let edited = if notes.is_empty() {
                    edited
                } else {
                    let plan =
                        edited.or_else(|| path.and_then(|p| std::fs::read_to_string(p).ok()));
                    Some(format!(
                        "{}\n\n## Reviewer notes\n\n{notes}",
                        plan.unwrap_or_default().trim_end()
                    ))
                };
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
            Command::RunCell => self.run_cell(),
            Command::ClearOutputs => self.clear_all_outputs(),
            Command::RunAllCells => self.run_all_cells(),
            Command::InterruptKernel => {
                if let Some(kernel) = self.active_file().and_then(|f| self.kernels.get(&f)) {
                    kernel.interrupt();
                }
            }
            Command::RestartKernel => {
                if let Some(file) = self.active_file() {
                    self.kernels.remove(&file);
                    self.say("The kernel will start again with the next cell");
                }
                if let Some(book) = self.editor_mut().and_then(|e| e.notebook.as_mut()) {
                    book.running.clear();
                }
            }
            Command::ShowGuide => {
                let folder = self.built_folder.with_file_name("guide");
                let opened = crate::guide::write(&folder).and_then(|path| self.open(&path));
                if let Err(e) = opened {
                    self.say(format!("Could not open the guide: {e}"));
                }
            }
            Command::NewDocument => {
                let root = self.workspace.root().to_path_buf();
                self.ask(
                    "New document, such as notes.md",
                    PromptAction::NewFile(root),
                );
            }
            Command::ToggleSpelling => {
                self.spell_on = !self.spell_on;
                let on = self.spell_on;
                for editor in &mut self.editors {
                    editor.issues.1.clear();
                    editor.checked = 0;
                }
                let _ = Settings::store("spell_check", json!(on));
                self.say(if on {
                    "Spell check is on"
                } else {
                    "Spell check is off"
                });
            }
            Command::ToggleTracking => {
                if let Some(editor) = self.editor_mut() {
                    let saved = !editor.buffer.is_dirty();
                    let on = editor.toggle_tracking();
                    // A paper gains `\\usepackage{changes}`; saved before, saved after, so an
                    // agent's next edit still arrives as suggestions.
                    if saved && editor.buffer.is_dirty() {
                        let _ = editor.buffer.save();
                    }
                    let path = editor.buffer.path().map(Path::to_path_buf);
                    // A file the paper takes in has no preamble: the package goes in its root's.
                    let text = editor.buffer.text();
                    let latex = editor.dialect() == Dialect::Latex;
                    if let Some(path) = path.as_ref().filter(|_| on && latex) {
                        let root = cellix_doc::latex::root_of(path, &text);
                        if root != *path {
                            self.track_in_root(&root);
                        }
                    }
                    if let Some(file) = path.map(|p| self.relative(&p)) {
                        let mut tracked = self.tracked_files();
                        tracked.retain(|f| *f != file);
                        if on {
                            tracked.push(file);
                        }
                        self.keep_tracked_files(&tracked);
                    }
                    self.say(if on {
                        "Track changes is on: edits are kept as suggestions"
                    } else {
                        "Track changes is off"
                    });
                }
            }
            Command::AcceptChange | Command::RejectChange => {
                let accept = command == Command::AcceptChange;
                if let Some(editor) = self.editor_mut() {
                    editor.settle_change(accept);
                }
            }
            Command::AcceptAllChanges | Command::RejectAllChanges => {
                let accept = command == Command::AcceptAllChanges;
                if let Some(editor) = self.editor_mut() {
                    let count = editor.settle_all_changes(accept);
                    let done = if accept { "Accepted" } else { "Rejected" };
                    self.say(format!("{done} {count} changes"));
                }
            }
            Command::ShowAgents => self.toggle_dock(),
            Command::ToggleTerminal => self.toggle_terminal(),
            Command::NewTerminal => self.open_agent(crate::agents::SHELL.1),
            Command::ShowBrowser => self.show_browser(),
            Command::ShowComments if self.dock.showing(Front::Comments) => {
                self.dock.close_view(Front::Comments);
            }
            Command::ShowComments => {
                self.dock.open = true;
                self.dock.show_comments();
                self.dock.focused = false;
            }
            Command::ListAgents if self.dock.showing(Front::Agents) => {
                self.dock.close_view(Front::Agents);
            }
            Command::ListAgents => {
                self.dock.open = true;
                self.refresh_agents();
                self.dock.show_list();
            }
            // Hiding stops nothing: what runs here keeps running, and the pane comes back.
            Command::CloseDock => {
                self.dock.open = false;
                self.dock.focused = false;
            }
            Command::BuildPdf => self.build_pdf(),
            Command::ExportPdf => self.export_pdf(),
            Command::ComparePlanVersions => self.compare_plan_versions(),
            Command::ShowInPdf => {
                let place = self.cursor_line();
                let shown = match (place, &mut self.dock.pdf) {
                    (Some((path, line)), Some(view)) => view.show_source(&path, line),
                    _ => false,
                };
                if shown {
                    self.dock.open = true;
                    self.dock.front = Front::Pdf;
                } else {
                    self.say("Build the PDF first, and this line will show in it");
                }
            }
            Command::FocusAgent => {
                let wanted = !self.dock.focused;
                self.dock.open |= wanted;
                self.dock.focused = wanted && self.dock.session().is_some();
            }
            Command::OpenGemini => self.open_agent("gemini"),
            Command::MaximizePane => self.toggle_maximized(Maximized::Pane),
            Command::MaximizeEditor => self.toggle_maximized(Maximized::Editor),
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
            Command::GoToDefinition | Command::GoToReferences | Command::QuickFix => {
                self.lsp_request(command);
            }
            Command::ShowHover => {
                let area = self.focused_area();
                let caret = self
                    .editors
                    .get(self.active())
                    .and_then(|e| e.caret_cell(area));
                self.hover_at = caret.map(|(x, y)| (x as u16, y));
                self.lsp_request(command);
            }
            Command::FormatDocument => self.format_document(),
            Command::Complete => {
                self.suggest_asked = true;
                self.poll();
                self.refresh_suggestions();
            }
            Command::GoToLine => self.ask("Go to line", PromptAction::GoToLine),
            Command::RenameSymbol => self.ask("Rename to", PromptAction::RenameSymbol),
            Command::BlameLine => self.blame(),
            Command::ToggleSidebar => self.toggle_sidebar(),
            Command::ShowSourceControl if self.rail_on(Command::ShowSourceControl) => {
                self.hide_sidebar();
            }
            Command::ShowSourceControl => {
                self.search.focus = None;
                self.review = None;
                self.sidebar_view = SidebarView::Source;
                self.show_sidebar = true;
                self.drawer = self.compact();
                self.source_age = 0;
                self.source.refresh(self.workspace.root());
            }
            Command::ShowOutline if self.rail_on(Command::ShowOutline) => {
                self.hide_sidebar();
            }
            Command::ShowOutline => {
                self.search.focus = None;
                self.review = None;
                self.sidebar_view = SidebarView::Outline;
                self.show_sidebar = true;
                self.drawer = self.compact();
            }
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
                    editor.cut_selection();
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
        let built = self.built_folder.clone();
        let picture = move |body: &str| built_as_html(&built, body);
        let page = cellix_doc::html::page_with(
            &title,
            &editor.buffer.text(),
            base.as_deref().unwrap_or_default(),
            &picture,
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

    /// The command that shows a page inside Cellix, if this terminal can show one. A browser
    /// named in the settings is someone's choice, so it comes first.
    fn pane_browser(&self) -> Option<Vec<String>> {
        let chosen = self.settings.browser.as_ref().and_then(|c| c.first());
        let wanted = chosen.is_none_or(|program| program.ends_with("terminal-browser"));
        self.measure.cell_pixels().and(wanted.then(|| {
            let program = chosen.map_or("terminal-browser", String::as_str);
            vec![program.to_string(), "open".into(), "--no-merge".into()]
        }))
    }

    /// Opens a page in the side pane, in place of any page already there.
    fn open_in_pane(&mut self, target: &str) -> bool {
        let Some(mut argv) = self.pane_browser() else {
            return false;
        };
        let var = |name: &str| std::env::var(name).ok();
        let found = Path::new(&argv[0]).is_file() || crate::integrations::on_path(&argv[0], var);
        if !found {
            return false;
        }
        argv.push(target.to_string());
        let old = self.dock.sessions.iter().position(|s| s.browser);
        if let Some(index) = old {
            self.dock.close(index);
        }
        let root = self.workspace.root().to_path_buf();
        self.start_session(BROWSER, &argv, &[], &root);
        self.dock.front == Front::Session
            && self.dock.session().is_some_and(|session| session.browser)
    }

    fn open_in_browser(&mut self) {
        let Some(target) = self.browser_target() else {
            self.say("Nothing to open: put the cursor on a link, or open an HTML file");
            return;
        };
        if self.open_in_pane(&target) {
            return;
        }
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
        if let Some(note) = self
            .browser_install
            .as_ref()
            .and_then(|r| r.try_recv().ok())
        {
            self.browser_install = None;
            self.browser_offer = false;
            let installed = note.starts_with("Installed");
            self.say(note);
            if installed {
                self.show_browser();
            }
            return true;
        }
        let opened = self.browser_result.as_ref().and_then(|(target, result)| {
            let opened = result.try_recv().ok()?;
            Some((target.clone(), opened))
        });
        let Some((target, opened)) = opened else {
            return false;
        };
        self.browser_result = None;
        let mut parts = target.rsplit(['/', std::path::MAIN_SEPARATOR]);
        let name = parts.next().unwrap_or(&target).to_string();
        if !opened && self.offer_browser(browser_installed()) {
            return true;
        }
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
            .and_then(|exe| exe.parent().map(Path::to_path_buf));
        if let (Some(folder), Some(path)) = (own_folder, std::env::var_os("PATH"))
            && let Some(path) = path_with(&folder, &path)
        {
            env.push(format!("PATH={path}"));
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
        match self.maximized {
            Some(Maximized::Editor) => return None,
            Some(Maximized::Pane) if self.dock.open => {
                return Some(Rect::new(left, 0, self.size.0.saturating_sub(left), height));
            }
            _ => {}
        }
        if self.dock_under() {
            let rows = self.dock.height.unwrap_or(height * STACK_SHARE / 100);
            let rows = rows.clamp(STACK_LEAST, height - STACK_LEAST);
            let room = self.size.0.saturating_sub(left);
            return Some(Rect::new(left, height - rows, room, rows));
        }
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
        let shell = crate::agents::shell();
        let (name, env, launch) = if agent == crate::agents::SHELL.1 {
            (crate::agents::SHELL.0, Vec::new(), vec![shell])
        } else {
            let (env, launch) = self.agent_launch(agent);
            (crate::agents::display_name(agent), env, launch)
        };
        self.start_session(name, &launch, &env, &root);
    }

    /// The first time something starts, says how the two sides work together.
    fn explain_pane(&mut self, name: &str) {
        if !std::mem::replace(&mut self.pane_explained, true) {
            self.say(format!(
                "{name} is on the right. Click it to type there, or press Ctrl+` to switch"
            ));
        }
    }

    fn start_session(&mut self, name: &str, argv: &[String], env: &[String], folder: &Path) {
        self.dock.open = true;
        let size = self.dock_screen().map_or((80, 24), |a| (a.w, a.h));
        // A browser has nothing worth keeping, and nothing else should end with Cellix.
        let held = self
            .host_program
            .clone()
            .filter(|_| name != BROWSER)
            .and_then(|program| {
                let cell = self.cell_size();
                let job = crate::hold::Job {
                    name: name.to_string(),
                    argv: argv.to_vec(),
                    env: env.to_vec(),
                    folder: folder.to_path_buf(),
                    size,
                    pixels: (size.0.saturating_mul(cell.0), size.1.saturating_mul(cell.1)),
                };
                let sessions = crate::hold::folder_of(self.workspace.root());
                crate::hold::start(&program, &sessions, &job).ok()
            });
        let started = match held {
            Some(held) => Session::join(name, held, size, self.cell_size()),
            None => Session::start(name, argv, env, folder, size, self.cell_size()),
        };
        match started {
            Ok(session) => {
                self.dock.sessions.push(session);
                self.dock.active = Some(self.dock.sessions.len() - 1);
                self.dock.focused = true;
                self.dock.front = Front::Session;
                self.dock.agents_tab = false;
                if name != BROWSER {
                    self.explain_pane(name);
                }
            }
            Err(e) => self.say(format!("Could not start {name}: {e}")),
        }
    }

    /// Where a panel goes in the pane: from its edge, so its text lines up with the tabs.
    fn dock_panel(&self) -> Option<Rect> {
        let area = self.dock_area()?;
        Some(Rect::new(
            area.x,
            area.y + 1,
            area.w,
            area.h.saturating_sub(1),
        ))
    }

    /// Quits. What's in the side pane stops with it unless it was asked to stay.
    fn quit_now(&mut self) {
        if !self.keep_running {
            self.dock.stop_all();
        }
        self.quit = true;
    }

    /// Picks up what the last Cellix here left running, in tabs of the side pane.
    pub fn rejoin_sessions(&mut self) {
        let folder = crate::hold::folder_of(self.workspace.root());
        let mut names = Vec::new();
        for (name, socket) in crate::hold::left_running(&folder) {
            let Ok(held) = crate::hold::Held::connect(&socket) else {
                continue;
            };
            let size = self.dock_screen().map_or((80, 24), |a| (a.w, a.h));
            if let Ok(session) = Session::join(&name, held, size, self.cell_size()) {
                self.dock.sessions.push(session);
                names.push(name);
            }
        }
        if !names.is_empty() {
            self.dock.active = Some(self.dock.sessions.len() - 1);
            self.dock.open = true;
            self.dock.front = Front::Session;
            self.fit_dock();
            let names = names.join(", ");
            self.say(format!("{names} kept running while you were away"));
        }
    }

    /// Pixels in a cell, for programs that draw pictures.
    fn cell_size(&self) -> CellPixels {
        let cell = self.measure.cell_pixels().unwrap_or_default();
        (cell.0 as u16, cell.1 as u16)
    }

    /// Keeps each session as big as the pane it's in.
    fn fit_dock(&mut self) {
        if let Some(screen) = self.dock_screen() {
            let cell = self.cell_size();
            for session in &mut self.dock.sessions {
                session.resize((screen.w, screen.h), cell);
            }
            if let (Some(view), Some(cell)) = (&mut self.dock.pdf, self.measure.cell_pixels()) {
                view.fit((screen.w, screen.h), cell);
            }
        }
    }

    /// Opens or closes the side pane. With nothing running it shows what can be started.
    fn toggle_dock(&mut self) {
        self.dock.open = !self.dock.open;
        self.dock.focused = self.dock.open && self.dock.session().is_some();
        if self.dock.open {
            self.refresh_agents();
        }
    }

    /// Finds out what can be started and what's running.
    fn refresh_agents(&mut self) {
        self.agents.worktrees = worktrees(self.workspace.root());
        self.agents.look_for_agents();
        self.agents.scan(self.workspace.root());
    }

    /// Whether the pointer is on the line between the file and the pane. It's easy to miss,
    /// so the row or column before it counts, and so does an empty stretch of the tab row.
    fn on_dock_edge(&self, x: u16, y: u16) -> bool {
        let Some(area) = self.dock_area() else {
            return false;
        };
        if self.dock_under() {
            let empty_title = y == area.y && self.dock.hit(area, x).is_none();
            let on_strip = self.action_at(x, y).is_some();
            return !on_strip && x >= area.x && (y + 1 == area.y || empty_title);
        }
        (x == area.x || x + 1 == area.x) && y > area.y && y < area.bottom()
    }

    /// Asks to close a tab of the pane, checking first if that would stop something.
    fn close_session_tab(&mut self, index: usize) {
        if !self.dock.sessions[index].busy() {
            self.dock.close(index);
            return;
        }
        let name = self.dock.sessions[index].name.clone();
        self.overlay = Overlay::Confirm(Confirm {
            title: format!("Stop {name}?"),
            detail: "What it's doing will stop.".to_string(),
            buttons: vec![
                ("Cancel", Choice::Cancel),
                ("Stop", Choice::CloseSession(index)),
            ],
            selected: 0,
        });
    }

    /// Everything the pane does with the pointer. Returns true if it took the event.
    fn dock_mouse(&mut self, mouse: MouseEvent) -> bool {
        let Some(area) = self.dock_area() else {
            return false;
        };
        let (x, y) = (mouse.x, mouse.y);
        let dragging = matches!(self.drag, Drag::Dock | Drag::Session);
        if mouse.kind == MouseKind::Drag && self.drag == Drag::Dock {
            if self.dock_under() {
                self.dock.height = Some(self.size.1.saturating_sub(y + 1));
            } else {
                self.dock.width = Some(self.size.0.saturating_sub(x));
            }
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
        let in_preview = matches!(self.drag, Drag::Preview | Drag::PreviewBar(_));
        if in_preview && matches!(mouse.kind, MouseKind::Drag | MouseKind::Up) {
            self.preview_mouse(mouse);
            return true;
        }
        // The line between the two sides is easy to miss, so the column before it counts.
        let on_edge = self.on_dock_edge(x, y);
        if on_edge && mouse.kind == MouseKind::Left {
            self.drag = Drag::Dock;
            return true;
        }
        if !area.contains(x, y) {
            if mouse.kind == MouseKind::Left {
                self.dock.focused = false;
            }
            return false;
        }
        if y == area.y {
            if mouse.kind == MouseKind::Middle {
                match self.dock.hit(area, x) {
                    Some(TabHit::Tab(index) | TabHit::Close(index)) => {
                        self.close_session_tab(index);
                    }
                    Some(TabHit::View(view) | TabHit::CloseView(view)) => {
                        self.dock.close_view(view);
                    }
                    _ => {}
                }
            }
            if mouse.kind == MouseKind::Left {
                let hit = self.dock.hit(area, x);
                let on_tab = matches!(hit, Some(TabHit::Tab(_) | TabHit::View(_)));
                if on_tab && self.count_click(x, y) == 2 {
                    self.toggle_maximized(Maximized::Pane);
                    return true;
                }
                match hit {
                    Some(TabHit::Tab(index)) => self.press_tab(Dragged::Session(index), x, y),
                    Some(TabHit::View(view)) => self.press_tab(Dragged::View(view), x, y),
                    _ => {}
                }
                match hit {
                    Some(TabHit::Maximize) => self.toggle_maximized(Maximized::Pane),
                    Some(TabHit::Hide) => self.run(Command::CloseDock),
                    Some(TabHit::Tab(index)) => {
                        self.dock.active = Some(index);
                        self.dock.focused = true;
                        self.dock.front = Front::Session;
                    }
                    Some(TabHit::Close(index)) => self.close_session_tab(index),
                    Some(TabHit::New | TabHit::View(Front::Agents)) => {
                        self.refresh_agents();
                        self.dock.show_list();
                    }
                    Some(TabHit::View(view)) => {
                        self.dock.front = view;
                        self.dock.focused = false;
                    }
                    Some(TabHit::CloseView(view)) => self.dock.close_view(view),
                    None => {}
                }
            }
            return true;
        }
        if self.dock.front == Front::Problems {
            match mouse.kind {
                MouseKind::ScrollUp => {
                    self.dock.problems_top = self.dock.problems_top.saturating_sub(3);
                }
                MouseKind::ScrollDown => self.dock.problems_top += 3,
                MouseKind::Left => self.click_problem(y),
                _ => {}
            }
            return true;
        }
        if self.dock.front == Front::Log {
            match mouse.kind {
                MouseKind::ScrollUp => self.dock.log_top = self.dock.log_top.saturating_sub(3),
                MouseKind::ScrollDown => self.dock.log_top += 3,
                MouseKind::Left => self.log_click(y),
                _ => {}
            }
            return true;
        }
        if self.dock.front == Front::Pdf {
            self.preview_mouse(mouse);
            return true;
        }
        if self.dock.front == Front::Comments {
            let screen = self.dock_panel().unwrap_or(area);
            match mouse.kind {
                MouseKind::Left => {
                    let threads = self.open_threads();
                    let agent = self.dock.agent_name();
                    let threads = (threads.as_slice(), self.dock.draft.as_ref(), agent.as_str());
                    let spot = (x, y);
                    let hit = threads::hit(
                        screen,
                        threads,
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
        if self.dock.listing() {
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
            MouseKind::Left if mouse.mods.ctrl => self.open_session_path(x, y),
            MouseKind::Left => {
                self.dock.focused = true;
                self.drag = Drag::Session;
                self.dock_session_mouse(mouse);
            }
            _ => {}
        }
        true
    }

    /// The pointer in the Preview. A press on the pages pans once it moves, and goes to the
    /// source when it's let go where it started; the wheel tipped sideways, or with Shift,
    /// moves across.
    fn preview_mouse(&mut self, mouse: MouseEvent) {
        let (Some(screen), Some(view)) = (self.dock_screen(), self.dock.pdf.as_mut()) else {
            return;
        };
        let (x, y) = (mouse.x, mouse.y);
        let wide = view.wide(screen.w);
        let sideways = match (mouse.kind, mouse.mods.shift) {
            (MouseKind::ScrollUp, true) | (MouseKind::ScrollLeft, _) => -crate::pdfview::SIDEWAYS,
            (MouseKind::ScrollDown, true) | (MouseKind::ScrollRight, _) => crate::pdfview::SIDEWAYS,
            _ => 0.0,
        };
        match (mouse.kind, self.drag) {
            (MouseKind::ScrollUp | MouseKind::ScrollDown, _) if sideways == 0.0 => {
                let rows = if mouse.kind == MouseKind::ScrollUp {
                    -3.0
                } else {
                    3.0
                };
                view.pan(0.0, rows);
            }
            (MouseKind::ScrollUp | MouseKind::ScrollDown | MouseKind::ScrollLeft, _)
            | (MouseKind::ScrollRight, _) => {
                view.pan(sideways, 0.0);
                self.dock.pan_hint_done |= wide;
            }
            (MouseKind::Left, _) => {
                // The keyboard comes here too, so a Space held for a drag types nothing.
                self.dock.focused = true;
                match view.hit(screen, x, y) {
                    Some(zoom @ (PdfHit::ZoomIn | PdfHit::ZoomOut | PdfHit::Fit)) => {
                        view.zoom(zoom);
                        // Still zooming, the hint stays as long again.
                        if self.dock.pan_hint.is_some() {
                            self.dock.pan_hint = Some(std::time::Instant::now());
                        }
                    }
                    Some(bar @ (PdfHit::Sideways | PdfHit::Upright)) => {
                        view.follow_bar(bar, screen, mouse.fine_x, y);
                        self.drag = Drag::PreviewBar(bar);
                    }
                    _ if y > screen.y => {
                        self.preview_press = Some(PreviewPress {
                            x,
                            y,
                            fine_x: mouse.fine_x,
                            left: view.left,
                            top: view.top,
                            panning: false,
                        });
                        self.drag = Drag::Preview;
                    }
                    _ => {}
                }
            }
            (MouseKind::Drag, Drag::PreviewBar(bar)) => {
                view.follow_bar(bar, screen, mouse.fine_x, y);
            }
            (MouseKind::Drag, Drag::Preview) => {
                let Some(press) = &mut self.preview_press else {
                    return;
                };
                let across = mouse.fine_x - press.fine_x;
                let down = f32::from(y) - f32::from(press.y);
                press.panning |= across.abs() >= PAN_START || y != press.y;
                if press.panning {
                    view.place(press.left - across, press.top - down);
                    self.dock.pan_hint_done |= wide;
                }
            }
            (MouseKind::Up, _) => {
                let was = std::mem::replace(&mut self.drag, Drag::None);
                let press = self.preview_press.take();
                if let (Drag::Preview, Some(press)) = (was, press)
                    && !press.panning
                {
                    self.pdf_click(press.x, press.y);
                }
            }
            (MouseKind::Right, _) => {
                self.note_view_spot(x, y);
                self.comment_on_view();
            }
            _ => {}
        }
    }

    /// Keys while the Preview has the keyboard. The arrows move the pages, Esc gives the
    /// keyboard back to the file, and Space and other plain keys do nothing, so they never
    /// land in the file or a session out of sight. Returns false for keys that work anywhere.
    fn preview_key(&mut self, key: KeyEvent) -> bool {
        let step = match key.key {
            Key::Left => Some((-crate::pdfview::SIDEWAYS, 0.0)),
            Key::Right => Some((crate::pdfview::SIDEWAYS, 0.0)),
            Key::Up => Some((0.0, -3.0)),
            Key::Down => Some((0.0, 3.0)),
            _ => None,
        };
        if let (Some((across, down)), false) = (step, key.mods.ctrl || key.mods.alt) {
            if let Some(view) = &mut self.dock.pdf {
                view.pan(across, down);
            }
            return true;
        }
        if key.key == Key::Esc {
            self.dock.focused = false;
            return true;
        }
        // A chord means something wherever it's pressed, and it's about the file again.
        let chord = key.mods.ctrl || key.mods.alt || matches!(key.key, Key::F(_));
        if chord && simple_keymap(key).is_some() {
            self.dock.focused = false;
            return false;
        }
        true
    }

    /// Lets go of a press in the Preview, putting the pages back where they were. Returns
    /// whether there was one.
    fn cancel_drag(&mut self) -> bool {
        if self.tab_drag.take().is_some_and(|drag| drag.moving) {
            return true;
        }
        if self.drag != Drag::Preview {
            return false;
        }
        self.drag = Drag::None;
        if let (Some(press), Some(view)) = (self.preview_press.take(), &mut self.dock.pdf) {
            view.place(press.left, press.top);
        }
        true
    }

    /// Shows the hint on moving a page the first time one is wider than the pane, and takes
    /// it away a few seconds later. Once a run. Returns whether it came or went.
    fn poll_pan_hint(&mut self) -> bool {
        if self.dock.pan_hint_done {
            return false;
        }
        let wide = self.dock.showing(Front::Pdf)
            && self
                .dock_screen()
                .zip(self.dock.pdf.as_ref())
                .is_some_and(|(screen, view)| view.wide(screen.w));
        match self.dock.pan_hint {
            None if wide => {
                self.dock.pan_hint = Some(std::time::Instant::now());
                true
            }
            Some(since) if since.elapsed() >= PAN_HINT => {
                self.dock.pan_hint_done = true;
                true
            }
            _ => false,
        }
    }

    /// The hint on moving a page wider than the pane, floating over the bottom of the pages.
    fn draw_pan_hint(&self, grid: &mut Grid, screen: Rect) {
        if self.dock.pan_hint.is_none() || self.dock.pan_hint_done || self.measure.in_cells() {
            return;
        }
        let font = Font {
            size: TOOLTIP_SIZE,
            ..Font::default()
        };
        let wide = self.measure.width(PAN_HINT_TEXT, font) + 3.0;
        if wide + 2.0 > f32::from(screen.w) {
            return;
        }
        let left = f32::from(screen.x) + (f32::from(screen.w) - wide) / 2.0;
        // Clear of the scrollbar under it.
        let top = f32::from(screen.bottom()) - 1.0 - TOOLTIP_HEIGHT;
        let theme = &self.theme;
        let bounds = Bounds::cells(left, top, wide, TOOLTIP_HEIGHT);
        grid.popup(bounds, 24, theme.popup, theme.popup_border);
        let text_top = top + (TOOLTIP_HEIGHT - 1.0) / 2.0;
        let colour = (theme.popup_dim, false);
        grid.text_run(
            (left + 1.5, text_top),
            left + wide,
            PAN_HINT_TEXT,
            colour,
            font,
        );
    }

    /// A click on a page goes to the line that made it, and the bar sets the size.
    fn pdf_click(&mut self, x: u16, y: u16) {
        let screen = self.dock_screen().unwrap_or_default();
        let hit = self.dock.pdf.as_ref().and_then(|v| v.hit(screen, x, y));
        match hit {
            Some(PdfHit::Page(page, across, down)) => {
                let view = self.dock.pdf.as_ref();
                let place = view.and_then(|v| v.source_at(page, across, down));
                let image = view.is_some_and(PdfView::is_image);
                let Some((path, line)) = place else {
                    // An image has no source to go to, so a click on one says nothing.
                    if !image {
                        self.say("Build with SyncTeX to jump from the PDF to the source");
                    }
                    return;
                };
                if self.open(&path).is_ok() {
                    self.dock.focused = false;
                    let area = self.focused_area();
                    if let Some(editor) = self.editor_mut() {
                        editor.go_to_line(line, area);
                    }
                }
            }
            Some(zoom @ (PdfHit::ZoomIn | PdfHit::ZoomOut | PdfHit::Fit)) => {
                if let Some(view) = &mut self.dock.pdf {
                    view.zoom(zoom);
                }
            }
            _ => {}
        }
    }

    /// Opens the `file:line` under a Ctrl+click in a session.
    fn open_session_path(&mut self, x: u16, y: u16) {
        let spot = self.session_spot(x, y);
        let found = self.dock.session().and_then(|s| s.path_at(spot));
        let Some((file, line)) = found else {
            return;
        };
        let path = self.workspace.root().join(&file);
        if !path.is_file() {
            self.say(format!("No file called {file} here"));
            return;
        }
        if let Err(e) = self.open(&path) {
            self.say(format!("Could not open {file}: {e}"));
            return;
        }
        let area = self.focused_area();
        if let (Some(line), Some(editor)) = (line, self.editor_mut()) {
            editor.go_to_line(line, area);
        }
        self.dock.focused = false;
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
        let on_session = self.dock.front == Front::Session && self.dock.session().is_some();
        grid.fill(
            area,
            if on_session {
                theme.text
            } else {
                theme.sidebar
            },
        );
        let threads = self.open_threads();
        self.dock.comment_count.set(threads.len());
        let actions = self.pane_actions();
        let held = actions
            .first()
            .filter(|(cell, ..)| cell.x >= area.x && area.y == 0);
        let end = held.map_or(area.right().saturating_sub(PANE_CLOSE), |(cell, ..)| {
            cell.x - ACTION_MARGIN
        });
        self.dock.tabs_end.set(end);
        self.dock.tabs_flush.set(self.dock_under());
        let maximized = self.maximized == Some(Maximized::Pane);
        self.dock
            .draw_tabs(grid, area, (self.hover, maximized), theme);
        let line = if self.dock_under() {
            Bounds::cells(area.x as f32, area.y as f32, area.w as f32, 0.0)
        } else {
            Bounds::cells(area.x as f32, area.y as f32, 0.0, area.h as f32)
        };
        grid.hairline(line, theme.border);
        if let (Front::Pdf, Some(view)) = (self.dock.front, &self.dock.pdf) {
            view.draw(grid, screen, self.measure.in_cells(), theme);
            self.draw_pan_hint(grid, screen);
            return;
        }
        if self.dock.front == Front::Log {
            self.dock.draw_log(grid, screen, self.hover, theme);
            return;
        }
        if self.dock.front == Front::Problems {
            self.draw_problems(grid, screen);
            return;
        }
        if self.dock.front == Front::Comments {
            let look = (&*self.measure, theme);
            let top = self.dock.comments_top;
            let agent = self.dock.agent_name();
            let shown = (threads.as_slice(), self.dock.draft.as_ref(), agent.as_str());
            let panel = self.dock_panel().unwrap_or(screen);
            threads::draw(grid, panel, shown, top, self.hover, look);
            return;
        }
        match self.dock.session().filter(|_| !self.dock.listing()) {
            Some(session) => {
                session.draw(grid, screen, theme, self.dock.focused);
                if session.exited {
                    session.draw_ended(grid, screen, theme);
                }
            }
            None => {
                let panel = self.dock_panel().unwrap_or(screen);
                self.agents.draw(grid, panel, self.hover, theme);
            }
        }
    }

    /// The comments on a plan and any edits to it, as an answer the agent can act on.
    fn plan_feedback(&self, path: &Path, note: &str) -> String {
        let file = self.comment_file(path);
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

    /// The reviewer's comments on a plan, one line each with what they're about.
    fn plan_notes(&self, path: &Path) -> String {
        let file = self.comment_file(path);
        let comments = self
            .comments
            .lock()
            .map_or_else(|_| Vec::new(), |c| c.clone());
        let mut mine: Vec<&Comment> = comments.iter().filter(|c| c.file == file).collect();
        mine.sort_by_key(|c| c.line);
        mine.iter()
            .map(|c| format!("- {}\n", c.message()))
            .collect()
    }

    /// Keeps an approved plan with the project: the agent's copy goes when the session does,
    /// and it's what the work gets checked against.
    fn keep_plan(&self, path: &Path, decision: &Decision) -> Option<PathBuf> {
        let text = match decision {
            Decision::Approve(Some(edited)) => edited.clone(),
            _ => std::fs::read_to_string(path).unwrap_or_default(),
        };
        let folder = self.workspace.root().join(".cellix/plans");
        let count = std::fs::read_dir(&folder).map_or(0, Iterator::count);
        let kept = folder.join(format!("{:03}.md", count + 1));
        std::fs::create_dir_all(&folder)
            .and_then(|()| std::fs::write(&kept, text))
            .ok()?;
        Some(kept)
    }

    /// An approved plan's threads are settled: resolved, and kept with the plan as approved,
    /// so the next plan an agent sends starts with none of them.
    fn settle_plan_threads(&mut self, path: &Path, kept: Option<&Path>) {
        let file = self.relative(path);
        let kept = kept.map(|k| {
            (
                self.relative(k),
                std::fs::read_to_string(k).unwrap_or_default(),
            )
        });
        let mut settled = false;
        for thread in self.threads.iter_mut().filter(|t| t.file == file) {
            thread.resolved = true;
            if let Some((name, text)) = &kept {
                thread.file.clone_from(name);
                if thread.earlier.is_none() {
                    thread.follow(text);
                }
            }
            settled = true;
        }
        if settled {
            self.threads_seen = None;
            self.save_threads();
        }
    }

    /// Answers the waiting agent and closes the plan.
    fn finish_plan(&mut self, decision: Decision) {
        let Some(plan) = self.plan.take() else {
            self.say("No plan is waiting for review");
            return;
        };
        if matches!(decision, Decision::Approve(_)) {
            let kept = self.keep_plan(&plan.path, &decision);
            self.settle_plan_threads(&plan.path, kept.as_deref());
        }
        let _ = plan.reply.send(decision);
        self.agents.plan_answered();
        let file = self.comment_file(&plan.path);
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
        // Inside Claude Code the mod starts a turn, and the prompt hook takes the comments.
        if self.in_pane {
            self.attention = Some("send");
            self.say(format!(
                "Sending {} comments to Claude Code",
                comments.len()
            ));
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

    /// The headings of the document in view, and which one the caret is under.
    fn refresh_outline(&mut self) {
        let Some(editor) = self.editors.get(self.active()) else {
            self.outline.headings.clear();
            return;
        };
        // Code has no headings: its symbols stand in, from its language server.
        if !editor.has_page()
            && let Some(path) = editor.buffer.path().map(Path::to_path_buf)
        {
            let revision = editor.buffer.revision();
            let language = Registry::language_name(&path);
            let asked = self.symbols_asked.as_ref() == Some(&(path.clone(), revision));
            if !asked && let Some(client) = language.and_then(|l| self.servers.get_mut(l)) {
                client.symbols(&path);
                self.symbols_asked = Some((path.clone(), revision));
            }
            self.outline.headings = self.symbols.get(&path).cloned().unwrap_or_default();
            return;
        }
        let folder = editor.buffer.path().and_then(Path::parent);
        self.outline.headings =
            crate::outline::of_in(&editor.buffer.text(), editor.dialect(), folder);
    }

    fn outline_here(&self) -> Option<usize> {
        let line = self.cursor_line()?.1.saturating_sub(1);
        // Above the first heading still counts as being in the document's first part.
        crate::outline::current(&self.outline.headings, line).or((!self
            .outline
            .headings
            .is_empty())
        .then_some(0))
    }

    fn click_outline(&mut self, y: u16) {
        let Some(area) = self.sidebar_area() else {
            return;
        };
        let Some((line, file)) = self.outline.at(area, y).map(|h| (h.line, h.file.clone())) else {
            return;
        };
        // A heading in an `\\input` file opens that file.
        if let Some(file) = file
            && self.open(&file).is_err()
        {
            return;
        }
        self.drawer = false;
        let text = self.focused_area();
        if let Some(editor) = self.editor_mut() {
            editor.go_to_line(line + 1, text);
        }
    }

    /// Closes the left side. Review takes back the list it gave up to the panel, and a Review
    /// with no panel beside it is what the lit icon stood for, so it closes.
    fn hide_sidebar(&mut self) {
        self.search.focus = None;
        self.show_sidebar = false;
        self.drawer = false;
        match &mut self.review {
            Some(review) if review.beside_panel => review.beside_panel = false,
            review => *review = None,
        }
    }

    fn toggle_sidebar(&mut self) {
        if self.compact() {
            self.drawer = !self.drawer;
        } else if self.sidebar_area().is_some() {
            self.hide_sidebar();
        } else {
            self.show_sidebar = true;
        }
    }

    /// The button that goes between a file's page and its source. A page with a toolbar has
    /// that instead, and a window too short for this row has it in the right-click menu.
    fn page_switch(&self) -> Option<(Rect, &'static str)> {
        let editor = self.editors.get(self.active())?;
        let free = self.review.is_none() && self.settings_page.is_none();
        let row = self.chrome_rows() == 2 && !self.plan_in_view();
        if !free || !row || !editor.has_page() || self.toolbar_row().is_some() {
            return None;
        }
        let name = if editor.rendered {
            "Show Source"
        } else {
            "Show Page"
        };
        let main = self.main_area();
        let width = name.chars().count() as u16 + 2;
        let x = main.right().checked_sub(width + 1)?;
        Some((Rect::new(x, 1, width, 1), name))
    }

    fn show_explorer(&mut self) {
        self.search.focus = None;
        self.review = None;
        self.sidebar_view = SidebarView::Explorer;
    }

    /// Keys for the search boxes. Returns false to let a shortcut through.
    fn search_key(&mut self, key: KeyEvent) -> bool {
        if let Key::Char(c) = key.key
            && key.mods.alt
            && !key.mods.ctrl
            && self.search.options.flip_key(c)
        {
            self.search.start();
            return true;
        }
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
        match self
            .workspace
            .replace_all(&query, &with, self.search.options, &dirty)
        {
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
        self.mark_jump();
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
            Some(search::Hit::Switch(switch)) => {
                self.search.options.flip(switch);
                self.search.start();
            }
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

    /// How comments name a file: from the project's root, or by its name alone when it's
    /// outside, as a plan held in a private folder is.
    fn comment_file(&self, path: &Path) -> String {
        let relative = self.relative(path);
        if Path::new(&relative).is_absolute() {
            path.file_name()
                .map_or(relative, |n| n.to_string_lossy().into_owned())
        } else {
            relative
        }
    }

    /// A path inside the workspace as comments, threads and agents name it: with `/` on every
    /// platform, which Windows reads too, so a thread saved on one machine matches on another.
    fn relative(&self, path: &Path) -> String {
        let relative = path
            .strip_prefix(self.workspace.root())
            .unwrap_or(path)
            .to_string_lossy()
            .into_owned();
        if cfg!(windows) {
            relative.replace('\\', "/")
        } else {
            relative
        }
    }

    fn add_comment(&mut self, file: String, line: usize, text: String) {
        // A plan's comments are its answer, and `--wait` writes them into the file.
        let answering = self.plan.is_some() || self.wait_file.is_some();
        // What it's about, from Review's line or the editor's selection or line.
        let (quote, removed) = self.comment_quote.take().unwrap_or_else(|| {
            let quote = self.editors.get(self.active()).map(|editor| {
                editor
                    .selected_text()
                    .unwrap_or_else(|| editor.line_text(line.saturating_sub(1)))
            });
            (quote.unwrap_or_default(), false)
        });
        let quote = quote.lines().next().unwrap_or_default().trim().to_string();
        // A plan lives in a private folder, so it goes by its name, not that folder's path.
        let file = if answering && Path::new(&file).is_absolute() {
            self.comment_file(Path::new(&file))
        } else {
            file
        };
        let comment = Comment {
            file: file.clone(),
            line,
            text: text.clone(),
            quote: quote.clone(),
            removed,
        };
        // A plan's comment also stays on the plan as a thread, so the next version shows it
        // and whether it was dealt with. The agent hears of it with the plan, so it's told.
        if let Some(plan) = self.plan.as_ref().map(|p| p.path.clone()) {
            self.threads.push(Thread {
                id: cellix_review::threads::next_id(&self.threads),
                file: self.relative(&plan),
                line,
                quote: quote.clone(),
                text: text.clone(),
                replies: Vec::new(),
                resolved: false,
                told: Some(0),
                asked: false,
                kind: String::new(),
                suggested_by: String::new(),
                copy: (0, 0),
                earlier: None,
                reworded: false,
            });
            self.save_threads();
        }
        let Some(editor) = self
            .editors
            .get(self.active())
            .filter(|_| !answering && !removed)
        else {
            if let Ok(mut comments) = self.comments.lock() {
                comments.push(comment);
            }
            return;
        };
        // Every comment shows beside its file. A document's is a conversation there, and
        // code's also goes with the review, so an agent is told of it by that road.
        let page = editor.rendered;
        self.threads.push(Thread {
            id: cellix_review::threads::next_id(&self.threads),
            file: file.clone(),
            line,
            quote,
            text: text.clone(),
            replies: Vec::new(),
            resolved: false,
            told: (!page).then_some(0),
            asked: false,
            kind: String::new(),
            suggested_by: String::new(),
            copy: (0, 0),
            earlier: None,
            reworded: false,
        });
        self.save_threads();
        if page {
            self.dock.open = true;
            self.dock.show_comments();
            self.dock.focused = false;
        } else if let Ok(mut comments) = self.comments.lock() {
            comments.push(comment);
        }
    }

    /// Keeps comments on their text as the document changes.
    fn follow_threads(&mut self) {
        let Some(editor) = self.editors.get(self.active()) else {
            return;
        };
        let revision = editor.buffer.revision();
        if self.threads_seen == Some((self.active(), revision)) {
            return;
        }
        let Some(file) = editor.buffer.path().map(|p| self.relative(p)) else {
            return;
        };
        let text = editor.buffer.text();
        let mut quotes = Vec::new();
        for thread in self
            .threads
            .iter_mut()
            .filter(|t| t.file == file && !t.resolved && t.earlier.is_none())
        {
            thread.follow(&text);
            quotes.push((thread.line, thread.quote.clone()));
        }
        editor.set_quotes(&quotes);
        self.threads_seen = Some((self.active(), revision));
    }

    /// Gives a paper's root the package its tracked changes print with, in its open tab if it
    /// has one, saved if it was, or else in the file.
    fn track_in_root(&mut self, root: &Path) {
        let open = self
            .editors
            .iter_mut()
            .find(|e| e.buffer.path() == Some(root));
        if let Some(editor) = open {
            let saved = !editor.buffer.is_dirty();
            let text = editor.buffer.text();
            if let Some(with) = cellix_doc::changes::with_package(&text) {
                editor.set_all_text(&with);
                if saved {
                    let _ = editor.buffer.save();
                }
            }
            return;
        }
        let Ok(text) = std::fs::read_to_string(root) else {
            return;
        };
        if let Some(with) = cellix_doc::changes::with_package(&text)
            && let Err(e) = std::fs::write(root, with)
        {
            self.say(format!(
                "Could not add the changes package to the paper: {e}"
            ));
        }
    }

    /// A plan sent again keeps its comments as threads: each moves onto the new text, and one
    /// whose words are gone stays open, marked with `replaced`, the version it was made on.
    /// Replies and whether it was resolved come with it, and the Comments pane shows them.
    fn carry_plan_threads(&mut self, path: &Path, replaced: usize) {
        let Ok(text) = std::fs::read_to_string(path) else {
            return;
        };
        let file = self.relative(path);
        let (mut carried, mut open) = (false, false);
        for thread in self.threads.iter_mut().filter(|t| t.file == file) {
            thread.carry(&text, replaced);
            carried = true;
            open |= !thread.resolved;
        }
        if !carried {
            return;
        }
        self.threads_seen = None;
        self.save_threads();
        if !open {
            return;
        }
        self.dock.open = true;
        self.dock.show_comments();
        self.dock.focused = false;
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
                // Words the plan changed or dropped are only in what changed, so that's where
                // it goes, whether or not the plan still waits.
                let earlier = self
                    .threads
                    .iter()
                    .find(|t| t.id == id)
                    .and_then(|t| Some((t.earlier?, self.workspace.root().join(&t.file))));
                if let Some((version, path)) = earlier {
                    self.compare_plan_with(Some(&path), Some(version));
                    self.dock.focused = false;
                    return;
                }
                let line = self.threads.iter().find(|t| t.id == id).map(|t| t.line);
                let area = self.focused_area();
                if let (Some(line), Some(editor)) = (line, self.editor_mut()) {
                    editor.go_to_line(line, area);
                }
                self.dock.focused = false;
            }
            ThreadHit::Reply(id) => {
                self.dock.draft = Some(Draft {
                    thread: Some(id),
                    file: String::new(),
                    line: 0,
                    quote: String::new(),
                    text: String::new(),
                    ask: false,
                    code: false,
                });
            }
            ThreadHit::Ask(id) => {
                let message = self
                    .threads
                    .iter()
                    .find(|t| t.id == id)
                    .map(Thread::message);
                match (message, self.dock.agent_mut()) {
                    (Some(message), Some(session)) => {
                        session.send_line(&message);
                        if let Some(thread) = self.threads.iter_mut().find(|t| t.id == id) {
                            thread.told = Some(thread.replies.len());
                        }
                        self.save_threads();
                        self.say("Sent to the agent. Its answer lands here");
                    }
                    // Something's already at work on this project, outside the pane. Its
                    // hook hands it this comment, so there's nothing to start.
                    _ if self.agents.outside().is_some() => {
                        let name = self.agents.outside().unwrap_or_default();
                        if let Some(thread) = self.threads.iter_mut().find(|t| t.id == id) {
                            thread.asked = true;
                        }
                        self.save_threads();
                        self.dock.show_comments();
                        self.say(format!("{name} sees this with its next prompt"));
                    }
                    // Nobody to ask yet: it goes to whichever agent starts.
                    _ => {
                        self.pending_ask = Some(id);
                        self.refresh_agents();
                        self.dock.show_list();
                        self.say("Pick an agent, and your comment goes to it when it's ready");
                    }
                }
            }
            ThreadHit::Check => self.check_review(),
            ThreadHit::Apply(id) => self.apply_rewrite(id),
            ThreadHit::Decline(id) => {
                if let Some(thread) = self.threads.iter_mut().find(|t| t.id == id) {
                    thread.replies.push(Reply {
                        author: YOU.to_string(),
                        text: "Not taking this rewrite.".to_string(),
                    });
                }
                self.save_threads();
            }
            ThreadHit::Keep(id) => {
                if let Some(thread) = self.threads.iter_mut().find(|t| t.id == id) {
                    thread.suggested_by.clear();
                }
                self.save_threads();
                self.say("Kept as your comment");
            }
            ThreadHit::Dismiss(id) => {
                if let Some(thread) = self.threads.iter_mut().find(|t| t.id == id) {
                    thread.resolved = true;
                }
                self.save_threads();
            }
            ThreadHit::Resolve(id) => {
                if let Some(thread) = self.threads.iter_mut().find(|t| t.id == id) {
                    thread.resolved = true;
                }
                self.save_threads();
            }
        }
    }

    /// Puts an agent's rewrite in place of the words its comment is about, as a suggestion to
    /// accept or reject like any other, so nothing changes until the reviewer says.
    fn apply_rewrite(&mut self, id: u32) {
        let Some(thread) = self.threads.iter().find(|t| t.id == id).cloned() else {
            return;
        };
        let Some(words) = thread
            .replies
            .last()
            .and_then(|r| crate::threads::rewrite(&r.text))
            .map(str::to_string)
        else {
            return;
        };
        let path = self.workspace.root().join(&thread.file);
        if self.open(&path).is_err() {
            return;
        }
        let Some(editor) = self.editor_mut() else {
            return;
        };
        let text = editor.buffer.text();
        // The quote nearest the line it was on.
        let near = editor
            .buffer
            .rope()
            .line_to_char(thread.line.saturating_sub(1));
        let chars: Vec<char> = text.chars().collect();
        let wanted: Vec<char> = thread.quote.chars().collect();
        let found = (0..chars.len().saturating_sub(wanted.len().saturating_sub(1)))
            .filter(|at| !wanted.is_empty() && chars[*at..].starts_with(&wanted))
            .min_by_key(|at| at.abs_diff(near));
        let Some(start) = found else {
            self.say("The words it's about aren't in the document any more");
            return;
        };
        let marked = cellix_doc::changes::mark(&thread.quote, &words, editor.change_syntax());
        editor
            .buffer
            .replace_range(&mut editor.selections, start, start + wanted.len(), &marked);
        if let Some(thread) = self.threads.iter_mut().find(|t| t.id == id) {
            thread.replies.push(Reply {
                author: YOU.to_string(),
                text: "Applied the rewrite as a suggestion.".to_string(),
            });
        }
        self.save_threads();
        self.say("The rewrite is in as a suggestion: accept or reject it");
    }

    /// Asks the agent to check the reviewer's comments on the document in view, and to
    /// suggest the ones a careful reviewer would add, each through the command line.
    fn check_review(&mut self) {
        let Some(path) = self
            .editors
            .get(self.active())
            .and_then(|e| e.buffer.path())
            .map(Path::to_path_buf)
        else {
            self.say("Open the document you're reviewing first");
            return;
        };
        let file = self.relative(&path);
        let mine: Vec<&Thread> = self
            .threads
            .iter()
            .filter(|t| t.file == file && !t.resolved && t.suggested_by.is_empty())
            .collect();
        let text = review_prompt(&file, &mine);
        // Inside Claude Code, the mod submits it; with an agent in the pane, it's typed there.
        if self.in_pane {
            self.outgoing_prompt = Some(text);
            self.say("Asked the agent to check your review");
        } else if let Some(session) = self.dock.agent_mut() {
            session.send_line(&text.replace('\n', " "));
            self.say("Asked the agent to check your review");
        } else {
            self.pending_review = Some(text);
            self.refresh_agents();
            self.dock.show_list();
            self.say("Pick an agent, and it checks your review when it's ready");
        }
    }

    /// The prompt for the mod to submit to the agent it runs in, once.
    pub fn take_outgoing_prompt(&mut self) -> Option<String> {
        self.outgoing_prompt.take()
    }

    /// What the mod puts in Claude Code's prompt box for the person to finish, once.
    pub fn take_outgoing_fill(&mut self) -> Option<String> {
        self.outgoing_fill.take()
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

    fn prompt_comment_at_cursor(&mut self, ask: bool) {
        if let Some((path, line)) = self.cursor_line() {
            let file = self.relative(&path);
            let answering = self.plan.is_some() || self.wait_file.is_some();
            let open = self.editors.get(self.active());
            if let Some(editor) = open.filter(|_| !answering) {
                // Written beside the file, so the text it's about stays in view.
                let quote = editor
                    .selected_text()
                    .unwrap_or_else(|| editor.line_text(line.saturating_sub(1)));
                let quote = quote.lines().next().unwrap_or_default().trim().to_string();
                // Inside Claude Code the agent is the conversation beside the pane, so the
                // question is asked there: what it's about goes in the prompt box to finish.
                if ask && self.in_pane {
                    self.outgoing_fill = Some(ask_in_prompt(&file, line, &quote));
                    // The bar goes, so the press shows it was taken; the words are in the prompt.
                    if let Some(editor) = self.editor_mut() {
                        let head = editor.selections.primary().head;
                        editor.selections.set_single(Selection::cursor(head));
                    }
                    self.say(asked_in_prompt(&quote, line));
                    return;
                }
                let code = !editor.rendered;
                self.write_comment(Draft {
                    thread: None,
                    file,
                    line,
                    quote,
                    text: String::new(),
                    ask,
                    code,
                });
                return;
            }
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
            Request::Suggest {
                file,
                line,
                kind,
                quote,
                text,
                author,
                made,
            } => {
                let path = Path::new(&file);
                let file = if path.is_absolute() {
                    self.relative(path)
                } else {
                    file
                };
                let id = cellix_review::threads::next_id(&self.threads);
                self.threads.push(Thread {
                    id,
                    file,
                    line: line.max(1),
                    quote,
                    text,
                    replies: Vec::new(),
                    resolved: false,
                    // The agent wrote it, so it has nothing to be told.
                    told: Some(0),
                    asked: false,
                    kind,
                    suggested_by: author.clone(),
                    copy: (0, 0),
                    earlier: None,
                    reworded: false,
                });
                self.save_threads();
                let _ = made.send(id);
                self.dock.show_comments();
                self.say(format!(
                    "{author} suggested a comment: keep it or dismiss it"
                ));
            }
            Request::Reply {
                id,
                author,
                text,
                found,
            } => {
                let thread = self.threads.iter_mut().find(|t| t.id == id);
                let _ = found.send(thread.is_some());
                if let Some(thread) = thread {
                    // A verdict on a comment may say what kind of comment it is.
                    if thread.kind.is_empty()
                        && let Some((_, kind, _)) = crate::threads::verdict(&text)
                    {
                        thread.kind = kind.to_string();
                    }
                    thread.replies.push(Reply {
                        author: author.clone(),
                        text,
                    });
                    self.save_threads();
                    self.say(format!("{author} answered a comment"));
                }
            }
            Request::TakeThreads(reply) => {
                let mut sent = Vec::new();
                for thread in self.threads.iter_mut().filter(|t| t.waiting()) {
                    thread.told = Some(thread.replies.len());
                    sent.push(thread.clone());
                }
                let json = serde_json::to_string(&sent).unwrap_or_else(|_| "[]".to_string());
                let _ = reply.send(json);
                if !sent.is_empty() {
                    self.save_threads();
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
                // A plan still open from before shows the version that came.
                if let Some(editor) = self
                    .editors
                    .iter_mut()
                    .find(|e| e.buffer.path() == Some(path.as_path()) && !e.buffer.is_dirty())
                {
                    let _ = editor.reload();
                }
                match self.open(&path) {
                    Ok(()) => {
                        let earlier = plan_versions(&path).len();
                        self.carry_plan_threads(&path, earlier.max(1));
                        self.plan = Some(PendingPlan { path, reply });
                        self.attention = Some("plan");
                        if earlier == 0 {
                            self.say("An agent is waiting for you to review this plan");
                        } else {
                            self.say(format!(
                                "An agent is waiting on version {} of this plan. Compare Plan Versions shows what changed",
                                earlier + 1
                            ));
                        }
                    }
                    // Dropping `reply` lets the agent ask in its own way.
                    Err(e) => self.say(format!("Could not open the plan: {e}")),
                }
            }
        }
    }

    /// Writes comments into the `--wait` file. Called once, as the app exits.
    pub fn finish(&self) -> io::Result<()> {
        self.keep_unsent();
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

    /// The documents with Track Changes on, kept beside the project's sessions so it stays as
    /// it was left. Out of the project itself, since it's how one person works.
    fn tracked_file(&self) -> PathBuf {
        crate::hold::folder_of(self.workspace.root()).join("tracking.json")
    }

    fn tracked_files(&self) -> Vec<String> {
        std::fs::read_to_string(self.tracked_file())
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    fn keep_tracked_files(&self, tracked: &[String]) {
        let file = self.tracked_file();
        if tracked.is_empty() {
            let _ = std::fs::remove_file(&file);
            return;
        }
        if let Some(folder) = file.parent() {
            let _ = std::fs::create_dir_all(folder);
        }
        if let Ok(json) = serde_json::to_string(tracked) {
            let _ = std::fs::write(file, json);
        }
    }

    /// Where the comments no agent has taken yet wait between runs: beside the project's
    /// sessions, out of the project itself.
    fn unsent_file(&self) -> PathBuf {
        crate::hold::folder_of(self.workspace.root()).join("unsent.json")
    }

    /// Comments no agent has taken yet outlive this run, so the next one still hands them
    /// over. Only those on the project's own files: a plan's answer went with the plan, and
    /// `--wait` writes its comments into the file.
    fn keep_unsent(&self) {
        let root = self.workspace.root();
        let waited = self.wait_file.as_deref().map(|p| self.relative(p));
        let unsent: Vec<Comment> = self.comments.lock().map_or_else(
            |_| Vec::new(),
            |c| {
                c.iter()
                    .filter(|c| Some(&c.file) != waited.as_ref() && root.join(&c.file).is_file())
                    .cloned()
                    .collect()
            },
        );
        let file = self.unsent_file();
        if unsent.is_empty() {
            let _ = std::fs::remove_file(&file);
            return;
        }
        let Ok(json) = serde_json::to_string(&unsent) else {
            return;
        };
        if let Some(folder) = file.parent() {
            let _ = std::fs::create_dir_all(folder);
        }
        let _ = std::fs::write(file, json);
    }

    /// Takes back the comments the last run left unsent. Only the Cellix that serves the
    /// project does, so a second window doesn't hand them over twice.
    pub fn restore_unsent(&mut self) {
        let file = self.unsent_file();
        let Ok(text) = std::fs::read_to_string(&file) else {
            return;
        };
        let _ = std::fs::remove_file(&file);
        let kept: Vec<Comment> = serde_json::from_str(&text).unwrap_or_default();
        if kept.is_empty() {
            return;
        }
        let count = kept.len();
        if let Ok(mut comments) = self.comments.lock() {
            comments.extend(kept);
        }
        self.say(format!(
            "{count} comment{} from last time still waiting for an agent",
            if count == 1 { "" } else { "s" }
        ));
    }

    fn click_review(&mut self, x: u16, y: u16) {
        let clicks = self.count_click(x, y);
        let area = self.main_area();
        let Some(review) = &mut self.review else {
            return;
        };
        let comments = self.comments.lock().map(|c| c.clone()).unwrap_or_default();
        match review.hit(area, x, y, &comments) {
            Some(Hit::File(index)) => review.select(index),
            Some(Hit::AcceptProposal) => self.accept_proposal(),
            Some(Hit::RejectProposal) => self.ask(REJECT_ASK, PromptAction::RejectProposal),
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
            Some(Hit::Reject(hunk)) => self.ask(REJECT_ASK, PromptAction::RejectHunk(hunk)),
            Some(Hit::PreviousFile) => review.step_file(-1),
            Some(Hit::NextFile) => review.step_file(1),
            Some(Hit::Line(file, line, quote, removed)) => {
                self.comment_quote = Some((quote, removed));
                self.ask(
                    &format!("Comment on line {line}"),
                    PromptAction::Comment(file, line),
                );
            }
            Some(Hit::Text(file, line)) if clicks >= 2 => {
                self.review = None;
                let path = self.workspace.root().join(file);
                if self.open(&path).is_ok() {
                    let area = self.focused_area();
                    if let Some(editor) = self.editor_mut() {
                        editor.go_to_line(line, area);
                    }
                }
            }
            Some(Hit::Text(..)) | None => {}
        }
    }

    /// Puts one hunk back on disk. Refuses if the file has unsaved edits, which the
    /// rejection would silently overwrite.
    /// Asks the language server about the symbol under the pointer once it has rested there,
    /// as VS Code's hover does. Any key or move clears what it said.
    fn follow_rest(&mut self, event: &Event) {
        const REST: std::time::Duration = std::time::Duration::from_millis(600);
        match event {
            Event::Key(_) => {
                self.hover_info = None;
                self.rest = None;
            }
            Event::Mouse(MouseEvent {
                kind: MouseKind::Move,
                x,
                y,
                ..
            }) => {
                let cell = (*x, *y);
                if self.rest.map(|r| r.0) != Some(cell) {
                    self.rest = Some((cell, std::time::Instant::now(), false));
                    if self.hover_info.as_ref().is_some_and(|(_, at)| *at != cell) {
                        self.hover_info = None;
                    }
                }
            }
            Event::Mouse(_) => self.hover_info = None,
            Event::Tick => {
                let Some((cell, since, false)) = self.rest else {
                    return;
                };
                if since.elapsed() < REST || !matches!(self.overlay, Overlay::None) {
                    return;
                }
                self.rest = Some((cell, since, true));
                self.hover_on(cell);
            }
            _ => {}
        }
    }

    /// Asks the server about the code at a cell, if it's code with a server.
    fn hover_on(&mut self, (x, y): (u16, u16)) {
        let Some(pane) = self.pane_at(x, y) else {
            return;
        };
        let Some(index) = self.panes.get(pane).copied() else {
            return;
        };
        let area = self.pane_area(pane);
        let Some(editor) = self.editors.get(index).filter(|e| !e.rendered) else {
            return;
        };
        let Some(path) = editor.buffer.path().map(Path::to_path_buf) else {
            return;
        };
        let Some(language) = Registry::language_name(&path) else {
            return;
        };
        let at = editor.position_at(area, x as f32, y);
        let rope = editor.buffer.rope();
        let line = rope.char_to_line(at);
        let column = at - rope.line_to_char(line);
        // Past the end of a line is no symbol.
        if rope
            .line(line)
            .chars()
            .nth(column)
            .is_none_or(char::is_whitespace)
        {
            return;
        }
        if let Some(client) = self.servers.get_mut(language) {
            self.hover_at = Some((x, y));
            client.hover(&path, line, column);
        }
    }

    /// What the server said, on a panel beside the place it's about.
    fn draw_hover(&self, grid: &mut Grid) {
        const WIDTH: usize = 72;
        const LINES: usize = 12;
        let Some((text, (x, y))) = &self.hover_info else {
            return;
        };
        let mut lines: Vec<String> = Vec::new();
        for line in text.lines() {
            let chars: Vec<char> = line.chars().collect();
            if chars.is_empty() {
                lines.push(String::new());
            }
            for chunk in chars.chunks(WIDTH) {
                lines.push(chunk.iter().collect());
            }
        }
        lines.truncate(LINES);
        let wide = lines.iter().map(|l| l.chars().count()).max().unwrap_or(0) as u16 + 2;
        let high = lines.len() as u16;
        let (width, height) = self.size;
        let left = (*x).min(width.saturating_sub(wide + 1));
        // Under the line it's about, or over it when that runs off the bottom.
        let top = if y + 2 + high < height {
            y + 2
        } else {
            y.saturating_sub(high + 1)
        };
        let area = Rect::new(left, top, wide, high);
        crate::palette::raised(grid, area, &self.theme);
        let theme = &self.theme;
        for (n, line) in lines.iter().enumerate() {
            let style = Style {
                fg: if n == 0 {
                    theme.text.fg
                } else {
                    theme.popup_dim
                },
                bg: theme.popup,
                ..theme.bar
            };
            // Painted like the panel's other text, so the panel's paint doesn't cover it.
            grid.label(area.x + 1, area.y + n as u16, line, style, area.right());
        }
    }

    /// Moves to the next suggestion in the document, or the one before, going round, and says
    /// which of how many it is.
    fn step_change(&mut self, forward: bool) {
        let area = self.focused_area();
        let Some(editor) = self.editor_mut() else {
            return;
        };
        let changes = cellix_doc::changes::find(&editor.buffer.text(), editor.change_syntax());
        if changes.is_empty() {
            self.say("No suggestions in this document");
            return;
        }
        let head = editor.selections.primary().head;
        let index = if forward {
            changes.iter().position(|c| c.start > head).unwrap_or(0)
        } else {
            changes
                .iter()
                .rposition(|c| c.start < head)
                .unwrap_or(changes.len() - 1)
        };
        let start = changes[index].start;
        editor.selections.set_single(Selection::cursor(start));
        editor.reveal(area);
        self.say(format!("Suggestion {} of {}", index + 1, changes.len()));
    }

    /// Moves to the next problem the language server reported in this file, or the one
    /// before, going round at the ends, and says what it is.
    fn step_problem(&mut self, forward: bool) {
        let Some((path, head)) = self.here() else {
            return;
        };
        let mut lines: Vec<(usize, String)> = self
            .diagnostics
            .get(&path)
            .map(|found| found.iter().map(|d| (d.line, d.message.clone())).collect())
            .unwrap_or_default();
        lines.sort();
        lines.dedup_by_key(|(line, _)| *line);
        let Some(editor) = self.editors.get(self.active()) else {
            return;
        };
        let here = editor.buffer.rope().char_to_line(head);
        let next = if forward {
            lines.iter().find(|(l, _)| *l > here).or(lines.first())
        } else {
            lines.iter().rev().find(|(l, _)| *l < here).or(lines.last())
        };
        let Some((line, message)) = next.cloned() else {
            self.say("No problems in this file");
            return;
        };
        let area = self.focused_area();
        if let Some(editor) = self.editor_mut() {
            editor.go_to_line(line + 1, area);
        }
        self.say(message);
    }

    /// Where the caret is, as a file and a place in it.
    fn here(&self) -> Option<(PathBuf, usize)> {
        let editor = self.editors.get(self.active())?;
        let path = editor.buffer.path()?.to_path_buf();
        Some((path, editor.selections.primary().head))
    }

    /// Remembers where a jump leaves from. A new jump forgets the way forward.
    fn mark_jump(&mut self) {
        const KEPT: usize = 100;
        if let Some(here) = self.here()
            && self.back.last() != Some(&here)
        {
            self.back.push(here);
            if self.back.len() > KEPT {
                self.back.remove(0);
            }
        }
        self.forward.clear();
    }

    /// Goes back to where the last jump left from, or forward again.
    fn travel(&mut self, back: bool) {
        let from = if back {
            &mut self.back
        } else {
            &mut self.forward
        };
        let Some((path, at)) = from.pop() else {
            self.say(if back {
                "Nowhere to go back to"
            } else {
                "Nowhere to go forward to"
            });
            return;
        };
        let here = self.here();
        let to = if back {
            &mut self.forward
        } else {
            &mut self.back
        };
        to.extend(here);
        if self.open(&path).is_err() {
            return;
        }
        let area = self.focused_area();
        if let Some(editor) = self.editor_mut() {
            let at = at.min(editor.buffer.rope().len_chars());
            editor.selections.set_single(Selection::cursor(at));
            editor.reveal(area);
        }
    }

    /// Tells the agent why a change was turned down, as a comment on where it was, so it
    /// doesn't try the same again. No reason, no comment.
    fn note_rejection(&mut self, hunk: usize, reason: &str) {
        let Some(review) = &self.review else {
            return;
        };
        let (Some(path), Some((line, quote, removed))) =
            (review.current_path(), review.hunk_place(hunk))
        else {
            return;
        };
        if reason.trim().is_empty() {
            return;
        }
        let comment = Comment {
            file: path.to_string_lossy().into_owned(),
            line,
            text: format!("I rejected this change: {}", reason.trim()),
            quote,
            removed,
        };
        if let Ok(mut comments) = self.comments.lock() {
            comments.push(comment);
        }
    }

    /// A changed line's own choices: comment on it, or undo just it when most of its hunk is
    /// right.
    fn review_line_menu(&mut self, x: u16, y: u16) {
        let area = self.main_area();
        let comments = self.comments.lock().map(|c| c.clone()).unwrap_or_default();
        let Some(review) = &self.review else {
            return;
        };
        if review.is_proposal() {
            return;
        }
        let Some((undo, line, quote)) = review.changed_line_at(area, y, &comments) else {
            return;
        };
        let Some(file) = review
            .current_path()
            .map(|p| p.to_string_lossy().into_owned())
        else {
            return;
        };
        let removed = matches!(undo, cellix_review::LineUndo::Removed { .. });
        let what = match undo {
            cellix_review::LineUndo::Added(_) => "Remove This Added Line",
            cellix_review::LineUndo::Removed { .. } => "Put This Line Back",
        };
        let items = vec![
            MenuItem::new(
                format!("Comment on Line {line}"),
                MenuAction::Run(Command::AddComment),
            ),
            MenuItem::new(what, MenuAction::RejectLine(undo)),
        ];
        self.review_line = Some((file, line, quote, removed));
        self.show_menu((x, y.saturating_add(1)), items);
    }

    /// Writes the file with one line of its change undone.
    fn reject_line(&mut self, undo: cellix_review::LineUndo) {
        let Some(review) = &self.review else {
            return;
        };
        let Some(path) = review.absolute_path() else {
            return;
        };
        let text = review.line_rejected_text(undo);
        let dirty = self
            .editors
            .iter()
            .any(|e| e.buffer.path() == Some(path.as_path()) && e.buffer.is_dirty());
        if dirty {
            self.say("Save or undo your edits to this file first");
            return;
        }
        let note = match std::fs::write(&path, text) {
            Ok(()) => "Undid that line of the change".to_string(),
            Err(e) => format!("Could not write the file: {e}"),
        };
        self.say(note);
        if let Some(review) = &mut self.review {
            review.refresh();
        }
    }

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
        if self.editors.get(self.active()).is_some_and(|e| e.conflict) {
            self.confirm_conflict();
            return;
        }
        self.write_file();
    }

    /// Asks before saving over a change made while this file had unsaved edits.
    fn confirm_conflict(&mut self) {
        let Some(editor) = self.editors.get(self.active()) else {
            return;
        };
        self.overlay = Overlay::Confirm(Confirm {
            title: format!("{} changed on disk", editor.title()),
            detail: "Something else edited it while you were. Saving replaces that edit."
                .to_string(),
            buttons: vec![
                ("Cancel", Choice::Cancel),
                ("Reload Theirs", Choice::Reload),
                ("Overwrite", Choice::Overwrite),
            ],
            selected: 0,
        });
    }

    fn write_file(&mut self) {
        let Some(editor) = self.editor_mut() else {
            return;
        };
        editor.conflict = false;
        // Open suggestions are saved as the marks that hold them, so whoever opens the file
        // next sees them too. Saying so keeps that from being a surprise.
        let open = if editor.rendered {
            cellix_doc::changes::find(&editor.buffer.text(), editor.change_syntax()).len()
        } else {
            0
        };
        let note = match (editor.save(), open) {
            (Ok(()), 0) => format!("Saved {}", editor.title()),
            (Ok(()), n) => format!(
                "Saved {}, with {n} suggestion{} still open: {} in the file as marks until \
                 accepted or rejected",
                editor.title(),
                if n == 1 { "" } else { "s" },
                if n == 1 { "it stays" } else { "they stay" },
            ),
            (Err(e), _) => format!("Could not save: {e}"),
        };
        self.say(note);
        // A paper's count takes in its other files, so every open one counts again.
        for editor in &self.editors {
            editor.recount();
        }
        // With its PDF open, saving a paper rebuilds it.
        self.rebuild_on_save();
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
        // Inside Claude Code the wheel comes with no position. Over the side pane it scrolls
        // the pane, which only the pointer's last move can tell; elsewhere it means the file.
        let mouse = match self.hover {
            Some((x, y))
                if mouse.x == u16::MAX
                    && matches!(mouse.kind, MouseKind::ScrollUp | MouseKind::ScrollDown)
                    && self.dock_area().is_some_and(|area| area.contains(x, y)) =>
            {
                MouseEvent {
                    x,
                    y,
                    fine_x: f32::from(x),
                    ..mouse
                }
            }
            _ => mouse,
        };
        if mouse.kind == MouseKind::Move {
            self.hover = Some((mouse.x, mouse.y));
            let in_pane = self
                .dock_screen()
                .is_some_and(|a| a.contains(mouse.x, mouse.y));
            if in_pane && self.dock.front == Front::Session {
                self.dock_session_mouse(mouse);
            }
            if in_pane && self.dock.front == Front::Pdf {
                self.note_view_spot(mouse.x, mouse.y);
            }
            return;
        }
        if self.tab_drag.is_some() && matches!(mouse.kind, MouseKind::Drag | MouseKind::Up) {
            self.drag_tab(mouse);
            return;
        }
        if !matches!(self.overlay, Overlay::None) {
            self.overlay_mouse(mouse);
            return;
        }
        if mouse.kind == MouseKind::Left && self.click_companion(mouse.x, mouse.y) {
            return;
        }
        if mouse.kind == MouseKind::Left && self.click_offer(mouse.x, mouse.y) {
            return;
        }
        if mouse.kind == MouseKind::Left
            && let Some(command) = self.action_at(mouse.x, mouse.y)
        {
            self.run(command);
            return;
        }
        if mouse.x != u16::MAX && self.dock_mouse(mouse) {
            return;
        }
        if let Some(area) = self.suggest_area() {
            let on_row = area.contains(mouse.x, mouse.y) && mouse.kind == MouseKind::Left;
            let row = mouse.y.saturating_sub(area.y) as usize;
            if on_row {
                self.accept_suggestion(row);
                return;
            }
            self.close_suggestions();
        }
        // The wheel over a pane comes with no position, so it means the file in front.
        if mouse.x == u16::MAX {
            if !matches!(mouse.kind, MouseKind::ScrollUp | MouseKind::ScrollDown) {
                return;
            }
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
            let on_problems = self
                .status_problems()
                .is_some_and(|(at, end)| x >= at && x < end);
            if mouse.kind == MouseKind::Left && on_problems {
                self.toggle_problems();
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
                MouseKind::Right => self.review_line_menu(x, y),
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
                let down = mouse.kind == MouseKind::ScrollDown;
                let step = self.wheel.lines(down, std::time::Instant::now()) as isize;
                let delta = if down { step } else { -step };
                if in_sidebar && self.sidebar_view == SidebarView::Search {
                    self.search.scroll_by(delta);
                } else if in_sidebar && self.sidebar_view == SidebarView::Outline {
                    self.outline.scroll_by(delta);
                } else if in_sidebar && self.sidebar_view == SidebarView::Source {
                    self.source.scroll_by(delta);
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
            MouseKind::Left if !in_sidebar && self.click_welcome(x, y) => {}
            MouseKind::Left if in_sidebar && self.sidebar_view == SidebarView::Source => {
                self.click_source(x, y);
            }
            MouseKind::Right if in_sidebar && self.sidebar_view == SidebarView::Source => {}
            MouseKind::Left if in_sidebar && self.sidebar_view == SidebarView::Outline => {
                self.click_outline(y);
            }
            MouseKind::Right if in_sidebar && self.sidebar_view == SidebarView::Outline => {}
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
                self.toggle_sidebar();
            }
            MouseKind::Left
                if self
                    .page_switch()
                    .is_some_and(|(cell, _)| cell.contains(x, y)) =>
            {
                self.run(Command::MarkdownPreview);
            }
            MouseKind::Left if self.tabs_area().contains(x, y) => self.click_tab(x),
            MouseKind::Left => {
                let Some(pane) = self.pane_at(x, y) else {
                    return;
                };
                self.search.focus = None;
                self.focus = pane;
                let area = self.focused_area();
                let target = self
                    .editors
                    .get(self.active())
                    .filter(|e| e.rendered)
                    .and_then(|e| e.row_target(area, y));
                match target {
                    Some(Target::File(path)) => {
                        if let Err(e) = self.open(&path) {
                            self.say(format!("Could not open it: {e}"));
                        }
                        return;
                    }
                    Some(Target::Preamble) => {
                        if let Some(editor) = self.editor_mut() {
                            editor.toggle_preamble();
                        }
                        return;
                    }
                    None => {}
                }
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
                    if let Some(line) = editor.image_edge(area, x as f32, y) {
                        self.drag = Drag::Image(line);
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
            MouseKind::Up => {
                if let Drag::Image(line) = self.drag {
                    let area = self.focused_area();
                    if let Some(editor) = self.editor_mut() {
                        editor.drag_image(area, line, x as f32);
                    }
                }
                self.drag = Drag::None;
            }
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
        let notebook = editor.notebook.is_some();
        let agent = [
            run("Add Comment", Command::AddComment).after_rule(),
            run("Send Selection to Agent", Command::MentionSelection).when(selected),
        ];
        let mut items = Vec::new();
        if !editor.rendered && editor.has_page() {
            items.push(run("Show Page", Command::MarkdownPreview));
        }
        if editor.rendered {
            if let Some(issue) = editor.issue_at_caret() {
                let word = editor.words_in(issue.start, issue.end);
                for fix in &issue.fixes {
                    let shown = if fix.is_empty() {
                        "Remove"
                    } else {
                        fix.as_str()
                    };
                    let action = MenuAction::Fix(issue.start, issue.end, fix.clone());
                    items.push(MenuItem::new(shown.to_string(), action));
                }
                if issue.spelling {
                    items.push(
                        MenuItem::new("Add to Dictionary", MenuAction::Learn(word)).after_rule(),
                    );
                }
                let last = items.last_mut();
                if let Some(item) = last.filter(|_| !issue.spelling) {
                    item.rule = false;
                }
            }
            if editor.change_at_caret().is_some() {
                items.extend([
                    run("Accept Change", Command::AcceptChange),
                    run("Reject Change", Command::RejectChange),
                    run("Accept All Changes", Command::AcceptAllChanges).after_rule(),
                    run("Reject All Changes", Command::RejectAllChanges),
                ]);
                items.extend(agent);
                return items;
            }
            if editor.image_at_caret() {
                let size = |name, percent| MenuItem::new(name, MenuAction::ImageWidth(percent));
                items.extend([
                    size("Small", 25),
                    size("Medium", 50),
                    size("Large", 75),
                    size("Full Width", 100),
                    MenuItem::new("Replace Image…", MenuAction::ReplaceImage).after_rule(),
                    MenuItem::new("Delete Image", MenuAction::RemoveImage),
                ]);
                items.extend(agent);
                return items;
            }
            if notebook {
                items.extend([
                    run("Run Cell", Command::RunCell),
                    run("Run All Cells", Command::RunAllCells),
                    run("Interrupt Kernel", Command::InterruptKernel),
                    run("Restart Kernel", Command::RestartKernel),
                    run("Clear All Outputs", Command::ClearOutputs),
                    MenuItem::new("Add Code Cell", MenuAction::AddCodeCell).after_rule(),
                ]);
            }
            items.push(run("Show Source", Command::MarkdownPreview));
            items.extend(edit);
            let format = |name, format| run(name, Command::Format(format));
            let data = !editor.dialect().is_writing();
            if data {
                let table = |name, edit| MenuItem::new(name, MenuAction::Table(edit));
                items.extend([
                    table("Insert Row Above", TableEdit::RowAbove).after_rule(),
                    table("Insert Row Below", TableEdit::RowBelow),
                    table("Insert Column Left", TableEdit::ColumnLeft),
                    table("Insert Column Right", TableEdit::ColumnRight),
                    table("Delete Row", TableEdit::DeleteRow).after_rule(),
                    table("Delete Column", TableEdit::DeleteColumn),
                ]);
                return items;
            }
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
            // A paper's menu names LaTeX's own things, and builds.
            if editor.dialect() == Dialect::Latex {
                items.extend([
                    format("Bold", Format::Bold).after_rule(),
                    format("Italic", Format::Italic),
                    format("Typewriter", Format::Code),
                    format("Paragraph", Format::PlainText).after_rule(),
                    format("Section", Format::Heading),
                    format("Verbatim", Format::CodeBlock),
                    MenuItem::new("Insert Citation…", MenuAction::Tool(Tool::Cite)).after_rule(),
                    MenuItem::new("Insert Formula…", MenuAction::Tool(Tool::Math)),
                    MenuItem::new("Insert Table…", MenuAction::Tool(Tool::Table)),
                    MenuItem::new("Insert Figure…", MenuAction::Tool(Tool::Image)),
                    run("Build PDF", Command::BuildPdf).after_rule(),
                    run("Show in PDF", Command::ShowInPdf),
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
                MenuItem::new("Insert Diagram…", MenuAction::Tool(Tool::Diagram)),
            ]);
            items.extend(agent);
        } else {
            let server = language.is_some_and(|l| self.servers.contains_key(l));
            items.extend([
                run("Go to Definition", Command::GoToDefinition).when(server),
                run("Go to References", Command::GoToReferences).when(server),
                run("Rename Symbol", Command::RenameSymbol).when(server),
                run("Quick Fix…", Command::QuickFix).when(server),
                run("Format Document", Command::FormatDocument)
                    .when(server || language.is_some_and(|l| formatter(l).is_some())),
            ]);
            if matches!(language, Some("Markdown" | "LaTeX")) {
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
        // A Review line's menu left unanswered isn't what this menu's Comment is about.
        self.review_line = None;
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
            MenuAction::Run(Command::AddComment) if self.review_line.is_some() => {
                // Comment on the line the menu was opened on, as a click on its number does.
                if let Some((file, line, quote, removed)) = self.review_line.take() {
                    self.comment_quote = Some((quote, removed));
                    self.ask(
                        &format!("Comment on line {line}"),
                        PromptAction::Comment(file, line),
                    );
                }
            }
            MenuAction::Run(command) => self.run(command),
            MenuAction::Tool(tool) => self.use_tool(tool, self.main_area().x),
            MenuAction::Fix(start, end, text) => {
                if let Some(editor) = self.editor_mut() {
                    editor.apply_fix(start, end, &text);
                }
            }
            MenuAction::ServerFix(index) => {
                let Some((title, files)) = self.code_actions.get(index).cloned() else {
                    return;
                };
                for (path, edits) in files {
                    if let Err(e) = self.apply_edits(&path, edits) {
                        self.say(format!("Could not apply it in {}: {e}", path.display()));
                        return;
                    }
                }
                self.say(format!("Applied: {title}"));
            }
            MenuAction::RejectLine(undo) => self.reject_line(undo),
            MenuAction::Learn(word) => {
                if let Some(spell) = &self.spell {
                    spell.learn(&word);
                }
                for editor in &mut self.editors {
                    editor.forget_issues(&word);
                }
            }
            MenuAction::InPane(name, argv) => {
                let root = self.workspace.root().to_path_buf();
                self.start_session(&name, &argv, &[], &root);
            }
            MenuAction::AddCodeCell => self.add_cell(true),
            MenuAction::ImageWidth(percent) => {
                if let Some(editor) = self.editor_mut() {
                    editor.set_image_width(percent);
                }
            }
            MenuAction::RemoveImage => {
                if let Some(editor) = self.editor_mut() {
                    editor.remove_image();
                }
            }
            MenuAction::ReplaceImage => self.choose_image(true),
            MenuAction::PasteImage => self.paste_image(),
            MenuAction::ChooseImage => self.choose_image(false),
            MenuAction::TypeImagePath => {
                self.overlay = Overlay::Prompt(Prompt {
                    label: "Image file".to_string(),
                    input: String::new(),
                    action: PromptAction::Image,
                });
            }
            MenuAction::Diagram(index) => {
                let (_, language, source) = cellix_doc::diagram::TEMPLATES[index];
                let area = self.focused_area();
                if let Some(editor) = self.editor_mut() {
                    editor.prepare_edit(area);
                    editor.insert_diagram(language, source);
                }
                self.reveal();
            }
            MenuAction::PaperDiagram(index) => {
                let (_, source) = cellix_doc::diagram::PAPER_TEMPLATES[index];
                let area = self.focused_area();
                if let Some(editor) = self.editor_mut() {
                    editor.prepare_edit(area);
                    editor.insert_block(source, 2);
                }
                self.reveal();
            }
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
        let on_tabs = self.tabs_area().contains(mouse.x, mouse.y);
        // A middle click closes the Settings tab, as it does any other.
        if mouse.kind == MouseKind::Middle && on_tabs {
            self.settings_page = None;
            return;
        }
        if mouse.kind != MouseKind::Left {
            return;
        }
        if on_tabs {
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
                (self.vim, self.neovim) = keymap_engines(Some(name));
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
            Control::Codex
            | Control::Gemini
            | Control::InstallServer(_)
            | Control::InstallBrowser => {
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
                Control::InstallBrowser => install_with(BROWSER_INSTALL, BROWSER_TOOL),
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
        let index = self.rail_at(y, area);
        if let Some((_, command)) = index.and_then(|i| ACTIVITY.get(i)) {
            let settings = *command == Command::OpenSettings;
            let lit = !settings && self.settings_page.is_none() && self.rail_on(*command);
            if !settings {
                self.settings_page = None;
            }
            // A click on what's already showing closes it, as it does in VS Code.
            if lit {
                self.hide_sidebar();
                return;
            }
            match command {
                Command::ToggleSidebar => {
                    self.show_explorer();
                    self.show_sidebar = true;
                    self.drawer = self.compact();
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
                let y = self.tabs_area().y;
                if x + 3 >= at + width {
                    self.run(Command::CloseTab);
                } else if self.count_click(x, y) == 2 {
                    self.toggle_maximized(Maximized::Editor);
                } else {
                    self.press_tab(Dragged::Editor(i), x, y);
                }
                return;
            }
        }
    }

    /// Where tiles are cut: at the edges of what moves on its own, so that text scrolling in
    /// one place doesn't redraw what stands beside it.
    fn tile_bands(&self) -> Vec<u16> {
        let main = self.main_area();
        let mut bands = vec![main.x, main.right()];
        if let Some(area) = self.dock_area().filter(|_| !self.dock_under()) {
            bands.push(area.x);
        }
        for (pane, index) in self.panes.iter().enumerate() {
            let area = self.pane_area(pane);
            if let (Some(editor), true) = (self.editors.get(*index), area.w > 0) {
                let text = editor.scroll_region(area);
                bands.extend([text.x, text.right()]);
            }
        }
        bands.sort_unstable();
        bands.dedup();
        bands
    }

    pub fn draw(&self, grid: &mut Grid) {
        grid.bands = self.tile_bands();
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
                SidebarView::Outline => {
                    let here = (self.outline_here(), !self.editors.is_empty());
                    self.outline.draw(grid, area, here, self.hover, theme);
                }
                SidebarView::Source => {
                    let branch = self.branch.as_deref();
                    let measure = self.measure.clone();
                    let source = &self.source;
                    source.draw(grid, area, branch, self.hover, measure.as_ref(), theme);
                }
            }
        }
        self.draw_dock(grid);
        if self.maximized == Some(Maximized::Pane) && self.dock.open {
            self.draw_status(grid);
            self.draw_tab_drag(grid);
            self.draw_pane_actions(grid);
            self.draw_overlay(grid);
            self.draw_tooltip(grid);
            return;
        }
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
            self.draw_pane_actions(grid);
            self.draw_overlay(grid);
            self.draw_tooltip(grid);
            return;
        }
        if let Some(review) = &self.review {
            let comments = self.comments.lock().map(|c| c.clone()).unwrap_or_default();
            review.title_room.set(self.actions_over_main());
            review.draw(grid, self.main_area(), self.hover, theme, &comments);
            self.draw_status(grid);
            self.draw_pane_actions(grid);
            self.draw_overlay(grid);
            self.draw_tooltip(grid);
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
                    comments: file
                        .as_ref()
                        .map_or_else(Vec::new, |f| self.comment_lines(f)),
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
                if let Some(hover) = self.hover.filter(|h| area.contains(h.0, h.1)) {
                    editor.draw_hover(grid, area, hover, &self.theme);
                }
                if pane + 1 < self.panes.len() && self.maximized.is_none() {
                    let line = Rect::new(area.right(), area.y, 1, area.h);
                    grid.fill(line, theme.tab);
                }
            }
        }
        if self.plan_in_view() {
            self.draw_plan_bar(grid);
        }
        self.draw_companion(grid);
        self.draw_status(grid);
        self.draw_suggest(grid);
        self.draw_hover(grid);
        self.draw_offer(grid);
        self.draw_pane_actions(grid);
        self.draw_tab_drag(grid);
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
        let cut = self.dock_area().filter(|a| y == a.y);
        let cut = cut.and_then(|area| self.dock.cut_name(area, x));
        // Where each one goes: its name, left edge and top, in cells and rows.
        let spot = match &self.overlay {
            Overlay::Find(_) => {
                let area = self.find_bar();
                let (field, _, _, first) = Self::find_layout(area);
                let replacing = area.h > 1;
                // Each button's name and the column it starts at.
                let mut buttons: Vec<(&str, u16, u16)> = vec![(FIND_NAMES[3], area.x + 1, area.y)];
                for (n, name) in FIND_NAMES[..3].iter().enumerate() {
                    buttons.push((*name, first + n as u16 * FIND_BUTTON, area.y));
                }
                if replacing {
                    let at = field.right() + 1;
                    buttons.push((FIND_NAMES[4], at, area.y + 1));
                    buttons.push((FIND_NAMES[5], at + FIND_BUTTON, area.y + 1));
                }
                let found = buttons
                    .into_iter()
                    .find(|(_, left, row)| y == *row && x >= *left && x < left + FIND_BUTTON);
                found.map(|(name, left, _)| {
                    // Under its button, clear of the bar's edge.
                    let middle = left as f32 + 1.5;
                    let right_edge = self.size.0 as f32 - wide(name) - 0.5;
                    let start = (middle - wide(name) / 2.0).min(right_edge);
                    (name, start, area.bottom() as f32 + 0.3)
                })
            }
            Overlay::None if self.on_maximize_button(x, y) => {
                let name = if self.maximized == Some(Maximized::Pane) {
                    RESTORE_TIP
                } else {
                    MAXIMIZE_TIP
                };
                // Starting where the button's chip does, clear of the bar's text to its left.
                let area = self.dock_area().unwrap_or_default();
                let start = self.dock.maximize_button(area) as f32 + 0.5;
                let left = start.min(self.size.0 as f32 - wide(name) - 0.5);
                Some((name, left.max(0.5), area.y as f32 + 1.2))
            }
            Overlay::None if cut.is_some() => cut.as_deref().map(|name| {
                let left = (x as f32).min(self.size.0 as f32 - wide(name) - 0.5);
                (name, left, self.dock_area().map_or(0, |a| a.y) as f32 + 1.2)
            }),
            Overlay::None if self.action_at(x, y).is_some() => {
                let actions = self.pane_actions();
                let found = actions.iter().find(|(cell, ..)| cell.contains(x, y));
                found.map(|(cell, _, _, name)| {
                    let middle = cell.x as f32 + cell.w as f32 / 2.0;
                    let left = middle - wide(name) / 2.0;
                    (*name, left.min(self.size.0 as f32 - wide(name) - 0.5), 1.2)
                })
            }
            Overlay::None
                if self.dock_area().is_some_and(|a| {
                    y == a.y && x + PANE_CLOSE_REACH >= a.right() && x < a.right()
                }) =>
            {
                let name = "Close pane. What runs here keeps running.";
                Some((name, self.size.0 as f32 - wide(name) - 0.5, 1.2))
            }
            Overlay::None if rail.w > 0 && rail.contains(x, y) => {
                self.rail_at(y, rail).map(|i| {
                    let top = self.rail_top(i, rail);
                    // Level with the icon's middle.
                    (
                        RAIL_NAMES[i],
                        rail.right() as f32 + 1.0,
                        top + 0.5 - TOOLTIP_HEIGHT / 2.0,
                    )
                })
            }
            Overlay::None => self
                .toolbar_row()
                .filter(|_| y == 1)
                .and_then(|row| toolbar::hit(row, x, self.page_bar()))
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
            // What it does, and then, quieter, the other way to do it.
            let (head, tail) = name
                .split_once(" or ")
                .map_or((name, ""), |(head, _)| (head, &name[head.len()..]));
            let start = (left + 1.5, text_top);
            grid.text_run(start, end, head, (theme.text.fg, false), font);
            if !tail.is_empty() {
                let after = left + 1.5 + self.measure.width(head, font);
                let quiet = (theme.popup_dim, false);
                grid.text_run((after, text_top), end, tail, quiet, font);
            }
        }
    }

    fn press_tab(&mut self, what: Dragged, x: u16, y: u16) {
        self.tab_drag = Some(TabDrag {
            what,
            from: (x, y),
            at: (x, y),
            fine_x: f32::from(x),
            moving: false,
        });
    }

    /// Follows a tab being dragged, and lands it when it's let go.
    fn drag_tab(&mut self, mouse: MouseEvent) {
        let Some(mut drag) = self.tab_drag.take() else {
            return;
        };
        drag.at = (mouse.x, mouse.y);
        drag.fine_x = mouse.fine_x;
        drag.moving |= mouse.x.abs_diff(drag.from.0) >= DRAG_START || mouse.y != drag.from.1;
        if mouse.kind == MouseKind::Drag {
            self.tab_drag = Some(drag);
        } else if drag.moving {
            let landing = self.landing(&drag);
            self.land(drag.what, landing);
        }
    }

    /// Whether there's room for one more pane beside the others.
    fn room_to_split(&self) -> bool {
        let count = self.panes.len() as u16 + 1;
        !self.compact() && self.main_area().w / count >= LEAST_PANE
    }

    /// The editor tab a drag at `x` would go before, as an index into all the tabs.
    fn tab_slot(&self, x: u16) -> usize {
        let tabs = self.visible_tabs();
        let found = tabs.iter().find(|(_, at, width)| x < at + width / 2);
        found.map_or(self.editors.len(), |(index, ..)| *index)
    }

    /// Where a dragged tab would land if it were let go now.
    fn landing(&self, drag: &TabDrag) -> Landing {
        let (x, y) = drag.at;
        let main = self.main_area();
        match drag.what {
            Dragged::Editor(editor) => {
                if y == self.tabs_area().y && x >= main.x && x < main.right() {
                    return Landing::Tabs(self.tab_slot(x));
                }
                let Some(pane) = self.pane_at(x, y) else {
                    return Landing::Nowhere;
                };
                let area = self.pane_area(pane);
                let here = self.panes[pane] == editor;
                let third = area.w / 3;
                let middle = x >= area.x + third && x < area.right() - third;
                if !here && (middle || !self.room_to_split()) {
                    return Landing::Show(pane);
                }
                if !self.room_to_split() {
                    return Landing::Nowhere;
                }
                Landing::Split(pane, x >= area.x + area.w / 2)
            }
            Dragged::View(_) | Dragged::Session(_) => {
                let views = matches!(drag.what, Dragged::View(_));
                if let Some(area) = self.dock_area().filter(|a| y == a.y && a.contains(x, y)) {
                    return Landing::PaneTabs(self.dock.slot(area, x, views).0);
                }
                if self.maximized.is_some() {
                    return Landing::Nowhere;
                }
                let room = self.size.0.saturating_sub(self.main_left());
                let edge = (self.size.0 / 12).max(6);
                if self.dock_under() {
                    let fits = !self.dock.too_narrow(room);
                    let on_edge = x + edge >= self.size.0 && y + 1 < self.size.1;
                    return if fits && on_edge {
                        Landing::Right
                    } else {
                        Landing::Nowhere
                    };
                }
                if main.contains(x, y) && y >= main.y + main.h / 2 && self.size.1 > STACK_HEIGHT {
                    return Landing::Under;
                }
                Landing::Nowhere
            }
        }
    }

    /// What letting go would do, for the status bar.
    fn landing_said(&self, drag: &TabDrag) -> String {
        let other = " · on the tab bar to move it there";
        match self.landing(drag) {
            Landing::Tabs(_) | Landing::PaneTabs(_) => "Drop to move the tab here".to_string(),
            Landing::Split(_, true) => format!("Drop to split right{other}"),
            Landing::Split(_, false) => format!("Drop to split left{other}"),
            Landing::Show(_) => "Drop to show it in this pane".to_string(),
            Landing::Under => {
                "Drop to put the pane under the file · on the right edge to put it back".to_string()
            }
            Landing::Right => "Drop to put the pane back on the right".to_string(),
            Landing::Nowhere if matches!(drag.what, Dragged::Editor(_)) => {
                "Drop on the tab bar to move it, or on a pane to split it · Esc cancels".to_string()
            }
            Landing::Nowhere => {
                "Drop under the file, or on the right edge, to move the pane · Esc cancels"
                    .to_string()
            }
        }
    }

    /// Puts a dragged tab where it was let go.
    fn land(&mut self, what: Dragged, landing: Landing) {
        match (what, landing) {
            (Dragged::Editor(editor), Landing::Tabs(to)) => self.move_editor(editor, to),
            (Dragged::Editor(editor), Landing::Split(pane, right)) => {
                let at = pane + usize::from(right);
                self.move_out_of_panes(editor);
                self.panes.insert(at, editor);
                self.focus = at;
            }
            (Dragged::Editor(editor), Landing::Show(pane)) => {
                self.move_out_of_panes(editor);
                self.panes[pane] = editor;
                self.focus = pane;
            }
            (Dragged::View(view), Landing::PaneTabs(to)) => self.dock.move_view(view, to),
            (Dragged::Session(from), Landing::PaneTabs(to)) => self.dock.move_session(from, to),
            (_, Landing::Under) => self.dock.under = true,
            (_, Landing::Right) => self.dock.under = false,
            _ => {}
        }
        self.fit_dock();
    }

    /// Moves an editor's tab to stand before the one at `to`, counted before it moves.
    fn move_editor(&mut self, from: usize, to: usize) {
        let to = if to > from { to - 1 } else { to };
        let to = to.min(self.editors.len().saturating_sub(1));
        if to == from || from >= self.editors.len() {
            return;
        }
        let editor = self.editors.remove(from);
        self.editors.insert(to, editor);
        for pane in &mut self.panes {
            *pane = crate::dock::moved(*pane, from, to);
        }
        self.threads_seen = None;
    }

    /// Panes showing an editor that's moving to another pane show its neighbour instead, so
    /// the tab moves rather than copies. With one editor there's nothing else to show.
    fn move_out_of_panes(&mut self, editor: usize) {
        if self.editors.len() < 2 {
            return;
        }
        let other = if editor > 0 { editor - 1 } else { editor + 1 };
        for pane in &mut self.panes {
            if *pane == editor {
                *pane = other;
            }
        }
    }

    /// Where a dragged tab would land, and the tab under the pointer.
    fn draw_tab_drag(&self, grid: &mut Grid) {
        let Some(drag) = self.tab_drag.filter(|d| d.moving) else {
            return;
        };
        let theme = &self.theme;
        match self.landing(&drag) {
            Landing::Tabs(slot) => {
                let tabs = self.visible_tabs();
                // In the gap before a tab, and never left of the bar.
                let first = f32::from(self.tabs_area().x) + 0.2;
                let x = match tabs.iter().find(|(index, ..)| *index == slot) {
                    Some((_, at, _)) => (f32::from(*at) - 0.5).max(first),
                    None => tabs
                        .last()
                        .map_or(f32::from(self.tabs_area().x), |(_, at, width)| {
                            f32::from(at + width) + 0.5
                        }),
                };
                self.insertion_bar(grid, x, self.tabs_area().y);
            }
            Landing::PaneTabs(_) => {
                if let Some(area) = self.dock_area() {
                    let views = matches!(drag.what, Dragged::View(_));
                    let (_, x) = self.dock.slot(area, drag.at.0, views);
                    self.insertion_bar(grid, f32::from(x), area.y);
                }
            }
            Landing::Split(pane, right) => {
                let area = self.pane_area(pane);
                let half = area.w / 2;
                let x = if right { area.x + half } else { area.x };
                self.drop_zone(grid, Rect::new(x, area.y, area.w - half, area.h));
            }
            Landing::Show(pane) => self.drop_zone(grid, self.pane_area(pane)),
            Landing::Under => {
                // Where the pane will stand: under the file and across where it stands now.
                let left = self.main_left();
                let height = self.size.1.saturating_sub(1);
                let rows = self.dock.height.unwrap_or(height * STACK_SHARE / 100);
                let rows = rows.clamp(STACK_LEAST, height.saturating_sub(STACK_LEAST));
                let zone = Rect::new(left, height - rows, self.size.0 - left, rows);
                self.drop_zone(grid, zone);
            }
            Landing::Right => {
                let left = self.main_left();
                let room = self.size.0.saturating_sub(left);
                let width = self.dock.width.unwrap_or(room * 46 / 100).clamp(1, room);
                let zone = Rect::new(self.size.0 - width, 1, width, self.size.1.saturating_sub(2));
                self.drop_zone(grid, zone);
            }
            Landing::Nowhere => {}
        }
        // The tab itself, under the pointer.
        let (icon, name) = match drag.what {
            Dragged::Editor(index) => (Icon::File, self.editors[index].title()),
            Dragged::View(view) => {
                let icon = match view {
                    Front::Pdf => Icon::Picture,
                    Front::Comments => Icon::Comment,
                    Front::Agents => Icon::Agents,
                    _ => Icon::Outline,
                };
                (icon, self.dock.view_name(view).to_string())
            }
            Dragged::Session(index) => {
                let session = self.dock.sessions.get(index);
                let icon = match session {
                    Some(s) if s.browser => Icon::Globe,
                    Some(s) if s.shell => Icon::Terminal,
                    _ => Icon::Agents,
                };
                (icon, session.map(|s| s.name.clone()).unwrap_or_default())
            }
        };
        let font = Font::default();
        let wide = self.measure.width(&name, font) + 4.5;
        // Below and right of the pointer, so what it's aiming at stays in sight.
        let left = (drag.fine_x + 0.7).min(f32::from(self.size.0) - wide - 0.5);
        let top = f32::from(drag.at.1) + 1.0;
        let top = top.min(f32::from(self.size.1) - 2.4);
        if self.measure.in_cells() {
            let style = theme.tab_active;
            let label = format!(" {name} ");
            grid.put_str(left as u16, top.round() as u16, &label, style, self.size.0);
            return;
        }
        let bounds = Bounds::cells(left, top, wide, 1.0);
        grid.popup(bounds, 26, theme.tint, theme.popup_border);
        let icon_bounds = Bounds::cells(left + 0.9, top + 0.18, 2.0, 0.64);
        grid.paint.push(cellix_render::Paint::Icon {
            bounds: icon_bounds,
            icon,
            color: theme.text.fg,
        });
        let colour = (theme.text.fg, false);
        grid.text_run((left + 3.0, top), left + wide, &name, colour, font);
    }

    /// The thin accent bar between tabs where a dragged one would go.
    fn insertion_bar(&self, grid: &mut Grid, x: f32, y: u16) {
        let theme = &self.theme;
        if self.measure.in_cells() {
            let style = Style {
                fg: theme.accent,
                ..theme.tab
            };
            grid.put_str(x.round() as u16, y, "▏", style, self.size.0);
            return;
        }
        let bounds = Bounds::cells(x - 0.06, f32::from(y) + 0.18, 0.12, 0.64);
        grid.panel(bounds, 6, Some(theme.accent), None);
    }

    /// The accent wash, with a border, over where a dragged tab would land.
    fn drop_zone(&self, grid: &mut Grid, zone: Rect) {
        let theme = &self.theme;
        if zone.w < 3 || zone.h < 2 {
            return;
        }
        if self.measure.in_cells() {
            grid.set_style(zone, |style| Style {
                bg: theme.tint,
                ..style
            });
            return;
        }
        // Clear of the edges, as the design's 12 px are.
        let bounds = Bounds::cells(
            f32::from(zone.x) + 0.7,
            f32::from(zone.y) + 0.35,
            f32::from(zone.w) - 1.4,
            f32::from(zone.h) - 0.7,
        );
        grid.wash(bounds, 28, (theme.accent, DROP_WASH), Some(theme.accent));
    }

    /// Whether the pointer is on the side pane's maximize button.
    fn on_maximize_button(&self, x: u16, y: u16) -> bool {
        self.dock_area()
            .is_some_and(|area| y == area.y && self.dock.hit(area, x) == Some(TabHit::Maximize))
    }

    /// Lets the side pane or the editor in front take the window, or gives it back.
    fn toggle_maximized(&mut self, which: Maximized) {
        if self.maximized == Some(which) {
            self.maximized = None;
        } else {
            match which {
                Maximized::Pane if !self.dock.open => self.toggle_dock(),
                Maximized::Editor if self.editors.is_empty() => {
                    self.say("Open a file to maximize it");
                    return;
                }
                Maximized::Editor => self.dock.focused = false,
                Maximized::Pane => {}
            }
            self.maximized = Some(which);
            self.drawer = false;
        }
        self.fit_dock();
    }

    fn in_view(&self) -> InView {
        InView {
            dock: (self.dock.open, self.dock.front),
            sidebar: (self.show_sidebar, self.sidebar_view, self.drawer),
            editor: (self.active(), self.editors.len()),
            panes: self.panes.len(),
            review: self.review.is_some(),
            settings: self.settings_page.is_some(),
        }
    }

    /// Gives the window back when what was just asked for stands behind what's maximized,
    /// so nothing opens out of sight: the pane or the sidebar behind an editor, or a file
    /// behind the pane.
    fn keep_in_view(&mut self, before: Option<InView>) {
        let (Some(before), Some(which)) = (before, self.maximized) else {
            return;
        };
        let now = self.in_view();
        let opened = |was: bool, is: bool| is && !was;
        let around = now.sidebar != before.sidebar
            || now.panes != before.panes
            || opened(before.review, now.review)
            || opened(before.settings, now.settings);
        let behind = match which {
            Maximized::Editor => self.dock.open && now.dock != before.dock,
            Maximized::Pane => !self.dock.open || now.editor != before.editor,
        };
        if around || behind {
            self.maximized = None;
            self.fit_dock();
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

    /// The top of an icon's row, which can fall half way down a cell. The last one is
    /// Settings, which sits at the bottom given room.
    fn rail_top(&self, index: usize, area: Rect) -> f32 {
        let step = if self.measure.in_cells() {
            ACTIVITY_STEP_CELLS
        } else {
            ACTIVITY_STEP
        };
        // The first level with the sidebar's header, as the design has it.
        let stacked = area.y as f32 + index as f32 * step;
        if index + 1 == ACTIVITY.len() {
            stacked.max(area.bottom().saturating_sub(2) as f32)
        } else {
            stacked
        }
    }

    /// The row an icon's middle is on, for clicking it.
    #[cfg(test)]
    fn rail_row(&self, index: usize, area: Rect) -> u16 {
        (self.rail_top(index, area) + 0.5).floor() as u16
    }

    /// The icon a row belongs to: each owns the rows whose middle is nearer it than the next.
    fn rail_at(&self, y: u16, area: Rect) -> Option<usize> {
        let middle = y as f32 + 0.5;
        (0..ACTIVITY.len())
            .min_by(|a, b| {
                let off = |i: usize| (self.rail_top(i, area) + 0.5 - middle).abs();
                off(*a).total_cmp(&off(*b))
            })
            .filter(|i| (self.rail_top(*i, area) + 0.5 - middle).abs() < 1.0)
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
        if !self.measure.in_cells() {
            // A hairline between the rail and the sidebar, half way to the border's colour.
            let (a, b) = (self.theme.activity.bg, self.theme.border);
            let mix = |x: u8, y: u8| ((u16::from(x) + u16::from(y)) / 2) as u8;
            let line = Rgb(mix(a.0, b.0), mix(a.1, b.1), mix(a.2, b.2));
            let edge = Bounds::cells(
                area.right() as f32 - 0.08,
                area.y as f32,
                0.08,
                area.h as f32,
            );
            grid.panel(edge, 0, Some(line), None);
        }
        for (i, (icon, command)) in ACTIVITY.iter().enumerate() {
            let top = self.rail_top(i, area);
            let y = top.floor() as u16;
            // How far below its cell the icon sits, for the paint that can.
            let lift = top - y as f32;
            if y + 1 >= area.bottom() {
                break;
            }
            let on = self.rail_on(*command);
            let row = Rect::new(area.x, y, area.w, ACTIVITY_ICON.1 as u16);
            let hovered = self
                .hover
                .is_some_and(|h| area.contains(h.0, h.1) && self.rail_at(h.1, area) == Some(i));
            let style = Style {
                fg: if on || hovered {
                    self.theme.text.fg
                } else {
                    self.theme.activity.fg
                },
                ..self.theme.activity
            };
            let wash = on.then_some(self.theme.tint);
            let wash = wash.or(hovered.then_some(self.theme.row_hover));
            let style = wash.map_or(style, |bg| Style { bg, ..style });
            if on && self.measure.in_cells() {
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
            if let Some(fill) = wash.filter(|_| !self.measure.in_cells()) {
                // With pixels, the pill is paint and sized to the icon.
                let (wide, high) = RAIL_PILL;
                let left = area.x as f32 + (area.w as f32 - wide) / 2.0;
                let pill_top = top + (row.h as f32 - high) / 2.0;
                let pill = Bounds::cells(left, pill_top, wide, high);
                grid.panel(pill, PILL_RADIUS, Some(fill), None);
            } else if wash.is_some() {
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
            if self.measure.in_cells() {
                grid.put_icon(area.x + 1, y, *icon, ACTIVITY_ICON, style);
            } else {
                let cell = Rect::new(area.x, y, area.w, row.h);
                let shape = (*icon, RAIL_ICON);
                // The pill is paint now, so the cells under it keep the rail's own colour.
                let plain = Style {
                    bg: self.theme.activity.bg,
                    ..style
                };
                toolbar::draw_icon(grid, shape, cell, style.fg, plain);
                grid.drop_last(lift);
            }
            let changed = self.changed_count();
            if *command == Command::ShowSourceControl && changed > 0 {
                let color = self.theme.button_text;
                let font = Font {
                    bold: true,
                    size: BADGE_SIZE,
                    ..Font::default()
                };
                let count = changed.min(99).to_string();
                // On the chip's bottom right corner, ringed in the rail's colour so it stands
                // clear of the icon.
                let (wide, high) = RAIL_PILL;
                let chip_right = area.x as f32 + (area.w as f32 + wide) / 2.0;
                let chip_bottom = top + (1.0 + high) / 2.0;
                let width = (self.measure.width(&count, font) + 0.5).max(1.0);
                let (left, top) = (chip_right - width + 0.3, chip_bottom - 0.52);
                let ring = Bounds::cells(left - 0.12, top - 0.06, width + 0.24, 0.58);
                grid.panel(ring, 30, Some(self.theme.activity.bg), None);
                grid.panel(
                    Bounds::cells(left, top, width, 0.46),
                    30,
                    Some(self.theme.accent),
                    None,
                );
                let text = self.measure.width(&count, font);
                grid.text_run(
                    (left + (width - text) / 2.0, top - 0.27),
                    area.right() as f32 + 1.0,
                    &count,
                    (color, false),
                    font,
                );
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
            if active && !self.measure.in_cells() {
                let edge = Bounds::cells(x as f32, area.y as f32, tab.w as f32, 0.06);
                grid.panel(edge, 0, Some(theme.accent), None);
            }
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
        let author = self.agents.plan_author().unwrap_or("An agent");
        let end = grid.label(
            main.x + 2,
            main.y,
            &format!("{author} is waiting on this plan"),
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
            let bar = self.page_bar();
            let tracking = self.editors[self.active()].tracking;
            // Text scrolled under the bar needs a line to go under.
            if self.editors[self.active()].top() > 0 {
                let edge = row.y as f32 + 1.0;
                let line = Bounds::cells(row.x as f32, edge, row.w as f32, 0.0);
                grid.hairline(line, self.theme.popup_border);
            }
            toolbar::draw(
                grid,
                row,
                (paragraph, &active, tracking),
                bar,
                self.hover,
                &self.theme,
            );
            if bar == Bar::Notebook {
                let status = self.kernel_status();
                let right = row.right().saturating_sub(KERNEL_STATUS_END);
                let quiet = Style {
                    fg: self.theme.status.fg,
                    ..self.theme.breadcrumb
                };
                grid.label_right(right, row.y, &status, quiet, 90);
            }
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
        if let Some((cell, name)) = self.page_switch() {
            if self.hover.is_some_and(|h| cell.contains(h.0, h.1)) {
                let fill = Some(self.theme.row_hover);
                grid.panel(Bounds::inset(cell, 0.1, 0.08), 16, fill, None);
            }
            grid.label_at(
                cell.x as f32 + 1.0,
                cell.x + 1,
                1,
                name,
                style,
                cell.right(),
                90,
            );
        }
    }

    /// The row the document toolbar sits in, if the page in view has one.
    fn toolbar_row(&self) -> Option<Rect> {
        let main = self.main_area();
        let page = self
            .editors
            .get(self.active())
            .is_some_and(|e| e.rendered && e.dialect().is_writing());
        let room = main.w >= toolbar::MIN_WIDTH;
        (page && room && self.chrome_rows() == 2 && !self.plan_in_view())
            .then(|| Rect::new(main.x, 1, main.w, 1))
    }

    fn click_toolbar(&mut self, x: u16) -> bool {
        let bar = self.page_bar();
        let button = self.toolbar_row().and_then(|row| toolbar::hit(row, x, bar));
        let Some(button) = button else {
            return false;
        };
        self.use_tool(button.tool, button.x);
        true
    }

    /// Whether the keyboard is on a document's page.
    fn on_page(&self) -> bool {
        let page = self.editors.get(self.active()).is_some_and(|e| e.rendered);
        page && self.review.is_none() && self.settings_page.is_none() && self.search.focus.is_none()
    }

    fn page_bar(&self) -> Bar {
        let notebook = self
            .editors
            .get(self.active())
            .is_some_and(|e| e.notebook.is_some());
        if notebook {
            Bar::Notebook
        } else {
            Bar::Page(self.page_dialect())
        }
    }

    /// What the kernel is doing, for the notebook's bar.
    fn kernel_status(&self) -> String {
        let Some(editor) = self.editors.get(self.active()) else {
            return String::new();
        };
        let Some(book) = editor.notebook.as_ref() else {
            return String::new();
        };
        let started = self
            .active_file()
            .is_some_and(|f| self.kernels.contains_key(&f));
        let name = book.kernel();
        match (started, book.running.len()) {
            (_, 0) if started => format!("{name}, idle"),
            (_, 0) => format!("{name}, not started"),
            (_, 1) => format!("{name}, running"),
            (_, many) => format!("{name}, running {many} cells"),
        }
    }

    fn page_dialect(&self) -> Dialect {
        let editor = self.editors.get(self.active());
        editor.map(Editor::dialect).unwrap_or_default()
    }

    /// Lists the pictures in this project, to pick one.
    fn choose_image(&mut self, replacing: bool) {
        let items: Vec<Item> = self
            .workspace
            .files()
            .into_iter()
            .filter(|path| is_image(path))
            .map(|path| {
                let label = self.relative(&path);
                Item::Image(path, label, replacing)
            })
            .collect();
        if items.is_empty() {
            self.say("No pictures in this project yet");
            return;
        }
        self.overlay = Overlay::Palette(Palette::asking(items, "Image"));
    }

    /// The folder this document's relative paths start from.
    fn document_folder(&self) -> PathBuf {
        let editor = self.editors.get(self.active());
        let folder = editor.and_then(|e| e.buffer.path()).and_then(Path::parent);
        folder.map_or_else(|| self.workspace.root().to_path_buf(), Path::to_path_buf)
    }

    /// Where a picture goes in the document. One from outside the project is copied in
    /// beside it, so the document still works on another machine.
    fn image_reference(&self, path: &Path) -> io::Result<String> {
        let folder = self.document_folder();
        // Written with `/` between folders, on Windows too, so the document reads anywhere.
        let link = |path: &Path| {
            path.to_string_lossy()
                .replace(std::path::MAIN_SEPARATOR, "/")
        };
        if let Ok(relative) = path.strip_prefix(&folder) {
            return Ok(link(relative));
        }
        let name = path.file_name().ok_or(io::ErrorKind::InvalidInput)?;
        let target = folder.join(IMAGE_FOLDER).join(name);
        std::fs::create_dir_all(folder.join(IMAGE_FOLDER))?;
        if target != path {
            std::fs::copy(path, &target)?;
        }
        Ok(link(&Path::new(IMAGE_FOLDER).join(name)))
    }

    fn place_image(&mut self, path: &Path, replacing: bool) {
        let reference = match self.image_reference(path) {
            Ok(reference) => reference,
            Err(e) => {
                self.say(format!("Could not add the picture: {e}"));
                return;
            }
        };
        let area = self.focused_area();
        if let Some(editor) = self.editor_mut() {
            editor.prepare_edit(area);
            if replacing {
                editor.replace_image(&reference);
            } else {
                editor.insert_image(&reference);
            }
        }
        self.reveal();
    }

    /// Saves the picture on the clipboard beside the document, and puts it in.
    fn paste_image(&mut self) {
        let folder = self.document_folder().join(IMAGE_FOLDER);
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let target = folder.join(format!("pasted-{stamp}.png"));
        let saved = std::fs::create_dir_all(&folder).is_ok()
            && ran_well(&self.clipboard_image, &target.display().to_string())
            && target.metadata().is_ok_and(|m| m.len() > 0);
        if saved {
            self.place_image(&target, false);
        } else {
            let _ = std::fs::remove_file(&target);
            self.say("There's no picture on the clipboard");
        }
    }

    /// The keys in the project's bibliographies, to pick one to cite: the ones the paper names,
    /// from its root's folder, then any in the usual places.
    fn show_citations(&mut self) {
        let root = self.workspace.root().to_path_buf();
        let editor = self.editors.get(self.active());
        let path = editor.and_then(|e| e.buffer.path()).map(Path::to_path_buf);
        let named = match (editor, &path) {
            (Some(editor), Some(path)) if editor.dialect() == Dialect::Latex => {
                let paper = cellix_doc::latex::root_of(path, &editor.buffer.text());
                cellix_doc::latex::Project::read(&paper).bibliographies
            }
            _ => Vec::new(),
        };
        let beside = path
            .as_deref()
            .and_then(Path::parent)
            .map(Path::to_path_buf);
        let mut folders = vec![root.clone(), root.join("references"), root.join("bib")];
        folders.extend(beside.filter(|folder| *folder != root));
        let found = folders.into_iter().flat_map(|folder| {
            std::fs::read_dir(folder)
                .into_iter()
                .flatten()
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|e| e == "bib"))
        });
        let mut seen: Vec<PathBuf> = Vec::new();
        let mut items = Vec::new();
        for path in named.into_iter().chain(found) {
            let canonical = path.canonicalize().unwrap_or_else(|_| path.clone());
            if seen.contains(&canonical) {
                continue;
            }
            seen.push(canonical);
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            let name = self.relative(&path);
            let entries = cellix_doc::latex::bib_entries(&text);
            items.extend(entries.into_iter().map(|e| {
                let note = format!("{}: {}", e.short(), e.title);
                Item::Citation(
                    e.key,
                    if e.title.is_empty() {
                        name.clone()
                    } else {
                        note
                    },
                )
            }));
        }
        if items.is_empty() {
            self.say("No .bib file in this project yet");
            return;
        }
        self.overlay = Overlay::Palette(Palette::asking(items, "Cite"));
    }

    /// What builds papers here, ready to take a file.
    fn latex_engine(&self) -> Option<Vec<String>> {
        if let Some(engine) = &self.settings.latex_engine {
            return Some(engine.clone());
        }
        let var = |name: &str| std::env::var(name).ok();
        let found = ENGINES
            .iter()
            .find(|engine| crate::integrations::on_path(engine[0], var))?;
        Some(found.iter().map(ToString::to_string).collect())
    }

    /// Runs a LaTeX file through the engine on a thread, and shows the PDF when it's done.
    fn start_build(&mut self, file: PathBuf) {
        if self.build.is_some() {
            self.say("A build is already running");
            return;
        }
        let Some(mut engine) = self.latex_engine() else {
            self.note_missing("LaTeX");
            self.not_now.retain(|l| l != "LaTeX");
            self.say("Building needs a LaTeX engine");
            return;
        };
        let (done, result) = std::sync::mpsc::channel();
        let name = file.file_name().map(PathBuf::from).unwrap_or_default();
        let folder = file.parent().map(Path::to_path_buf).unwrap_or_default();
        engine.push(name.display().to_string());
        self.say(format!("Building {}…", name.display()));
        let started = std::time::Instant::now();
        std::thread::spawn(move || {
            let run = |program: &str, args: &[String]| {
                std::process::Command::new(program)
                    .args(args)
                    .current_dir(&folder)
                    .stdin(std::process::Stdio::null())
                    .output()
            };
            let mut output = run(&engine[0], &engine[1..]);
            let mut bibliography = None;
            let mut bib_said = String::new();
            // pdflatex makes one pass, where tectonic and latexmk go on until references settle.
            let worked = output.as_ref().is_ok_and(|o| o.status.success());
            let program = Path::new(&engine[0]);
            if worked && program.file_name() == Some(std::ffi::OsStr::new("pdflatex")) {
                let beside = |ext: &str| folder.join(&name).with_extension(ext);
                let read = |ext: &str| std::fs::read_to_string(beside(ext)).unwrap_or_default();
                let (tool, passes) =
                    cellix_doc::latex::more_passes(&read("aux"), beside("bcf").is_file());
                if let Some(tool) = tool {
                    // TeX Live keeps its tools together, so a pdflatex named by path brings its
                    // own BibTeX and Biber.
                    let near = program
                        .parent()
                        .filter(|p| !p.as_os_str().is_empty())
                        .map(|p| p.join(tool.program()))
                        .filter(|p| p.is_file());
                    let found = near.or_else(|| {
                        let var = |name: &str| std::env::var(name).ok();
                        crate::integrations::on_path(tool.program(), var)
                            .then(|| PathBuf::from(tool.program()))
                    });
                    let stem = name.with_extension("").display().to_string();
                    match found.map(|path| run(&path.display().to_string(), &[stem])) {
                        Some(Ok(ran)) => {
                            let text = format!(
                                "{}{}",
                                String::from_utf8_lossy(&ran.stdout),
                                String::from_utf8_lossy(&ran.stderr)
                            );
                            bibliography = tool
                                .failure(ran.status.code(), &text)
                                .map(|why| (tool, why));
                            bib_said = text;
                        }
                        Some(Err(e)) => bibliography = Some((tool, e.to_string())),
                        None => {
                            bibliography = Some((tool, format!("{} isn't installed", tool.name())));
                        }
                    }
                }
                for _ in 0..passes {
                    output = run(&engine[0], &engine[1..]);
                }
                // Labels in `\include`d chapters' own .aux files, and anything else still
                // moving, show only in the log.
                for _ in 0..3 {
                    if !cellix_doc::latex::asks_rerun(&read("log")) {
                        break;
                    }
                    output = run(&engine[0], &engine[1..]);
                }
            }
            let (worked, said) = match output {
                Ok(done) => (
                    done.status.success(),
                    format!(
                        "{}{}{bib_said}",
                        String::from_utf8_lossy(&done.stdout),
                        String::from_utf8_lossy(&done.stderr)
                    ),
                ),
                Err(e) => (false, format!("error: {e}")),
            };
            let _ = done.send(Build {
                file,
                worked,
                said,
                took: started.elapsed(),
                bibliography,
            });
        });
        self.build = Some(result);
    }

    fn rebuild_on_save(&mut self) {
        let showing = self.dock.pdf.is_some() && self.build.is_none();
        let editor = self.editors.get(self.active());
        let Some(editor) = editor.filter(|e| showing && e.dialect() == Dialect::Latex) else {
            return;
        };
        let Some(path) = editor.buffer.path().map(Path::to_path_buf) else {
            return;
        };
        let root = cellix_doc::latex::root_of(&path, &editor.buffer.text());
        self.start_build(root);
    }

    /// Saves the paper's other open files with unsaved edits, so a build reads them as they
    /// stand, as LaTeX Workshop does. One changed on disk meanwhile is left for its own tab.
    fn save_paper(&mut self, root: &Path) {
        let active = self.active();
        let mut saved = Vec::new();
        let mut failed = Vec::new();
        for (index, editor) in self.editors.iter_mut().enumerate() {
            let Some(path) = editor.buffer.path().map(Path::to_path_buf) else {
                continue;
            };
            if index == active
                || !editor.buffer.is_dirty()
                || editor.conflict
                || editor.dialect() != Dialect::Latex
            {
                continue;
            }
            let theirs = cellix_doc::latex::root_of(&path, &editor.buffer.text());
            if !cellix_doc::paper::same_file(&theirs, root) {
                continue;
            }
            match editor.save() {
                Ok(()) => saved.push(editor.title()),
                Err(e) => failed.push(format!("{}: {e}", editor.title())),
            }
        }
        if !failed.is_empty() {
            self.say(format!("Could not save {}", failed.join(", ")));
        } else if !saved.is_empty() {
            self.say(format!("Saved {}", saved.join(", ")));
        }
    }

    fn build_pdf(&mut self) {
        let Some(editor) = self.editors.get(self.active()) else {
            return;
        };
        let Some(path) = editor.buffer.path().map(Path::to_path_buf) else {
            self.say("Save the file first");
            return;
        };
        if editor.dialect() != Dialect::Latex {
            self.say("Build works on LaTeX files. Export to PDF works on Markdown");
            return;
        }
        let root = cellix_doc::latex::root_of(&path, &editor.buffer.text());
        self.save_paper(&root);
        self.save();
        // Saving already builds when the PDF is open.
        if self.build.is_none() {
            self.start_build(root);
        }
    }

    /// Opens what changed since the plan's last version, as suggestions on a page of its own.
    fn compare_plan_versions(&mut self) {
        self.compare_plan_with(None, None);
    }

    /// What changed since a given earlier version of a plan, or the last one. The plan is the
    /// one given, else the one waiting, else the one in view.
    fn compare_plan_with(&mut self, plan: Option<&Path>, version: Option<usize>) {
        let path = match (plan, &self.plan) {
            (Some(path), _) => path.to_path_buf(),
            (None, Some(plan)) => plan.path.clone(),
            (None, None) => match self
                .editors
                .get(self.active())
                .and_then(|e| e.buffer.path())
            {
                Some(path) => path.to_path_buf(),
                None => return,
            },
        };
        let mut versions = plan_versions(&path);
        if let Some(version) = version.filter(|v| (1..=versions.len()).contains(v)) {
            versions.truncate(version);
        }
        let Some(earlier) = versions.pop() else {
            self.say("This plan has no earlier version");
            return;
        };
        let old = std::fs::read_to_string(&earlier).unwrap_or_default();
        let new = std::fs::read_to_string(&path).unwrap_or_default();
        let marked = cellix_doc::changes::compare(&old, &new);
        let stem = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let changes = path.with_file_name(format!("{stem} changes.md"));
        match std::fs::write(&changes, marked).and_then(|()| self.open(&changes)) {
            Ok(()) => self.say(format!(
                "What changed since {}",
                earlier
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default()
            )),
            Err(e) => self.say(format!("Could not compare: {e}")),
        }
    }

    /// Markdown goes through LaTeX, so its math comes out as well as a paper's. Suggestions
    /// not yet settled are asked about first, since a PDF would print them as they stand.
    fn export_pdf(&mut self) {
        let Some(editor) = self.editors.get(self.active()) else {
            return;
        };
        if editor.dialect() == Dialect::Latex {
            self.build_pdf();
            return;
        }
        let text = editor.buffer.text();
        let settled =
            cellix_doc::changes::settle_all(&text, cellix_doc::changes::Syntax::Critic, true);
        if settled == text {
            self.export_pdf_with(false);
            return;
        }
        self.overlay = Overlay::Confirm(Confirm {
            title: "This document has suggestions".to_string(),
            detail: "The PDF can show them accepted, or as they stand.".to_string(),
            buttons: vec![
                ("Accept All in the PDF", Choice::ExportPdf(true)),
                ("Show Them", Choice::ExportPdf(false)),
                ("Cancel", Choice::Cancel),
            ],
            selected: 0,
        });
    }

    fn export_pdf_with(&mut self, accepted: bool) {
        let Some(editor) = self.editors.get(self.active()) else {
            return;
        };
        let Some(path) = editor.buffer.path().map(Path::to_path_buf) else {
            self.say("Save the file first");
            return;
        };
        // Built under a hidden name, so nothing of the build is left in the project.
        let (out, pdf) = export_names(&path);
        // LaTeX can't read a diagram, so each goes in as a picture beside the document.
        let folder = out.parent().map(Path::to_path_buf).unwrap_or_default();
        let mut made = vec![out.clone()];
        let mut text = editor.buffer.text();
        if accepted {
            text =
                cellix_doc::changes::settle_all(&text, cellix_doc::changes::Syntax::Critic, true);
        }
        let text = svg_as_pictures(&text, &folder, &mut made);
        let tex = cellix_doc::latex::from_markdown_with(&text, &mut |kind, source| {
            let name = format!(".diagram-{}.png", made.len());
            let file = folder.join(&name);
            write_diagram(kind, source, &file)?;
            made.push(file);
            Some(name)
        });
        match std::fs::write(&out, tex) {
            Ok(()) => {
                self.remove_after_build = made;
                self.export_to = Some(pdf);
                self.start_build(out);
            }
            Err(e) => self.say(format!("Could not write the LaTeX file: {e}")),
        }
    }

    /// Takes in a finished build: the PDF beside the source, or its errors on their lines.
    fn poll_build(&mut self) -> bool {
        let finished = self
            .build
            .as_ref()
            .and_then(|result| result.try_recv().ok());
        let Some(build) = finished else {
            return false;
        };
        self.build = None;
        for path in self.remove_after_build.drain(..) {
            let _ = std::fs::remove_file(path);
        }
        let folder = build
            .file
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_default();
        let mut problems = cellix_doc::latex::problems(&build.said, &folder);
        // An export's LaTeX is gone by now, and its lines aren't the document's, so its errors
        // are only told, with nothing to jump to.
        let exported = self.export_to.is_some();
        let hidden: Vec<String> = if exported {
            let (lost, kept) = problems.into_iter().partition(|p| p.file == build.file);
            problems = kept;
            lost.into_iter().map(|p| p.message).collect()
        } else {
            Vec::new()
        };
        self.dock.log = build.said.lines().map(str::to_string).collect();
        self.dock.log_top = 0;
        self.dock.log_folder = folder.clone();
        let mut by_file: HashMap<PathBuf, Vec<Diagnostic>> = HashMap::new();
        for problem in &problems {
            by_file
                .entry(problem.file.clone())
                .or_default()
                .push(Diagnostic {
                    line: problem.line.saturating_sub(1),
                    column: 0,
                    message: problem.message.clone(),
                    severity: 1,
                });
        }
        let pdf = build.file.with_extension("pdf");
        let log = std::fs::read_to_string(build.file.with_extension("log")).unwrap_or_default();
        for place in cellix_doc::latex::warning_places(&build.said, &log, &folder, &build.file) {
            if exported && place.file == build.file {
                continue;
            }
            by_file.entry(place.file).or_default().push(Diagnostic {
                line: place.line.saturating_sub(1),
                column: 0,
                message: place.message,
                severity: 2,
            });
        }
        self.diagnostics
            .retain(|p, _| p.extension().is_none_or(|e| e != "tex"));
        self.diagnostics.extend(by_file);
        let warned = cellix_doc::latex::warnings(&build.said, &log).len();
        // An export was built under a hidden name: the PDF takes the document's, and the rest
        // of the build goes.
        let pdf = match self.export_to.take() {
            Some(target) => {
                // pdflatex and latexmk write a PDF despite errors: that one never replaces the
                // last good export.
                if build.worked && pdf.exists() {
                    let _ = std::fs::rename(&pdf, &target);
                } else {
                    let _ = std::fs::remove_file(&pdf);
                }
                let leftovers = [
                    "log",
                    "aux",
                    "out",
                    "synctex.gz",
                    "fls",
                    "fdb_latexmk",
                    "bbl",
                    "blg",
                    "toc",
                ];
                for leftover in leftovers {
                    let _ = std::fs::remove_file(build.file.with_extension(leftover));
                }
                target
            }
            None => pdf,
        };
        if build.worked && pdf.exists() {
            self.show_pdf(&pdf);
            let seconds = build.took.as_secs_f32();
            self.say(match (&build.bibliography, warned) {
                (Some((tool, why)), _) => {
                    format!("Built in {seconds:.1} s, but {} failed: {why}", tool.name())
                }
                (None, 0) => format!("Built in {seconds:.1} s"),
                (None, 1) => format!("Built in {seconds:.1} s, with a warning"),
                (None, many) => format!("Built in {seconds:.1} s, with {many} warnings"),
            });
        } else if let Some(first) = problems.first() {
            self.dock.open = true;
            self.dock.front = Front::Log;
            self.dock.focused = false;
            self.go_to_problem(first);
        } else if let Some(message) = hidden.first() {
            self.dock.open = true;
            self.dock.front = Front::Log;
            self.dock.focused = false;
            self.say(format!("The PDF wasn't made: {message}"));
        } else {
            self.dock.open = true;
            self.dock.front = Front::Log;
            self.dock.focused = false;
            // The first error is the cause. What follows is the engine giving up.
            let first = build.said.lines().find(|l| l.starts_with("error:"));
            let last = build.said.lines().rfind(|l| !l.trim().is_empty());
            let reason = first.or(last).unwrap_or("no output").trim();
            let reason = reason.trim_start_matches("error: ");
            self.say(format!("The build failed: {reason}"));
        }
        true
    }

    /// Every problem known, file by file, as rows of the Problems tab.
    fn problem_rows(&self) -> Vec<ProblemRow> {
        let mut files: Vec<&PathBuf> = self.diagnostics.keys().collect();
        files.sort();
        let mut rows = Vec::new();
        for file in files {
            let mut found = self.diagnostics[file].clone();
            if found.is_empty() {
                continue;
            }
            found.sort_by_key(|d| (d.severity, d.line));
            rows.push(ProblemRow::File(self.relative(file)));
            let items = found.into_iter().map(|d| ProblemRow::Item(file.clone(), d));
            rows.extend(items);
        }
        rows
    }

    fn draw_problems(&self, grid: &mut Grid, screen: Rect) {
        let theme = &self.theme;
        grid.fill(screen, theme.sidebar);
        let rows = self.problem_rows();
        let quiet = Style {
            fg: theme.gutter.fg,
            ..theme.sidebar
        };
        if rows.is_empty() {
            grid.label(
                screen.x + 2,
                screen.y + 1,
                "No problems here",
                quiet,
                screen.right(),
            );
            return;
        }
        let shown = rows.iter().skip(self.dock.problems_top);
        for (index, row) in shown.take(screen.h as usize).enumerate() {
            let y = screen.y + index as u16;
            let line = Rect::new(screen.x, y, screen.w, 1);
            if self.hover.is_some_and(|h| line.contains(h.0, h.1)) {
                grid.panel(
                    Bounds::inset(line, 0.4, 0.06),
                    12,
                    Some(theme.row_hover),
                    None,
                );
            }
            match row {
                ProblemRow::File(name) => {
                    let bold = Style {
                        bold: true,
                        ..theme.sidebar
                    };
                    grid.label(screen.x + 1, y, name, bold, screen.right());
                }
                ProblemRow::Item(_, problem) => {
                    let colour = if problem.severity == 1 {
                        theme.deleted()
                    } else {
                        theme.attention
                    };
                    let mark = Style {
                        fg: colour,
                        ..theme.sidebar
                    };
                    grid.label(screen.x + 2, y, "●", mark, screen.x + 4);
                    let at = format!("{}  ", problem.line + 1);
                    let text = grid.label(screen.x + 4, y, &at, quiet, screen.right());
                    grid.label(text, y, &problem.message, theme.sidebar, screen.right());
                }
            }
        }
    }

    /// A problem goes to its place.
    fn click_problem(&mut self, y: u16) {
        let Some(screen) = self.dock_screen() else {
            return;
        };
        let Some(row) = y.checked_sub(screen.y) else {
            return;
        };
        let rows = self.problem_rows();
        let picked = rows.get(self.dock.problems_top + row as usize);
        if let Some(ProblemRow::Item(file, problem)) = picked {
            let line = problem.line + 1;
            if self.open(file).is_ok() {
                let area = self.focused_area();
                if let Some(editor) = self.editor_mut() {
                    editor.go_to_line(line, area);
                }
            }
            self.dock.focused = false;
        }
    }

    /// Opens or closes the list of problems, from the count in the status bar.
    fn toggle_problems(&mut self) {
        if self.dock.open && self.dock.front == Front::Problems {
            self.dock.close_view(Front::Problems);
            return;
        }
        self.dock.problems = true;
        self.dock.problems_top = 0;
        self.dock.open = true;
        self.dock.front = Front::Problems;
        self.dock.focused = false;
    }

    /// A line of the build's log that names a place goes there.
    fn log_click(&mut self, y: u16) {
        let Some(line) = self.dock_screen().and_then(|s| self.dock.log_line(s, y)) else {
            return;
        };
        let folder = self.dock.log_folder.clone();
        let problems = cellix_doc::latex::problems(&line, &folder);
        if let Some(first) = problems.first() {
            self.go_to_problem(first);
        }
    }

    fn go_to_problem(&mut self, problem: &cellix_doc::latex::Problem) {
        let name = self.relative(&problem.file);
        // A file the build has since removed would only open empty.
        if !problem.file.is_file() {
            self.say(format!("{name}:{}: {}", problem.line, problem.message));
            return;
        }
        if self.open(&problem.file).is_ok() {
            let area = self.focused_area();
            if let Some(editor) = self.editor_mut() {
                editor.go_to_line(problem.line, area);
            }
        }
        self.say(format!("{name}:{}: {}", problem.line, problem.message));
    }

    /// Shows a PDF in the side pane, keeping the place if it's the one already there.
    fn show_pdf(&mut self, path: &Path) {
        self.dock.open = true;
        self.dock.front = Front::Pdf;
        self.dock.focused = false;
        let same = self.dock.pdf.as_ref().is_some_and(|p| p.path == path);
        if !same {
            self.dock.pdf = Some(PdfView::new(path));
        }
        let width = self.dock_screen().map_or(40, |a| a.w);
        if let (Some(view), Some(cell)) = (&mut self.dock.pdf, self.measure.cell_pixels()) {
            view.load(width, cell);
        }
    }

    fn use_tool(&mut self, tool: Tool, x: u16) {
        let below = (x, 2);
        match tool {
            Tool::Format(format) => self.run(Command::Format(format)),
            Tool::Styles => {
                let below = (x, 3);
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
                let items = vec![
                    MenuItem::new("From This Project…", MenuAction::ChooseImage),
                    MenuItem::new("Paste from Clipboard", MenuAction::PasteImage),
                    MenuItem::new("Type a Path…", MenuAction::TypeImagePath),
                ];
                self.show_menu((x, 3), items);
            }
            Tool::Diagram if self.page_dialect() == Dialect::Latex => {
                let templates = cellix_doc::diagram::PAPER_TEMPLATES.iter().enumerate();
                let items = templates
                    .map(|(index, (name, _))| MenuItem::new(*name, MenuAction::PaperDiagram(index)))
                    .collect();
                self.show_menu((x, 3), items);
            }
            Tool::Diagram => {
                let templates = cellix_doc::diagram::TEMPLATES.iter().enumerate();
                let items = templates
                    .map(|(index, (name, ..))| MenuItem::new(*name, MenuAction::Diagram(index)))
                    .collect();
                self.show_menu((x, 3), items);
            }
            Tool::Comment => self.run(Command::AddComment),
            Tool::Track => self.run(Command::ToggleTracking),
            Tool::RunCell => self.run(Command::RunCell),
            Tool::RunAll => self.run(Command::RunAllCells),
            Tool::Interrupt => self.run(Command::InterruptKernel),
            Tool::Restart => self.run(Command::RestartKernel),
            Tool::AddCode => self.add_cell(true),
            Tool::AddText => self.add_cell(false),
            Tool::Cite => self.show_citations(),
            Tool::Build => self.run(Command::BuildPdf),
            Tool::Export => {
                let below = (x, 3);
                let items = vec![
                    MenuItem::run("Export to PDF", Command::ExportPdf),
                    MenuItem::run("Open in Browser", Command::OpenInBrowser),
                ];
                self.show_menu(below, items);
            }
            Tool::More => {
                let row = self.toolbar_row().unwrap_or_default();
                let items = toolbar::overflow(row, self.page_bar())
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

    /// What to do next with nothing open, which depends on what's already going.
    fn welcome_lead(&self) -> String {
        let running = self.dock.sessions.iter().find(|s| s.agent());
        match running {
            Some(agent) => format!(
                "{} is running. Click a file on the left to work on it together.",
                agent.name
            ),
            None if self.dock.session().is_some() => {
                "Click a file on the left to open it beside the terminal.".to_string()
            }
            None => "Open a file, or start an agent to work beside it.".to_string(),
        }
    }

    /// The actions of an empty window, as a row of the screen and what each does.
    fn welcome_rows(&self) -> Vec<(u16, &'static str, Option<&'static str>, Command)> {
        let main = self.main_area();
        if main.h < WELCOME_MIN_HEIGHT {
            return Vec::new();
        }
        let top = main.y + main.h / 4 + WELCOME_LEAD_ROWS;
        let has_agent = self.dock.sessions.iter().any(Session::agent);
        let mut rows = vec![
            ("Open a File", Some("Ctrl+P"), Command::QuickOpen),
            ("New Document", None, Command::NewDocument),
        ];
        if !has_agent {
            rows.push(("Start an Agent", None, Command::ShowAgents));
        }
        rows.push(("Open a Terminal", Some("Ctrl+J"), Command::ToggleTerminal));
        rows.extend([
            ("Review Changes", Some("Ctrl+G"), Command::OpenDiff),
            (
                "Search the Project",
                Some("Ctrl+Shift+F"),
                Command::FindInProject,
            ),
            (
                "All Commands",
                Some("Ctrl+Shift+P"),
                Command::CommandPalette,
            ),
            ("Getting Started", None, Command::ShowGuide),
        ]);
        rows.iter()
            .enumerate()
            .map(|(i, (name, keys, command))| (top + i as u16, *name, *keys, *command))
            .collect()
    }

    /// Where the actions of an empty window stand.
    fn welcome_box(&self) -> Rect {
        let main = self.main_area();
        let width = WELCOME_WIDTH.min(main.w.saturating_sub(4));
        Rect::new(
            main.x + main.w.saturating_sub(width) / 2,
            main.y,
            width,
            main.h,
        )
    }

    /// Runs the action of an empty window under a click.
    fn click_welcome(&mut self, x: u16, y: u16) -> bool {
        if !self.editors.is_empty() || !self.welcome_box().contains(x, y) {
            return false;
        }
        let rows = self.welcome_rows();
        let Some((.., command)) = rows.iter().find(|row| row.0 == y) else {
            return false;
        };
        self.run(*command);
        true
    }

    fn draw_welcome(&self, grid: &mut Grid) {
        let main = self.main_area();
        if main.h < WELCOME_MIN_HEIGHT {
            return;
        }
        let theme = &self.theme;
        let area = self.welcome_box();
        let y = main.y + main.h / 4;
        let x = area.x;
        let title = Style {
            bold: true,
            ..theme.text
        };
        grid.label_at(x as f32, x, y, "Cellix", title, area.right(), 180);
        let quiet = Style {
            fg: theme.gutter.fg,
            ..theme.text
        };
        grid.label_at(
            x as f32,
            x,
            y + 2,
            &self.welcome_lead(),
            quiet,
            main.right(),
            95,
        );
        for (line, name, keys, _) in self.welcome_rows() {
            let row = Rect::new(area.x, line, area.w, 1);
            if self.hover.is_some_and(|h| row.contains(h.0, h.1)) {
                let wash = Bounds::cells(
                    area.x as f32 - 0.6,
                    line as f32 + 0.04,
                    area.w as f32 + 1.2,
                    0.92,
                );
                grid.panel(wash, 16, Some(theme.row_hover), None);
            }
            let link = Style {
                fg: theme.accent,
                ..theme.text
            };
            grid.label_at(x as f32, x, line, name, link, area.right(), 95);
            if let Some(keys) = keys {
                grid.label_right(area.right(), line, keys, quiet, 90);
            }
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
        let hidden = match self.maximized {
            Some(Maximized::Pane) => self
                .editors
                .get(self.active())
                .map(|e| format!("{} hidden while the pane is maximized", e.title())),
            Some(Maximized::Editor) => self.editors.get(self.active()).map(|e| {
                let name = e.title();
                format!("Sidebar and panes hidden while {name} is maximized · double-click its tab to restore")
            }),
            None => None,
        };
        let landing = self
            .tab_drag
            .filter(|d| d.moving)
            .map(|d| self.landing_said(&d));
        let left = if let Some(landing) = &landing {
            landing.as_str()
        } else if !self.neovim_line.is_empty() {
            typed.as_str()
        } else if !message.is_empty() {
            message.as_str()
        } else if let Some(hidden) = &hidden {
            hidden.as_str()
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
                match editor.dialect() {
                    Dialect::Data(',') => "CSV".to_string(),
                    Dialect::Data(_) => "TSV".to_string(),
                    _ if editor.notebook.is_some() => "Notebook".to_string(),
                    _ => language.to_string(),
                },
                "UTF-8".to_string(),
                // A writer counts words, and has no use for indent width.
                if editor.rendered {
                    let others: Vec<&Editor> = self.editors.iter().collect();
                    let (all, selected, paper) = editor.word_count(&others);
                    // A paper over several files counts whole, and says so.
                    let whole = if paper { " in the paper" } else { "" };
                    let words = match selected {
                        Some(selected) => format!("{selected} of {all} words{whole}"),
                        None => format!("{all} words{whole}"),
                    };
                    if editor.tracking {
                        format!("Tracking changes  {words}")
                    } else {
                        words
                    }
                } else if editor.tracking {
                    "Tracking changes".to_string()
                } else {
                    "Spaces: 4".to_string()
                },
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
            if let Some((at, end)) = self.status_problems()
                && self
                    .hover
                    .is_some_and(|h| h.1 == y && h.0 >= at && h.0 < end)
            {
                let chip = Rect::new(at, y, end.saturating_sub(at + 1), 1);
                let wash = Bounds::inset(chip, -0.3, 0.12);
                grid.panel(wash, 16, Some(self.theme.row_hover), None);
            }
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

    /// Where the count of problems sits in the status bar, as (start, end) columns.
    fn status_problems(&self) -> Option<(u16, u16)> {
        let count: usize = self.diagnostics.values().map(Vec::len).sum();
        if count == 0 || self.compact() {
            return None;
        }
        let at = self.status_branch().map_or(1, |(_, end)| end + 1);
        Some((at, at + format!("⊗ {count}  ").chars().count() as u16))
    }

    /// The pointer for what's under it: a hand for what can be pressed, a bar for text.
    pub fn pointer(&self) -> &'static str {
        const ARROW: &str = "default";
        const HAND: &str = "pointer";
        const TEXT: &str = "text";
        let Some((x, y)) = self.hover else {
            return ARROW;
        };
        if !matches!(self.overlay, Overlay::None) {
            return ARROW;
        }
        if self.drag == Drag::Preview && self.preview_press.is_some_and(|p| p.panning) {
            return "grabbing";
        }
        let on_pages = self.dock.showing(Front::Pdf)
            && self.dock_screen().is_some_and(|s| {
                y > s.y && s.contains(x, y) && self.dock.pdf.as_ref().is_some_and(|v| v.wide(s.w))
            });
        if on_pages {
            return "grab";
        }
        if self.on_dock_edge(x, y) {
            return if self.dock_under() {
                "ns-resize"
            } else {
                "ew-resize"
            };
        }
        let on_chip = self
            .selection_bar()
            .is_some_and(|(comment, ask)| comment.contains(x, y) || ask.contains(x, y))
            || self
                .picture_comment()
                .is_some_and(|(chip, _)| chip.contains(x, y))
            || self
                .run_button()
                .is_some_and(|(chip, _)| chip.contains(x, y))
            || self.view_comment().is_some_and(|chip| chip.contains(x, y))
            || self
                .page_switch()
                .is_some_and(|(chip, _)| chip.contains(x, y));
        let status = y + 1 == self.size.1
            && (self
                .status_branch()
                .is_some_and(|(at, end)| x >= at && x < end)
                || self
                    .status_problems()
                    .is_some_and(|(at, end)| x >= at && x < end));
        let dock = self.dock_area().filter(|a| a.contains(x, y));
        let dock_title = dock.is_some_and(|a| y == a.y);
        let in_rail = self.activity_area().contains(x, y);
        let in_sidebar = self.sidebar_area().is_some_and(|a| a.contains(x, y));
        let in_tabs = self.tabs_area().contains(x, y);
        let in_toolbar = self.toolbar_row().is_some_and(|r| r.contains(x, y));
        if on_chip || status || dock_title || in_rail || in_sidebar || in_tabs || in_toolbar {
            return HAND;
        }
        if dock.is_some() {
            let writing = self.dock.front == Front::Session && self.dock.session().is_some();
            return if writing { TEXT } else { ARROW };
        }
        let in_file = self.review.is_none()
            && self.settings_page.is_none()
            && !self.editors.is_empty()
            && self.pane_at(x, y).is_some();
        if in_file { TEXT } else { ARROW }
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

/// `path` with `folder` in front, joined as this platform joins a PATH: with `;` on Windows,
/// where a `:` would run into a drive letter.
fn path_with(folder: &Path, path: &std::ffi::OsStr) -> Option<String> {
    let folders = std::iter::once(folder.to_path_buf()).chain(std::env::split_paths(path));
    std::env::join_paths(folders).ok()?.into_string().ok()
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

    /// An installer that asks on the terminal, as terminal-browser's does: with a terminal it
    /// waits for an answer that never comes, without one it goes on. The terminal is tried on
    /// `true`, not `exec`: dash, Linux's `sh`, ends the script when `exec` can't open it.
    const ASKS_ON_THE_TERMINAL: &str = "if true 2>/dev/null <>/dev/tty; then \
         printf 'Install it? [Y/n] ' >/dev/tty; read answer </dev/tty; fi; true";

    /// Run inside a terminal by the test below; skipped on its own.
    #[test]
    #[ignore = "run by an_installer_that_asks_on_the_terminal_still_finishes"]
    fn install_inside_a_terminal() {
        println!("said: {}", install_with(ASKS_ON_THE_TERMINAL, "the tool"));
    }

    /// Cellix runs in a terminal, so its installs did too, and terminal-browser's setup asked
    /// on Cellix's own screen and waited for good: Settings said another install was running
    /// until Cellix was closed. This runs an install from inside a real terminal.
    #[cfg(unix)]
    #[test]
    fn an_installer_that_asks_on_the_terminal_still_finishes() {
        use portable_pty::{CommandBuilder, PtySize, native_pty_system};
        use std::io::Read;
        let pty = native_pty_system()
            .openpty(PtySize {
                rows: 24,
                cols: 80,
                pixel_width: 0,
                pixel_height: 0,
            })
            .unwrap();
        let mut command = CommandBuilder::new(std::env::current_exe().unwrap());
        command.args([
            "--exact",
            "app::tests::install_inside_a_terminal",
            "--ignored",
            "--nocapture",
        ]);
        let mut child = pty.slave.spawn_command(command).unwrap();
        drop(pty.slave);
        let mut reader = pty.master.try_clone_reader().unwrap();
        let (sender, said) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut text = String::new();
            let mut chunk = [0; 4096];
            while let Ok(n) = reader.read(&mut chunk) {
                if n == 0 {
                    break;
                }
                text.push_str(&String::from_utf8_lossy(&chunk[..n]));
                let _ = sender.send(text.clone());
            }
        });
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        let mut text = String::new();
        while !text.contains("said: ") && std::time::Instant::now() < deadline {
            if let Ok(more) = said.recv_timeout(std::time::Duration::from_millis(100)) {
                text = more;
            }
        }
        let _ = child.kill();
        assert!(
            text.contains("said: Installed the tool"),
            "the install waited on the terminal:\n{text}"
        );
        assert!(
            !text.contains("Install it?"),
            "it asked on the terminal:\n{text}"
        );
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
        // As the platform writes it: `src\lib.rs` on Windows, as VS Code shows it there.
        let shown = Path::new("src").join("lib.rs").display().to_string();
        assert!(screen(&app).text().contains(&shown));
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
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while app.disk_look.is_some() {
            assert!(std::time::Instant::now() < deadline);
            app.poll();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(screen(&app).text().contains("from-agent.txt"));
    }

    #[test]
    fn the_regular_look_at_the_disk_never_holds_up_the_screen() {
        let (dir, mut app) = project();
        fs::write(dir.path().join("from-agent.txt"), "x").unwrap();
        for _ in 0..EXPLORER_REFRESH_TICKS {
            app.handle(Event::Tick);
        }
        assert!(app.disk_look.is_some(), "it went off on its own");
        assert!(
            !app.explorer
                .entries
                .iter()
                .any(|e| e.name == "from-agent.txt"),
            "and the tick didn't wait for it"
        );
    }

    #[test]
    fn a_look_that_finds_nothing_new_changes_nothing() {
        let (_dir, mut app) = reviewed();
        app.open(&app.workspace.root().join("alpha.txt")).unwrap();
        let settled = |app: &mut App| {
            app.look_at_disk();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            loop {
                let changed = app.poll_disk();
                if app.disk_look.is_none() {
                    return changed;
                }
                assert!(std::time::Instant::now() < deadline);
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        };
        settled(&mut app);
        assert!(
            !settled(&mut app),
            "so nothing is redrawn or compared again"
        );
    }

    #[test]
    fn a_stale_look_cant_undo_what_just_happened() {
        let (_dir, mut app) = project();
        app.look_at_disk();
        app.refresh_tree();
        assert!(app.disk_look.is_none());
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
        app.run(Command::CloseDock);
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
        // It asks why, and Enter turns it down.
        enter(&mut app);
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
        // It asks why, and Enter turns it down.
        enter(&mut app);
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
                text: "why this".into(),
                quote: "CHANGED".into(),
                removed: false,
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
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while app.disk_look.is_some() {
            assert!(std::time::Instant::now() < deadline);
            app.poll();
            std::thread::sleep(std::time::Duration::from_millis(5));
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
        // As the platform writes it: `src\lib.rs` on Windows, as VS Code shows it there.
        let shown = Path::new("src").join("lib.rs").display().to_string();
        assert!(screen(&app).text().contains(&shown));
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
        let rail = app.activity_area();
        app.handle(mouse(MouseKind::Left, 1, app.rail_row(1, rail)));
        assert!(screen(&app).text().contains("SEARCH"));
        app.handle(mouse(MouseKind::Left, 1, app.rail_row(0, rail)));
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
    fn an_empty_cell_takes_a_click_and_keeps_what_is_typed() {
        let source = "| Alpha | Beta |\n|---|---|\n| one | two |\n";
        let (_dir, mut app) = page_app(source);
        let menu = |app: &mut App, name: &str| {
            let (x, y) = cell_of(app, "two");
            app.handle(mouse(MouseKind::Right, x + 1, y));
            let (x, y) = find(&screen(app), name);
            app.handle(mouse(MouseKind::Left, x, y));
        };
        menu(&mut app, "Insert Row Below");
        menu(&mut app, "Insert Column Right");
        // Three columns and a blank row: click the row's first cell, under "one".
        let (x, y) = cell_of(&app, "one");
        app.handle(mouse(MouseKind::Left, x, y + 1));
        typed(&mut app, "first");
        let text = app.active_text().unwrap();
        let row = text.lines().nth(3).unwrap().to_string();
        assert!(row.starts_with("| first"), "{text}");
        assert_eq!(row.matches('|').count(), 4, "{text}");
        // Now the new column, in the row above: right of "two" is an empty cell.
        let (x, y) = cell_of(&app, "two");
        app.handle(mouse(MouseKind::Left, x + 12, y));
        typed(&mut app, "more");
        let text = app.active_text().unwrap();
        let row = text.lines().nth(2).unwrap().to_string();
        assert!(row.contains("two") && row.contains("more"), "{text}");
        assert!(
            row.find("two") < row.find("more") && row.trim_end().ends_with('|'),
            "{text}"
        );
        assert_eq!(row.matches('|').count(), 4, "{text}");
    }

    #[test]
    fn arrows_walk_through_empty_cells() {
        // Words no temporary folder's name will hold, since the sidebar shows that name.
        let source = "| Alpha | Beta |\n|---|---|\n|     |     |\n";
        let (_dir, mut app) = page_app(source);
        let (x, y) = cell_of(&app, "Alpha");
        app.handle(mouse(MouseKind::Left, x, y + 1));
        key(&mut app, Key::Right);
        typed(&mut app, "x");
        let text = app.active_text().unwrap();
        let row = text.lines().nth(2).unwrap();
        assert_eq!(row.matches('|').count(), 3, "{text}");
        assert!(row.split('|').nth(2).unwrap().contains('x'), "{text}");
    }

    #[test]
    fn a_tabular_edit_leaves_the_caret_in_the_new_cell() {
        let source = "\\begin{tabular}{ll}\n  A & B \\\\\n  one & two \\\\\n\\end{tabular}\n";
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("paper.tex");
        fs::write(&path, source).unwrap();
        let mut app = App::new(dir.path().to_path_buf(), (150, 40));
        app.open(&path).unwrap();
        let (x, y) = cell_of(&app, "two");
        app.handle(mouse(MouseKind::Left, x, y));
        app.editors[0].edit_table(TableEdit::RowBelow);
        typed(&mut app, "new");
        let text = app.active_text().unwrap();
        assert!(text.lines().nth(3).unwrap().contains("new"), "{text}");
    }

    #[test]
    fn tab_moves_between_table_cells() {
        let (_dir, mut app) = page("| a | b |\n|---|---|\n| c | d |\n");
        key(&mut app, Key::Tab);
        typed(&mut app, "x");
        assert_eq!(
            app.active_text().unwrap(),
            "| a | x |\n|---|---|\n| c | d |\n"
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
        // Quoted, so a Windows path keeps its backslashes in the sh Git brings.
        let script = format!("printf '%s' \"$1\" > '{}'", log.display());
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

    /// A page and a stand-in for the browser that draws one, which says what it was asked
    /// to open and then shows a frame the way the real one does.
    #[cfg(unix)]
    fn pane_browser_app() -> (TempDir, App) {
        let (dir, mut app, _log) = browser_app("<h1>hi</h1>\n", "page.html");
        let program = dir.path().join("terminal-browser");
        fs::write(
            &program,
            "#!/bin/sh\nprintf 'showing %s' \"$(basename \"$3\")\"\n\
             printf '\\033_Ga=T,f=24,s=1,v=1,i=1,p=1,C=1,q=2;AQID\\033\\\\'\nsleep 30\n",
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&program, fs::Permissions::from_mode(0o755)).unwrap();
        app.settings.browser = Some(vec![program.display().to_string()]);
        app.set_measure(Rc::new(cellix_render::FontMeasure::new(10, 20)));
        (dir, app)
    }

    #[test]
    #[cfg(unix)]
    fn a_page_opens_in_the_side_pane_and_its_frames_show() {
        let (_dir, mut app) = pane_browser_app();
        app.run(Command::OpenInBrowser);
        assert!(app.dock.open);
        assert_eq!(app.dock.session().unwrap().name, BROWSER);
        shows(&mut app, "showing page.html");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let count = |app: &App| {
            let grid = screen(app);
            let images =
                |p: &&cellix_render::Paint| matches!(p, cellix_render::Paint::Image { .. });
            grid.paint.iter().filter(images).count()
        };
        while count(&app) == 0 {
            assert!(std::time::Instant::now() < deadline, "no frame");
            app.poll();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        // Opening another page replaces it rather than piling up tabs.
        app.run(Command::OpenInBrowser);
        let browsers = app.dock.sessions.iter().filter(|s| s.browser).count();
        assert_eq!(browsers, 1);
    }

    #[test]
    #[cfg(unix)]
    fn a_comment_never_reaches_a_browser() {
        let (_dir, mut app) = pane_browser_app();
        app.run(Command::OpenInBrowser);
        assert!(app.dock.agent_mut().is_none());
        assert!(app.dock.busy().is_empty(), "closing a page loses no work");
    }

    #[test]
    fn a_terminal_without_pictures_keeps_the_system_browser() {
        let (dir, mut app, log) = browser_app("<h1>hi</h1>\n", "page.html");
        let script = format!("printf %s \"$1\" > '{}'", log.display());
        app.settings.browser = Some(vec!["terminal-browser".to_string()]);
        app.system_browser = vec![
            "sh".to_string(),
            "-c".to_string(),
            script,
            "browser".to_string(),
        ];
        app.settings.browser = None;
        app.run(Command::OpenInBrowser);
        assert_eq!(
            wait_for_file(&log),
            dir.path().join("page.html").display().to_string()
        );
        assert!(!app.dock.open);
    }

    /// ConPTY keeps the graphics protocol from reaching Cellix, so Windows can't show these.
    #[test]
    #[cfg(unix)]
    fn a_program_in_the_pane_is_told_graphics_work() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = App::new(dir.path().to_path_buf(), (150, 40));
        let script = "stty -echo -icanon min 1; \
            printf '\\033_Gi=9,a=q,t=d,f=24,s=1,v=1;AAAA\\033\\\\'; \
            r=$(dd bs=1 count=11 2>/dev/null | tr -d '\\033\\\\'); printf 'ready %s' \"$r\"; sleep 30";
        session(
            &mut app,
            &script.replace("ready %s", "said %s; printf ready"),
        );
        assert!(screen(&app).text().contains("_Gi=9;OK"));
    }

    #[test]
    fn a_failed_browser_falls_back_to_the_system_one() {
        let (dir, mut app, log) = browser_app("<h1>hi</h1>\n", "page.html");
        let script = format!("printf %s \"$1\" > '{}'", log.display());
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
    fn with_no_browser_the_corner_card_offers_terminal_browser() {
        let (_dir, mut app) = project();
        assert!(!app.offer_browser(true), "nothing to offer when it's there");
        assert!(app.offer_browser(false));
        let text = screen(&app).text();
        assert!(text.contains("Install terminal-browser?"), "{text}");
        assert!(
            !text.contains("curl"),
            "no command, just the buttons: {text}"
        );
        for label in OFFER_BUTTONS {
            assert!(text.contains(label), "{label}: {text}");
        }
        let area = app.offer_area();
        let (_, _, at) = app.offer_buttons()[1].clone();
        app.handle(mouse(MouseKind::Left, at + 1, area.y + OFFER_BUTTON_ROW));
        assert!(
            !screen(&app).text().contains("Install terminal-browser?"),
            "Not now"
        );
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
        let (x, _) = find(&screen(&app), "Stand-in");
        app.handle(mouse(MouseKind::Left, x + 9, 0));
        assert_eq!(app.dock.sessions.len(), 1, "it asks first");
        let (x, y) = find(&screen(&app), "Stop");
        app.handle(mouse(MouseKind::Left, x + 1, y + 1));
        let Overlay::Confirm(_) = app.overlay else {
            assert!(app.dock.sessions.is_empty());
            return;
        };
        app.choose(Choice::CloseSession(0));
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
    fn the_side_pane_closes_from_its_own_title_and_keeps_what_runs() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = App::new(dir.path().to_path_buf(), (150, 40));
        app.open_agent(crate::agents::SHELL.1);
        let area = app.dock_area().unwrap();
        assert!(app.dock.focused);
        assert!(screen(&app).text().lines().next().unwrap().contains('×'));
        app.handle(mouse(MouseKind::Left, area.right() - 2, area.y));
        assert!(!app.dock.open && !app.dock.focused);
        assert_eq!(app.dock.sessions.len(), 1, "closing the pane stops nothing");
        assert!(app.dock_area().is_none());
        app.run(Command::ShowAgents);
        assert!(app.dock.open);
        assert!(!app.dock.sessions[0].exited);
        // From the keyboard too, even with the keys going to the program in the pane.
        app.dock.focused = true;
        app.handle(Event::Key(cellix_input::KeyEvent {
            key: Key::Char('`'),
            mods: cellix_input::Mods {
                ctrl: true,
                shift: true,
                alt: false,
            },
        }));
        assert!(!app.dock.open);
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
        assert!(screen(&app).text().contains("HOW IT WORKS"));
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

    #[test]
    fn the_plan_banner_names_the_agent_that_sent_it() {
        let (dir, mut app) = project();
        let mut event = agent(
            cellix_agent::AgentState::NeedsYou,
            cellix_agent::hook::PLAN_WAITING,
        );
        event.agent = "Codex".into();
        app.serve(Request::Agent(event));
        let _decision = pending_plan(&dir, &mut app);
        let text = screen(&app).text();
        assert!(text.contains("Codex is waiting on this plan"), "{text}");
    }

    #[test]
    fn a_plan_sent_again_shows_what_changed_since_the_last_version() {
        let (dir, mut app) = project();
        fs::write(dir.path().join("Plan v1.md"), "# Plan\n\nCache first.\n").unwrap();
        let path = dir.path().join("Plan.md");
        fs::write(&path, "# Plan\n\nCache last.\n").unwrap();
        let (reply, _decision) = std::sync::mpsc::channel();
        app.serve(Request::Plan { path, reply });
        let text = screen(&app).text();
        assert!(text.contains("version 2"), "{text}");
        app.run(Command::ComparePlanVersions);
        assert_eq!(
            app.active_text().unwrap(),
            "# Plan\n\nCache {~~first.~>last.~~}\n"
        );
        assert!(app.plan.is_some(), "the plan still waits");
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
    fn a_plans_comments_carry_into_its_next_version() {
        let (dir, mut app) = project();
        let _decision = pending_plan(&dir, &mut app);
        app.run(Command::AddComment);
        typed(&mut app, "Too big");
        enter(&mut app);
        let (x, y) = find(&screen(&app), SEND_BACK);
        app.handle(mouse(MouseKind::Left, x + 1, y));
        typed(&mut app, "Split it");
        enter(&mut app);
        let path = dir.path().join("plan.md");
        fs::write(&path, "# Plan\n\nstep one, split\n").unwrap();
        let (reply, _again) = std::sync::mpsc::channel();
        app.serve(Request::Plan {
            path: path.clone(),
            reply,
        });
        let file = app.relative(&path);
        let carried: Vec<_> = app.threads.iter().filter(|t| t.file == file).collect();
        assert_eq!(carried.len(), 1, "{:?}", app.threads);
        assert_eq!(carried[0].text, "Too big");
        assert_eq!(carried[0].quote, "# Plan");
        assert!(!carried[0].waiting(), "the agent heard it with the plan");
    }

    #[test]
    fn a_plans_threads_move_onto_its_next_version_and_keep_what_it_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = App::new(dir.path().to_path_buf(), (150, 40));
        // Held where the hook holds it, outside the project, and versioned as the hook does.
        let held = tempfile::tempdir().unwrap();
        let first = "# Plan\n\nCache first.\n\nKeep the tests.\n\nShip it.\n";
        let path = cellix_agent::hook::keep_versions(held.path(), first).unwrap();
        let (reply, decision) = std::sync::mpsc::channel();
        app.serve(Request::Plan {
            path: path.clone(),
            reply,
        });
        for (line, comment) in [
            ("Cache first.", "Why first"),
            ("Keep the tests.", "Which tests"),
            ("Ship it.", "Ship when green"),
        ] {
            let (x, y) = find(&screen(&app), line);
            app.handle(mouse(MouseKind::Left, x + 1, y));
            app.run(Command::AddComment);
            typed(&mut app, comment);
            enter(&mut app);
        }
        let file = app.relative(&path);
        let id_of = |app: &App, text: &str| {
            app.threads
                .iter()
                .find(|t| t.file == file && t.text == text)
                .map(|t| t.id)
                .unwrap()
        };
        let (found, _) = std::sync::mpsc::channel();
        app.serve(Request::Reply {
            id: id_of(&app, "Which tests"),
            author: "Claude Code".into(),
            text: "The parser's unit tests.".into(),
            found,
        });
        app.thread_hit(ThreadHit::Resolve(id_of(&app, "Ship when green")));
        let (x, y) = find(&screen(&app), SEND_BACK);
        app.handle(mouse(MouseKind::Left, x + 1, y));
        typed(&mut app, "See comments");
        enter(&mut app);
        assert!(matches!(decision.try_recv(), Ok(Decision::SendBack(_))));

        let second =
            "# Plan\n\nWrite the tests first.\n\nThen the parser.\n\nKeep the tests.\n\nShip it.\n";
        let again = cellix_agent::hook::keep_versions(held.path(), second).unwrap();
        assert_eq!(again, path, "comments stay on Plan.md");
        let (reply, _decision) = std::sync::mpsc::channel();
        app.serve(Request::Plan {
            path: path.clone(),
            reply,
        });
        let text = screen(&app).text();
        assert!(
            text.contains("Then the parser."),
            "version 2 is open: {text}"
        );
        assert!(text.contains("Removed in version 2"), "{text}");
        assert!(
            text.contains("Cache first."),
            "the dropped words show: {text}"
        );
        assert!(text.contains("Why first"), "{text}");
        assert!(text.contains("Which tests"), "{text}");
        assert!(text.contains("The parser's unit tests."), "{text}");
        assert!(!text.contains("Ship when green"), "resolved stays resolved");
        let thread = |text: &str| {
            app.threads
                .iter()
                .find(|t| t.file == file && t.text == text)
                .unwrap()
        };
        let gone = thread("Why first");
        assert_eq!(gone.earlier, Some(1));
        assert_eq!(gone.quote, "Cache first.");
        assert!(!gone.resolved);
        let kept = thread("Which tests");
        assert_eq!((kept.line, kept.earlier), (7, None), "it follows its words");
        assert_eq!(kept.replies.len(), 1);
        let shipped = thread("Ship when green");
        assert!(shipped.resolved);
        assert_eq!(shipped.line, 9);
        let saved = cellix_review::threads::load(dir.path()).unwrap();
        assert!(saved.iter().any(|t| t.earlier == Some(1)), "and it's kept");
        // Its card leads to what changed, where the dropped words still stand.
        let (x, y) = find(&screen(&app), "Why first");
        app.handle(mouse(MouseKind::Left, x + 1, y));
        let compared = app.active_text().unwrap();
        assert!(compared.contains("{~~Cache~>"), "{compared}");
        assert!(app.plan.is_some(), "the plan still waits");
    }

    /// A plan the hook holds in `held`, with a comment typed on each of `comments`' lines,
    /// sent back, and then `next` sent as its next version. Gives the plan's path.
    fn plan_sent_again(
        app: &mut App,
        held: &Path,
        (first, comments): (&str, &[(&str, &str)]),
        next: &str,
    ) -> PathBuf {
        let path = cellix_agent::hook::keep_versions(held, first).unwrap();
        let (reply, decision) = std::sync::mpsc::channel();
        app.serve(Request::Plan {
            path: path.clone(),
            reply,
        });
        for (line, comment) in comments {
            let (x, y) = find(&screen(app), line);
            app.handle(mouse(MouseKind::Left, x + 1, y));
            app.run(Command::AddComment);
            typed(app, comment);
            enter(app);
        }
        let (x, y) = find(&screen(app), SEND_BACK);
        app.handle(mouse(MouseKind::Left, x + 1, y));
        typed(app, "See comments");
        enter(app);
        assert!(matches!(decision.try_recv(), Ok(Decision::SendBack(_))));
        cellix_agent::hook::keep_versions(held, next).unwrap();
        let (reply, decision) = std::sync::mpsc::channel();
        app.serve(Request::Plan {
            path: path.clone(),
            reply,
        });
        // Held open, as the agent would be.
        std::mem::forget(decision);
        path
    }

    #[test]
    fn a_plan_comment_on_reworded_words_says_they_changed_and_never_takes_the_opposite() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = App::new(dir.path().to_path_buf(), (150, 40));
        let held = tempfile::tempdir().unwrap();
        let first = "# Plan\n\nWe will not drop the users table.\n\nThen the cache stays warm across runs.\n";
        let next =
            "# Plan\n\nWe will drop the users table.\n\nThen the cache stays hot across runs.\n";
        let comments = [
            ("We will not drop", "Good, keep it"),
            ("stays warm", "How warm"),
        ];
        let path = plan_sent_again(&mut app, held.path(), (first, &comments), next);
        let text = screen(&app).text();
        assert!(
            text.contains("We will drop the users table."),
            "version 2: {text}"
        );
        // Six of seven words match, but the "not" is gone: the opposite, so not the same.
        assert!(text.contains("Removed in version 2"), "{text}");
        assert!(
            text.contains("We will not drop the users table."),
            "its words as said: {text}"
        );
        // A word changed: kept as said, and shown as changed.
        assert!(text.contains("Changed in version 2"), "{text}");
        assert!(
            text.contains("Then the cache stays warm across runs."),
            "{text}"
        );
        let file = app.relative(&path);
        let thread = |text: &str| {
            app.threads
                .iter()
                .find(|t| t.file == file && t.text == text)
                .cloned()
                .unwrap()
        };
        let opposite = thread("Good, keep it");
        assert_eq!((opposite.earlier, opposite.reworded), (Some(1), false));
        assert_eq!(opposite.quote, "We will not drop the users table.");
        let changed = thread("How warm");
        assert_eq!(
            (changed.earlier, changed.reworded, changed.line),
            (Some(1), true, 5)
        );
        assert!(changed.quote.contains("warm"), "the quote isn't replaced");
        // Its card leads to what changed.
        let (x, y) = find(&screen(&app), "How warm");
        app.handle(mouse(MouseKind::Left, x + 1, y));
        let compared = app.active_text().unwrap();
        assert!(compared.contains("{~~warm~>hot~~}"), "{compared}");
    }

    #[test]
    fn a_plan_comments_pill_keeps_clear_of_its_header_at_any_width() {
        for columns in [150, 100, 80, 70, 64] {
            let dir = tempfile::tempdir().unwrap();
            let mut app = App::new(dir.path().to_path_buf(), (columns, 40));
            let held = tempfile::tempdir().unwrap();
            let first = "# Plan\n\nCache first.\n\nShip it.\n";
            let comments = [("Cache first.", "Why first")];
            plan_sent_again(
                &mut app,
                held.path(),
                (first, &comments),
                "# Plan\n\nShip it.\n",
            );
            for thread in &mut app.threads {
                thread.kind = "Correctness".into();
            }
            let grid = screen(&app);
            let text = grid.text();
            let rows: Vec<&str> = text.lines().collect();
            let pill = rows.iter().position(|r| r.contains("Removed in version 2"));
            let header = rows.iter().position(|r| r.contains("You · Correctness"));
            let (Some(pill), Some(header)) = (pill, header) else {
                panic!("at {columns} columns, both whole: {text}");
            };
            if pill == header {
                let row = rows[pill];
                let label_end = row.find("You · Correctness").unwrap() + "You · Correctness".len();
                let pill_start = row.find("Removed in version 2").unwrap();
                assert!(pill_start > label_end + 1, "at {columns} columns: {row}");
            } else {
                assert_eq!(pill, header + 1, "under its header at {columns} columns");
            }
        }
    }

    #[test]
    fn an_approved_plans_threads_stay_with_it_and_the_next_plan_starts_clean() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = App::new(dir.path().to_path_buf(), (150, 40));
        let held = tempfile::tempdir().unwrap();
        let first = "# Plan\n\nCache first.\n\nShip it.\n";
        let path = cellix_agent::hook::keep_versions(held.path(), first).unwrap();
        let (reply, decision) = std::sync::mpsc::channel();
        app.serve(Request::Plan {
            path: path.clone(),
            reply,
        });
        let (x, y) = find(&screen(&app), "Cache first.");
        app.handle(mouse(MouseKind::Left, x + 1, y));
        app.run(Command::AddComment);
        typed(&mut app, "Why first");
        enter(&mut app);
        let (x, y) = find(&screen(&app), APPROVE);
        app.handle(mouse(MouseKind::Left, x + 1, y));
        assert!(matches!(decision.try_recv(), Ok(Decision::Approve(_))));
        // Settled: resolved, and kept with the approved plan in the project.
        let settled = |threads: &[Thread]| {
            threads
                .iter()
                .find(|t| t.text == "Why first")
                .map(|t| (t.resolved, t.file.clone(), t.line))
        };
        let kept = Some((true, ".cellix/plans/001.md".to_string(), 3));
        assert_eq!(settled(&app.threads), kept);
        let saved = cellix_review::threads::load(dir.path()).unwrap();
        assert_eq!(settled(&saved), kept, "and saved");
        if app.dock.showing(Front::Comments) {
            app.run(Command::ShowComments);
        }
        // The next plan in the session, unrelated. Even counted as version 2, as an older hook
        // would, nothing of the last one comes onto it.
        cellix_agent::hook::keep_versions(held.path(), "# Plan\n\nRename the parser.\n").unwrap();
        let (reply, _decision) = std::sync::mpsc::channel();
        app.serve(Request::Plan {
            path: path.clone(),
            reply,
        });
        let text = screen(&app).text();
        assert!(text.contains("Rename the parser."), "{text}");
        assert!(!text.contains("Removed in version"), "{text}");
        assert!(!text.contains("Why first"), "{text}");
        assert!(
            !app.dock.showing(Front::Comments),
            "the Comments pane stays shut"
        );
        // As the hook does now: the approved plan's versions are put away, so it's version 1.
        cellix_agent::hook::put_versions_away(held.path()).unwrap();
        cellix_agent::hook::keep_versions(held.path(), "# Plan\n\nA third.\n").unwrap();
        let (reply, _decision) = std::sync::mpsc::channel();
        app.serve(Request::Plan { path, reply });
        let text = screen(&app).text();
        assert!(
            text.contains("waiting for you to review this plan"),
            "{text}"
        );
    }

    #[test]
    fn a_plans_resolved_moves_are_saved_and_a_dropped_comment_opens_the_comparison_later() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = App::new(dir.path().to_path_buf(), (150, 40));
        let held = tempfile::tempdir().unwrap();
        let first = "# Plan\n\nCache first.\n\nShip it.\n";
        let comments = [
            ("Cache first.", "Why first"),
            ("Ship it.", "Ship when green"),
        ];
        let next = "# Plan\n\nWrite the tests.\n\nThen the parser.\n\nShip it.\n";
        let path = plan_sent_again(&mut app, held.path(), (first, &comments), next);
        let file = app.relative(&path);
        let id = |app: &App, text: &str| app.threads.iter().find(|t| t.text == text).unwrap().id;
        app.thread_hit(ThreadHit::Resolve(id(&app, "Ship when green")));
        // Version 3 moves the resolved one again, and nothing on it is open.
        app.thread_hit(ThreadHit::Resolve(id(&app, "Why first")));
        let (x, y) = find(&screen(&app), SEND_BACK);
        app.handle(mouse(MouseKind::Left, x + 1, y));
        typed(&mut app, "Again");
        enter(&mut app);
        cellix_agent::hook::keep_versions(
            held.path(),
            "# Plan\n\nOne.\n\nTwo.\n\nThree.\n\nShip it.\n",
        )
        .unwrap();
        let (reply, _decision) = std::sync::mpsc::channel();
        app.serve(Request::Plan {
            path: path.clone(),
            reply,
        });
        let saved = cellix_review::threads::load(dir.path()).unwrap();
        let shipped = saved.iter().find(|t| t.text == "Ship when green").unwrap();
        assert_eq!(
            (shipped.file.as_str(), shipped.line),
            (file.as_str(), 9),
            "its move is kept"
        );
        // A dropped comment, reopened: the agent hears the version, not a line it isn't on.
        let gone = app
            .threads
            .iter_mut()
            .find(|t| t.text == "Why first")
            .unwrap();
        gone.resolved = false;
        let message = gone.message();
        assert!(
            message.contains("(made on version 1; version 2 dropped these words)"),
            "{message}"
        );
        assert!(!message.contains(" line "), "{message}");
        // With the plan no longer waiting, its card still opens what changed.
        app.run(Command::CloseTab);
        assert!(app.plan.is_none());
        if !app.dock.showing(Front::Comments) {
            app.run(Command::ShowComments);
        }
        let (x, y) = find(&screen(&app), "Why first");
        app.handle(mouse(MouseKind::Left, x + 1, y));
        let compared = app.active_text().unwrap();
        assert!(compared.contains("Cache first."), "{compared}");
        let open = app.editors[app.active()].buffer.path().unwrap();
        assert!(open.ends_with("Plan changes.md"), "{}", open.display());
    }

    #[test]
    fn a_plan_held_outside_the_project_sends_its_comments_and_approves_with_notes() {
        let (_dir, mut app) = project();
        let held = tempfile::tempdir().unwrap();
        let path = held.path().join("Plan.md");
        fs::write(&path, "# Plan\n\nDelete the old function\n").unwrap();
        let (reply, decision) = std::sync::mpsc::channel();
        app.serve(Request::Plan {
            path: path.clone(),
            reply,
        });
        app.run(Command::AddComment);
        typed(&mut app, "Keep it for now");
        enter(&mut app);
        app.run(Command::ApprovePlan);
        let Ok(Decision::Approve(Some(text))) = decision.try_recv() else {
            panic!("approved with the plan and its notes");
        };
        assert!(text.contains("## Reviewer notes"), "{text}");
        assert!(text.contains("Keep it for now"), "{text}");
        let (reply, decision) = std::sync::mpsc::channel();
        app.serve(Request::Plan { path, reply });
        app.run(Command::AddComment);
        typed(&mut app, "Split it");
        enter(&mut app);
        let (x, y) = find(&screen(&app), SEND_BACK);
        app.handle(mouse(MouseKind::Left, x + 1, y));
        typed(&mut app, "See comments");
        enter(&mut app);
        let Ok(Decision::SendBack(text)) = decision.try_recv() else {
            panic!("no answer");
        };
        assert!(
            text.contains("Split it"),
            "the comment goes with it: {text}"
        );
    }

    #[test]
    fn neovim_ends_and_wipes_go_through_cellix() {
        let Some((_dir, mut app, path)) = neovim_app() else {
            return;
        };
        typed(&mut app, ":bd");
        enter(&mut app);
        assert!(app.editors.is_empty(), ":bd closes the tab");
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "one\ntwo\nthree\n",
            "and empties nothing"
        );
        app.open(&path).unwrap();
        typed(&mut app, "x");
        typed(&mut app, ":w!");
        enter(&mut app);
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "ne\ntwo\nthree\n",
            ":w! saves"
        );
        typed(&mut app, "xZZ");
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "e\ntwo\nthree\n",
            "ZZ saves"
        );
        assert!(app.editors.is_empty(), "and closes the tab");
    }

    #[test]
    fn ctrl_r_opens_replace_with_find_already_open() {
        let (dir, mut app) = project();
        app.open(&dir.path().join("alpha.txt")).unwrap();
        app.run(Command::FindInFile);
        typed(&mut app, "line");
        app.handle(Event::Key(KeyEvent {
            key: Key::Char('r'),
            mods: cellix_input::Mods {
                ctrl: true,
                ..cellix_input::Mods::default()
            },
        }));
        let Overlay::Find(find) = &app.overlay else {
            panic!("Find is still open");
        };
        assert!(find.replace.is_some() && find.in_replace);
        assert_eq!(find.query, "line", "nothing typed into the query");
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
        assert!(
            text.contains("plan.md line 1, on \"# Plan\": Too big"),
            "{text}"
        );
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

    #[test]
    fn inside_claude_code_the_wheel_scrolls_the_comments_under_the_pointer() {
        let (dir, mut app) = project();
        app.in_pane = true;
        app.open(&dir.path().join("alpha.txt")).unwrap();
        app.run(Command::ShowComments);
        let area = app.dock_area().expect("the comments are open");
        let (x, y) = (area.x + area.w / 2, area.y + area.h / 2);
        // The mod's wheel has no position, as Claude Code's scroll event gives none.
        let wheel = Event::Mouse(MouseEvent {
            kind: MouseKind::ScrollDown,
            x: u16::MAX,
            y: u16::MAX,
            fine_x: 0.0,
            mods: Mods::default(),
        });
        app.handle(mouse(MouseKind::Move, x, y));
        app.handle(wheel.clone());
        app.handle(wheel.clone());
        assert_eq!(app.dock.comments_top, 6);
        // Back over the file, it scrolls the file again.
        app.handle(mouse(MouseKind::Move, 2, 3));
        app.handle(wheel);
        assert_eq!(app.dock.comments_top, 6);
    }

    #[test]
    fn inside_claude_code_sending_starts_a_turn_for_the_comments() {
        let (dir, mut app) = project();
        app.in_pane = true;
        app.open(&dir.path().join("alpha.txt")).unwrap();
        app.run(Command::AddComment);
        typed(&mut app, "Now please");
        enter(&mut app);
        app.run(Command::SendToAgent);
        assert_eq!(app.take_attention(), Some("send"));
        assert_eq!(
            app.comments.lock().unwrap().len(),
            1,
            "the prompt hook takes them"
        );
    }

    fn alt(c: char) -> Event {
        Event::Key(KeyEvent {
            key: Key::Char(c),
            mods: cellix_input::Mods {
                alt: true,
                ..cellix_input::Mods::default()
            },
        })
    }

    #[test]
    fn find_replaces_one_match_then_all_of_them() {
        let (dir, mut app) = project();
        app.open(&dir.path().join("alpha.txt")).unwrap();
        app.run(Command::ReplaceInFile);
        typed(&mut app, "LINE");
        app.handle(Event::Key(KeyEvent::plain(Key::Tab)));
        typed(&mut app, "row");
        assert!(
            screen(&app).text().contains("1 of 2"),
            "any case by default"
        );
        enter(&mut app);
        assert_eq!(app.active_text().unwrap(), "first row\nsecond line\n");
        app.handle(Event::Key(KeyEvent {
            key: Key::Enter,
            mods: cellix_input::Mods {
                alt: true,
                ..cellix_input::Mods::default()
            },
        }));
        assert_eq!(app.active_text().unwrap(), "first row\nsecond row\n");
        app.run(Command::Undo);
        assert_eq!(
            app.active_text().unwrap(),
            "first row\nsecond line\n",
            "one edit"
        );
    }

    #[test]
    fn find_switches_match_case_whole_words_and_expressions() {
        let (dir, mut app) = project();
        app.open(&dir.path().join("alpha.txt")).unwrap();
        app.run(Command::FindInFile);
        typed(&mut app, "lin");
        assert!(screen(&app).text().contains("1 of 2"));
        app.handle(alt('w'));
        assert!(
            screen(&app).text().contains("No results"),
            "not a whole word"
        );
        app.handle(alt('w'));
        app.handle(alt('r'));
        typed(&mut app, "(");
        assert!(screen(&app).text().contains("Invalid"));
        app.handle(Event::Key(KeyEvent::plain(Key::Backspace)));
        typed(&mut app, "e$");
        assert!(
            screen(&app).text().contains("1 of 2"),
            "an expression per line"
        );
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
        let first = toolbar::layout(row, Bar::Page(Dialect::Markdown))[1].x;
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
        let bar = app.page_bar();
        let button = toolbar::layout(row, bar)
            .into_iter()
            .find(|b| b.name == name);
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
        app.handle(Event::Resize(100, 40));
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
        assert_eq!(pictures(&app), 1, "open, with its preview");
        let source = |p: &cellix_render::Paint| matches!(p, cellix_render::Paint::Text { text, .. } if text == "$x^2$");
        assert!(screen(&app).paint.iter().any(source));
        typed(&mut app, "+1");
        assert_eq!(app.active_text().as_deref(), Some("See $x+1^2$ here\n"));
        let editor = app.editor_mut().unwrap();
        editor.selections.set_single(Selection::cursor(0));
        assert_eq!(pictures(&app), 1, "and it closes on leaving");
        assert!(!screen(&app).paint.iter().any(source));
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
        app.dock.front = Front::Comments;
        let id = app.threads[0].id;
        app.thread_hit(ThreadHit::Ask(id));
        app.dock.front = Front::Session;
        shows(&mut app, "cellix reply 1");
        app.dock.front = Front::Comments;
        let (found, known) = std::sync::mpsc::channel();
        app.serve(Request::Reply {
            id,
            author: "Claude Code".into(),
            text: "It saves a read.".into(),
            found: found.clone(),
        });
        assert_eq!(known.try_recv(), Ok(true));
        app.serve(Request::Reply {
            id: 99,
            author: "Claude Code".into(),
            text: "Lost".into(),
            found,
        });
        assert_eq!(known.try_recv(), Ok(false), "a wrong number is an error");
        let text = screen(&app).text();
        assert!(text.contains("It saves a read."), "{text}");
        assert!(text.contains("Claude Code"), "{text}");
        let (sender, answer) = std::sync::mpsc::channel();
        app.serve(Request::Threads(sender));
        assert!(answer.recv().unwrap().contains("Why first"));
    }

    #[test]
    fn saving_never_erases_a_change_made_on_disk() {
        let (dir, mut app) = project();
        let path = dir.path().join("alpha.txt");
        app.open(&path).unwrap();
        typed(&mut app, "mine ");
        std::thread::sleep(std::time::Duration::from_millis(20));
        fs::write(&path, "theirs\n").unwrap();
        app.handle(Event::Tick);
        app.run(Command::Save);
        assert_eq!(fs::read_to_string(&path).unwrap(), "theirs\n", "it asks");
        assert!(screen(&app).text().contains("changed on disk"));
        app.overlay = Overlay::None;
        app.choose(Choice::Overwrite);
        assert!(fs::read_to_string(&path).unwrap().starts_with("mine"));
    }

    #[test]
    fn quitting_asks_before_stopping_a_running_job() {
        let (_dir, mut app) = project();
        session(&mut app, "echo ready; sleep 9");
        app.run(Command::Quit);
        assert!(!app.quit);
        assert!(screen(&app).text().contains("Stand-in will stop"));
        app.overlay = Overlay::None;
        app.choose(Choice::Stop);
        assert!(app.quit);
    }

    #[test]
    fn a_shell_that_exits_closes_its_tab() {
        let (_dir, mut app) = project();
        let root = app.workspace.root().to_path_buf();
        let argv = ["sh".to_string(), "-c".to_string(), "exit 0".to_string()];
        app.start_session(crate::agents::SHELL.0, &argv, &[], &root);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !app.dock.sessions.is_empty() {
            assert!(std::time::Instant::now() < deadline);
            app.poll();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    #[test]
    fn images_open_beside_the_file_and_binaries_say_what_they_are() {
        let (dir, mut app) = project();
        let image = image::RgbaImage::from_pixel(8, 8, image::Rgba([1, 2, 3, 255]));
        image.save(dir.path().join("loss.png")).unwrap();
        app.set_measure(Rc::new(cellix_render::FontMeasure::new(17, 37)));
        app.open(&dir.path().join("loss.png")).unwrap();
        assert_eq!(app.dock.front, Front::Pdf);
        assert!(app.editors.is_empty());
        fs::write(dir.path().join("best.pt"), [0xff, 0xfe, 0x00, 0xff]).unwrap();
        let said = app
            .open(&dir.path().join("best.pt"))
            .unwrap_err()
            .to_string();
        assert!(said.contains("binary file"), "{said}");
    }

    #[test]
    fn code_comments_show_in_the_panel_too() {
        let (dir, mut app) = project();
        app.open(&dir.path().join("alpha.txt")).unwrap();
        comment_on(&mut app, "Why this");
        assert_eq!(app.threads.len(), 1);
        assert_eq!(
            app.comments.lock().unwrap().len(),
            1,
            "and go with the review"
        );
        assert!(!app.threads[0].waiting(), "so no agent hears of it twice");
    }

    #[test]
    fn a_path_in_a_session_opens_at_its_line() {
        let (_dir, mut app) = project();
        session(
            &mut app,
            "echo ready; echo 'error in beta.rs:1: broken'; sleep 9",
        );
        shows(&mut app, "beta.rs:1");
        let (x, y) = find(&screen(&app), "beta.rs:1");
        let mut press = mouse(MouseKind::Left, x + 2, y);
        if let Event::Mouse(m) = &mut press {
            m.mods.ctrl = true;
        }
        app.handle(press);
        assert!(app.active_text().unwrap().contains("fn main"));
    }

    #[test]
    fn a_comment_is_written_beside_what_it_is_about() {
        let (_dir, mut app) = page_app("the cache goes first\n");
        app.run(Command::AddComment);
        let text = screen(&app).text();
        assert!(text.contains("the cache goes first"), "{text}");
        assert!(text.contains("Enter sends"), "{text}");
        assert!(matches!(app.overlay, Overlay::None), "no floating box");
        typed(&mut app, "One");
        app.handle(Event::Key(KeyEvent {
            key: Key::Enter,
            mods: Mods {
                shift: true,
                ..Mods::default()
            },
        }));
        typed(&mut app, "Two");
        enter(&mut app);
        assert_eq!(app.threads[0].text, "One\nTwo");
        assert!(app.dock.draft.is_none());
    }

    #[test]
    fn tab_walks_a_templates_slots_and_a_selection_fills_the_first() {
        let (_dir, mut app) = page_app("x\n");
        app.run(Command::Move(cellix_input::Direction::LineEnd, false));
        click_tool(&mut app, "Formula");
        let Overlay::Math(panel) = &app.overlay else {
            panic!("no panel");
        };
        let area = panel.area();
        app.handle(mouse(MouseKind::Left, area.x + 2, area.y));
        typed(&mut app, "a");
        screen(&app);
        app.run(Command::Indent);
        typed(&mut app, "b");
        assert_eq!(app.active_text().as_deref(), Some("x$\\frac{a}{b}$\n"));
    }

    #[test]
    fn a_half_typed_formula_keeps_its_last_picture() {
        let (_dir, mut app) = page_app("$$\nx^2\n$$\n");
        app.set_measure(Rc::new(cellix_render::FontMeasure::new(17, 37)));
        let pictures = |app: &App| {
            screen(app)
                .paint
                .iter()
                .filter(|p| matches!(p, cellix_render::Paint::Image { .. }))
                .count()
        };
        assert_eq!(pictures(&app), 1);
        let editor = app.editor_mut().unwrap();
        editor.selections.set_single(Selection::cursor(3));
        assert_eq!(pictures(&app), 1, "open, with the result under it");
        typed(&mut app, "\\frac{");
        assert_eq!(pictures(&app), 1, "and it holds while the source is broken");
    }

    /// A page at the pixel tier, where pictures can show.
    fn picture_page(text: &str) -> (TempDir, App) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("note.md");
        fs::write(&path, text).unwrap();
        let image = image::RgbaImage::from_pixel(40, 20, image::Rgba([200, 30, 30, 255]));
        image.save(dir.path().join("plot.png")).unwrap();
        let mut app = App::new(dir.path().to_path_buf(), (150, 40));
        app.set_measure(Rc::new(cellix_render::FontMeasure::new(10, 20)));
        app.open(&path).unwrap();
        (dir, app)
    }

    fn pictures(app: &App) -> Vec<cellix_render::Bounds> {
        let grid = screen(app);
        let bounds = grid.paint.iter().filter_map(|p| match p {
            cellix_render::Paint::Image { bounds, .. } => Some(*bounds),
            _ => None,
        });
        bounds.collect()
    }

    /// Everything drawn as text, which a pixel page sets between the cells.
    fn painted_text(app: &App) -> String {
        let grid = screen(app);
        let runs = grid.paint.iter().filter_map(|p| match p {
            cellix_render::Paint::Text { text, .. } => Some(text.as_str()),
            _ => None,
        });
        runs.collect::<Vec<_>>().join("\n")
    }

    const FLOW: &str = "before\n\n```mermaid\nflowchart LR\n  Alpha --> Beta\n```\n\nafter\n";

    #[test]
    fn a_diagram_stands_as_a_picture_and_opens_to_its_source() {
        let (_dir, mut app) = picture_page(FLOW);
        assert_eq!(pictures(&app).len(), 1);
        assert!(!painted_text(&app).contains("flowchart"));
        let picture = pictures(&app)[0];
        let (x, y) = ((picture.x / 64) as u16 + 2, (picture.y / 64) as u16 + 1);
        app.handle(mouse(MouseKind::Left, x, y));
        let text = painted_text(&app);
        assert!(text.contains("flowchart"), "{text}");
        assert_eq!(pictures(&app).len(), 1, "it stays in view while written");
        typed(&mut app, " ");
        assert!(app.active_text().unwrap().contains("```mermaid"));
    }

    #[test]
    fn a_half_written_diagram_keeps_its_last_picture_and_says_why() {
        let (_dir, mut app) = picture_page(FLOW);
        let picture = pictures(&app)[0];
        let (x, y) = ((picture.x / 64) as u16 + 2, (picture.y / 64) as u16 + 1);
        app.handle(mouse(MouseKind::Left, x, y));
        assert_eq!(pictures(&app).len(), 1);
        typed(&mut app, "wrong ");
        assert_eq!(pictures(&app).len(), 1, "{}", painted_text(&app));
        let text = painted_text(&app);
        assert!(text.to_lowercase().contains("diagram"), "{text}");
    }

    #[test]
    fn on_the_cell_tier_a_diagram_stays_as_its_source() {
        let (_dir, app) = page_app(FLOW);
        assert!(screen(&app).text().contains("flowchart"));
    }

    #[test]
    fn the_toolbar_adds_a_diagram() {
        let (_dir, mut app) = picture_page("words\n");
        app.use_tool(Tool::Diagram, 40);
        let (x, y) = find(&screen(&app), "Sequence");
        app.handle(mouse(MouseKind::Left, x + 1, y));
        let text = app.active_text().unwrap();
        assert!(text.contains("```mermaid\nsequenceDiagram"), "{text}");
        assert_eq!(pictures(&app).len(), 1);
        assert!(painted_text(&app).contains("sequenceDiagram"));
    }

    #[test]
    fn an_image_is_sized_from_its_menu_and_the_page_follows() {
        let (_dir, mut app) = picture_page("![Plot](plot.png)\n\nwords\n");
        let before = pictures(&app)[0];
        let (x, y) = ((before.x / 64) as u16 + 1, (before.y / 64) as u16);
        app.handle(mouse(MouseKind::Right, x, y));
        let (mx, my) = find(&screen(&app), "Full Width");
        app.handle(mouse(MouseKind::Left, mx + 1, my));
        assert!(
            app.active_text()
                .unwrap()
                .starts_with("![Plot](plot.png){width=100%}"),
        );
        let wide = pictures(&app)[0];
        assert!(wide.w > before.w * 4, "{wide:?} {before:?}");
    }

    #[test]
    fn dragging_an_images_edge_sizes_it() {
        let (_dir, mut app) = picture_page("![Plot](plot.png)\n\nwords\n");
        // The first frame is drawn before the view knows its height.
        let _ = screen(&app);
        let before = pictures(&app)[0];
        let right = ((before.x + before.w) / 64) as u16;
        let y = (before.y / 64) as u16;
        app.handle(mouse(MouseKind::Left, right, y));
        let far = app.main_area().right() - 6;
        app.handle(mouse(MouseKind::Drag, far, y));
        app.handle(mouse(MouseKind::Up, far, y));
        let text = app.active_text().unwrap();
        assert!(text.contains("{width="), "{text}");
        assert!(pictures(&app)[0].w > before.w * 4);
    }

    #[test]
    fn a_click_on_the_middle_of_an_image_does_not_size_it() {
        let (_dir, mut app) = picture_page("![Plot](plot.png)\n\nwords\n");
        let before = pictures(&app)[0];
        let (x, y) = (
            ((before.x + before.w / 4) / 64) as u16,
            (before.y / 64) as u16,
        );
        app.handle(mouse(MouseKind::Left, x, y));
        app.handle(mouse(MouseKind::Up, x, y));
        assert!(!app.active_text().unwrap().contains("{width="));
    }

    #[test]
    fn an_image_comes_out_from_its_menu() {
        let (_dir, mut app) = picture_page("![Plot](plot.png)\n\nwords\n");
        let picture = pictures(&app)[0];
        let (x, y) = ((picture.x / 64) as u16 + 1, (picture.y / 64) as u16);
        app.handle(mouse(MouseKind::Right, x, y));
        let (mx, my) = find(&screen(&app), "Delete Image");
        app.handle(mouse(MouseKind::Left, mx + 1, my));
        assert_eq!(app.active_text().unwrap(), "\nwords\n");
    }

    #[test]
    fn a_paper_figure_is_sized_the_way_papers_size_them() {
        let dir = tempfile::tempdir().unwrap();
        let source = "\\begin{document}\n\\begin{figure}\n  \\includegraphics{plot.png}\n  \\caption{A}\n\\end{figure}\n\\end{document}\n";
        fs::write(dir.path().join("paper.tex"), source).unwrap();
        let image = image::RgbaImage::from_pixel(40, 20, image::Rgba([200, 30, 30, 255]));
        image.save(dir.path().join("plot.png")).unwrap();
        let mut app = App::new(dir.path().to_path_buf(), (150, 40));
        app.set_measure(Rc::new(cellix_render::FontMeasure::new(10, 20)));
        app.open(&dir.path().join("paper.tex")).unwrap();
        let picture = pictures(&app)[0];
        let (x, y) = ((picture.x / 64) as u16 + 1, (picture.y / 64) as u16);
        app.handle(mouse(MouseKind::Right, x, y));
        let (mx, my) = find(&screen(&app), "Medium");
        app.handle(mouse(MouseKind::Left, mx + 1, my));
        let text = app.active_text().unwrap();
        assert!(
            text.contains("\\includegraphics[width=0.50\\linewidth]{plot.png}"),
            "{text}"
        );
    }

    #[test]
    fn a_section_files_pictures_are_found_as_latex_finds_them() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("sections")).unwrap();
        fs::create_dir_all(dir.path().join("figures")).unwrap();
        fs::write(
            dir.path().join("main.tex"),
            "\\documentclass{article}\n\\usepackage{graphicx}\n\\graphicspath{{figures/}}\n\\begin{document}\n\\input{sections/results}\n\\end{document}\n",
        )
        .unwrap();
        let section = dir.path().join("sections/results.tex");
        fs::write(
            &section,
            "Results.\n\n\\begin{figure}\n  \\includegraphics{shared.png}\n\\end{figure}\n\nMore.\n\n\\begin{figure}\n  \\includegraphics{plot}\n\\end{figure}\n",
        )
        .unwrap();
        let image = image::RgbaImage::from_pixel(40, 20, image::Rgba([200, 30, 30, 255]));
        // One beside the root, one in the folder `\graphicspath` names; none beside the section.
        image.save(dir.path().join("shared.png")).unwrap();
        image.save(dir.path().join("figures/plot.png")).unwrap();
        let mut app = App::new(dir.path().to_path_buf(), (150, 40));
        app.set_measure(Rc::new(cellix_render::FontMeasure::new(10, 20)));
        app.open(&section).unwrap();
        assert_eq!(pictures(&app).len(), 2, "{}", painted_text(&app));
    }

    #[test]
    fn a_dropped_picture_is_copied_in_beside_the_document() {
        let (dir, mut app) = picture_page("words\n");
        let outside = tempfile::tempdir().unwrap();
        let picture = outside.path().join("my plot.png");
        fs::copy(dir.path().join("plot.png"), &picture).unwrap();
        let dropped = picture.display().to_string().replace(' ', "\\ ");
        app.handle(Event::Paste(dropped));
        let text = app.active_text().unwrap();
        assert!(text.contains("![](images/my plot.png)"), "{text}");
        assert!(dir.path().join("images/my plot.png").is_file());
    }

    #[test]
    fn a_picture_on_the_clipboard_lands_in_the_document() {
        let (dir, mut app) = picture_page("words\n");
        let source = dir.path().join("plot.png");
        app.clipboard_image = vec![
            "sh".to_string(),
            "-c".to_string(),
            format!("cp '{}' \"$1\"", source.display()),
            "clipboard".to_string(),
        ];
        app.paste_image();
        let text = app.active_text().unwrap();
        assert!(text.contains("![](images/pasted-"), "{text}");
        assert_eq!(pictures(&app).len(), 1, "{text}");
    }

    #[test]
    fn an_empty_clipboard_says_so_and_leaves_nothing() {
        let (dir, mut app) = picture_page("words\n");
        app.clipboard_image = vec!["false".to_string()];
        app.paste_image();
        assert!(screen(&app).text().contains("no picture on the clipboard"));
        assert_eq!(app.active_text().unwrap(), "words\n");
        let left = fs::read_dir(dir.path().join("images")).unwrap().count();
        assert_eq!(left, 0);
    }

    /// A stand-in for LaTeX that makes a real PDF, and proves it ran.
    #[cfg(unix)]
    fn fake_latex(dir: &Path, reply: &str) -> Vec<String> {
        let script = dir.join("latex.sh");
        fs::write(&script, format!("#!/bin/sh\n{reply}\n")).unwrap();
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
        vec![script.display().to_string()]
    }

    #[cfg(unix)]
    fn waits_for(app: &mut App, done: impl Fn(&App) -> bool) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !done(app) {
            assert!(
                std::time::Instant::now() < deadline,
                "{}",
                painted_text(app)
            );
            app.poll();
            let _ = screen(app);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    #[cfg(unix)]
    fn tikz_paper(reply: &str) -> (TempDir, App) {
        let dir = tempfile::tempdir().unwrap();
        let sample = dir.path().join("sample.pdf");
        fs::write(&sample, cellix_doc::pdf::sample()).unwrap();
        let reply = reply.replace("SAMPLE", &sample.display().to_string());
        let source = "\\documentclass{article}\n\\usepackage{tikz}\n\\begin{document}\n\\begin{tikzcd}\nA \\arrow[r] & B\n\\end{tikzcd}\n\\end{document}\n";
        fs::write(dir.path().join("paper.tex"), source).unwrap();
        let mut app = App::new(dir.path().to_path_buf(), (150, 40));
        app.set_measure(Rc::new(cellix_render::FontMeasure::new(10, 20)));
        app.settings.latex_engine = Some(fake_latex(dir.path(), &reply));
        app.built_folder = dir.path().join("built");
        app.open(&dir.path().join("paper.tex")).unwrap();
        (dir, app)
    }

    #[test]
    #[cfg(unix)]
    fn a_commutative_diagram_is_built_in_the_background_and_shows() {
        let (dir, mut app) = tikz_paper("grep -q tikz-cd snippet.tex && cp SAMPLE snippet.pdf");
        assert!(painted_text(&app).contains("Drawing with LaTeX"));
        assert!(pictures(&app).is_empty());
        waits_for(&mut app, |app| !pictures(app).is_empty());
        assert!(!painted_text(&app).contains("Drawing with LaTeX"));
        // The file it came from is gone, and the picture stays for next time.
        let kept = fs::read_dir(dir.path().join("built")).unwrap().count();
        assert_eq!(kept, 1);
    }

    #[test]
    #[cfg(unix)]
    fn a_piece_latex_refuses_says_why() {
        let (_dir, mut app) = tikz_paper("echo '! Package tikz Error: no such library'; exit 1");
        waits_for(&mut app, |app| {
            painted_text(app).contains("no such library")
        });
    }

    #[test]
    #[cfg(unix)]
    fn a_paper_is_offered_drawings_of_its_own() {
        let (_dir, mut app) = tikz_paper("cp SAMPLE snippet.pdf");
        app.use_tool(Tool::Diagram, 40);
        let (x, y) = find(&screen(&app), "Flow");
        app.handle(mouse(MouseKind::Left, x + 1, y));
        let text = app.active_text().unwrap();
        assert!(text.contains("\\begin{tikzpicture}[every node"), "{text}");
    }

    #[test]
    #[cfg(unix)]
    fn a_tikz_block_in_markdown_is_built_too() {
        let dir = tempfile::tempdir().unwrap();
        let sample = dir.path().join("sample.pdf");
        fs::write(&sample, cellix_doc::pdf::sample()).unwrap();
        fs::write(
            dir.path().join("note.md"),
            "```tikz\n\\draw (0,0) -- (1,1);\n```\n",
        )
        .unwrap();
        let mut app = App::new(dir.path().to_path_buf(), (150, 40));
        app.set_measure(Rc::new(cellix_render::FontMeasure::new(10, 20)));
        let reply = format!(
            "grep -q 'begin{{tikzpicture}}' snippet.tex && cp {} snippet.pdf",
            sample.display()
        );
        app.settings.latex_engine = Some(fake_latex(dir.path(), &reply));
        app.built_folder = dir.path().join("built");
        app.open(&dir.path().join("note.md")).unwrap();
        waits_for(&mut app, |app| !pictures(app).is_empty());
    }

    #[test]
    fn the_outline_lists_a_documents_headings_and_a_click_goes_there() {
        let mut text = "# First\n\nwords\n\n".to_string();
        text.push_str(&"line\n\n".repeat(60));
        text.push_str("## Second part\n\nend\n");
        let (_dir, mut app) = page_app(&text);
        app.run(Command::ShowOutline);
        app.poll();
        let area = app.sidebar_area().unwrap();
        let grid = screen(&app);
        let (x, y) = find(&grid, "Second part");
        assert!(area.contains(x, y));
        app.handle(mouse(MouseKind::Left, x, y));
        let (_, line) = app.cursor_line().unwrap();
        assert_eq!(line, 125);
        assert!(app.editors[0].top() > 0, "the page moved there");
        // A second press closes it, as a second click on the rail does.
        app.run(Command::ShowOutline);
        assert!(app.sidebar_area().is_none());
    }

    #[test]
    #[cfg(unix)]
    fn a_failed_build_leaves_its_log_in_the_pane() {
        let dir = tempfile::tempdir().unwrap();
        let source = "\\documentclass{article}\n\\begin{document}\nx\n\\end{document}\n";
        fs::write(dir.path().join("paper.tex"), source).unwrap();
        let mut app = App::new(dir.path().to_path_buf(), (150, 40));
        let script = dir.path().join("latex.sh");
        fs::write(&script, "#!/bin/sh\necho 'first line of the log'\necho 'paper.tex:3: Undefined control sequence'\nexit 1\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
        app.settings.latex_engine = Some(vec![script.display().to_string()]);
        app.open(&dir.path().join("paper.tex")).unwrap();
        app.run(Command::BuildPdf);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while app.dock.log.is_empty() {
            assert!(std::time::Instant::now() < deadline);
            app.poll();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(app.dock.front, Front::Log);
        let grid = screen(&app);
        assert!(grid.text().contains("first line of the log"));
        let (x, y) = find(&grid, "paper.tex:3:");
        app.handle(mouse(MouseKind::Left, x, y));
        assert_eq!(app.cursor_line().unwrap().1, 3);
    }

    fn tones_on_page(app: &App) -> Vec<(char, crate::rendered::Tone)> {
        let area = app.focused_area();
        let rows = app.editors[0].page_rows(area);
        rows.iter()
            .flat_map(|r| r.glyphs.iter().map(|g| (g.ch, g.tone)))
            .collect()
    }

    fn text_with(app: &App, tone: crate::rendered::Tone) -> String {
        let tones = tones_on_page(app);
        tones
            .iter()
            .filter(|(_, t)| *t == tone)
            .map(|(c, _)| *c)
            .collect()
    }

    #[test]
    fn writing_marks_stand_on_the_page_with_their_markers_hidden() {
        let source = "~~gone~~ ==key== H~2~O x^2^ here\n";
        let (_dir, app) = picture_page(source);
        assert_eq!(text_with(&app, crate::rendered::Tone::Struck), "gone");
        assert_eq!(text_with(&app, crate::rendered::Tone::Marked), "key");
        assert_eq!(text_with(&app, crate::rendered::Tone::Lowered), "2");
        assert_eq!(text_with(&app, crate::rendered::Tone::Raised), "2");
        let shown: String = tones_on_page(&app).iter().map(|(c, _)| *c).collect();
        assert!(!shown.contains('~') && !shown.contains('='), "{shown}");
    }

    #[test]
    fn a_price_and_a_path_keep_their_tildes() {
        let (_dir, app) = picture_page("see ~/notes and ~/work today\n");
        let shown: String = tones_on_page(&app).iter().map(|(c, _)| *c).collect();
        assert!(shown.contains("~/notes and ~/work"), "{shown}");
    }

    #[test]
    fn footnotes_are_numbered_as_they_are_cited() {
        let source = "First[^b] and second[^a].\n\n[^a]: Words of a.\n[^b]: Words of b.\n";
        let (_dir, app) = picture_page(source);
        assert_eq!(text_with(&app, crate::rendered::Tone::Footnote), "1221");
        let shown: String = tones_on_page(&app).iter().map(|(c, _)| *c).collect();
        assert!(shown.contains("Words of a."), "{shown}");
        assert!(!shown.contains("[^"), "{shown}");
    }

    #[test]
    fn a_callout_has_its_title_and_colour() {
        let source = "> [!WARNING]\n> Mind the gap.\n\nAfter.\n";
        let (_dir, app) = picture_page(source);
        assert_eq!(
            text_with(
                &app,
                crate::rendered::Tone::Callout(crate::rendered::Callout::Warning)
            ),
            "Warning"
        );
        let shown: String = tones_on_page(&app).iter().map(|(c, _)| *c).collect();
        assert!(!shown.contains("[!"), "{shown}");
        let area = app.focused_area();
        let rows = app.editors[0].page_rows(area);
        assert_eq!(rows.iter().filter(|r| r.callout.is_some()).count(), 2);
    }

    #[test]
    fn tracked_changes_stand_on_the_page() {
        let source = "Keep {++added++} and {--gone--} and {~~old~>new~~}.\n";
        let (_dir, app) = picture_page(source);
        assert_eq!(text_with(&app, crate::rendered::Tone::Inserted), "addednew");
        assert_eq!(text_with(&app, crate::rendered::Tone::Deleted), "goneold");
    }

    fn tracking_page(text: &str) -> (TempDir, App) {
        let (dir, mut app) = page_app(text);
        app.run(Command::ToggleTracking);
        (dir, app)
    }

    #[test]
    fn with_tracking_on_an_agents_edit_arrives_as_suggestions() {
        let (dir, mut app) = tracking_page("The cache goes first.\n");
        std::thread::sleep(std::time::Duration::from_millis(20));
        fs::write(dir.path().join("note.md"), "The cache goes last.\n").unwrap();
        app.handle(Event::Tick);
        assert_eq!(
            app.active_text().unwrap(),
            "The cache goes {~~first.~>last.~~}\n"
        );
        let text = screen(&app).text();
        assert!(text.contains("accept or reject"), "{text}");
    }

    #[test]
    fn the_status_counts_words_and_the_selection() {
        let (_dir, mut app) =
            tracking_page("# Title\n\nThe cache goes **first**, then [docs](http://x).\n");
        let text = screen(&app).text();
        assert!(text.contains("Tracking changes  7 words"), "{text}");
        select_word(&mut app, "cache");
        let text = screen(&app).text();
        assert!(text.contains("1 of 7 words"), "{text}");
    }

    #[test]
    fn an_agents_edits_in_one_turn_pile_up_as_one_review() {
        let (dir, mut app) = tracking_page("The cache goes first.\n\nKeep it small.\n");
        let path = dir.path().join("note.md");
        std::thread::sleep(std::time::Duration::from_millis(20));
        fs::write(&path, "The cache goes last.\n\nKeep it small.\n").unwrap();
        app.handle(Event::Tick);
        std::thread::sleep(std::time::Duration::from_millis(20));
        fs::write(&path, "The cache goes last.\n\nKeep it tiny.\n").unwrap();
        app.handle(Event::Tick);
        let text = app.active_text().unwrap();
        assert!(text.contains("{~~first.~>last.~~}"), "{text}");
        assert!(text.contains("{~~small.~>tiny.~~}"), "{text}");
        assert!(
            !screen(&app).text().contains("keeping your edits"),
            "no conflict"
        );
    }

    #[test]
    fn typing_with_tracking_on_is_kept_as_one_suggestion() {
        let (_dir, mut app) = tracking_page("Hello world\n");
        let (x, y) = cell_of(&app, "world");
        app.handle(mouse(MouseKind::Left, x, y));
        typed(&mut app, "big ");
        assert_eq!(app.active_text().unwrap(), "Hello {++big ++}world\n");
        key(&mut app, Key::Backspace);
        assert_eq!(app.active_text().unwrap(), "Hello {++big++}world\n");
        for _ in 0..3 {
            key(&mut app, Key::Backspace);
        }
        assert_eq!(
            app.active_text().unwrap(),
            "Hello world\n",
            "the last of it takes the marks"
        );
    }

    #[test]
    fn deleting_with_tracking_on_crosses_out() {
        let (_dir, mut app) = tracking_page("one two\n");
        let (x, y) = cell_of(&app, "two");
        app.handle(mouse(MouseKind::Left, x + 3, y));
        for _ in 0..3 {
            key(&mut app, Key::Backspace);
        }
        assert_eq!(app.active_text().unwrap(), "one {--two--}\n");
    }

    #[test]
    fn a_selection_replaced_with_tracking_on_shows_both() {
        let (_dir, mut app) = tracking_page("one two three\n");
        let (x, y) = cell_of(&app, "two");
        double_click(&mut app, x + 1, y);
        typed(&mut app, "2");
        assert_eq!(app.active_text().unwrap(), "one {--two--}{++2++} three\n");
    }

    #[test]
    fn changes_are_accepted_and_rejected_from_their_menu() {
        let (_dir, mut app) = page_app("Keep {++added++} and {--gone--} end\n");
        let (x, y) = cell_of(&app, "added");
        app.handle(mouse(MouseKind::Right, x + 1, y));
        let (mx, my) = find(&screen(&app), "Accept Change");
        app.handle(mouse(MouseKind::Left, mx + 1, my));
        assert_eq!(
            app.active_text().unwrap(),
            "Keep added and {--gone--} end\n"
        );
        let (x, y) = cell_of(&app, "gone");
        app.handle(mouse(MouseKind::Right, x + 1, y));
        let (mx, my) = find(&screen(&app), "Reject Change");
        app.handle(mouse(MouseKind::Left, mx + 1, my));
        assert_eq!(app.active_text().unwrap(), "Keep added and gone end\n");
    }

    #[test]
    fn all_changes_can_be_settled_at_once() {
        let (_dir, mut app) = page_app("a {++b++} c {--d--} e {~~f~>g~~}\n");
        app.run(Command::AcceptAllChanges);
        assert_eq!(app.active_text().unwrap(), "a b c  e g\n");
        assert!(screen(&app).text().contains("Accepted 3 changes"));
    }

    #[test]
    fn a_paper_tracks_changes_with_its_own_package() {
        let dir = tempfile::tempdir().unwrap();
        let source =
            "\\documentclass{article}\n\\begin{document}\nSome words here\n\\end{document}\n";
        fs::write(dir.path().join("paper.tex"), source).unwrap();
        let mut app = App::new(dir.path().to_path_buf(), (150, 40));
        app.open(&dir.path().join("paper.tex")).unwrap();
        app.run(Command::ToggleTracking);
        assert!(
            app.active_text()
                .unwrap()
                .contains("\\usepackage{changes}\n")
        );
        let (x, y) = cell_of(&app, "words");
        app.handle(mouse(MouseKind::Left, x, y));
        typed(&mut app, "new ");
        let text = app.active_text().unwrap();
        assert!(text.contains("Some \\added{new }words"), "{text}");
    }

    /// A page whose words have been checked for real.
    fn checked_page(text: &str) -> (TempDir, App) {
        let (dir, mut app) = page_app(text);
        app.spell_on = true;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        while app.editors[0].issues.1.is_empty() {
            assert!(std::time::Instant::now() < deadline, "nothing was found");
            app.poll();
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        (dir, app)
    }

    #[test]
    fn a_misspelled_word_is_underlined_and_its_menu_fixes_it() {
        let (_dir, mut app) = checked_page("This sentence has a mispelled word in it.\n");
        let grid = screen(&app);
        let (x, y) = find(&grid, "mispelled");
        assert!(grid.cell(x, y).style.underline, "marked where it stands");
        app.handle(mouse(MouseKind::Right, x + 1, y));
        let menu = screen(&app);
        let (mx, my) = find(&menu, "misspelled");
        assert!(menu.text().contains("Add to Dictionary"));
        app.handle(mouse(MouseKind::Left, mx + 1, my));
        assert_eq!(
            app.active_text().unwrap(),
            "This sentence has a misspelled word in it.\n"
        );
    }

    #[test]
    fn a_word_added_to_the_dictionary_stops_being_marked() {
        let (_dir, mut app) = checked_page("This sentence has a mispelled word in it.\n");
        let grid = screen(&app);
        let (x, y) = find(&grid, "mispelled");
        app.handle(mouse(MouseKind::Right, x + 1, y));
        let (mx, my) = find(&screen(&app), "Add to Dictionary");
        app.handle(mouse(MouseKind::Left, mx + 1, my));
        let (x, y) = find(&screen(&app), "mispelled");
        assert!(!screen(&app).cell(x, y).style.underline);
    }

    fn data_app(name: &str, text: &str) -> (TempDir, App) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(name);
        fs::write(&path, text).unwrap();
        let mut app = App::new(dir.path().to_path_buf(), (150, 40));
        app.open(&path).unwrap();
        (dir, app)
    }

    #[test]
    fn a_csv_opens_as_a_table_and_its_cells_edit_in_place() {
        let (_dir, mut app) = data_app("runs.csv", "model,loss\nalpha,0.25\nbeta,0.5\n");
        let grid = screen(&app);
        let text = grid.text();
        assert!(text.contains('│'), "{text}");
        assert!(!text.contains("alpha,0.25"), "{text}");
        let (x, y) = find(&grid, "beta");
        app.handle(mouse(MouseKind::Left, x + 4, y));
        typed(&mut app, "s");
        assert_eq!(
            app.active_text().unwrap(),
            "model,loss\nalpha,0.25\nbetas,0.5\n"
        );
    }

    #[test]
    fn a_tsv_and_its_empty_cells_work_the_same() {
        let (_dir, mut app) = data_app("runs.tsv", "model\tloss\nalpha\t\n");
        let (x, y) = find(&screen(&app), "loss");
        app.handle(mouse(MouseKind::Left, x, y + 1));
        typed(&mut app, "9");
        assert_eq!(app.active_text().unwrap(), "model\tloss\nalpha\t9\n");
    }

    #[test]
    fn rows_are_added_to_data_from_the_menu() {
        let (_dir, mut app) = data_app("runs.csv", "model,loss\nalpha,1\n");
        let (x, y) = find(&screen(&app), "alpha");
        app.handle(mouse(MouseKind::Right, x + 1, y));
        let (mx, my) = find(&screen(&app), "Insert Row Below");
        app.handle(mouse(MouseKind::Left, mx + 1, my));
        assert_eq!(app.active_text().unwrap(), "model,loss\nalpha,1\n,\n");
    }

    #[test]
    fn a_long_file_shows_its_first_rows_and_says_so() {
        let mut text = "id,value\n".to_string();
        for n in 0..3000 {
            text.push_str(&format!("{n},1\n"));
        }
        let (_dir, app) = data_app("big.csv", &text);
        let rows = app.editors[0].page_rows(app.focused_area());
        assert_eq!(rows.iter().filter(|r| r.table.is_some()).count(), 2000);
        let last: String = rows
            .iter()
            .rfind(|r| !r.glyphs.is_empty())
            .unwrap()
            .glyphs
            .iter()
            .map(|g| g.ch)
            .collect();
        assert!(last.contains("first 2000 of 3001 rows"), "{last}");
    }

    const NOTEBOOK: &str = r##"{
 "cells": [
  {"cell_type": "markdown", "metadata": {}, "source": ["# Training run\n", "\n", "Some words.\n"]},
  {"cell_type": "code", "execution_count": 3, "metadata": {},
   "outputs": [
    {"output_type": "stream", "name": "stdout", "text": ["epoch one loss\n"]},
    {"output_type": "error", "ename": "E", "evalue": "v", "traceback": ["Boom happened"]}
   ],
   "source": ["import torch\n", "train()"]}
 ],
 "metadata": {"kernelspec": {"language": "python", "name": "python3"}},
 "nbformat": 4, "nbformat_minor": 5
}"##;

    #[test]
    fn a_notebook_opens_as_a_page_with_its_outputs() {
        let (_dir, app) = data_app("run.ipynb", NOTEBOOK);
        let text = screen(&app).text();
        assert!(
            text.contains("Training run") && text.contains("import torch"),
            "{text}"
        );
        assert!(
            text.contains("epoch one loss") && text.contains("Boom happened"),
            "{text}"
        );
        assert!(
            !text.contains("cell_type") && !text.contains("<!--"),
            "{text}"
        );
    }

    #[test]
    fn saving_a_notebook_keeps_its_json_and_its_outputs() {
        let (dir, mut app) = data_app("run.ipynb", NOTEBOOK);
        let (x, y) = find(&screen(&app), "train()");
        app.handle(mouse(MouseKind::Left, x, y));
        typed(&mut app, "re");
        app.run(Command::Save);
        let saved = fs::read_to_string(dir.path().join("run.ipynb")).unwrap();
        let json: Value = serde_json::from_str(&saved).unwrap();
        let cell = &json["cells"][1];
        assert_eq!(cell["outputs"].as_array().unwrap().len(), 2, "{saved}");
        assert_eq!(cell["source"], json!(["import torch\n", "retrain()"]));
        assert_eq!(cell["execution_count"], 3);
        assert_eq!(json["nbformat_minor"], 5);
        assert!(!app.editors[0].buffer.is_dirty());
        assert!(screen(&app).text().contains("epoch one loss"));
    }

    #[test]
    fn a_notebook_changed_on_disk_reads_again_as_a_page() {
        let (dir, mut app) = data_app("run.ipynb", NOTEBOOK);
        let path = dir.path().join("run.ipynb");
        let changed = NOTEBOOK.replace("Some words.", "Other words.");
        std::thread::sleep(std::time::Duration::from_millis(20));
        fs::write(&path, changed).unwrap();
        app.editors[0].reload().unwrap();
        let text = app.active_text().unwrap();
        assert!(
            text.contains("Other words.") && !text.contains("cell_type"),
            "{text}"
        );
    }

    /// A stand-in for the kernel's runner that speaks its side of the conversation.
    fn fake_runner(dir: &Path, first: &str) -> Vec<String> {
        let script = dir.join("runner.py");
        let body = format!(
            "import json, sys\nprint(json.dumps({first}), flush=True)\nfor line in sys.stdin:\n    job = json.loads(line)\n    print(json.dumps({{'id': job['id'], 'output': {{'output_type': 'stream', 'name': 'stdout', 'text': 'ran: ' + job['code'] + '\\n'}}}}), flush=True)\n    print(json.dumps({{'id': job['id'], 'done': True, 'count': 41}}), flush=True)\n"
        );
        fs::write(&script, body).unwrap();
        vec![
            "python3".to_string(),
            "-I".to_string(),
            script.display().to_string(),
        ]
    }

    fn cell_ran(app: &mut App, wanted: &str) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !screen(app).text().contains(wanted) {
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
    fn a_cell_runs_and_its_output_lands_under_it_and_in_the_file() {
        let (dir, mut app) = data_app("run.ipynb", NOTEBOOK);
        app.kernel_command = Some(fake_runner(dir.path(), "{'ready': True}"));
        let (x, y) = find(&screen(&app), "train()");
        app.handle(mouse(MouseKind::Left, x, y));
        app.run(Command::RunCell);
        assert!(screen(&app).text().contains("Running"));
        cell_ran(&mut app, "ran: import torch");
        let text = screen(&app).text();
        assert!(
            !text.contains("epoch one loss"),
            "the old output went: {text}"
        );
        assert!(text.contains("python  [41]"), "{text}");
        let saved = fs::read_to_string(dir.path().join("run.ipynb")).unwrap();
        let json: Value = serde_json::from_str(&saved).unwrap();
        assert_eq!(json["cells"][1]["execution_count"], 41);
        assert_eq!(json["cells"][1]["outputs"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn a_new_cell_runs_and_keeps_its_output() {
        let (dir, mut app) = data_app("run.ipynb", NOTEBOOK);
        app.kernel_command = Some(fake_runner(dir.path(), "{'ready': True}"));
        let area = app.focused_area();
        let lines = app.active_text().unwrap().lines().count();
        app.editors[0].go_to_line(lines, area);
        app.editors[0].insert_cell(true);
        typed(&mut app, "fresh()");
        app.run(Command::RunCell);
        cell_ran(&mut app, "ran: fresh()");
        let saved = fs::read_to_string(dir.path().join("run.ipynb")).unwrap();
        let json: Value = serde_json::from_str(&saved).unwrap();
        assert_eq!(json["cells"].as_array().unwrap().len(), 3);
    }

    #[test]
    fn a_python_without_jupyter_offers_to_install_it_where_it_can_be_seen() {
        let (dir, mut app) = data_app("run.ipynb", NOTEBOOK);
        app.kernel_command = Some(fake_runner(dir.path(), "{'missing': True}"));
        app.run(Command::RunAllCells);
        cell_ran(&mut app, "Install Jupyter in the Side Pane");
        assert!(!screen(&app).text().contains("Running…"));
    }

    #[test]
    fn a_notebook_has_its_own_bar_that_says_what_the_kernel_is_doing() {
        let (dir, mut app) = data_app("run.ipynb", NOTEBOOK);
        app.kernel_command = Some(fake_runner(dir.path(), "{'ready': True}"));
        assert_eq!(app.page_bar(), Bar::Notebook);
        assert!(screen(&app).text().contains("python3, not started"));
        let row = app.toolbar_row().unwrap();
        let button = toolbar::layout(row, Bar::Notebook)
            .into_iter()
            .find(|b| b.name == "Run All Cells")
            .unwrap();
        app.handle(mouse(MouseKind::Left, button.x + 1, row.y));
        cell_ran(&mut app, "ran: import torch");
        assert!(screen(&app).text().contains("python3, idle"));
    }

    #[test]
    fn shift_enter_runs_a_cell_and_moves_on() {
        let two = NOTEBOOK.replace(r#""nbformat": 4"#, r#""nbformat": 4, "x": 1"#);
        let (dir, mut app) = data_app("run.ipynb", &two);
        app.kernel_command = Some(fake_runner(dir.path(), "{'ready': True}"));
        let (x, y) = find(&screen(&app), "train()");
        app.handle(mouse(MouseKind::Left, x, y));
        app.handle(Event::Key(cellix_input::KeyEvent {
            key: Key::Enter,
            mods: cellix_input::Mods {
                ctrl: false,
                shift: true,
                alt: false,
            },
        }));
        cell_ran(&mut app, "ran: import torch");
        assert_eq!(
            app.active_text().unwrap().matches("train()").count(),
            1,
            "no new line typed"
        );
    }

    #[test]
    fn a_cell_can_be_added_below_the_one_at_the_caret() {
        let (_dir, mut app) = data_app("run.ipynb", NOTEBOOK);
        let (x, y) = find(&screen(&app), "train()");
        app.handle(mouse(MouseKind::Left, x, y));
        app.use_tool(Tool::AddCode, 10);
        typed(&mut app, "after()");
        let text = app.active_text().unwrap();
        let fences = text.matches("```python").count();
        assert_eq!(fences, 2, "{text}");
        assert!(text.find("train()") < text.find("after()"), "{text}");
        assert!(!text.contains("```\n```"), "{text}");
    }

    #[test]
    fn outputs_can_be_cleared() {
        let (dir, mut app) = data_app("run.ipynb", NOTEBOOK);
        app.run(Command::ClearOutputs);
        assert!(!screen(&app).text().contains("epoch one loss"));
        let saved = fs::read_to_string(dir.path().join("run.ipynb")).unwrap();
        assert!(!saved.contains("epoch one loss"));
    }

    #[test]
    fn a_broken_notebook_opens_as_text() {
        let (_dir, app) = data_app("bad.ipynb", "{ not json");
        assert_eq!(app.active_text().unwrap(), "{ not json");
    }

    #[test]
    fn an_empty_window_says_what_to_do_and_its_actions_are_clickable() {
        let (_dir, mut app) = project();
        let text = screen(&app).text();
        assert!(
            text.contains("Start an Agent") && text.contains("Getting Started"),
            "{text}"
        );
        let rows = app.welcome_rows();
        let (y, ..) = rows.iter().find(|row| row.1 == "Start an Agent").unwrap();
        let x = app.welcome_box().x + 1;
        app.handle(mouse(MouseKind::Left, x, *y));
        assert!(app.dock.open, "the row does what it says");
    }

    #[test]
    fn the_agents_panel_explains_itself_before_anything_runs() {
        let (_dir, mut app) = project();
        app.run(Command::ShowAgents);
        let text = screen(&app).text();
        for want in [
            "HOW IT WORKS",
            "Start an agent below",
            "Click a file on the left",
        ] {
            assert!(text.contains(want), "{want}: {text}");
        }
        assert!(text.contains("Terminal"), "{text}");
    }

    #[test]
    fn with_an_agent_running_and_nothing_open_the_next_step_is_named() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("alpha.txt"), "x\n").unwrap();
        let mut app = App::new(dir.path().to_path_buf(), (150, 40));
        let script = "printf ready; sleep 30";
        let argv = ["sh".to_string(), "-c".to_string(), script.to_string()];
        let root = app.workspace.root().to_path_buf();
        app.start_session("Claude Code", &argv, &[], &root);
        shows(&mut app, "ready");
        assert!(app.welcome_lead().contains("Claude Code is running"));
        assert!(app.welcome_lead().contains("Click a file on the left"));
        let names: Vec<_> = app.welcome_rows().iter().map(|row| row.1).collect();
        assert!(!names.contains(&"Start an Agent"), "{names:?}");
    }

    #[test]
    fn the_first_start_says_how_the_sides_work_together() {
        let (_dir, mut app) = project();
        let root = app.workspace.root().to_path_buf();
        app.start_session("Stand-in", &["sh".to_string()], &[], &root);
        assert!(screen(&app).text().contains("Ctrl+`"));
    }

    #[test]
    fn the_guide_opens_as_a_page() {
        let (dir, mut app) = project();
        app.built_folder = dir.path().join("figures");
        app.run(Command::ShowGuide);
        assert!(app.editors[0].rendered);
        assert!(app.active_text().unwrap().contains("Work beside an agent"));
    }

    #[test]
    fn a_new_document_starts_from_the_empty_window() {
        let (_dir, mut app) = project();
        app.run(Command::NewDocument);
        assert!(screen(&app).text().contains("New document"));
    }

    fn select_word(app: &mut App, word: &str) {
        let (x, y) = find(&screen(app), word);
        double_click(app, x + 1, y);
    }

    #[test]
    fn a_selection_offers_a_comment_and_writes_it_beside_the_page() {
        let (_dir, mut app) = page_app("# Plan\n\nThe cache goes first.\n");
        assert!(!screen(&app).text().contains("Ask Agent"));
        select_word(&mut app, "cache");
        let grid = screen(&app);
        assert!(grid.text().contains("Ask Agent"), "{}", grid.text());
        let (x, y) = find(&grid, "Comment");
        app.handle(mouse(MouseKind::Left, x, y));
        typed(&mut app, "Why first");
        enter(&mut app);
        assert_eq!(app.threads[0].quote, "cache");
        assert_eq!(app.threads[0].text, "Why first");
    }

    #[test]
    fn the_selection_bar_stands_clear_of_every_selected_line() {
        let (dir, mut app) = project();
        let path = dir.path().join("lines.txt");
        fs::write(&path, "one\ntwo\nthree\nfour\nfive\n").unwrap();
        app.open(&path).unwrap();
        let editor = app.editor_mut().unwrap();
        // Dragged down from "two" to the end of "four", so the caret is on the last line.
        editor.selections.set_single(Selection {
            anchor: 4,
            head: 18,
            goal: None,
        });
        let grid = screen(&app);
        let (_, first) = find(&grid, "two");
        let (comment, _) = app.selection_bar().expect("a bar");
        assert_eq!(comment.y, first - 1, "above the first selected line");
        let editor = app.editor_mut().unwrap();
        editor.selections.add(Selection {
            anchor: 20,
            head: 22,
            goal: None,
        });
        assert!(app.selection_bar().is_none(), "several cursors get none");
    }

    #[test]
    fn ask_agent_on_a_selection_goes_straight_to_the_agent() {
        let (dir, mut app) = page_app("# Plan\n\nThe cache goes first.\n");
        session(&mut app, "printf ready; cat > heard.txt");
        select_word(&mut app, "cache");
        let (x, y) = find(&screen(&app), "Ask Agent");
        app.handle(mouse(MouseKind::Left, x, y));
        typed(&mut app, "Is this right");
        enter(&mut app);
        let log = dir.path().join("heard.txt");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let heard = loop {
            let text = fs::read_to_string(&log).unwrap_or_default();
            if text.contains("Is this right") {
                break text;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "the agent heard nothing"
            );
            app.poll();
            std::thread::sleep(std::time::Duration::from_millis(20));
        };
        assert!(heard.contains("cache"), "what it's about: {heard}");
        assert!(heard.contains("cellix reply"), "how to answer: {heard}");
    }

    /// Inside Claude Code the agent is the conversation beside the pane, so Ask Agent asks it
    /// there, where it opened a question box for an agent in Cellix's own pane.
    #[test]
    fn inside_claude_code_ask_agent_goes_to_its_prompt() {
        let (_dir, mut app) = page_app("# Plan\n\nThe cache goes first.\n");
        app.in_pane = true;
        select_word(&mut app, "cache");
        let (x, y) = find(&screen(&app), "Ask Agent");
        app.handle(mouse(MouseKind::Left, x, y));
        let fill = app
            .take_outgoing_fill()
            .expect("words for Claude Code's prompt");
        assert!(fill.starts_with('@'), "a mention of the file: {fill}");
        assert!(fill.contains("line 3, \"cache\""), "{fill}");
        assert!(app.dock.draft.is_none(), "no question box in Cellix");
        assert!(
            status_line(&app).contains("\"cache\" is in Claude Code's prompt. Press Esc to ask"),
            "{}",
            status_line(&app)
        );
        assert!(
            app.selection_bar().is_none(),
            "the bar goes once it's pressed"
        );
        assert!(app.take_outgoing_fill().is_none(), "once");
    }

    #[test]
    fn code_can_be_commented_on_from_a_selection_too() {
        let (dir, mut app) = project();
        app.open(&dir.path().join("alpha.txt")).unwrap();
        select_word(&mut app, "first");
        let (x, y) = find(&screen(&app), "Comment");
        app.handle(mouse(MouseKind::Left, x, y));
        assert_eq!(app.dock.front, Front::Comments);
        typed(&mut app, "Look here");
        enter(&mut app);
        assert_eq!(app.threads[0].quote, "first");
        assert_eq!(
            app.comments.lock().unwrap().len(),
            1,
            "it goes with the review"
        );
    }

    #[test]
    fn a_paper_takes_comments_the_same_way() {
        let dir = tempfile::tempdir().unwrap();
        let source =
            "\\documentclass{article}\n\\begin{document}\nThe cache goes first.\n\\end{document}\n";
        fs::write(dir.path().join("paper.tex"), source).unwrap();
        let mut app = App::new(dir.path().to_path_buf(), (150, 40));
        app.open(&dir.path().join("paper.tex")).unwrap();
        select_word(&mut app, "cache");
        let grid = screen(&app);
        assert!(grid.text().contains("Ask Agent"), "{}", grid.text());
        let (x, y) = find(&grid, "Comment");
        app.handle(mouse(MouseKind::Left, x, y));
        typed(&mut app, "Why first");
        enter(&mut app);
        assert_eq!(app.threads[0].quote, "cache");
        assert_eq!(app.threads[0].line, 3);
    }

    #[test]
    fn data_takes_comments_the_same_way() {
        let (_dir, mut app) = data_app("runs.csv", "model,loss\nalpha,0.25\n");
        select_word(&mut app, "alpha");
        let (x, y) = find(&screen(&app), "Comment");
        app.handle(mouse(MouseKind::Left, x, y));
        typed(&mut app, "Odd value");
        enter(&mut app);
        assert_eq!(app.threads[0].quote, "alpha");
        assert_eq!(app.threads[0].file, "runs.csv");
    }

    #[test]
    fn a_notebook_takes_comments_the_same_way() {
        let (_dir, mut app) = data_app("run.ipynb", NOTEBOOK);
        select_word(&mut app, "torch");
        let (x, y) = find(&screen(&app), "Ask Agent");
        app.handle(mouse(MouseKind::Left, x, y));
        assert_eq!(app.dock.draft.as_ref().unwrap().quote, "torch");
    }

    #[test]
    fn bold_italics_hide_all_their_stars() {
        let (_dir, app) = picture_page("Some ***both*** here\n");
        let shown: String = tones_on_page(&app).iter().map(|(c, _)| *c).collect();
        assert!(!shown.contains('*'), "{shown}");
    }

    #[test]
    fn nested_list_markers_keep_their_distance_from_their_text() {
        let (_dir, app) = picture_page("- one\n  - two\n    - three\n");
        let rows = app.editors[0].page_rows(app.focused_area());
        let gaps: Vec<f32> = rows
            .iter()
            .filter(|r| r.glyphs.iter().any(|g| g.at.is_some()))
            .map(|r| {
                let text = r.glyphs.iter().find(|g| g.at.is_some()).unwrap();
                let mark = r.glyphs.iter().find(|g| "•◦▪".contains(g.ch)).unwrap();
                text.x - mark.x
            })
            .collect();
        assert_eq!(gaps.len(), 3);
        // Painted as one run its spaces would take the font's width, and the marker stay put.
        let row = rows
            .iter()
            .find(|r| r.glyphs.iter().any(|g| g.ch == '◦'))
            .unwrap();
        assert_eq!(row.glyphs[0].tone, crate::rendered::Tone::TableLine);
        let spread =
            gaps.iter().cloned().fold(0.0, f32::max) - gaps.iter().cloned().fold(9.0, f32::min);
        assert!(spread < 0.2, "{gaps:?}");
    }

    #[test]
    fn a_notebooks_error_is_red_and_not_crossed_out() {
        let (_dir, app) = data_app("run.ipynb", NOTEBOOK);
        assert_eq!(
            text_with(&app, crate::rendered::Tone::Failed),
            "Boom happened"
        );
        assert_eq!(text_with(&app, crate::rendered::Tone::Deleted), "");
    }

    #[test]
    fn data_keeps_whole_cells_when_there_is_room() {
        let long = "a long note that runs on well past forty columns of text here";
        let (_dir, app) = data_app("runs.csv", &format!("name,note\nalpha,{long}\n"));
        assert!(screen(&app).text().contains(long));
    }

    #[test]
    fn data_says_what_it_is_in_the_status_bar() {
        let (_dir, app) = data_app("runs.csv", "a,b\n1,2\n");
        assert!(screen(&app).text().contains("CSV"));
    }

    #[test]
    #[cfg(unix)]
    fn an_algorithm_is_built_from_its_steps_with_its_caption_on_the_page() {
        let dir = tempfile::tempdir().unwrap();
        let sample = dir.path().join("sample.pdf");
        fs::write(&sample, cellix_doc::pdf::sample()).unwrap();
        let source = "\\begin{document}\n\\begin{algorithm}[h]\n\\caption{Descent}\n\\begin{algorithmic}[1]\n\\State go\n\\end{algorithmic}\n\\end{algorithm}\n\\end{document}\n";
        fs::write(dir.path().join("paper.tex"), source).unwrap();
        let mut app = App::new(dir.path().to_path_buf(), (150, 40));
        app.set_measure(Rc::new(cellix_render::FontMeasure::new(10, 20)));
        let reply = format!(
            "grep -q caption snippet.tex && exit 1; grep -q algorithmic snippet.tex && cp {} snippet.pdf",
            sample.display()
        );
        app.settings.latex_engine = Some(fake_latex(dir.path(), &reply));
        app.built_folder = dir.path().join("built");
        app.open(&dir.path().join("paper.tex")).unwrap();
        waits_for(&mut app, |app| !pictures(app).is_empty());
        assert!(painted_text(&app).contains("Algorithm 1"));
    }

    fn tex_rows(source: &str) -> (TempDir, App) {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("paper.tex"), source).unwrap();
        let mut app = App::new(dir.path().to_path_buf(), (150, 40));
        app.set_measure(Rc::new(cellix_render::FontMeasure::new(10, 20)));
        app.open(&dir.path().join("paper.tex")).unwrap();
        (dir, app)
    }

    #[test]
    fn the_end_of_a_proof_closes_its_last_line() {
        let (_dir, app) =
            tex_rows("\\begin{document}\n\\begin{proof}\nShort.\n\\end{proof}\n\\end{document}\n");
        let rows = app.editors[0].page_rows(app.focused_area());
        let row = rows
            .iter()
            .find(|r| r.glyphs.iter().any(|g| g.ch == '∎'))
            .unwrap();
        assert!(
            row.glyphs.iter().any(|g| g.ch == 'S'),
            "on the same row as the words"
        );
    }

    #[test]
    fn a_caption_stands_in_the_middle_of_the_page() {
        let (_dir, app) = tex_rows(
            "\\begin{document}\n\\begin{figure}\n\\caption{Short}\n\\end{figure}\n\\end{document}\n",
        );
        let rows = app.editors[0].page_rows(app.focused_area());
        let row = rows
            .iter()
            .find(|r| r.glyphs.iter().any(|g| g.ch == 'F'))
            .unwrap();
        assert!(row.glyphs[0].x > 20.0, "{}", row.glyphs[0].x);
    }

    #[test]
    fn the_list_of_agents_reads_as_a_tab_of_its_own() {
        let (_dir, mut app) = project();
        app.run(Command::ShowAgents);
        let grid = screen(&app);
        let area = app.dock_area().unwrap();
        assert!(grid.row_text(area.y).contains("Agents"), "{}", grid.text());
        // The panel doesn't say it twice.
        assert!(!grid.row_text(area.y + 1).contains("Agents"));
    }

    #[test]
    fn a_built_drawing_is_carried_inside_the_web_page() {
        let dir = tempfile::tempdir().unwrap();
        let body = "\\begin{tikzpicture}\n\\draw (0,0);\n\\end{tikzpicture}";
        let snippet = Snippet::new(cellix_doc::latex::standalone(&[], body));
        fs::write(
            dir.path().join(format!("{:016x}.pdf", snippet.key)),
            cellix_doc::pdf::sample(),
        )
        .unwrap();
        let html = built_as_html(dir.path(), body).unwrap();
        assert!(
            html.starts_with("<img src=\"data:image/png;base64,"),
            "{html}"
        );
        assert!(built_as_html(dir.path(), "something else").is_none());
    }

    #[test]
    fn the_pointer_says_what_a_place_is() {
        let (dir, mut app) = project();
        app.open(&dir.path().join("alpha.txt")).unwrap();
        let (x, y) = find(&screen(&app), "first");
        app.handle(mouse(MouseKind::Move, x, y));
        assert_eq!(app.pointer(), "text");
        app.handle(mouse(MouseKind::Move, 2, 4));
        assert_eq!(app.pointer(), "pointer");
        let (x, y) = find(&screen(&app), "beta.rs");
        app.handle(mouse(MouseKind::Move, x, y));
        assert_eq!(app.pointer(), "pointer");
        app.handle(Event::Tick);
        app.run(Command::CommandPalette);
        app.handle(mouse(MouseKind::Move, 90, 30));
        assert_eq!(app.pointer(), "default", "a dialog is in the way");
    }

    #[test]
    fn a_minimap_shows_a_documents_pictures() {
        let mut text = "![Plot](plot.png)\n\n".to_string();
        text.push_str(&"line\n\n".repeat(80));
        let (_dir, app) = picture_page(&text);
        let _ = screen(&app);
        assert_eq!(
            pictures(&app).len(),
            2,
            "the picture, and its place in the map"
        );
    }

    fn sheet_keys(source: &str) -> (TempDir, App) {
        let (dir, mut app) = data_app("runs.csv", source);
        let (x, y) = find(&screen(&app), "alpha");
        app.handle(mouse(MouseKind::Left, x, y));
        (dir, app)
    }

    #[test]
    fn tab_moves_along_a_row_and_on_to_the_next_replacing_what_it_lands_on() {
        let (_dir, mut app) = sheet_keys("name,loss\nalpha,1\nbeta,2\n");
        key(&mut app, Key::Tab);
        assert_eq!(app.selected_text().as_deref(), Some("1"));
        key(&mut app, Key::Tab);
        assert_eq!(app.selected_text().as_deref(), Some("beta"));
        typed(&mut app, "gamma");
        assert_eq!(app.active_text().unwrap(), "name,loss\nalpha,1\ngamma,2\n");
    }

    #[test]
    fn shift_tab_goes_back_a_cell() {
        let (_dir, mut app) = sheet_keys("name,loss\nalpha,1\n");
        key(&mut app, Key::Tab);
        key(&mut app, Key::BackTab);
        assert_eq!(app.selected_text().as_deref(), Some("alpha"));
    }

    #[test]
    fn tab_past_the_last_cell_adds_a_row() {
        let (_dir, mut app) = sheet_keys("name,loss\nalpha,1\n");
        key(&mut app, Key::Tab);
        key(&mut app, Key::Tab);
        typed(&mut app, "new");
        assert_eq!(app.active_text().unwrap(), "name,loss\nalpha,1\nnew,\n");
    }

    #[test]
    fn enter_goes_down_the_column_and_adds_a_row_at_the_bottom() {
        let (_dir, mut app) = sheet_keys("name,loss\nalpha,1\nbeta,2\n");
        key(&mut app, Key::Tab);
        key(&mut app, Key::Enter);
        assert_eq!(app.selected_text().as_deref(), Some("2"));
        key(&mut app, Key::Enter);
        typed(&mut app, "3");
        assert_eq!(
            app.active_text().unwrap(),
            "name,loss\nalpha,1\nbeta,2\n,3\n"
        );
    }

    #[test]
    fn a_markdown_table_moves_the_same_way() {
        let source = "| Name | Loss |\n|---|---|\n| alpha | one |\n";
        let (_dir, mut app) = page_app(source);
        let (x, y) = cell_of(&app, "alpha");
        app.handle(mouse(MouseKind::Left, x, y));
        key(&mut app, Key::Tab);
        assert_eq!(app.selected_text().as_deref(), Some("one"));
        key(&mut app, Key::Tab);
        typed(&mut app, "next");
        let text = app.active_text().unwrap();
        assert_eq!(text.lines().count(), 4, "{text}");
        assert!(text.lines().nth(3).unwrap().contains("next"), "{text}");
    }

    /// A repository with a commit, one changed file and one new one, with the panel showing.
    fn source_app() -> (TempDir, App) {
        let dir = tempfile::tempdir().unwrap();
        git(dir.path(), &["init", "-q"]);
        git(dir.path(), &["config", "user.email", "t@example.com"]);
        git(dir.path(), &["config", "user.name", "Tester"]);
        fs::write(dir.path().join("kept.txt"), "one\n").unwrap();
        git(dir.path(), &["add", "."]);
        git(dir.path(), &["commit", "-q", "-m", "base"]);
        fs::write(dir.path().join("kept.txt"), "two\n").unwrap();
        fs::write(dir.path().join("fresh.txt"), "new\n").unwrap();
        let mut app = App::new(dir.path().to_path_buf(), (150, 40));
        app.run(Command::ShowSourceControl);
        app.source_settled();
        (dir, app)
    }

    /// Clicks a mark on a file's row.
    fn press_on_row(app: &mut App, name: &str, from_right: u16) {
        let area = app.sidebar_area().unwrap();
        let (_, y) = find(&screen(app), name);
        app.handle(mouse(MouseKind::Left, area.right() - from_right, y));
    }

    fn staged_files(dir: &Path) -> String {
        let output = std::process::Command::new("git")
            .args([
                "-C",
                &dir.display().to_string(),
                "diff",
                "--cached",
                "--name-only",
            ])
            .output()
            .unwrap();
        String::from_utf8_lossy(&output.stdout).into_owned()
    }

    #[test]
    fn source_control_lists_changes_and_stages_and_unstages_them() {
        let (dir, mut app) = source_app();
        let text = screen(&app).text();
        assert!(
            text.contains("SOURCE CONTROL") && text.contains("CHANGES"),
            "{text}"
        );
        assert!(!text.contains("STAGED CHANGES"), "{text}");
        press_on_row(&mut app, "kept.txt", crate::source::STAGE_FROM_RIGHT);
        assert_eq!(staged_files(dir.path()).trim(), "kept.txt");
        assert!(screen(&app).text().contains("STAGED CHANGES"));
        press_on_row(&mut app, "kept.txt", crate::source::STAGE_FROM_RIGHT);
        assert_eq!(staged_files(dir.path()).trim(), "");
    }

    #[test]
    fn a_message_commits_what_is_staged() {
        let (dir, mut app) = source_app();
        press_on_row(&mut app, "kept.txt", crate::source::STAGE_FROM_RIGHT);
        let area = app.sidebar_area().unwrap();
        app.handle(mouse(MouseKind::Left, area.x + 4, area.y + 3));
        typed(&mut app, "Change the words");
        enter(&mut app);
        let log = std::process::Command::new("git")
            .args([
                "-C",
                &dir.path().display().to_string(),
                "log",
                "-1",
                "--format=%s",
            ])
            .output()
            .unwrap();
        assert_eq!(
            String::from_utf8_lossy(&log.stdout).trim(),
            "Change the words"
        );
        assert!(app.source.message.is_empty());
        let text = screen(&app).text();
        assert!(
            text.contains("fresh.txt") && !text.contains("STAGED"),
            "{text}"
        );
    }

    #[test]
    fn with_nothing_staged_commit_takes_everything() {
        let (_dir, mut app) = source_app();
        assert!(screen(&app).text().contains("Commit All"));
        app.source.message = "All of it".to_string();
        let (x, y) = find(&screen(&app), "Commit All");
        app.handle(mouse(MouseKind::Left, x, y));
        assert!(screen(&app).text().contains("No changes"));
    }

    #[test]
    fn a_commit_with_no_message_asks_for_one() {
        let (_dir, mut app) = source_app();
        let (x, y) = find(&screen(&app), "Commit All");
        app.handle(mouse(MouseKind::Left, x, y));
        assert!(screen(&app).text().contains("Write a message first"));
        assert!(app.source.focused);
    }

    #[test]
    fn discarding_asks_first_and_then_puts_the_file_back() {
        let (dir, mut app) = source_app();
        press_on_row(&mut app, "kept.txt", crate::source::DISCARD_FROM_RIGHT);
        assert!(screen(&app).text().contains("Discard changes to kept.txt"));
        assert_eq!(
            fs::read_to_string(dir.path().join("kept.txt")).unwrap(),
            "two\n"
        );
        app.choose(Choice::DiscardFile(PathBuf::from("kept.txt"), false));
        assert_eq!(
            fs::read_to_string(dir.path().join("kept.txt")).unwrap(),
            "one\n"
        );
    }

    #[test]
    fn a_click_on_a_file_shows_its_diff_beside_the_panel() {
        let (_dir, mut app) = source_app();
        let (x, y) = find(&screen(&app), "kept.txt");
        app.handle(mouse(MouseKind::Left, x, y));
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while app.review.as_ref().unwrap().is_reading() {
            assert!(std::time::Instant::now() < deadline);
            app.poll();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(app.sidebar_area().is_some(), "the panel stays");
        assert_eq!(
            app.review.as_ref().unwrap().current_path(),
            Some(Path::new("kept.txt"))
        );
    }

    #[test]
    fn commits_to_push_show_and_push_goes_through() {
        let (dir, mut app) = source_app();
        let remote = dir.path().join("remote.git");
        let bare = remote.display().to_string();
        git(dir.path(), &["init", "-q", "--bare", &bare]);
        git(dir.path(), &["remote", "add", "origin", &bare]);
        let branch = cellix_review::current_branch(dir.path()).unwrap();
        git(dir.path(), &["push", "-q", "-u", "origin", &branch]);
        git(dir.path(), &["commit", "-q", "--allow-empty", "-m", "more"]);
        app.source.refresh(dir.path());
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while app.source.sync_label().as_deref() != Some("Push 1") {
            assert!(std::time::Instant::now() < deadline);
            app.source.poll();
            app.source.refresh(dir.path());
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        app.sync_remote();
        while app.source.busy.is_some() {
            assert!(std::time::Instant::now() < deadline);
            app.poll();
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(screen(&app).text().contains("Pushed"));
        assert_eq!(cellix_review::ahead_behind(dir.path()), Some((0, 0)));
    }

    /// Waits for the Preview to finish reading, as a frame would.
    fn preview_read(app: &mut App) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        loop {
            let view = app.dock.pdf.as_ref().unwrap();
            if view.pages_ready() > 0 || view.error.is_some() {
                return;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "the Preview never read it"
            );
            app.poll();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    #[test]
    fn the_preview_is_called_preview_and_says_image_for_an_image() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("broken.png"), b"not a picture").unwrap();
        let mut app = App::new(dir.path().to_path_buf(), (150, 44));
        app.set_measure(Rc::new(cellix_render::FontMeasure::new(10, 20)));
        app.open(&dir.path().join("broken.png")).unwrap();
        let area = app.dock_area().unwrap();
        let tab = screen(&app).row_text(area.y);
        assert!(tab.contains("Preview"), "{tab}");
        assert!(!tab.contains("PDF"), "{tab}");
        assert!(screen(&app).text().contains("Reading the image…"));
        preview_read(&mut app);
        let text = screen(&app).text();
        assert!(text.contains("This image can't be shown."), "{text}");
        assert!(!text.contains("PDF"), "{text}");
        // The Preview tab is as wide as its name and its ×, as the others are.
        let (front, _, rect) = app.dock.view_tabs_for_test(area)[0];
        assert_eq!((front, rect.w), (Front::Pdf, 12));
    }

    #[test]
    fn an_image_on_the_cell_tier_says_it_is_ready() {
        let dir = tempfile::tempdir().unwrap();
        let image = image::RgbaImage::from_pixel(40, 20, image::Rgba([1, 2, 3, 255]));
        image.save(dir.path().join("loss.png")).unwrap();
        let mut app = App::new(dir.path().to_path_buf(), (150, 44));
        app.open(&dir.path().join("loss.png")).unwrap();
        let text = screen(&app).text();
        assert!(text.contains("Image ready:"), "{text}");
        assert!(text.contains("Preview"), "{text}");
    }

    #[test]
    fn clicking_an_image_does_not_ask_for_synctex() {
        let dir = tempfile::tempdir().unwrap();
        let image = image::RgbaImage::from_pixel(80, 80, image::Rgba([1, 2, 3, 255]));
        image.save(dir.path().join("loss.png")).unwrap();
        let mut app = App::new(dir.path().to_path_buf(), (150, 44));
        app.set_measure(Rc::new(cellix_render::FontMeasure::new(10, 20)));
        app.open(&dir.path().join("loss.png")).unwrap();
        preview_read(&mut app);
        let picture = pictures(&app)[0];
        let (x, y) = (
            ((picture.x + 64) / 64) as u16,
            ((picture.y + 64) / 64) as u16,
        );
        app.handle(mouse(MouseKind::Left, x, y));
        app.handle(mouse(MouseKind::Up, x, y));
        assert!(!screen(&app).text().contains("SyncTeX"));
        let bar = screen(&app).row_text(app.dock_screen().unwrap().y);
        assert!(bar.contains("loss.png"), "the bar names the image: {bar}");
    }

    /// A wide image beside a page, zoomed until it's wider than the pane.
    fn zoomed_preview() -> (TempDir, App) {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("results.txt"), "The loss settles.\n").unwrap();
        let image = image::RgbaImage::from_pixel(2000, 1800, image::Rgba([200, 30, 30, 255]));
        image.save(dir.path().join("loss.png")).unwrap();
        let mut app = App::new(dir.path().to_path_buf(), (150, 44));
        app.set_measure(Rc::new(cellix_render::FontMeasure::new(10, 20)));
        app.open(&dir.path().join("results.txt")).unwrap();
        app.open(&dir.path().join("loss.png")).unwrap();
        preview_read(&mut app);
        let screen_area = app.dock_screen().unwrap();
        assert!(
            !app.dock.pdf.as_ref().unwrap().wide(screen_area.w),
            "fits at first"
        );
        let plus = (0..screen_area.w)
            .map(|dx| screen_area.x + dx)
            .find(|x| {
                let view = app.dock.pdf.as_ref().unwrap();
                view.hit(screen_area, *x, screen_area.y) == Some(PdfHit::ZoomIn)
            })
            .unwrap();
        for _ in 0..4 {
            app.handle(mouse(MouseKind::Left, plus, screen_area.y));
            app.handle(mouse(MouseKind::Up, plus, screen_area.y));
        }
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while !app.dock.pdf.as_ref().unwrap().wide(screen_area.w) {
            assert!(
                std::time::Instant::now() < deadline,
                "never grew past the pane"
            );
            app.poll();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        // Zooming keeps the middle in view; these start from the page's left edge.
        app.dock.pdf.as_mut().unwrap().place(0.0, 0.0);
        (dir, app)
    }

    /// The scrollbar thumbs drawn over the Preview: (sideways, upright).
    fn preview_thumbs(app: &App) -> (Vec<cellix_render::Bounds>, Vec<cellix_render::Bounds>) {
        let screen_area = app.dock_screen().unwrap();
        let left = i32::from(screen_area.x) * 64;
        let thumbs: Vec<_> = screen(app)
            .paint
            .iter()
            .filter_map(|p| match p {
                Paint::Panel {
                    bounds,
                    fill: Some(fill),
                    ..
                } if *fill == app.theme.scrollbar && bounds.x >= left => Some(*bounds),
                _ => None,
            })
            .collect();
        thumbs.into_iter().partition(|b| b.w > b.h)
    }

    #[test]
    fn a_zoomed_preview_pans_by_dragging_and_a_click_still_clicks() {
        let (_dir, mut app) = zoomed_preview();
        let screen_area = app.dock_screen().unwrap();
        // The page is cut off at the right, so it says how to see the rest.
        assert!(painted_text(&app).contains(PAN_HINT_TEXT));
        let (sideways, _) = preview_thumbs(&app);
        assert_eq!(sideways.len(), 1, "a thumb under the pages");
        let before = pictures(&app)[0];
        let (x, y) = (screen_area.x + 30, screen_area.y + 6);
        // A press and release in one place is a click, and moves nothing.
        app.handle(mouse(MouseKind::Left, x, y));
        app.handle(mouse(MouseKind::Up, x, y));
        assert_eq!(pictures(&app)[0], before);
        // Dragged, the page follows the pointer.
        app.handle(mouse(MouseKind::Left, x, y));
        app.handle(mouse(MouseKind::Drag, x - 10, y));
        app.handle(mouse(MouseKind::Up, x - 10, y));
        let after = pictures(&app)[0];
        assert_eq!(after.x, before.x - 10 * 64, "ten columns along");
        assert!((app.dock.pdf.as_ref().unwrap().left - 10.0).abs() < 0.01);
        let (moved, _) = preview_thumbs(&app);
        assert!(moved[0].x > sideways[0].x, "the thumb moves with it");
        // The page's outline stops at the pane, however far the page has moved.
        let dock_x = i32::from(app.dock_area().unwrap().x) * 64;
        let below_bar = i32::from(screen_area.y + 1) * 64;
        let spills = screen(&app).paint.iter().any(|p| match p {
            Paint::Hairline { bounds, color } => {
                *color == app.theme.border && bounds.x < dock_x - 64 && bounds.y > below_bar
            }
            Paint::Panel {
                bounds,
                border: Some(color),
                ..
            } => *color == app.theme.border && bounds.x < dock_x,
            _ => false,
        });
        assert!(!spills, "nothing of the page beside the pane");
        // The first pan sends the hint away, for the rest of the run.
        assert!(!painted_text(&app).contains(PAN_HINT_TEXT));
        // It never pans past the page's edge.
        app.handle(mouse(MouseKind::Left, x, y));
        app.handle(mouse(MouseKind::Drag, x + 60, y));
        app.handle(mouse(MouseKind::Up, x + 60, y));
        assert_eq!(app.dock.pdf.as_ref().unwrap().left, 0.0);
    }

    #[test]
    fn the_maximize_button_stays_while_the_preview_is_zoomed_and_panned() {
        let (_dir, mut app) = zoomed_preview();
        let has_button = |app: &App| {
            let area = app.dock_area().unwrap();
            let x = i32::from(app.dock.maximize_button(area)) * 64;
            let row = i32::from(area.y);
            screen(app).paint.iter().any(|p| {
                matches!(p, Paint::Icon { bounds, icon: Icon::Maximize, .. }
                    if bounds.x >= x && bounds.x < x + 3 * 64 && bounds.y / 64 == row)
            })
        };
        assert!(has_button(&app), "zoomed");
        let screen_area = app.dock_screen().unwrap();
        let (x, y) = (screen_area.x + 30, screen_area.y + 6);
        app.handle(mouse(MouseKind::Left, x, y));
        app.handle(mouse(MouseKind::Drag, x - 10, y + 3));
        app.handle(mouse(MouseKind::Up, x - 10, y + 3));
        assert!(app.dock.pdf.as_ref().unwrap().left > 0.0);
        assert!(has_button(&app), "panned");
        // Hovered, as a pointer on its way to it leaves it.
        let area = app.dock_area().unwrap();
        app.handle(mouse(
            MouseKind::Move,
            app.dock.maximize_button(area) + 1,
            area.y,
        ));
        assert!(has_button(&app), "hovered");
    }

    #[test]
    fn the_pan_hint_goes_by_itself() {
        let (_dir, mut app) = zoomed_preview();
        assert!(painted_text(&app).contains(PAN_HINT_TEXT));
        app.dock.pan_hint = Some(std::time::Instant::now() - PAN_HINT);
        assert!(app.poll());
        assert!(!painted_text(&app).contains(PAN_HINT_TEXT));
    }

    #[test]
    fn escape_puts_a_page_back_mid_drag() {
        let (_dir, mut app) = zoomed_preview();
        let screen_area = app.dock_screen().unwrap();
        let (x, y) = (screen_area.x + 30, screen_area.y + 6);
        app.handle(mouse(MouseKind::Left, x, y));
        app.handle(mouse(MouseKind::Drag, x - 10, y));
        app.handle(Event::Key(KeyEvent::plain(Key::Esc)));
        assert_eq!(app.dock.pdf.as_ref().unwrap().left, 0.0);
        app.handle(mouse(MouseKind::Drag, x - 20, y));
        assert_eq!(app.dock.pdf.as_ref().unwrap().left, 0.0, "the drag is over");
    }

    #[test]
    fn shift_and_the_sideways_wheel_move_a_wide_page_across() {
        let (_dir, mut app) = zoomed_preview();
        let screen_area = app.dock_screen().unwrap();
        let (x, y) = (screen_area.x + 30, screen_area.y + 6);
        let left = |app: &App| app.dock.pdf.as_ref().unwrap().left;
        let shifted = |kind| {
            Event::Mouse(MouseEvent {
                kind,
                x,
                y,
                fine_x: f32::from(x),
                mods: Mods {
                    shift: true,
                    ..Mods::default()
                },
            })
        };
        app.handle(shifted(MouseKind::ScrollDown));
        assert_eq!(left(&app), crate::pdfview::SIDEWAYS);
        app.handle(mouse(MouseKind::ScrollRight, x, y));
        assert_eq!(left(&app), crate::pdfview::SIDEWAYS * 2.0);
        app.handle(mouse(MouseKind::ScrollLeft, x, y));
        app.handle(shifted(MouseKind::ScrollUp));
        assert_eq!(left(&app), 0.0);
        // The plain wheel still goes down the page.
        app.handle(mouse(MouseKind::ScrollDown, x, y));
        assert_eq!(left(&app), 0.0);
        assert!(app.dock.pdf.as_ref().unwrap().top > 0.0);
    }

    #[test]
    fn the_thumb_under_a_wide_page_drags_it_across() {
        let (_dir, mut app) = zoomed_preview();
        let screen_area = app.dock_screen().unwrap();
        let bottom = screen_area.bottom() - 1;
        app.handle(mouse(MouseKind::Left, screen_area.x + 1, bottom));
        assert_eq!(app.dock.pdf.as_ref().unwrap().left, 0.0);
        app.handle(mouse(MouseKind::Drag, screen_area.right() - 1, bottom));
        app.handle(mouse(MouseKind::Up, screen_area.right() - 1, bottom));
        let view = app.dock.pdf.as_ref().unwrap();
        assert!(view.left > 0.0);
        // At the far end, the page's right edge is in view.
        let picture = pictures(&app)[0];
        let right = (picture.x + picture.w) / 64;
        assert!(right <= i32::from(screen_area.right()), "{right}");
        assert!(right >= i32::from(screen_area.right()) - 2, "{right}");
    }

    #[test]
    fn space_in_the_preview_types_nothing() {
        let (_dir, mut app) = zoomed_preview();
        let screen_area = app.dock_screen().unwrap();
        let (x, y) = (screen_area.x + 30, screen_area.y + 6);
        app.handle(mouse(MouseKind::Left, x, y));
        app.handle(mouse(MouseKind::Up, x, y));
        typed(&mut app, " ");
        app.handle(mouse(MouseKind::Left, x, y));
        app.handle(mouse(MouseKind::Drag, x - 8, y));
        app.handle(mouse(MouseKind::Up, x - 8, y));
        typed(&mut app, " ");
        assert_eq!(app.active_text().unwrap(), "The loss settles.\n");
        assert!(
            app.dock.pdf.as_ref().unwrap().left > 0.0,
            "Space and a drag pans"
        );
        // Esc gives the keyboard back to the file.
        app.handle(Event::Key(KeyEvent::plain(Key::Esc)));
        typed(&mut app, "x");
        assert_eq!(app.active_text().unwrap(), "xThe loss settles.\n");
    }

    #[test]
    fn sideways_drift_leaves_the_editor_alone() {
        let dir = tempfile::tempdir().unwrap();
        let text: String = (0..200).map(|i| format!("line {i}\n")).collect();
        fs::write(dir.path().join("long.txt"), &text).unwrap();
        let mut app = App::new(dir.path().to_path_buf(), (100, 30));
        app.open(&dir.path().join("long.txt")).unwrap();
        app.say("Saved");
        app.handle(mouse(MouseKind::ScrollDown, 50, 10));
        let top = app.editors[0].top();
        let scrolled = app.scrolled;
        for kind in [MouseKind::ScrollRight, MouseKind::ScrollLeft] {
            app.handle(mouse(kind, 50, 10));
        }
        assert_eq!(app.editors[0].top(), top);
        assert_eq!(app.scrolled, scrolled, "a scroll on its way isn't spoilt");
        // A wheel with no place, as inside Claude Code, never goes sideways into the file.
        app.handle(mouse(MouseKind::ScrollRight, u16::MAX, u16::MAX));
        assert_eq!(app.editors[0].top(), top);
    }

    #[test]
    fn a_pane_drag_inside_claude_code_pans_the_preview() {
        let (_dir, mut app) = zoomed_preview();
        let screen_area = app.dock_screen().unwrap();
        let (x, y) = (screen_area.x + 30, screen_area.y + 6);
        let pointer = |kind: &str, x: u16, button: Option<&str>| crate::pane::Input::Pointer {
            kind: kind.to_string(),
            x,
            y,
            fine: Some(f32::from(x) + 0.5),
            button: button.map(str::to_string),
        };
        let steps = [
            pointer("move", x, None),
            pointer("down", x, Some("left")),
            pointer("move", x - 12, Some("left")),
            pointer("up", x - 12, Some("left")),
        ];
        for input in steps {
            for event in crate::pane::translate(input) {
                app.handle(event);
            }
        }
        assert!((app.dock.pdf.as_ref().unwrap().left - 12.0).abs() < 0.01);
    }

    /// A file beside an image in the Preview, in a window wide enough for the sidebar.
    fn file_and_preview() -> (TempDir, App) {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("notes.txt"), "Some words to read\n").unwrap();
        let image = image::RgbaImage::from_pixel(80, 80, image::Rgba([1, 2, 3, 255]));
        image.save(dir.path().join("loss.png")).unwrap();
        let mut app = App::new(dir.path().to_path_buf(), (150, 44));
        app.set_measure(Rc::new(cellix_render::FontMeasure::new(10, 20)));
        app.open(&dir.path().join("notes.txt")).unwrap();
        app.open(&dir.path().join("loss.png")).unwrap();
        preview_read(&mut app);
        (dir, app)
    }

    fn click(app: &mut App, x: u16, y: u16) {
        app.handle(mouse(MouseKind::Left, x, y));
        app.handle(mouse(MouseKind::Up, x, y));
    }

    #[test]
    fn the_side_pane_maximizes_from_its_button_and_its_tab() {
        let (_dir, mut app) = file_and_preview();
        let icons = app.pane_actions();
        let area = app.dock_area().unwrap();
        assert!(app.sidebar_area().is_some());
        let button = app.dock.maximize_button(area);
        app.handle(mouse(MouseKind::Move, button + 1, area.y));
        assert!(painted_text(&app).contains("Maximize"));
        click(&mut app, button + 1, area.y);
        // The pane takes everything right of the rail; the file and sidebar wait.
        let full = app.dock_area().unwrap();
        assert_eq!((full.x, full.right()), (app.activity_area().right(), 150));
        assert!(app.sidebar_area().is_none());
        let grid = screen(&app);
        assert!(!grid.text().contains("Some words to read"));
        let status = grid.row_text(43);
        assert!(
            status.contains("notes.txt hidden while the pane is maximized"),
            "{status}"
        );
        // The icons at the top right stay where they were.
        let same = |a: &[(Rect, Icon, Command, &str)], b: &[(Rect, Icon, Command, &str)]| {
            a.iter().map(|i| i.0).eq(b.iter().map(|i| i.0))
        };
        assert!(same(&icons, &app.pane_actions()));
        // The button now restores, on the chip a selected icon stands on.
        let button = app.dock.maximize_button(full);
        app.handle(mouse(MouseKind::Move, button + 1, full.y));
        let tip = painted_text(&app);
        assert!(
            tip.contains("Restore") && tip.contains("or double-click the tab"),
            "{tip}"
        );
        let chip = screen(&app).paint.iter().any(|p| {
            matches!(p, Paint::Panel { bounds, fill: Some(fill), .. }
                if *fill == app.theme.tint && bounds.x / 64 >= i32::from(button) - 1
                    && bounds.x / 64 <= i32::from(button) + 1 && bounds.y / 64 == i32::from(full.y))
        });
        assert!(chip, "the restore button stands on a chip");
        click(&mut app, button + 1, full.y);
        assert_eq!(app.dock_area(), Some(area));
        assert!(screen(&app).text().contains("Some words to read"));
        // A double-click on the pane's tab does the same, both ways.
        let (_, _, tab) = app.dock.view_tabs_for_test(area)[0];
        click(&mut app, tab.x + 2, area.y);
        click(&mut app, tab.x + 2, area.y);
        assert_eq!(app.maximized, Some(Maximized::Pane));
        let full = app.dock_area().unwrap();
        let (_, _, tab) = app.dock.view_tabs_for_test(full)[0];
        click(&mut app, tab.x + 2, full.y);
        click(&mut app, tab.x + 2, full.y);
        assert_eq!(app.maximized, None);
        assert!(same(&icons, &app.pane_actions()));
    }

    #[test]
    fn double_clicking_an_editor_tab_maximizes_it() {
        let (dir, mut app) = file_and_preview();
        app.open(&dir.path().join("loss.png")).unwrap();
        fs::write(dir.path().join("more.txt"), "Other words\n").unwrap();
        app.open(&dir.path().join("more.txt")).unwrap();
        app.run(Command::SplitPane);
        let tab_y = app.tabs_area().y;
        let (_, x, _) = app.visible_tabs()[1];
        click(&mut app, x + 1, tab_y);
        click(&mut app, x + 1, tab_y);
        assert_eq!(app.maximized, Some(Maximized::Editor));
        let rail = app.activity_area().right();
        let shown = app.focused_area();
        assert_eq!(
            (shown.x, shown.right()),
            (rail, 150),
            "the whole window past the rail"
        );
        let others = (0..app.panes.len()).filter(|p| *p != app.focus);
        assert!(others.into_iter().all(|p| app.pane_area(p).w == 0));
        assert!(app.dock_area().is_none() && app.sidebar_area().is_none());
        assert!(pictures(&app).is_empty(), "the Preview waits out of sight");
        let (_, x, _) = app.visible_tabs()[1];
        click(&mut app, x + 1, tab_y);
        click(&mut app, x + 1, tab_y);
        assert_eq!(app.maximized, None);
        assert!(app.dock_area().is_some() && app.sidebar_area().is_some());
        // The palette has both.
        app.run(Command::MaximizeEditor);
        assert_eq!(app.maximized, Some(Maximized::Editor));
        app.run(Command::MaximizePane);
        assert_eq!(app.maximized, Some(Maximized::Pane));
        app.run(Command::MaximizePane);
        assert_eq!(app.maximized, None);
    }

    #[test]
    fn opening_what_a_maximized_part_hides_restores_first() {
        let (_dir, mut app) = file_and_preview();
        app.run(Command::MaximizeEditor);
        // The Comments icon at the top right needs the pane, so the editor gives way.
        let comments = app.pane_actions();
        let (cell, ..) = comments
            .iter()
            .find(|(_, _, command, _)| *command == Command::ShowComments)
            .unwrap();
        click(&mut app, cell.x + 1, cell.y);
        assert_eq!(app.maximized, None);
        assert!(app.dock.showing(Front::Comments));
        assert!(app.dock_area().is_some());
        // The sidebar, asked for while the pane is maximized, brings everything back.
        app.run(Command::MaximizePane);
        app.handle(Event::Key(KeyEvent::ctrl('b')));
        assert_eq!(app.maximized, None);
        // Another view of the pane, while the pane is maximized, keeps it maximized.
        app.run(Command::MaximizePane);
        let area = app.dock_area().unwrap();
        let (_, _, preview) = app
            .dock
            .view_tabs_for_test(area)
            .into_iter()
            .find(|t| t.0 == Front::Pdf)
            .unwrap();
        click(&mut app, preview.x + 2, area.y);
        assert_eq!(app.maximized, Some(Maximized::Pane));
        assert!(app.dock.showing(Front::Pdf));
    }

    /// Three files open in tabs, in a window with room to split.
    fn three_tabs(pixels: bool) -> (TempDir, App) {
        let dir = tempfile::tempdir().unwrap();
        let mut app = App::new(dir.path().to_path_buf(), (150, 44));
        if pixels {
            app.set_measure(Rc::new(cellix_render::FontMeasure::new(10, 20)));
        }
        for name in ["alpha.txt", "beta.txt", "gamma.txt"] {
            fs::write(dir.path().join(name), format!("{name} words\n")).unwrap();
            app.open(&dir.path().join(name)).unwrap();
        }
        (dir, app)
    }

    fn titles(app: &App) -> Vec<String> {
        app.editors.iter().map(Editor::title).collect()
    }

    fn status_line(app: &App) -> String {
        screen(app).row_text(app.size().1 - 1)
    }

    fn washes(app: &App) -> Vec<cellix_render::Bounds> {
        let grid = screen(app);
        let found = grid.paint.iter().filter_map(|p| match p {
            Paint::Wash { bounds, .. } => Some(*bounds),
            _ => None,
        });
        found.collect()
    }

    #[test]
    fn dragging_an_editor_tab_reorders_with_a_bar_and_says_so() {
        let (_dir, mut app) = three_tabs(true);
        let y = app.tabs_area().y;
        let tabs = app.visible_tabs();
        let (gamma, alpha) = (tabs[2].1 + 2, tabs[0].1 + 1);
        // A click stays a click.
        click(&mut app, tabs[1].1 + 2, y);
        assert_eq!(titles(&app), ["alpha.txt", "beta.txt", "gamma.txt"]);
        assert_eq!(app.active(), 1);
        app.handle(mouse(MouseKind::Left, gamma, y));
        app.handle(mouse(MouseKind::Drag, alpha, y));
        assert!(status_line(&app).contains("Drop to move the tab here"));
        let grid = screen(&app);
        let bar = grid.paint.iter().any(|p| {
            matches!(p, Paint::Panel { bounds, fill: Some(fill), .. }
                if *fill == app.theme.accent && bounds.w < bounds.h && bounds.y / 64 == i32::from(y))
        });
        assert!(bar, "an accent bar where it would go");
        let ghost = grid.paint.iter().any(|p| {
            matches!(p, Paint::Panel { fill: Some(fill), shadow: true, .. } if *fill == app.theme.tint)
        });
        assert!(ghost, "the tab follows the pointer");
        app.handle(mouse(MouseKind::Up, alpha, y));
        assert_eq!(titles(&app), ["gamma.txt", "alpha.txt", "beta.txt"]);
        assert_eq!(app.editors[app.active()].title(), "gamma.txt");
        assert!(!status_line(&app).contains("Drop"));
    }

    #[test]
    fn a_tab_drag_works_on_the_cell_tier() {
        let (_dir, mut app) = three_tabs(false);
        let y = app.tabs_area().y;
        let tabs = app.visible_tabs();
        let (alpha, end) = (tabs[0].1 + 1, tabs[2].1 + tabs[2].2 - 3);
        app.handle(mouse(MouseKind::Left, alpha, y));
        app.handle(mouse(MouseKind::Drag, end, y));
        let grid = screen(&app);
        assert!(grid.row_text(y).contains('▏'), "{}", grid.row_text(y));
        assert!(status_line(&app).contains("Drop to move the tab here"));
        app.handle(mouse(MouseKind::Up, end, y));
        assert_eq!(titles(&app), ["beta.txt", "gamma.txt", "alpha.txt"]);
    }

    #[test]
    fn dropping_a_tab_beside_a_pane_splits_and_moves_it() {
        let (_dir, mut app) = three_tabs(true);
        let y = app.tabs_area().y;
        let area = app.pane_area(0);
        let (_, gamma, _) = app.visible_tabs()[2];
        app.handle(mouse(MouseKind::Left, gamma + 2, y));
        let right = (area.x + area.w * 3 / 4, area.y + 10);
        app.handle(mouse(MouseKind::Drag, right.0, right.1));
        let said = status_line(&app);
        assert!(
            said.contains("Drop to split right · on the tab bar to move it there"),
            "{said}"
        );
        let wash = washes(&app);
        assert_eq!(wash.len(), 1);
        assert!(
            wash[0].x / 64 >= i32::from(area.x + area.w / 2),
            "on the right half"
        );
        app.handle(mouse(MouseKind::Up, right.0, right.1));
        // gamma moves to a new pane on the right, and the first shows its neighbour.
        assert_eq!(app.panes, [1, 2]);
        assert_eq!(app.focus, 1);
        // Dropped in the middle of another pane, a tab shows there instead. The press shows
        // it in the focused pane first, as a click does, and the drop moves it on.
        let (_, alpha, _) = app.visible_tabs()[0];
        app.handle(mouse(MouseKind::Left, alpha + 2, y));
        assert_eq!(app.panes, [1, 0]);
        let middle = app.pane_area(0);
        let (x, y2) = (middle.x + middle.w / 2, middle.y + 5);
        app.handle(mouse(MouseKind::Drag, x, y2));
        assert!(status_line(&app).contains("Drop to show it in this pane"));
        app.handle(mouse(MouseKind::Up, x, y2));
        assert_eq!((app.panes.clone(), app.focus), (vec![0, 1], 0));
        // On the left half of a pane, it splits to the left.
        let (_, beta, _) = app.visible_tabs()[1];
        app.handle(mouse(MouseKind::Left, beta + 2, y));
        let first = app.pane_area(0);
        app.handle(mouse(MouseKind::Drag, first.x + 3, first.y + 5));
        assert!(status_line(&app).contains("Drop to split left"));
        app.handle(mouse(MouseKind::Up, first.x + 3, first.y + 5));
        assert_eq!((app.panes.len(), app.panes[0], app.focus), (3, 1, 0));
    }

    #[test]
    fn escape_cancels_a_tab_drag() {
        let (_dir, mut app) = three_tabs(true);
        let y = app.tabs_area().y;
        let (_, gamma, _) = app.visible_tabs()[2];
        let area = app.pane_area(0);
        app.handle(mouse(MouseKind::Left, gamma + 2, y));
        app.handle(mouse(MouseKind::Drag, area.x + area.w - 5, area.y + 5));
        app.handle(Event::Key(KeyEvent::plain(Key::Esc)));
        assert!(washes(&app).is_empty());
        assert!(!status_line(&app).contains("Drop"));
        app.handle(mouse(MouseKind::Up, area.x + area.w - 5, area.y + 5));
        assert_eq!(app.panes.len(), 1);
        assert_eq!(titles(&app), ["alpha.txt", "beta.txt", "gamma.txt"]);
    }

    #[test]
    fn dragging_a_pane_tab_docks_the_pane_under_the_file_and_back() {
        let (_dir, mut app) = file_and_preview();
        let area = app.dock_area().unwrap();
        assert!(!app.dock_under());
        let (_, _, tab) = app.dock.view_tabs_for_test(area)[0];
        app.handle(mouse(MouseKind::Left, tab.x + 2, area.y));
        let main = app.main_area();
        let low = (main.x + main.w / 2, main.bottom() - 4);
        app.handle(mouse(MouseKind::Drag, low.0, low.1));
        let said = status_line(&app);
        assert!(
            said.contains("Drop to put the pane under the file"),
            "{said}"
        );
        let wash = washes(&app);
        assert_eq!(wash.len(), 1);
        assert!(
            wash[0].y / 64 >= i32::from(main.y + main.h / 2),
            "over the bottom half"
        );
        app.handle(mouse(MouseKind::Up, low.0, low.1));
        assert!(app.dock_under());
        let under = app.dock_area().unwrap();
        assert_eq!((under.x, under.right()), (app.main_left(), 150));
        assert!(under.y > 10);
        // It stays there, even with room beside the file, until it's dragged to the edge.
        app.handle(Event::Resize(160, 44));
        assert!(app.dock_under());
        let (_, _, tab) = app.dock.view_tabs_for_test(under)[0];
        app.handle(mouse(MouseKind::Left, tab.x + 2, under.y));
        app.handle(mouse(MouseKind::Drag, 158, 12));
        assert!(status_line(&app).contains("Drop to put the pane back on the right"));
        app.handle(mouse(MouseKind::Up, 158, 12));
        assert!(!app.dock_under());
        assert_eq!(app.dock_area().unwrap().y, 0);
    }

    #[test]
    fn pane_tabs_reorder_by_dragging() {
        let (_dir, mut app) = file_and_preview();
        app.dock.show_comments();
        let area = app.dock_area().unwrap();
        let tabs = app.dock.view_tabs_for_test(area);
        assert_eq!((tabs[0].0, tabs[1].0), (Front::Comments, Front::Pdf));
        let (from, to) = (tabs[1].2.x + 2, tabs[0].2.x + 1);
        app.handle(mouse(MouseKind::Left, from, area.y));
        app.handle(mouse(MouseKind::Drag, to, area.y));
        assert!(status_line(&app).contains("Drop to move the tab here"));
        app.handle(mouse(MouseKind::Up, to, area.y));
        let tabs = app.dock.view_tabs_for_test(area);
        assert_eq!((tabs[0].0, tabs[1].0), (Front::Pdf, Front::Comments));
    }

    #[test]
    fn a_picture_in_the_pane_takes_a_comment() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = App::new(dir.path().to_path_buf(), (150, 44));
        let image = image::RgbaImage::from_pixel(80, 80, image::Rgba([1, 2, 3, 255]));
        image.save(dir.path().join("loss.png")).unwrap();
        app.set_measure(Rc::new(cellix_render::FontMeasure::new(10, 20)));
        app.open(&dir.path().join("loss.png")).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while app.dock.pdf.as_ref().unwrap().pages_ready() == 0 {
            assert!(std::time::Instant::now() < deadline);
            app.poll();
            let _ = screen(&app);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let picture = pictures(&app)[0];
        let (x, y) = (
            ((picture.x + 64) / 64) as u16,
            ((picture.y + 64) / 64) as u16,
        );
        app.handle(mouse(MouseKind::Right, x, y));
        assert_eq!(app.dock.draft.as_ref().unwrap().quote, "Image loss.png");
    }

    #[test]
    fn a_pdf_takes_a_comment_on_its_page() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("paper.pdf"), cellix_doc::pdf::sample()).unwrap();
        let mut app = App::new(dir.path().to_path_buf(), (150, 44));
        app.set_measure(Rc::new(cellix_render::FontMeasure::new(10, 20)));
        app.open(&dir.path().join("paper.pdf")).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while app.dock.pdf.as_ref().unwrap().pages_ready() == 0 {
            assert!(std::time::Instant::now() < deadline);
            app.poll();
            let _ = screen(&app);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let picture = pictures(&app)[0];
        let (x, y) = (
            ((picture.x + 64) / 64) as u16,
            ((picture.y + 64) / 64) as u16,
        );
        app.handle(mouse(MouseKind::Move, x, y));
        let chip = app.view_comment().expect("a button on the page");
        app.handle(mouse(MouseKind::Left, chip.x + 1, chip.y));
        assert_eq!(
            app.dock.draft.as_ref().unwrap().quote,
            "Page 1 of paper.pdf"
        );
    }

    #[test]
    fn a_picture_can_be_commented_on() {
        let (_dir, mut app) = picture_page("![Plot](plot.png)\n\nwords\n");
        let _ = screen(&app);
        let picture = pictures(&app)[0];
        let (x, y) = (
            ((picture.x + picture.w / 2) / 64) as u16,
            (picture.y / 64) as u16,
        );
        app.handle(mouse(MouseKind::Move, x, y));
        let (chip, _) = app.picture_comment().expect("a button on the picture");
        app.handle(mouse(MouseKind::Move, chip.x + 1, chip.y));
        app.handle(mouse(MouseKind::Left, chip.x + 1, chip.y));
        typed(&mut app, "Axis is wrong");
        enter(&mut app);
        assert_eq!(app.threads[0].quote, "Image plot.png");
        assert_eq!(app.threads[0].line, 1);
        assert!(app.threads[0].message().contains("Image plot.png"));
    }

    #[test]
    fn a_diagram_says_what_it_is_when_commented_on() {
        let (_dir, mut app) = picture_page(FLOW);
        let _ = screen(&app);
        let picture = pictures(&app)[0];
        let (x, y) = (
            ((picture.x + picture.w / 2) / 64) as u16,
            (picture.y / 64) as u16 + 1,
        );
        app.handle(mouse(MouseKind::Move, x, y));
        let (chip, line) = app.picture_comment().expect("a button on the diagram");
        assert_eq!(line, 2);
        app.handle(mouse(MouseKind::Left, chip.x + 1, chip.y));
        assert!(
            app.dock
                .draft
                .as_ref()
                .unwrap()
                .quote
                .contains("flowchart LR")
        );
    }

    #[test]
    fn the_sample_document_draws_everything_it_shows_off() {
        let samples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../samples");
        let mut app = App::new(samples.clone(), (150, 44));
        app.set_measure(Rc::new(cellix_render::FontMeasure::new(10, 20)));
        app.built_folder = tempfile::tempdir().unwrap().path().to_path_buf();
        app.open(&samples.join("writing.md")).unwrap();
        let rows = app.editors[0].page_rows(app.focused_area());
        let shown = rows.iter().filter(|r| r.figure.is_some()).count();
        assert!(shown >= 15, "pictures, diagrams and formulas: {shown}");
        let words: String = rows
            .iter()
            .flat_map(|r| r.glyphs.iter().map(|g| g.ch))
            .collect();
        for hidden in ["```mermaid", "[!NOTE]", "{++", "[^source]", "{width="] {
            assert!(!words.contains(hidden), "{hidden} shows as source");
        }
    }

    #[test]
    fn an_svg_image_reaches_a_pdf_as_a_picture() {
        let dir = tempfile::tempdir().unwrap();
        let svg = "<svg xmlns='http://www.w3.org/2000/svg' width='10' height='10'><rect width='10' height='10'/></svg>";
        fs::write(dir.path().join("mark.svg"), svg).unwrap();
        let mut made = vec![dir.path().join("out.tex")];
        let text = svg_as_pictures(
            "![Mark](mark.svg){width=50%}\n\nwords\n",
            dir.path(),
            &mut made,
        );
        assert_eq!(text, "![Mark](.diagram-1.png){width=50%}\n\nwords\n");
        assert!(made[1].is_file());
    }

    #[test]
    fn deleting_a_word_with_tracking_on_crosses_it_out() {
        let (_dir, mut app) = tracking_page("one two\n");
        let (x, y) = cell_of(&app, "two");
        app.handle(mouse(MouseKind::Left, x + 3, y));
        app.run(Command::DeleteWordLeft);
        assert_eq!(app.active_text().unwrap(), "one {--two--}\n");
    }

    #[test]
    fn cutting_with_tracking_on_keeps_the_words_as_a_suggestion() {
        let (_dir, mut app) = tracking_page("one two three\n");
        let (x, y) = cell_of(&app, "two");
        double_click(&mut app, x + 1, y);
        app.run(Command::Cut);
        assert_eq!(app.active_text().unwrap(), "one {--two--} three\n");
        assert_eq!(app.clipboard, "two");
    }

    #[test]
    fn a_page_counts_words() {
        let (_dir, app) = page_app("one two three\n");
        assert!(screen(&app).text().contains("3 words"));
    }

    #[test]
    fn a_comment_never_reaches_a_shell() {
        let (dir, mut app) = page_app("line\n");
        comment_on(&mut app, "Odd; touch PWNED");
        let root = app.workspace.root().to_path_buf();
        let shell = ["sh".to_string()];
        app.start_session(crate::agents::SHELL.0, &shell, &[], &root);
        app.dock.front = Front::Comments;
        app.thread_hit(ThreadHit::Ask(app.threads[0].id));
        assert!(screen(&app).text().contains("Pick an agent"));
        std::thread::sleep(std::time::Duration::from_millis(300));
        assert!(!dir.path().join("PWNED").exists(), "it ran as a command");
    }

    #[test]
    fn a_comment_follows_its_text() {
        let (_dir, mut app) = page_app("one\n\nthe cache goes first\n");
        let (x, y) = find(&screen(&app), "the cache");
        app.handle(mouse(MouseKind::Left, x + 1, y));
        comment_on(&mut app, "Why");
        assert_eq!(app.threads[0].line, 3);
        app.run(Command::Move(cellix_input::Direction::Top, false));
        enter(&mut app);
        enter(&mut app);
        assert_eq!(app.threads[0].line, 5);
    }

    #[test]
    fn a_link_asks_for_its_address() {
        let (_dir, mut app) = page_app("word tail\n");
        click_tool(&mut app, "Link");
        typed(&mut app, "https://a.b");
        enter(&mut app);
        assert_eq!(
            app.active_text().as_deref(),
            Some("[word](https://a.b) tail\n")
        );
    }

    #[test]
    fn ctrl_b_is_bold_on_words_picked_on_a_page() {
        let (_dir, mut app) = page_app("word tail\n");
        app.run(Command::Move(cellix_input::Direction::WordRight, true));
        app.handle(Event::Key(KeyEvent::ctrl('b')));
        assert_eq!(app.active_text().as_deref(), Some("**word** tail\n"));
    }

    #[test]
    fn ctrl_b_closes_and_opens_the_left_side_with_nothing_picked() {
        let (_dir, mut app) = page_app("word tail\n");
        assert!(app.sidebar_area().is_some());
        app.handle(Event::Key(KeyEvent::ctrl('b')));
        assert!(app.sidebar_area().is_none());
        assert_eq!(app.active_text().as_deref(), Some("word tail\n"), "no bold");
        app.handle(Event::Key(KeyEvent::ctrl('b')));
        assert!(app.sidebar_area().is_some());
    }

    #[test]
    fn a_click_on_the_lit_rail_icon_closes_the_left_side() {
        for (row, (_, command)) in ACTIVITY.iter().copied().enumerate().take(4) {
            let (_dir, mut app) = page_app("# Title\n\nword tail\n");
            let area = app.activity_area();
            let y = app.rail_row(row, area);
            if !app.rail_on(command) {
                app.handle(mouse(MouseKind::Left, 2, y));
            }
            assert!(app.rail_on(command), "{command:?} opens");
            app.handle(mouse(MouseKind::Left, 2, y));
            assert!(app.sidebar_area().is_none(), "{command:?} closes");
            app.handle(mouse(MouseKind::Left, 2, y));
            assert!(app.rail_on(command), "{command:?} opens again");
            assert_eq!(app.active_text().as_deref(), Some("# Title\n\nword tail\n"));
        }
    }

    #[test]
    fn a_click_on_git_in_full_review_closes_it() {
        let (_dir, mut app) = reviewed();
        app.run(Command::OpenDiff);
        let area = app.activity_area();
        app.handle(mouse(MouseKind::Left, 2, app.rail_row(3, area)));
        assert!(app.review.is_none());
    }

    #[test]
    fn hiding_the_panel_gives_a_diff_beside_it_its_own_list_back() {
        let (_dir, mut app) = source_app();
        let (x, y) = find(&screen(&app), "kept.txt");
        app.handle(mouse(MouseKind::Left, x, y));
        assert!(app.review.as_ref().unwrap().beside_panel);
        app.handle(Event::Key(KeyEvent::ctrl('b')));
        assert!(!app.review.as_ref().unwrap().beside_panel);
    }

    /// A data file open on its page.
    fn csv_app() -> (TempDir, App) {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("data.csv");
        fs::write(&file, "a,b\n1,2\n").unwrap();
        let mut app = App::new(dir.path().to_path_buf(), (120, 30));
        app.open(&file).unwrap();
        (dir, app)
    }

    #[test]
    fn a_csv_goes_to_its_source_and_comes_back() {
        let (_dir, mut app) = csv_app();
        assert!(app.editors[0].rendered);
        let (x, y) = find(&screen(&app), "Show Source");
        app.handle(mouse(MouseKind::Left, x + 1, y));
        assert!(!app.editors[0].rendered);
        let (x, y) = find(&screen(&app), "Show Page");
        app.handle(mouse(MouseKind::Left, x + 1, y));
        assert!(app.editors[0].rendered);
    }

    #[test]
    fn the_source_of_a_page_offers_the_way_back_on_a_right_click() {
        let (_dir, mut app) = csv_app();
        app.run(Command::MarkdownPreview);
        let names: Vec<String> = app
            .editor_items()
            .iter()
            .map(|i| i.label.to_string())
            .collect();
        assert!(names.iter().any(|name| name == "Show Page"), "{names:?}");
    }

    #[test]
    fn plain_code_has_no_page_to_go_to() {
        let (_dir, app) = project();
        assert!(app.page_switch().is_none());
    }

    #[test]
    fn prose_has_no_code_completion() {
        let (_dir, mut app) = page_app("somewhere\n\n");
        app.run(Command::Move(cellix_input::Direction::Bottom, false));
        typed(&mut app, "som");
        assert!(app.suggest.is_none());
    }

    #[test]
    fn a_new_comment_reaches_the_agent_once() {
        let (_dir, mut app) = page_app("line\n");
        comment_on(&mut app, "Note");
        let take = |app: &mut App| {
            let (sender, answer) = std::sync::mpsc::channel();
            app.serve(Request::TakeThreads(sender));
            answer.recv().unwrap()
        };
        assert!(take(&mut app).contains("Note"));
        assert_eq!(take(&mut app), "[]", "only once");
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
        assert!(screen(&app).text().contains("No comments yet"));
    }

    #[test]
    fn asking_with_no_agent_goes_to_the_one_you_start() {
        let (dir, mut app) = page_app("line\n");
        comment_on(&mut app, "Note for later");
        let (x, y) = find(&screen(&app), "Ask agent");
        app.handle(mouse(MouseKind::Left, x, y));
        assert!(screen(&app).text().contains("Pick an agent"));
        let script = "printf ready; cat > heard.txt";
        let argv = ["sh".to_string(), "-c".to_string(), script.to_string()];
        let root = app.workspace.root().to_path_buf();
        app.start_session("Stand-in", &argv, &[], &root);
        let log = dir.path().join("heard.txt");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(8);
        while !fs::read_to_string(&log)
            .unwrap_or_default()
            .contains("Note for later")
        {
            assert!(std::time::Instant::now() < deadline, "it never went");
            app.poll();
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }

    #[test]
    fn asking_when_an_agent_works_outside_the_pane_leaves_it_for_its_next_prompt() {
        let (_dir, mut app) = page_app("line\n");
        app.serve(Request::Agent(agent(
            cellix_agent::AgentState::Working,
            "Working",
        )));
        comment_on(&mut app, "Note");
        let (x, y) = find(&screen(&app), "Ask agent");
        app.handle(mouse(MouseKind::Left, x, y));
        assert_eq!(app.dock.front, Front::Comments, "it stays with the comment");
        assert!(app.threads[0].asked);
        let text = screen(&app).text();
        assert!(text.contains("sees this with its next prompt"), "{text}");
    }

    /// Runs a shell in the pane, with the keyboard left on the file.
    fn with_a_shell(app: &mut App) {
        let sh = ["sh".to_string()];
        let root = app.workspace.root().to_path_buf();
        app.start_session(crate::agents::SHELL.0, &sh, &[], &root);
        app.dock.focused = false;
    }

    /// Clicks an icon at the top right.
    fn press_action(app: &mut App, wanted: Command) {
        let actions = app.pane_actions();
        let found = actions.iter().find(|(_, _, command, _)| *command == wanted);
        let (cell, ..) = found.expect("the icon is there");
        app.handle(mouse(MouseKind::Left, cell.x + 1, cell.y));
    }

    fn ctrl_j(app: &mut App) {
        app.handle(Event::Key(KeyEvent {
            key: Key::Char('j'),
            mods: Mods {
                ctrl: true,
                ..Mods::default()
            },
        }));
    }

    #[test]
    fn the_icons_at_the_top_right_are_there_with_the_pane_shut() {
        let (_dir, mut app) = project();
        assert!(!app.dock.open);
        let right = app.size.0 - PANE_CLOSE - ACTION_MARGIN;
        assert_eq!(app.pane_actions().last().unwrap().0.right(), right);
        assert!(
            app.tabs_area().right() <= app.pane_actions()[0].0.x,
            "tabs stop short"
        );
        press_action(&mut app, Command::ShowComments);
        assert!(app.dock.showing(Front::Comments));
        press_action(&mut app, Command::ShowComments);
        assert!(!app.dock.open);
    }

    #[test]
    fn the_rail_has_no_agents_icon_and_the_strip_shows_when_one_needs_you() {
        assert!(ACTIVITY.iter().all(|(icon, _)| *icon != Icon::Agents));
        let names = PANE_ACTIONS.iter().map(|(icon, ..)| *icon);
        assert!(names.collect::<Vec<_>>().contains(&Icon::Agents));
    }

    /// Where the icons stand, which every test here compares.
    fn action_spots(app: &App) -> Vec<Rect> {
        app.pane_actions().iter().map(|(cell, ..)| *cell).collect()
    }

    /// The icons that were really drawn.
    fn drawn_icons(app: &App) -> Vec<Icon> {
        let paint = screen(app).paint;
        let icons = paint.iter().filter_map(|p| match p {
            cellix_render::Paint::Icon { icon, .. } => Some(*icon),
            _ => None,
        });
        icons.collect()
    }

    #[test]
    fn the_icons_stay_put_whatever_the_pane_does() {
        for size in [(110, 30), (150, 44)] {
            let (dir, _) = project();
            let mut app = App::new(dir.path().to_path_buf(), size);
            let shut = action_spots(&app);
            app.run(Command::ListAgents);
            assert_eq!(action_spots(&app), shut, "with the pane beside the file");
        }
        let (dir, _) = project();
        let mut app = App::new(dir.path().to_path_buf(), (72, 40));
        let shut = action_spots(&app);
        app.run(Command::ListAgents);
        assert!(app.dock_under());
        assert_eq!(action_spots(&app), shut, "with the pane under the file");
    }

    #[test]
    fn the_icons_stay_in_review_and_settings() {
        let (_dir, mut app) = reviewed();
        app.run(Command::OpenDiff);
        assert!(drawn_icons(&app).contains(&Icon::Terminal), "in Review");
        app.run(Command::OpenDiff);
        app.run(Command::OpenSettings);
        assert!(drawn_icons(&app).contains(&Icon::Terminal), "in Settings");
    }

    #[test]
    fn the_rail_only_lights_what_is_showing() {
        let (dir, mut app) = reviewed();
        assert!(app.rail_on(Command::ToggleSidebar));
        app.run(Command::OpenDiff);
        assert!(
            app.rail_on(Command::ShowSourceControl),
            "Review is Source Control"
        );
        assert!(!app.rail_on(Command::ToggleSidebar));
        let mut narrow = App::new(dir.path().to_path_buf(), (72, 40));
        narrow.run(Command::ListAgents);
        assert!(narrow.sidebar_area().is_none());
        assert!(
            !narrow.rail_on(Command::ToggleSidebar),
            "no sidebar, so no light"
        );
    }

    #[test]
    fn a_review_keeps_its_buttons_clear_of_the_icons() {
        let (_dir, mut app) = reviewed();
        app.run(Command::OpenDiff);
        let _ = screen(&app);
        let strip = ACTION_WIDTH * PANE_ACTIONS.len() as u16 + PANE_CLOSE + ACTION_MARGIN;
        assert_eq!(app.actions_over_main(), strip);
    }

    #[test]
    fn tabs_in_the_pane_share_the_row_with_the_icons() {
        let (dir, _) = project();
        let mut app = App::new(dir.path().to_path_buf(), (170, 40));
        let root = app.workspace.root().to_path_buf();
        let cat = ["cat".to_string()];
        for name in ["Claude Code", "Codex", "Gemini CLI", "Another Agent"] {
            app.start_session(name, &cat, &[], &root);
        }
        let _ = screen(&app);
        let area = app.dock_area().unwrap();
        let first = app.pane_actions()[0].0.x;
        assert!(first > area.x, "the icons are in the pane's row");
        assert!(
            app.dock.new_button_for_test(area) < first,
            "nothing falls under them"
        );
    }

    #[test]
    fn every_tab_in_a_narrow_pane_keeps_its_close_button_clear_of_the_icons() {
        let (_dir, mut app) = project();
        with_a_shell(&mut app);
        app.run(Command::ShowComments);
        app.run(Command::ListAgents);
        let _ = screen(&app);
        let area = app.dock_area().unwrap();
        let first = app.pane_actions()[0].0.x;
        let tabs = app.dock.view_tabs_for_test(area);
        assert!(
            tabs.iter().all(|(_, _, tab)| tab.right() <= first),
            "{tabs:?}"
        );
    }

    #[test]
    fn the_line_above_a_stacked_pane_can_be_dragged() {
        let (dir, _) = project();
        let mut app = App::new(dir.path().to_path_buf(), (72, 40));
        app.run(Command::ListAgents);
        let pane = app.dock_area().unwrap();
        app.handle(mouse(MouseKind::Left, 30, pane.y - 1));
        app.handle(mouse(MouseKind::Drag, 30, 10));
        app.handle(mouse(MouseKind::Up, 30, 10));
        let pane = app.dock_area().unwrap();
        assert_eq!((pane.y, app.main_area().bottom()), (10, 10));
        app.hover = Some((30, pane.y - 1));
        assert_eq!(app.pointer(), "ns-resize");
    }

    #[test]
    fn a_stacked_pane_leaves_both_sides_some_room() {
        let (dir, _) = project();
        let mut app = App::new(dir.path().to_path_buf(), (72, 40));
        app.run(Command::ListAgents);
        app.dock.height = Some(500);
        assert_eq!(app.main_area().h, STACK_LEAST);
        app.dock.height = Some(1);
        assert_eq!(app.dock_area().unwrap().h, STACK_LEAST);
    }

    #[test]
    fn the_tab_in_front_keeps_its_name_and_the_others_say_theirs_on_hover() {
        let (_dir, mut app) = project();
        with_a_shell(&mut app);
        app.run(Command::ShowComments);
        app.run(Command::ListAgents);
        let _ = screen(&app);
        let area = app.dock_area().unwrap();
        let tabs = app.dock.view_tabs_for_test(area);
        let agents = tabs.iter().find(|tab| tab.0 == Front::Agents).unwrap();
        assert_eq!(agents.2.w, 11, "it's in front");
        let comments = tabs.iter().find(|tab| tab.0 == Front::Comments).unwrap();
        assert!(comments.2.w < 13, "the rest give way");
        let name = app.dock.cut_name(area, comments.2.x + 1);
        assert_eq!(name.as_deref(), Some("Comments"));
        assert_eq!(
            app.dock.cut_name(area, agents.2.x + 1),
            None,
            "it shows in full"
        );
    }

    #[test]
    fn a_stacked_panes_tabs_line_up_with_the_files() {
        let (dir, _) = project();
        let mut app = App::new(dir.path().to_path_buf(), (72, 40));
        app.run(Command::ListAgents);
        let _ = screen(&app);
        let pane = app.dock_area().unwrap();
        let (_, _, tab) = app.dock.view_tabs_for_test(pane)[0];
        assert_eq!(tab.x, app.main_area().x);
    }

    #[test]
    fn tiles_are_cut_at_the_edges_of_what_scrolls() {
        let (dir, mut app) = project();
        app.open(&dir.path().join("alpha.txt")).unwrap();
        let grid = screen(&app);
        let text = app.editors[0].scroll_region(app.pane_area(0));
        assert!(grid.bands.contains(&text.x) && grid.bands.contains(&text.right()));
    }

    #[test]
    fn an_export_builds_under_a_name_tex_can_write_and_lands_beside_the_document() {
        let (tex, pdf) = export_names(Path::new("/p/rfc.md"));
        assert_eq!(tex, Path::new("/p/rfc.cellix-export.tex"));
        assert!(
            !tex.file_name().unwrap().to_string_lossy().starts_with('.'),
            "pdflatex won't write a log for a dot file"
        );
        assert_eq!(pdf, Path::new("/p/rfc.pdf"));
    }

    #[test]
    fn exporting_suggestions_asks_how() {
        let (_dir, mut app) = page_app("Keep {++this++} here.\n");
        app.run(Command::ExportPdf);
        let text = screen(&app).text();
        assert!(text.contains("This document has suggestions"), "{text}");
        assert!(text.contains("Accept All in the PDF"), "{text}");
    }

    #[test]
    fn a_tiny_window_goes_without_the_icons() {
        let (dir, _) = project();
        let app = App::new(dir.path().to_path_buf(), (40, 30));
        assert!(app.pane_actions().is_empty());
    }

    #[test]
    fn the_icons_stand_clear_of_the_panes_close_button() {
        let (dir, _) = project();
        let mut app = App::new(dir.path().to_path_buf(), (170, 40));
        app.run(Command::ListAgents);
        let area = app.dock_area().unwrap();
        assert_eq!(
            app.pane_actions().last().unwrap().0.right(),
            area.right() - PANE_CLOSE - ACTION_MARGIN
        );
        app.handle(mouse(MouseKind::Left, area.right() - 2, area.y));
        assert!(!app.dock.open, "the close button still closes");
    }

    #[test]
    fn the_terminal_icon_starts_one_and_then_finds_it() {
        let (_dir, mut app) = project();
        press_action(&mut app, Command::ToggleTerminal);
        assert_eq!(app.dock.sessions.len(), 1);
        assert!(app.dock.session().unwrap().shell && app.dock.focused);
        app.dock.focused = false;
        app.dock.front = Front::Comments;
        press_action(&mut app, Command::ToggleTerminal);
        assert_eq!(app.dock.sessions.len(), 1, "it doesn't start a second");
        assert!(app.dock.showing(Front::Session) && app.dock.focused);
        assert!(app.action_on(Command::ToggleTerminal));
    }

    #[test]
    fn ctrl_j_brings_a_terminal_up_and_puts_it_away() {
        let (_dir, mut app) = project();
        ctrl_j(&mut app);
        assert!(app.dock.open && app.dock.focused);
        ctrl_j(&mut app);
        assert!(!app.dock.open, "typing in the terminal didn't swallow it");
        ctrl_j(&mut app);
        assert!(app.dock.open);
        assert_eq!(app.dock.sessions.len(), 1, "the same terminal comes back");
    }

    #[test]
    fn ctrl_j_stays_a_new_line_for_an_agent() {
        let (_dir, mut app) = project();
        let cat = ["cat".to_string()];
        let root = app.workspace.root().to_path_buf();
        app.start_session("Claude Code", &cat, &[], &root);
        ctrl_j(&mut app);
        assert!(app.dock.focused && app.dock.open);
        assert_eq!(app.dock.sessions.len(), 1);
    }

    #[test]
    fn the_palette_reaches_the_terminal() {
        let names = crate::palette::COMMANDS.iter().map(|(name, _)| *name);
        let names: Vec<&str> = names.collect();
        assert!(names.contains(&"Toggle Terminal") && names.contains(&"New Terminal"));
    }

    #[test]
    fn an_empty_window_offers_a_terminal() {
        let (_dir, app) = project();
        assert!(screen(&app).text().contains("Open a Terminal"));
    }

    #[test]
    fn a_narrow_window_puts_the_pane_under_the_file() {
        let (dir, _) = project();
        let mut app = App::new(dir.path().to_path_buf(), (70, 40));
        app.show_sidebar = false;
        app.run(Command::ListAgents);
        let (main, pane) = (app.main_area(), app.dock_area().unwrap());
        assert_eq!(main.bottom(), pane.y, "one under the other");
        assert_eq!((main.w, pane.w), (pane.w, main.w), "both take the width");
        assert_eq!(pane.right(), 70);
        assert_eq!(pane.bottom(), 39, "down to the status bar");
        assert!(screen(&app).text().contains("START AN AGENT"));
    }

    #[test]
    fn a_session_under_the_file_is_told_its_real_size() {
        let (dir, _) = project();
        let mut app = App::new(dir.path().to_path_buf(), (70, 40));
        app.show_sidebar = false;
        with_a_shell(&mut app);
        let screen = app.dock_screen().unwrap();
        assert!(screen.h < 20 && screen.w > 60, "{screen:?}");
    }

    #[test]
    fn a_short_narrow_window_gives_the_pane_all_of_it() {
        let (dir, _) = project();
        let mut app = App::new(dir.path().to_path_buf(), (70, 14));
        app.show_sidebar = false;
        app.run(Command::ListAgents);
        assert_eq!(app.dock_area().unwrap().y, 0);
    }

    #[test]
    fn a_wide_window_keeps_the_pane_beside_the_file() {
        let (dir, _) = project();
        let mut app = App::new(dir.path().to_path_buf(), (150, 40));
        app.run(Command::ListAgents);
        let (main, pane) = (app.main_area(), app.dock_area().unwrap());
        assert_eq!((pane.y, main.right()), (0, pane.x));
    }

    #[test]
    fn the_plus_lists_what_can_start_however_the_pane_opened() {
        let (_dir, mut app) = project();
        app.dock.open = true;
        app.dock.front = Front::Comments;
        let area = app.dock_area().unwrap();
        let plus = app.dock.new_button_for_test(area);
        app.handle(mouse(MouseKind::Left, plus, area.y));
        assert!(app.dock.listing());
        assert!(screen(&app).text().contains("START AN AGENT"));
    }

    #[test]
    fn the_agents_list_is_a_tab_that_stays() {
        let (dir, _) = project();
        let mut app = App::new(dir.path().to_path_buf(), (170, 44));
        with_a_shell(&mut app);
        let area = app.dock_area().unwrap();
        let plus = app.dock.new_button_for_test(area);
        app.handle(mouse(MouseKind::Left, plus, area.y));
        assert!(
            app.dock.session().is_some(),
            "the shell keeps running behind it"
        );
        let tabs = app.dock.view_tabs_for_test(area);
        assert_eq!(tabs[0].0, Front::Agents);
        press_action(&mut app, Command::ShowComments);
        assert_eq!(app.dock.front, Front::Comments);
        let _ = screen(&app);
        let tabs = app.dock.view_tabs_for_test(area);
        assert_eq!(tabs[1].0, Front::Agents, "it's still there");
        let agents = tabs[1].2;
        app.handle(mouse(MouseKind::Left, agents.x + 1, area.y));
        assert!(app.dock.listing());
        assert!(screen(&app).text().contains("START AN AGENT"));
        app.handle(mouse(MouseKind::Left, agents.right() - 2, area.y));
        assert!(!app.dock.agents_tab);
        assert_eq!(app.dock.front, Front::Session);
    }

    #[test]
    fn starting_from_the_list_puts_the_agent_in_its_place() {
        let (_dir, mut app) = project();
        app.dock.open = true;
        app.dock.show_list();
        with_a_shell(&mut app);
        assert!(!app.dock.agents_tab);
        assert!(!app.dock.listing());
    }

    #[test]
    fn the_pane_holds_only_what_was_opened_and_closes_with_the_last_of_it() {
        let (_dir, mut app) = project();
        press_action(&mut app, Command::ToggleTerminal);
        let area = app.dock_area().unwrap();
        let row = screen(&app).row_text(area.y);
        assert!(
            row.contains("Terminal") && !row.contains("Comments"),
            "{row}"
        );
        assert!(!row.contains("Agents"), "{row}");
        let (x, _) = find(&screen(&app), "Terminal");
        app.handle(mouse(MouseKind::Middle, x, area.y));
        assert!(app.dock_area().is_none(), "nothing took its place");
        assert!(!app.dock.agents_tab);
    }

    #[test]
    fn closing_one_of_two_tabs_goes_to_the_other() {
        let (_dir, mut app) = project();
        press_action(&mut app, Command::ToggleTerminal);
        press_action(&mut app, Command::ShowComments);
        let area = app.dock_area().unwrap();
        let (comments, ..) = app.dock.view_tabs_for_test(area)[0];
        assert_eq!(comments, Front::Comments);
        app.dock.close_view(Front::Comments);
        assert!(
            app.dock.showing(Front::Session),
            "the terminal is what's left"
        );
        app.dock.close(0);
        assert!(!app.dock.open);
    }

    #[test]
    fn the_comments_tab_closes_like_any_other() {
        let (_dir, mut app) = project();
        press_action(&mut app, Command::ShowComments);
        let area = app.dock_area().unwrap();
        let (_, _, tab) = app.dock.view_tabs_for_test(area)[0];
        app.handle(mouse(MouseKind::Left, tab.right() - 2, area.y));
        assert!(!app.dock.open, "it was the only tab");
    }

    #[test]
    fn a_middle_click_closes_a_tab_in_the_pane() {
        let (_dir, mut app) = project();
        with_a_shell(&mut app);
        app.dock.problems = true;
        let area = app.dock_area().unwrap();
        let (_, _, problems) = app.dock.view_tabs_for_test(area)[0];
        app.handle(mouse(MouseKind::Middle, problems.x + 1, area.y));
        assert!(!app.dock.problems);
        let (x, _) = find(&screen(&app), "Terminal");
        app.handle(mouse(MouseKind::Middle, x, area.y));
        assert!(app.dock.sessions.is_empty());
    }

    #[test]
    fn a_middle_click_on_a_busy_tab_asks_first() {
        let (_dir, mut app) = project();
        let sleeper = ["sleep".to_string(), "30".to_string()];
        let root = app.workspace.root().to_path_buf();
        app.start_session("Job", &sleeper, &[], &root);
        let area = app.dock_area().unwrap();
        let (x, _) = find(&screen(&app), "Job");
        app.handle(mouse(MouseKind::Middle, x, area.y));
        assert_eq!(app.dock.sessions.len(), 1);
        assert!(screen(&app).text().contains("Stop Job?"));
    }

    #[test]
    fn the_tabs_for_a_pdf_and_a_log_close() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("paper.pdf"), cellix_doc::pdf::sample()).unwrap();
        let mut app = App::new(dir.path().to_path_buf(), (150, 44));
        app.set_measure(Rc::new(cellix_render::FontMeasure::new(10, 20)));
        app.open(&dir.path().join("paper.pdf")).unwrap();
        app.dock.log = vec!["built".to_string()];
        let area = app.dock_area().unwrap();
        let (tab, x) = {
            let (front, _, tab) = app.dock.view_tabs_for_test(area)[0];
            (front, tab.right() - 2)
        };
        assert_eq!(tab, Front::Pdf);
        app.handle(mouse(MouseKind::Left, x, area.y));
        assert!(app.dock.pdf.is_none());
        let (_, _, log) = app.dock.view_tabs_for_test(area)[0];
        app.handle(mouse(MouseKind::Left, log.right() - 2, area.y));
        assert!(app.dock.log.is_empty());
    }

    #[test]
    fn the_count_of_problems_opens_them_and_a_row_goes_there() {
        let (dir, mut app) = project();
        let file = dir.path().join("alpha.txt");
        app.open(&file).unwrap();
        app.diagnostics.insert(
            file.clone(),
            vec![Diagnostic {
                line: 1,
                column: 0,
                message: "Something broke here".to_string(),
                severity: 1,
            }],
        );
        let (at, _) = app.status_problems().unwrap();
        let bottom = app.size.1 - 1;
        app.handle(mouse(MouseKind::Left, at, bottom));
        assert_eq!(app.dock.front, Front::Problems);
        let grid = screen(&app);
        let (x, y) = find(&grid, "Something broke");
        app.handle(mouse(MouseKind::Left, x, y));
        assert_eq!(app.cursor_line().unwrap().1, 2);
    }

    fn paper_app(text: &str) -> (TempDir, App) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("paper.tex");
        fs::write(&path, text).unwrap();
        let mut app = App::new(dir.path().to_path_buf(), (170, 40));
        // Stands in for a LaTeX engine: writes a real PDF, or fails as tectonic does.
        let sample = dir.path().join("sample.pdf");
        fs::write(&sample, cellix_doc::pdf::sample()).unwrap();
        let script = format!(
            "if grep -q bad \"$1\"; then echo 'error: paper.tex:2: Undefined control sequence'; exit 1; fi; cp '{}' \"${{1%.tex}}.pdf\"",
            sample.display()
        );
        app.settings.latex_engine = Some(vec![
            "sh".to_string(),
            "-c".to_string(),
            script,
            "engine".to_string(),
        ]);
        app.set_measure(Rc::new(cellix_render::FontMeasure::new(17, 37)));
        app.open(&path).unwrap();
        (dir, app)
    }

    fn built(app: &mut App) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while app.build.is_some() {
            assert!(
                std::time::Instant::now() < deadline,
                "the build never ended"
            );
            app.poll();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    #[test]
    fn a_paper_builds_into_a_pdf() {
        let (dir, mut app) = paper_app("\\begin{document}\nHello\n\\end{document}\n");
        click_tool(&mut app, "Build PDF");
        built(&mut app);
        assert!(dir.path().join("paper.pdf").exists());
        assert_eq!(app.dock.front, Front::Pdf);
        assert!(app.dock.pdf.is_some());
    }

    #[test]
    fn a_paper_an_agent_rewrites_rebuilds_its_open_pdf() {
        let (dir, mut app) = paper_app("\\begin{document}\nHello\n\\end{document}\n");
        app.run(Command::BuildPdf);
        built(&mut app);
        fs::remove_file(dir.path().join("paper.pdf")).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        fs::write(
            dir.path().join("paper.tex"),
            "\\begin{document}\nHi\n\\end{document}\n",
        )
        .unwrap();
        app.handle(Event::Tick);
        built(&mut app);
        assert!(dir.path().join("paper.pdf").exists(), "built again");
    }

    #[test]
    fn a_paper_split_over_input_files_counts_tracks_and_rebuilds_as_one() {
        let (dir, mut app) = paper_app(
            "\\documentclass{article}\n\\begin{document}\nOne two.\n\\input{sections/intro}\n\\end{document}\n",
        );
        let root = dir.path().join("paper.tex");
        fs::create_dir(dir.path().join("sections")).unwrap();
        let intro = dir.path().join("sections/intro.tex");
        fs::write(&intro, "Three four five.\n").unwrap();
        app.open(&intro).unwrap();
        let text = screen(&app).text();
        assert!(text.contains("5 words in the paper"), "{text}");
        typed(&mut app, "Zero ");
        let text = screen(&app).text();
        assert!(
            text.contains("6 words in the paper"),
            "unsaved words count: {text}"
        );
        app.run(Command::Save);
        app.open(&root).unwrap();
        let text = screen(&app).text();
        assert!(
            text.contains("6 words in the paper"),
            "from the root too: {text}"
        );

        // Tracking a file the paper takes in puts the package in the root's preamble.
        app.open(&intro).unwrap();
        app.run(Command::ToggleTracking);
        let on_disk = fs::read_to_string(&root).unwrap();
        assert!(
            on_disk.contains("\\usepackage{changes}\n\\begin{document}"),
            "{on_disk}"
        );
        assert!(!app.active_text().unwrap().contains("usepackage"));
        app.run(Command::ToggleTracking);

        // An agent rewriting that file rebuilds the paper's PDF with the root in front.
        app.open(&root).unwrap();
        app.run(Command::BuildPdf);
        built(&mut app);
        fs::remove_file(dir.path().join("paper.pdf")).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        fs::write(&intro, "Three four.\n").unwrap();
        app.handle(Event::Tick);
        built(&mut app);
        assert!(dir.path().join("paper.pdf").exists(), "built again");
        let text = screen(&app).text();
        assert!(text.contains("4 words in the paper"), "{text}");
    }

    #[test]
    fn a_section_belongs_to_the_root_that_really_takes_it_in() {
        let (dir, mut app) = paper_app(
            "\\documentclass{article}\n\\begin{document}\nOne two.\n\\input sections/intro\n\\end{document}\n",
        );
        let root = dir.path().join("paper.tex");
        // A whole document first by name, which mentions the section but doesn't take it in.
        let notes = dir.path().join("a-notes.tex");
        let notes_text =
            "\\documentclass{article}\n\\begin{document}\nThe intro is long.\n\\end{document}\n";
        fs::write(&notes, notes_text).unwrap();
        fs::create_dir(dir.path().join("sections")).unwrap();
        let intro = dir.path().join("sections/intro.tex");
        fs::write(&intro, "Three four five.\n").unwrap();

        // The section's unsaved words count from the root's tab.
        app.open(&intro).unwrap();
        typed(&mut app, "Zero ");
        app.open(&root).unwrap();
        let text = screen(&app).text();
        assert!(text.contains("6 words in the paper"), "{text}");

        app.open(&intro).unwrap();
        app.run(Command::ToggleTracking);
        assert_eq!(
            fs::read_to_string(&notes).unwrap(),
            notes_text,
            "left alone"
        );
        let on_disk = fs::read_to_string(&root).unwrap();
        assert!(
            on_disk.contains("\\usepackage{changes}\n\\begin{document}"),
            "{on_disk}"
        );
    }

    #[test]
    fn building_saves_the_papers_other_open_files() {
        let (dir, mut app) = paper_app(
            "\\documentclass{article}\n\\begin{document}\n\\input{intro}\n\\end{document}\n",
        );
        let root = dir.path().join("paper.tex");
        let intro = dir.path().join("intro.tex");
        fs::write(&intro, "Old.\n").unwrap();
        let other = dir.path().join("other.tex");
        fs::write(&other, "Elsewhere.\n").unwrap();
        app.open(&other).unwrap();
        typed(&mut app, "Unsaved ");
        app.open(&intro).unwrap();
        typed(&mut app, "New ");
        app.open(&root).unwrap();
        click_tool(&mut app, "Build PDF");
        built(&mut app);
        assert_eq!(
            fs::read_to_string(&intro).unwrap(),
            "New Old.\n",
            "built as it stands"
        );
        assert_eq!(
            fs::read_to_string(&other).unwrap(),
            "Elsewhere.\n",
            "not part of the paper"
        );
        assert!(dir.path().join("paper.pdf").exists());
    }

    #[test]
    fn saving_a_paper_rebuilds_its_open_pdf() {
        let (dir, mut app) = paper_app("\\begin{document}\nHello\n\\end{document}\n");
        app.run(Command::BuildPdf);
        built(&mut app);
        fs::remove_file(dir.path().join("paper.pdf")).unwrap();
        typed(&mut app, "x");
        app.run(Command::Save);
        built(&mut app);
        assert!(dir.path().join("paper.pdf").exists());
    }

    #[test]
    fn clicking_a_pdf_goes_to_its_source() {
        let (dir, mut app) = paper_app("\\begin{document}\nHello\nAgain\n\\end{document}\n");
        app.run(Command::BuildPdf);
        built(&mut app);
        let source = dir.path().join("paper.tex");
        let map = format!(
            "SyncTeX Version:1\nInput:1:{}\nContent:\n!1\n{{1\nh1,3:6578176,6578176:0,0,0\n}}1\n",
            source.display()
        );
        let mut gz = flate2_write(map.as_bytes());
        fs::write(dir.path().join("paper.synctex.gz"), &mut gz).unwrap();
        let path = dir.path().join("paper.pdf");
        app.show_pdf(&path);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while app.dock.pdf.as_ref().unwrap().pages_ready() == 0 {
            assert!(std::time::Instant::now() < deadline, "no page");
            app.poll();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let screen_area = app.dock_screen().unwrap();
        let (x, y) = (screen_area.x + 20, screen_area.y + 5);
        app.handle(mouse(MouseKind::Left, x, y));
        // The jump waits for the release, since the press might yet become a pan.
        app.handle(mouse(MouseKind::Up, x, y));
        assert_eq!(app.cursor_line().unwrap().1, 3);
        app.run(Command::ShowInPdf);
        assert_eq!(app.dock.front, Front::Pdf);
    }

    fn flate2_write(bytes: &[u8]) -> Vec<u8> {
        use std::io::Write;
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        gz.write_all(bytes).unwrap();
        gz.finish().unwrap()
    }

    #[test]
    fn a_tabular_has_the_table_menu() {
        let source = "\\begin{tabular}{ll}\nModel & Size \\\\\nBase & 12 \\\\\n\\end{tabular}\n";
        let (_dir, mut app) = paper_app(source);
        screen(&app);
        let editor = app.editor_mut().unwrap();
        editor.selections.set_single(Selection::cursor(40));
        assert!(app.editors[0].table_at_caret().is_some());
        let editor = app.editor_mut().unwrap();
        editor.edit_table(TableEdit::RowBelow);
        let text = app.active_text().unwrap();
        assert_eq!(text.matches("\\\\").count(), 3, "{text}");
    }

    #[test]
    fn a_failed_build_lands_on_its_line() {
        let (_dir, mut app) = paper_app("\\begin{document}\nbad\n\\end{document}\n");
        app.run(Command::BuildPdf);
        built(&mut app);
        assert!(screen(&app).text().contains("Undefined control sequence"));
        assert_eq!(app.cursor_line().unwrap().1, 2);
        assert!(app.dock.pdf.is_none());
    }

    #[test]
    fn a_pdf_shows_beside_its_source() {
        let (dir, mut app) = paper_app("\\begin{document}\nHello\n\\end{document}\n");
        let sample = dir.path().join("sample.pdf");
        app.open(&sample).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while app.dock.pdf.as_ref().unwrap().pages_ready() == 0 {
            assert!(std::time::Instant::now() < deadline, "no page");
            app.poll();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let pictures = screen(&app)
            .paint
            .iter()
            .filter(|p| matches!(p, cellix_render::Paint::Image { .. }))
            .count();
        assert_eq!(pictures, 1);
        assert_eq!(app.editors.len(), 1, "the source stays open");
    }

    #[test]
    fn a_paper_formats_in_latex() {
        for (tool, want) in [
            ("Bold", "\\textbf{word} tail\n"),
            ("Italic", "\\emph{word} tail\n"),
            ("Code", "\\texttt{word} tail\n"),
            (
                "Bullet List",
                "\\begin{itemize}\n  \\item word tail\n\\end{itemize}\n",
            ),
        ] {
            let (_dir, mut app) = paper_app("word tail\n");
            click_tool(&mut app, tool);
            assert_eq!(app.active_text().as_deref(), Some(want), "{tool}");
        }
        let (_dir, mut app) = paper_app("word tail\n");
        app.run(Command::Format(Format::Level(1)));
        assert_eq!(app.active_text().as_deref(), Some("\\section{word tail}\n"));
        app.run(Command::Format(Format::Level(2)));
        assert_eq!(
            app.active_text().as_deref(),
            Some("\\subsection{word tail}\n")
        );
        app.run(Command::Format(Format::Level(0)));
        assert_eq!(app.active_text().as_deref(), Some("word tail\n"));
    }

    #[test]
    fn citations_come_from_the_bibliography() {
        let (dir, mut app) = paper_app("See\n");
        fs::write(dir.path().join("refs.bib"), "@article{knuth84,\n}\n").unwrap();
        app.run(Command::Move(cellix_input::Direction::LineEnd, false));
        click_tool(&mut app, "Cite");
        assert!(screen(&app).text().contains("knuth84"));
        enter(&mut app);
        assert_eq!(app.active_text().as_deref(), Some("See\\cite{knuth84}\n"));
    }

    #[test]
    fn citations_come_from_the_bibliography_the_paper_names() {
        let (dir, mut app) = paper_app(
            "\\documentclass{article}\n\\usepackage{biblatex}\n\\addbibresource[backend=biber]{lit/library.bib}\n\\begin{document}\n\\input{sections/intro}\n\\end{document}\n",
        );
        fs::create_dir(dir.path().join("lit")).unwrap();
        fs::write(
            dir.path().join("lit/library.bib"),
            "@article{lamport94,\n title={LaTeX}, author={Leslie Lamport}, year={1994}\n}\n",
        )
        .unwrap();
        fs::create_dir(dir.path().join("sections")).unwrap();
        let intro = dir.path().join("sections/intro.tex");
        fs::write(&intro, "See\n").unwrap();
        app.open(&intro).unwrap();
        app.run(Command::Move(cellix_input::Direction::LineEnd, false));
        click_tool(&mut app, "Cite");
        assert!(screen(&app).text().contains("lamport94"));
        enter(&mut app);
        assert_eq!(app.active_text().as_deref(), Some("See\\cite{lamport94}\n"));
    }

    #[test]
    fn ctrl_a_selects_everything_in_a_paper_shown_either_way() {
        let text = "\\documentclass{article}\n\\begin{document}\nHello there.\n\\end{document}\n";
        let (_dir, mut app) = paper_app(text);
        let ctrl_a = Event::Key(KeyEvent {
            key: Key::Char('a'),
            mods: cellix_input::Mods {
                ctrl: true,
                ..cellix_input::Mods::default()
            },
        });
        screen(&app);
        app.handle(ctrl_a.clone());
        assert_eq!(app.selected_text().as_deref(), Some(text), "on the page");
        key(&mut app, Key::Right);
        app.run(Command::MarkdownPreview);
        screen(&app);
        app.handle(ctrl_a);
        assert_eq!(app.selected_text().as_deref(), Some(text), "in the source");
    }

    #[test]
    fn closing_a_reference_at_a_lines_end_keeps_the_caret_there() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("paper.tex");
        let text = "\\section{Intro}\\label{sec:intro}\nSee \\Cref{sec:intro}.\n";
        fs::write(&path, text).unwrap();
        let mut app = App::new(dir.path().to_path_buf(), (170, 40));
        app.open(&path).unwrap();
        screen(&app);
        app.run(Command::Move(cellix_input::Direction::Down, false));
        app.run(Command::Move(cellix_input::Direction::LineEnd, false));
        typed(&mut app, " And \\Cref{sec");
        enter(&mut app);
        typed(&mut app, "}");
        screen(&app);
        typed(&mut app, "XYZ");
        let text = app.active_text().unwrap();
        assert!(
            text.contains("\\Cref{sec:intro}. And \\Cref{sec:intro}XYZ"),
            "{text}"
        );
    }

    #[test]
    fn references_and_citations_complete_inside_their_braces() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("refs.bib"), "@article{knuth84,\n}\n").unwrap();
        let path = dir.path().join("paper.tex");
        fs::write(&path, "See \n\\section{Intro}\\label{sec:intro}\n").unwrap();
        let mut app = App::new(dir.path().to_path_buf(), (170, 40));
        app.open(&path).unwrap();
        // Drawn once as it opens, which reads the paper.
        screen(&app);
        let offered = |app: &App, key: &str| {
            app.suggest
                .as_ref()
                .is_some_and(|s| s.items.iter().any(|c| c.label == key))
        };
        app.run(Command::Move(cellix_input::Direction::LineEnd, false));
        typed(&mut app, "\\ref{");
        assert!(offered(&app, "sec:intro"), "the label is offered");
        enter(&mut app);
        typed(&mut app, " \\cite{kn");
        assert!(offered(&app, "knuth84"), "the key is offered");
        enter(&mut app);
        let text = app.active_text().unwrap();
        assert!(text.contains("\\ref{sec:intro"), "{text}");
        assert!(text.contains("\\cite{knuth84"), "{text}");
        assert!(
            !text.contains("knkn") && !text.contains("sec:sec"),
            "{text}"
        );
    }

    #[test]
    fn markdown_exports_to_pdf() {
        let (dir, mut app) = paper_app("x\n");
        let path = dir.path().join("note.md");
        fs::write(&path, "# Note\n\nWith $x^2$.\n").unwrap();
        app.open(&path).unwrap();
        app.run(Command::ExportPdf);
        built(&mut app);
        assert!(dir.path().join("note.pdf").exists());
        assert!(
            !dir.path().join("note.tex").exists(),
            "the middle step is gone"
        );
    }

    /// A pdflatex in a folder of its own, with the tools TeX Live keeps beside it, each a shell
    /// script. Every run of each is counted in a file named for it.
    #[cfg(unix)]
    fn fake_tex(dir: &Path, tools: &[(&str, &str)]) -> Vec<String> {
        use std::os::unix::fs::PermissionsExt;
        let bin = dir.join("texbin");
        fs::create_dir_all(&bin).unwrap();
        for (name, body) in tools {
            let script = bin.join(name);
            let count = format!("echo \"$@\" >> {name}.runs");
            fs::write(&script, format!("#!/bin/sh\n{count}\n{body}\n")).unwrap();
            fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
        }
        vec![bin.join("pdflatex").display().to_string()]
    }

    #[cfg(unix)]
    fn runs(dir: &Path, tool: &str) -> Vec<String> {
        let text = fs::read_to_string(dir.join(format!("{tool}.runs"))).unwrap_or_default();
        text.lines().map(str::to_string).collect()
    }

    #[test]
    #[cfg(unix)]
    fn pdflatex_reruns_while_its_log_asks() {
        let (dir, mut app) =
            paper_app("\\documentclass{article}\n\\begin{document}\nx\n\\end{document}\n");
        // As `\include` leaves it: no labels in the root's .aux, and a log asking twice.
        let pdflatex = "cp sample.pdf paper.pdf\n: > paper.aux\nif [ $(wc -l < pdflatex.runs) -lt 3 ]; then echo 'LaTeX Warning: Label(s) may have changed. Rerun to get cross-references right.' > paper.log; else echo 'Output written on paper.pdf' > paper.log; fi";
        app.settings.latex_engine = Some(fake_tex(dir.path(), &[("pdflatex", pdflatex)]));
        app.run(Command::BuildPdf);
        built(&mut app);
        assert_eq!(runs(dir.path(), "pdflatex").len(), 3);
        let shown = screen(&app).text();
        assert!(shown.contains("Built in"), "{shown}");
    }

    #[test]
    #[cfg(unix)]
    fn pdflatex_runs_biber_for_biblatex() {
        let (dir, mut app) =
            paper_app("\\documentclass{article}\n\\begin{document}\nx\n\\end{document}\n");
        let pdflatex = "cp sample.pdf paper.pdf\nprintf '\\\\abx@aux@cite{0}{knuth84}\\n' > paper.aux\n: > paper.bcf\n: > paper.log";
        let tools = [("pdflatex", pdflatex), ("biber", "echo 'INFO - Done'")];
        app.settings.latex_engine = Some(fake_tex(dir.path(), &tools));
        app.run(Command::BuildPdf);
        built(&mut app);
        assert_eq!(runs(dir.path(), "biber"), ["paper"]);
        assert_eq!(runs(dir.path(), "pdflatex").len(), 3);
        let shown = screen(&app).text();
        assert!(shown.contains("Built in"), "{shown}");
    }

    #[test]
    #[cfg(unix)]
    fn a_bibtex_that_fails_says_why() {
        let (dir, mut app) =
            paper_app("\\documentclass{article}\n\\begin{document}\nx\n\\end{document}\n");
        let pdflatex = "cp sample.pdf paper.pdf\nprintf '\\\\citation{k}\\n\\\\bibdata{refs}\\n' > paper.aux\n: > paper.log";
        let bibtex = "echo 'This is BibTeX, Version 0.99d'\necho \"I couldn't open database file refs.bib\"\nexit 2";
        let tools = [("pdflatex", pdflatex), ("bibtex", bibtex)];
        app.settings.latex_engine = Some(fake_tex(dir.path(), &tools));
        app.run(Command::BuildPdf);
        built(&mut app);
        assert_eq!(runs(dir.path(), "bibtex"), ["paper"]);
        let shown = screen(&app).text();
        assert!(
            shown.contains("but BibTeX failed: I couldn't open database file refs.bib"),
            "{shown}"
        );
        assert!(
            app.dock
                .log
                .iter()
                .any(|l| l.contains("couldn't open database")),
            "its output is in the log"
        );
    }

    #[test]
    #[cfg(unix)]
    fn a_failed_export_keeps_the_last_good_pdf() {
        let (dir, mut app) = paper_app("x\n");
        let path = dir.path().join("note.md");
        fs::write(&path, "# Note\n\nWith $x^2$.\n").unwrap();
        fs::write(dir.path().join("note.pdf"), "the good one").unwrap();
        // As pdflatex in nonstop mode: a PDF is written all the same, and the run fails.
        let reply =
            "echo broken > \"${1%.tex}.pdf\"; echo \"./$1:7: Undefined control sequence.\"; exit 1";
        app.settings.latex_engine = Some(fake_latex(dir.path(), reply));
        app.open(&path).unwrap();
        app.run(Command::ExportPdf);
        built(&mut app);
        assert_eq!(
            fs::read_to_string(dir.path().join("note.pdf")).unwrap(),
            "the good one"
        );
        assert!(!dir.path().join("note.cellix-export.pdf").exists());
        assert!(!dir.path().join("note.cellix-export.tex").exists());
        let grid = screen(&app);
        assert!(
            grid.text()
                .contains("The PDF wasn't made: Undefined control sequence."),
            "{}",
            grid.text()
        );
        assert_eq!(app.dock.front, Front::Log);
        // The log names the hidden file: a click there says the error, and opens nothing.
        let (x, y) = find(&grid, "cellix-export.tex:7:");
        app.handle(mouse(MouseKind::Left, x, y));
        assert!(
            app.editors
                .iter()
                .all(|e| e.buffer.path() == Some(path.as_path())
                    || e.buffer.path().is_some_and(|p| p.ends_with("paper.tex"))),
            "only the note and the paper are open"
        );
        assert!(
            app.problem_rows().is_empty(),
            "no problem in a missing file"
        );
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
        assert_eq!(rows, 2, "a line for each event, with no jump to start");
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
    fn neovim_never_writes_another_files_lines_into_this_one() {
        let Some((dir, mut app, path)) = neovim_app() else {
            return;
        };
        let other = dir.path().join("other.txt");
        fs::write(&other, "elsewhere\n").unwrap();
        typed(&mut app, ":e other.txt");
        enter(&mut app);
        assert!(
            screen(&app).text().contains("Open files with Ctrl+P"),
            "it says why"
        );
        typed(&mut app, "x");
        typed(&mut app, ":w");
        enter(&mut app);
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "ne\ntwo\nthree\n",
            "still this file"
        );
        assert_eq!(fs::read_to_string(&other).unwrap(), "elsewhere\n");
        typed(&mut app, ":vsplit");
        enter(&mut app);
        typed(&mut app, "dd");
        assert_eq!(
            app.active_text().unwrap(),
            "two\nthree\n",
            "one window, still editing"
        );
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
        // A line breaks between words, so it can stop a word and the minimap's gap short.
        assert!(end > pane.right() - 12, "it runs across the pane: {end}");
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
    fn unsent_comments_wait_for_the_next_run() {
        let (dir, app) = project();
        let comment = |file: &str| Comment {
            file: file.into(),
            line: 1,
            text: "why?".into(),
            ..Comment::default()
        };
        // One on a project file, and one on a plan that has gone with its hook.
        app.comments
            .lock()
            .unwrap()
            .extend([comment("alpha.txt"), comment("Plan.md")]);
        app.finish().unwrap();
        let mut next = App::new(dir.path().to_path_buf(), (100, 30));
        next.restore_unsent();
        let comments = next.comments.lock().unwrap().clone();
        assert_eq!(comments, vec![comment("alpha.txt")]);
        assert!(screen(&next).text().contains("from last time"));
        let mut third = App::new(dir.path().to_path_buf(), (100, 30));
        third.restore_unsent();
        assert!(
            third.comments.lock().unwrap().is_empty(),
            "handed over once"
        );
        // The cache folder is this temporary project's alone.
        let _ = fs::remove_dir(crate::hold::folder_of(dir.path()));
    }

    #[test]
    fn saving_with_open_suggestions_says_they_stay_as_marks() {
        let (_dir, mut app) = page_app("One {++new ++}two.\n");
        app.run(Command::Save);
        let text = screen(&app).text();
        assert!(text.contains("1 suggestion still open"), "{text}");
    }

    #[test]
    fn track_changes_stays_as_it_was_left_for_a_document() {
        let (dir, mut app) = page_app("One two.\n");
        app.run(Command::ToggleTracking);
        assert!(app.editors[0].tracking);
        let path = app.editors[0].buffer.path().unwrap().to_path_buf();
        let mut next = App::new(dir.path().to_path_buf(), (100, 30));
        next.open(&path).unwrap();
        assert!(next.editors[0].tracking, "remembered for this document");
        next.run(Command::ToggleTracking);
        let mut third = App::new(dir.path().to_path_buf(), (100, 30));
        third.open(&path).unwrap();
        assert!(!third.editors[0].tracking, "and remembered off");
        let _ = fs::remove_dir(crate::hold::folder_of(dir.path()));
    }

    #[test]
    fn a_quick_fix_is_offered_and_applied() {
        let (dir, mut app) = project();
        let path = dir.path().join("beta.rs");
        app.open(&path).unwrap();
        let path = app.editors[app.active()]
            .buffer
            .path()
            .unwrap()
            .to_path_buf();
        let edit = TextEdit {
            start: (0, 12),
            end: (0, 12),
            text: " // fixed".into(),
        };
        app.lsp_event(LspEvent::Actions(vec![(
            "Mark it fixed".into(),
            vec![(path, vec![edit])],
        )]));
        assert!(screen(&app).text().contains("Mark it fixed"));
        // The first is ready for Enter, as in VS Code.
        key(&mut app, Key::Enter);
        assert_eq!(app.active_text().unwrap(), "fn main() {} // fixed\n");
    }

    #[test]
    fn code_lists_its_symbols_in_the_outline() {
        let (dir, mut app) = project();
        let path = dir.path().join("beta.rs");
        app.open(&path).unwrap();
        let path = app.editors[app.active()]
            .buffer
            .path()
            .unwrap()
            .to_path_buf();
        app.lsp_event(LspEvent::Symbols(
            path,
            vec![("main".into(), 0, 0), ("helper".into(), 1, 0)],
        ));
        app.run(Command::ShowOutline);
        let text = screen(&app).text();
        assert!(text.contains("main") && text.contains("helper"), "{text}");
        assert!(!text.contains("No headings here yet"), "{text}");
    }

    #[test]
    fn a_review_check_asks_the_agent_and_its_suggestions_can_be_kept() {
        let (_dir, mut app) = page_app("# Claim\n\nOur method is better on every benchmark.\n");
        select_word(&mut app, "better");
        app.run(Command::AddComment);
        typed(&mut app, "By how much?");
        enter(&mut app);
        app.in_pane = true;
        app.thread_hit(ThreadHit::Check);
        let prompt = app.take_outgoing_prompt().expect("a prompt for the agent");
        assert!(
            prompt.contains("By how much?") && prompt.contains("cellix suggest note.md"),
            "{prompt}"
        );
        let id = app.threads[0].id;
        let (found, _) = std::sync::mpsc::channel();
        app.serve(Request::Reply {
            id,
            author: "Claude".into(),
            text: "Disagree (Evidence): the table gives it".into(),
            found,
        });
        assert_eq!(app.threads[0].kind, "Evidence");
        let (made, number) = std::sync::mpsc::channel();
        app.serve(Request::Suggest {
            file: "note.md".into(),
            line: 3,
            kind: "Evidence".into(),
            quote: "every benchmark".into(),
            text: "Name the benchmarks.".into(),
            author: "Claude".into(),
            made,
        });
        let suggested = number.recv().unwrap();
        let text = screen(&app).text();
        assert!(text.contains("Suggested by Claude"), "{text}");
        // The verdict is a chip of its own, beside who gave it.
        assert!(text.contains("Pushes back"), "{text}");
        assert!(text.contains("Keep") && text.contains("Dismiss"), "{text}");
        app.thread_hit(ThreadHit::Keep(suggested));
        let kept = app.threads.iter().find(|t| t.id == suggested).unwrap();
        assert!(kept.suggested_by.is_empty(), "now the reviewer's own");
    }

    #[test]
    fn a_rewrite_reply_applies_as_a_suggestion() {
        let (_dir, mut app) = page_app("Our method is better on every benchmark.\n");
        select_word(&mut app, "better");
        app.run(Command::AddComment);
        typed(&mut app, "Vague");
        enter(&mut app);
        let id = app.threads[0].id;
        let (found, _) = std::sync::mpsc::channel();
        app.serve(Request::Reply {
            id,
            author: "Claude".into(),
            text: "Rewrite: 3.6 points better".into(),
            found,
        });
        let text = screen(&app).text();
        assert!(
            text.contains("suggests a rewrite") && text.contains("Apply"),
            "{text}"
        );
        app.thread_hit(ThreadHit::Apply(id));
        assert_eq!(
            app.active_text().unwrap(),
            "Our method is {~~better~>3.6 points better~~} on every benchmark.\n"
        );
    }

    #[test]
    fn later_agent_edits_join_a_review_already_begun() {
        let (dir, mut app) = tracking_page("One two.\n\nThree four.\n");
        let path = dir.path().join("note.md");
        std::thread::sleep(std::time::Duration::from_millis(20));
        fs::write(&path, "One TWO.\n\nThree four.\n").unwrap();
        app.handle(Event::Tick);
        app.run(Command::AcceptAllChanges);
        assert_eq!(app.active_text().unwrap(), "One TWO.\n\nThree four.\n");
        std::thread::sleep(std::time::Duration::from_millis(20));
        fs::write(&path, "One TWO.\n\nThree FOUR.\n").unwrap();
        app.handle(Event::Tick);
        let text = app.active_text().unwrap();
        assert_eq!(
            text, "One TWO.\n\nThree {~~four.~>FOUR.~~}\n",
            "only the new edit is marked"
        );
    }

    #[test]
    fn replace_under_tracking_suggests_and_changes_step_through() {
        let (_dir, mut app) = tracking_page("a cat, a cat.\n");
        app.run(Command::ReplaceInFile);
        typed(&mut app, "cat");
        app.handle(Event::Key(KeyEvent::plain(Key::Tab)));
        typed(&mut app, "dog");
        app.handle(Event::Key(KeyEvent {
            key: Key::Enter,
            mods: cellix_input::Mods {
                alt: true,
                ..cellix_input::Mods::default()
            },
        }));
        assert_eq!(
            app.active_text().unwrap(),
            "a {~~cat~>dog~~}, a {~~cat~>dog~~}.\n"
        );
        app.overlay = Overlay::None;
        app.run(Command::NextChange);
        assert!(
            screen(&app).text().contains(" of 2"),
            "it says which of the two"
        );
    }

    #[test]
    fn a_hover_shows_beside_its_place_until_a_key() {
        let (dir, mut app) = project();
        app.open(&dir.path().join("beta.rs")).unwrap();
        app.hover_at = Some((40, 3));
        app.lsp_event(LspEvent::Hover("fn main()\nWhere it starts.".into()));
        let text = screen(&app).text();
        assert!(
            text.contains("fn main()") && text.contains("Where it starts."),
            "{text}"
        );
        key(&mut app, Key::Right);
        assert!(
            !screen(&app).text().contains("Where it starts."),
            "a key clears it"
        );
    }

    #[test]
    fn line_shortcuts_edit_as_vs_code_does() {
        let (dir, mut app) = project();
        let path = dir.path().join("lines.txt");
        fs::write(&path, "one\n    two\nthree\n").unwrap();
        app.open(&path).unwrap();
        app.run(Command::Move(cellix_input::Direction::Down, false));
        app.run(Command::InsertLineBelow);
        typed(&mut app, "x");
        assert_eq!(app.active_text().unwrap(), "one\n    two\n    x\nthree\n");
        app.run(Command::DeleteLine);
        assert_eq!(app.active_text().unwrap(), "one\n    two\nthree\n");
        app.run(Command::InsertLineAbove);
        typed(&mut app, "y");
        assert_eq!(app.active_text().unwrap(), "one\n    two\ny\nthree\n");
        app.run(Command::AddCursorBelow);
        assert_eq!(app.editors[app.active()].selections.len(), 2);
        let animals = dir.path().join("animals.txt");
        fs::write(&animals, "cat dog cat bird cat\n").unwrap();
        app.open(&animals).unwrap();
        app.run(Command::SelectAllMatches);
        assert_eq!(app.editors[app.active()].selections.len(), 3, "every cat");
    }

    #[test]
    fn f8_walks_the_problems_in_a_file() {
        let (dir, mut app) = project();
        let path = dir.path().join("alpha.txt");
        app.open(&path).unwrap();
        let problem = |line: usize, message: &str| cellix_lang::lsp::Diagnostic {
            line,
            column: 0,
            message: message.to_string(),
            severity: 1,
        };
        app.diagnostics.insert(
            path.canonicalize().unwrap_or(path.clone()),
            vec![problem(1, "second is wrong")],
        );
        app.diagnostics
            .insert(path.clone(), vec![problem(1, "second is wrong")]);
        app.run(Command::NextProblem);
        assert_eq!(
            app.editors[app.active()]
                .buffer
                .rope()
                .char_to_line(app.editors[app.active()].selections.primary().head),
            1
        );
        assert!(screen(&app).text().contains("second is wrong"));
    }

    #[test]
    fn go_back_returns_from_a_jump_and_go_forward_repeats_it() {
        let (dir, mut app) = project();
        let path = dir.path().join("lines.txt");
        fs::write(&path, "one\ntwo\nthree\nfour\n").unwrap();
        app.open(&path).unwrap();
        let head = |app: &App| app.editors[app.active()].selections.primary().head;
        assert_eq!(head(&app), 0);
        app.run(Command::GoToLine);
        typed(&mut app, "3");
        enter(&mut app);
        assert_eq!(head(&app), 8, "line 3");
        app.run(Command::GoBack);
        assert_eq!(head(&app), 0, "back where the jump left from");
        app.run(Command::GoForward);
        assert_eq!(head(&app), 8, "and forward again");
        app.run(Command::GoForward);
        assert!(screen(&app).text().contains("Nowhere to go forward to"));
    }

    #[test]
    fn rejecting_asks_why_and_tells_the_agent() {
        let (dir, mut app) = project();
        let old: String = (0..10).map(|i| format!("line {i}\n")).collect();
        fs::write(dir.path().join("alpha.txt"), &old).unwrap();
        let new = old.replace("line 2\n", "two\n");
        let answer = propose(&mut app, diff_args(&dir, "t", &new));
        let (x, y) = find(&screen(&app), "Reject");
        app.handle(mouse(MouseKind::Left, x + 1, y));
        assert!(answer.try_recv().is_err(), "nothing yet: it asks first");
        assert!(screen(&app).text().contains("Why reject it?"));
        typed(&mut app, "keep the numbering");
        enter(&mut app);
        assert_eq!(answer.try_recv().unwrap()[0], "DIFF_REJECTED");
        let comments = app.comments.lock().unwrap().clone();
        assert_eq!(comments.len(), 1, "{comments:?}");
        assert_eq!(comments[0].line, 3);
        assert_eq!(comments[0].quote, "two");
        assert!(comments[0].text.contains("keep the numbering"));
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
        // Cellix's folder first, then the path as it was, joined as this platform joins them:
        // Windows separates with `;`, and a `:` there would break the first entry.
        let path = env.iter().find_map(|e| e.strip_prefix("PATH=")).unwrap();
        let folders: Vec<PathBuf> = std::env::split_paths(path).collect();
        let own = std::env::current_exe().unwrap();
        assert_eq!(Some(folders[0].as_path()), own.parent());
        let before: Vec<PathBuf> =
            std::env::split_paths(&std::env::var_os("PATH").unwrap()).collect();
        assert_eq!(folders[1..], before[..]);
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
        let rail = app.activity_area();
        app.handle(mouse(MouseKind::Left, 1, app.rail_row(3, rail)));
        assert!(app.sidebar_view == SidebarView::Source);
        app.handle(mouse(MouseKind::Left, 1, app.rail_row(0, rail)));
        assert!(app.sidebar_view == SidebarView::Explorer);
    }

    #[test]
    fn activity_icons_span_several_cells() {
        let (_dir, app) = project();
        let grid = screen(&app);
        let pieces = (0..ACTIVITY_WIDTH)
            .flat_map(|x| (0..1).map(move |y| (x, y)))
            .filter(|(x, y)| cellix_render::Piece::decode(&grid.cell(*x, *y).symbol).is_some())
            .count();
        assert_eq!(pieces, 2, "a 2 by 1 icon, one row each since the redesign");
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
        // It asks why, and Enter turns it down.
        enter(&mut app);
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
    fn right_clicking_a_paper_offers_latex_not_markdown() {
        let (_dir, mut app) = paper_app("Some words here.\n");
        let labels: Vec<String> = app
            .editor_items()
            .iter()
            .filter(|item| item.action.is_some())
            .map(|item| item.label.to_string())
            .collect();
        for item in ["Section", "Insert Citation…", "Build PDF", "Show in PDF"] {
            assert!(labels.iter().any(|l| l == item), "{item}: {labels:?}");
        }
        assert!(!labels.iter().any(|l| l == "Code Block"), "{labels:?}");
        app.run(Command::Format(Format::Heading));
        assert!(
            app.active_text().unwrap().starts_with("\\section{"),
            "{:?}",
            app.active_text()
        );
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
        app.handle(mouse(
            MouseKind::Left,
            2,
            app.rail_row(ACTIVITY.len() - 1, area),
        ));
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
    fn the_settings_tab_closes_like_any_other() {
        let (dir, mut app) = project();
        app.open(&dir.path().join("alpha.txt")).unwrap();
        app.run(Command::OpenSettings);
        app.handle(Event::Key(KeyEvent::ctrl('w')));
        assert!(app.settings_page.is_none(), "Ctrl+W closes Settings");
        assert_eq!(app.editors.len(), 1, "and leaves the file behind it open");
        assert!(screen(&app).text().contains("alpha.txt"));

        app.run(Command::OpenSettings);
        let (x, y) = find(&screen(&app), "Settings");
        assert_eq!(y, app.tabs_area().y, "the Settings tab");
        app.handle(mouse(MouseKind::Middle, x + 1, y));
        assert!(app.settings_page.is_none(), "a middle click closes it");
        assert_eq!(app.editors.len(), 1);
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
    fn the_vim_keymap_runs_neovim_when_it_is_installed() {
        let installed = crate::integrations::on_path("nvim", |name| std::env::var(name).ok());
        // On a loaded machine Neovim can miss its first reply, which is about speed, not the
        // choice this checks, so it gets a few tries.
        let (vim, neovim) = (0..3)
            .map(|_| engines_for(Some("vim"), true))
            .find(|(_, neovim)| neovim.is_some() == installed)
            .unwrap_or_else(|| engines_for(Some("vim"), true));
        assert_eq!(neovim.is_some(), installed, "Neovim when it's there");
        assert_eq!(vim.is_some(), !installed, "the built-in mode when it isn't");
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
        assert!(text.contains("HOW IT WORKS"), "{text}");
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
        let (dir, mut app) = project();
        app.open(&dir.path().join("alpha.txt")).unwrap();
        let rail = app.activity_area();
        app.handle(mouse(MouseKind::Move, 1, app.rail_row(1, rail)));
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
