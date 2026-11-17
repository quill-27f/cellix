//! The side pane. Agents and shells run here in a real terminal, beside the file, so working
//! with one doesn't mean leaving the other. The program is the real one, on a real PTY, and
//! it updates itself: nothing here knows what Claude Code looks like.

use std::cell::Cell;
use std::io::{self, Read, Write};
use std::path::Path;
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};

use alacritty_terminal::event::{Event as TermEvent, EventListener, WindowSize};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Line, Point};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{Config, Term, TermMode};
use alacritty_terminal::vte::ansi::{Color, CursorShape, NamedColor, Processor, Rgb as TermRgb};
use cellix_input::{Key, KeyEvent, MouseKind};
use cellix_render::{Bitmap, Bounds, Grid, Icon, Paint, Rect, Rgb, Style};
use portable_pty::{ChildKiller, CommandBuilder, MasterPty, PtySize, native_pty_system};
use unicode_width::UnicodeWidthStr;

use crate::hold::Held;
use crate::kitty::{Delete, Outcome, Piece, Protocol, Scanner, Shown};
use crate::pdfview::PdfView;
use crate::theme::Theme;
use crate::threads::Draft;

/// Columns the file beside the pane keeps, and the pane's own least, at the width it opens.
const MIN_MAIN: u16 = 44;
const MIN_WIDTH: u16 = 40;
/// Columns each side keeps however far the line between them is dragged. Less than what the
/// pane opens with: in a window of 90 columns, as inside Claude Code, those two leave the
/// line 6 columns to move in, which is no resize at all.
const DRAGGED_LEAST: u16 = 24;
/// Share of the room right of the sidebar, in percent.
const SHARE: u16 = 46;
/// Lines a wheel notch moves through scrollback.
const WHEEL_LINES: i32 = 3;
const TAB_PADDING: u16 = 2;
const CLOSE_WIDTH: u16 = 2;
/// The Comments tab at the far end, and what a count adds to it.
const COMMENTS_WIDTH: u16 = 10;
/// The Inbox tab, which counts what's waiting the same way.
const INBOX_WIDTH: u16 = 7;
const COUNT_WIDTH: u16 = 2;
/// The button that starts something, and the tab it becomes while that list shows.
const NEW_WIDTH: u16 = 3;
/// The button after it that maximizes the pane.
const MAXIMIZE_WIDTH: u16 = 3;
/// The column between it and the +, so the row keeps one step between its tabs and buttons.
const MAXIMIZE_GAP: u16 = 1;
/// Columns it keeps clear of the icons at the top right, so it isn't read as one of them.
const MAXIMIZE_CLEAR: u16 = 2;
/// Columns kept for it at the end of the pane's own part of its row, whatever the tabs do:
/// after the +, and in a row too short for the +, where it stands right at the row's end.
/// The column the icons keep clear before them and the air in its own cell and theirs put
/// 46 px between its strokes and the first icon's, where the icons are 28 px apart.
const MAXIMIZE_ROOM: u16 = MAXIMIZE_GAP + MAXIMIZE_WIDTH + MAXIMIZE_CLEAR;
const MAXIMIZE_SHORT: u16 = MAXIMIZE_WIDTH;
const AGENTS_WIDTH: u16 = 11;
const PREVIEW_WIDTH: u16 = 12;
const LOG_WIDTH: u16 = 8;
const PROBLEMS_WIDTH: u16 = 13;
/// Where a tab's name starts, as on the file's tabs.
const TAB_TEXT: f32 = 1.5;
/// The column between an agent's name and how full its context is.
const CONTEXT_GAP: u16 = 1;
/// Room between the name and the context, in cells: about 16 px.
const CONTEXT_AIR: f32 = 0.9;
/// The size the context is set at, smaller than the name it follows.
const CONTEXT_SIZE: u8 = 85;
/// Columns for the chevron on the tab in front that lists every tab, when they don't all fit.
const PICK_WIDTH: u16 = 2;
/// The fewest columns a tab behind the one in front is cut to: three letters and the "…"
/// that says there's more, with half a column of air either side. Two letters don't tell
/// Comments from Codex. Under that, the tabs go into a list.
const CUT_LEAST: u16 = 5;
/// The air a tab behind the one in front has either side of its name, in cells: about 8 px.
/// Its chip under the pointer is exactly its name and that air.
const CUT_AIR: f32 = 0.5;
/// What the tab in front has beside its name when the row is short, in cells: the lead a
/// tab's name always has, 12 px to its ×, the ×, and 10 px to the chip's edge. With room,
/// a tab has more than this; without, the slack is what would cut its neighbours.
const HUG: f32 = 3.92;
/// How far before a hugging tab's right edge its × is drawn, so 10 px stay past its ink.
const HUG_CLOSE: f32 = 2.07;
/// Columns a program's mark takes before its name on its tab, as a file's icon does on its
/// own, and how tall it is in rows: 13 pt. On cells it's one letter and a space.
const ICON_ROOM: u16 = 2;
const ICON_SIZE: f32 = 0.70;
/// How far the mark's middle stands past where a tab's name would start without it, and how
/// far the name moves for it, in cells: where a file's tab has them.
const ICON_MIDDLE: f32 = 0.5;
const ICON_SHIFT: f32 = 1.7;
/// Columns an agent's tab keeps before its × for what it's doing, whether it shows anything
/// or not, so a tab keeps its width as its agent starts and stops.
const MARK_ROOM: u16 = 2;
/// How tall that mark is, in the terminal's lines: a spinner of 12 pt, and a dot half that.
pub const MARK_SIZE: f32 = 0.65;
/// How far before the × the mark starts, in cells: its 24 px and about 9 px of air.
const MARK_SHIFT: f32 = 1.6;
/// A tab cut short behind the one in front: from its edge to its mark's middle, and from its
/// state's start to its other edge, in cells.
const CUT_ICON: f32 = 1.2;
const CUT_MARK: f32 = 1.9;
/// How wide that mark is, in cells, where a cell is 17 px and a line 37.
const MARK_WIDE: f32 = 1.4;
/// The button that closes the pane.
const HIDE_WIDTH: u16 = 3;
/// Rows tall each icon at the top right is: the pane's buttons and the icons beside them.
/// 13 pt where a row is 18.5 pt. VS Code's are 16 pt in a bar 35 pt tall; this bar is a row,
/// and more than this would stand against its chip.
pub const ACTION_ICON: f32 = 0.70;
/// The chip behind one of them that's showing or hovered, in columns and rows: as tall as a
/// button, with the icon's box 8 px inside it at the sides.
pub const ACTION_PILL: (f32, f32) = (2.5, 0.89);
/// The hover chip of the button that hides the pane, and of the one that shows it: as tall
/// and round as the icons' chips, narrower, so a column stays clear between it and the
/// sheet's edge.
pub const BESIDE_LINE_PILL: (f32, f32) = (1.95, 0.89);
const HIDE_PILL: (f32, f32) = BESIDE_LINE_PILL;
const HIDE_RADIUS: u8 = 30;
/// What the pane's browser tab is called.
pub const BROWSER: &str = "Browser";
/// Pictures kept per session, oldest going first.
const MAX_PICTURES: usize = 512;

/// Light and dark versions of the sixteen colours, chosen to read on the sheet.
const DARK: [Rgb; 16] = [
    Rgb(0x1e, 0x1f, 0x26),
    Rgb(0xf3, 0x8b, 0xa8),
    Rgb(0xa6, 0xe3, 0xa1),
    Rgb(0xf9, 0xe2, 0xaf),
    Rgb(0x89, 0xb4, 0xfa),
    Rgb(0xcb, 0xa6, 0xf7),
    Rgb(0x94, 0xe2, 0xd5),
    Rgb(0xba, 0xc2, 0xde),
    Rgb(0x6c, 0x70, 0x86),
    Rgb(0xf3, 0x8b, 0xa8),
    Rgb(0xa6, 0xe3, 0xa1),
    Rgb(0xf9, 0xe2, 0xaf),
    Rgb(0x89, 0xb4, 0xfa),
    Rgb(0xcb, 0xa6, 0xf7),
    Rgb(0x94, 0xe2, 0xd5),
    Rgb(0xee, 0xee, 0xf4),
];
const LIGHT: [Rgb; 16] = [
    Rgb(0x4c, 0x4f, 0x69),
    Rgb(0xd2, 0x0f, 0x39),
    Rgb(0x2f, 0x8a, 0x2b),
    Rgb(0xa8, 0x6a, 0x0a),
    Rgb(0x1e, 0x66, 0xf5),
    Rgb(0x88, 0x39, 0xef),
    Rgb(0x13, 0x87, 0x8f),
    Rgb(0x9c, 0xa0, 0xb0),
    Rgb(0x6c, 0x6f, 0x85),
    Rgb(0xd2, 0x0f, 0x39),
    Rgb(0x2f, 0x8a, 0x2b),
    Rgb(0xa8, 0x6a, 0x0a),
    Rgb(0x1e, 0x66, 0xf5),
    Rgb(0x88, 0x39, 0xef),
    Rgb(0x13, 0x87, 0x8f),
    Rgb(0xbc, 0xc0, 0xcc),
];

/// What the terminal state machine has to say, collected for the next poll.
#[derive(Clone, Default)]
struct Events(Arc<Mutex<Vec<TermEvent>>>);

impl EventListener for Events {
    fn send_event(&self, event: TermEvent) {
        if let Ok(mut events) = self.0.lock() {
            events.push(event);
        }
    }
}

#[derive(Clone, Copy)]
struct Size {
    columns: usize,
    lines: usize,
}

impl Dimensions for Size {
    fn total_lines(&self) -> usize {
        self.lines
    }

    fn screen_lines(&self) -> usize {
        self.lines
    }

    fn columns(&self) -> usize {
        self.columns
    }
}

/// How a session reaches its program.
enum Link {
    /// Cellix owns the PTY, so the program ends with Cellix.
    Own {
        master: Box<dyn MasterPty + Send>,
        killer: Box<dyn ChildKiller + Send + Sync>,
        /// Read only where a PTY can say what's in front of it.
        #[cfg_attr(not(unix), allow(dead_code))]
        pid: Option<u32>,
    },
    /// A host of its own owns the PTY, so the program can outlive Cellix.
    Held(Held),
    /// Its host died without finishing, as when the machine restarted: only what it last
    /// showed is left.
    Gone,
}

/// A place in the visible text, as (column, row).
type Spot = (u16, u16);

/// Pixels in a cell, and none when the terminal outside can't show pictures.
pub type CellPixels = (u16, u16);

/// A picture a program put on its screen.
struct Picture {
    image: u32,
    placement: u32,
    bitmap: Bitmap,
    /// Where it starts: counting from the top of the scrollback, so it scrolls with the text.
    line: usize,
    column: u16,
    /// How big it stands, in cells.
    columns: f32,
    rows: f32,
    /// How far into its first cell it starts, as parts of a cell.
    offset: (f32, f32),
    /// Which pictures it covers: those with lower numbers.
    layer: i32,
    /// Whether it belongs to the screen a full-screen program uses.
    alternate: bool,
}

/// How much of what a task printed is kept.
const PRINTED_MOST: usize = 8 << 20;
/// What starts a synchronized update.
const UPDATE_START: &[u8] = b"\x1b[?2026h";

/// A program running on a PTY, and the screen it has drawn.
pub struct Session {
    pub name: String,
    /// A plain shell, which runs what it's sent. Comments must never reach one.
    pub shell: bool,
    /// A browser, which is neither something to send comments to nor work to lose.
    pub browser: bool,
    term: Term<Events>,
    parser: Processor,
    events: Events,
    writer: Box<dyn Write + Send>,
    link: Link,
    /// Left running if Cellix goes without saying otherwise, as when a connection drops.
    keep: bool,
    output: Receiver<Vec<u8>>,
    finished: Receiver<()>,
    pub exited: bool,
    /// When it last wrote anything, which says when it's settled.
    wrote: Option<std::time::Instant>,
    /// Something sent to it was refused: it has left megabytes unread, and more would only
    /// pile up. Taken by whoever sent it, to say so.
    refused: bool,
    size: (u16, u16),
    cell: CellPixels,
    selection: Option<(Spot, Spot)>,
    scanner: Scanner,
    protocol: Protocol,
    pictures: Vec<Picture>,
    /// What it was, when it stopped without finishing, so it can be resumed.
    pub stopped: Option<crate::hold::Stopped>,
    /// What the program put on the clipboard, as Claude Code does when text is selected in
    /// it, for Cellix to pass on to the terminal's clipboard.
    pub copied: Option<String>,
    /// Where each command entered at a shell's prompt starts, as (line counting from the top
    /// of the scrollback, column), so output can be traced to the command that printed it.
    commands: Vec<(usize, u16)>,
    /// Where the command being typed at the prompt starts, from its first key.
    typing: Option<(usize, u16)>,
    /// Everything it printed, kept for a task, whose errors are read from it once it ends.
    pub printed: Option<Vec<u8>>,
    /// The agent's session id and where its transcript is, once known.
    pub transcript: Option<(String, std::path::PathBuf)>,
    /// What the transcript says of its tokens, which the tab shows.
    pub usage: Option<crate::transcript::Usage>,
    /// When it started here, or was joined.
    pub born: std::time::Instant,
    /// What Find found here, each as its line counting from the top of the scrollback, its
    /// first column and how many it takes, and which of them Find is on.
    found: Vec<(usize, u16, u16)>,
    found_at: Option<usize>,
    /// What the program said of itself (OSC 7501) since it was last asked, in order, each
    /// with whether it was only said again as a joined host showed what it kept.
    pub reports: Vec<(crate::program::Report, bool)>,
    /// Joined again, so what comes first is what the host kept, said before.
    pub replaying: bool,
    /// Tells it from the others when nothing outside names it.
    serial: u64,
    /// The title the program last gave its terminal: an agent's names what it's working on.
    pub title: Option<String>,
    /// What the person named its tab, which outranks anything the program says.
    pub renamed: Option<String>,
    /// What its agent is doing, as its hooks last said: its tab's mark.
    pub doing: Doing,
    /// Its agent finished a turn while its tab wasn't in front: a dot until it's looked at.
    pub done_unseen: bool,
    /// How it stands among the agents here: the name they reach it by, and who started it.
    /// None for a shell or a browser.
    pub member: Option<crate::hold::Member>,
    /// What the person has typed into it, so a message from an agent waits for them.
    pub typed: Typed,
    /// When Cellix last typed a message in and entered it, until the program says it took it.
    pub prompted: Option<std::time::Instant>,
    /// For a shell, the program in front of it by name, and since when: what its tab is
    /// called while it runs, once it has run a moment.
    running: Option<(String, std::time::Instant)>,
    /// What its tab last showed of that, so a poll can say the tab changed.
    shown_running: Option<String>,
    /// For a shell Cellix runs itself, the process group last in front of it, its name, and
    /// when that was read.
    own_front: Option<(i32, String, std::time::Instant)>,
}

/// A person's typing into a session, as it holds back what agents send it.
#[derive(Clone, Copy, Debug, Default)]
pub struct Typed {
    /// Their last key or paste.
    pub last: Option<std::time::Instant>,
    /// Characters of a prompt they've begun and not yet sent or cleared.
    pub draft: usize,
}

impl Typed {
    /// How much longer a message must wait: until `quiet` after their last key, or with a
    /// prompt half typed, until they send or clear it, but no more than `draft` after their
    /// last key.
    pub fn hold(
        &self,
        quiet: std::time::Duration,
        draft: std::time::Duration,
    ) -> std::time::Duration {
        let Some(last) = self.last else {
            return std::time::Duration::ZERO;
        };
        let hold = if self.draft > 0 {
            quiet.max(draft)
        } else {
            quiet
        };
        hold.saturating_sub(last.elapsed())
    }

    fn key(&mut self, key: KeyEvent) {
        self.last = Some(std::time::Instant::now());
        let plain = !key.mods.ctrl && !key.mods.alt;
        match key.key {
            // Sent. Shift or Alt with Enter is a new line in most agents' prompts.
            Key::Enter if plain && !key.mods.shift => self.draft = 0,
            // Thrown away, as Ctrl+C and Ctrl+U clear a prompt.
            Key::Char('c' | 'C' | 'u' | 'U') if key.mods.ctrl => self.draft = 0,
            Key::Esc => {}
            Key::Backspace => self.draft = self.draft.saturating_sub(1),
            Key::Char(_) if plain => self.draft += 1,
            _ => {}
        }
    }

    fn pasted(&mut self, text: &str) {
        self.last = Some(std::time::Instant::now());
        self.draft += text.chars().count();
    }
}

/// How often the name of what's in front of a shell Cellix runs itself is read again.
const OWN_FRONT_EVERY: std::time::Duration = std::time::Duration::from_millis(250);
/// How long a command runs before its shell's tab is named after it and turns: one that ends
/// sooner doesn't make the tab flicker.
const RUN_SHOWN_AFTER: std::time::Duration = std::time::Duration::from_secs(1);

/// What an agent in a tab is doing, as its hooks say.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Doing {
    #[default]
    Idle,
    Working,
    NeedsYou,
}

/// The mark at the end of a tab's name, and before its ×: what the agent in it is doing, as
/// far as it wants anyone to know.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TabMark {
    /// Turning, in the accent.
    Working,
    /// A dot in the attention colour: it's asking.
    NeedsYou,
    /// A dot in green: it finished while its tab wasn't in front.
    Done,
}

/// Glyphs an agent puts before its title to show it's busy, as Claude Code's turning star
/// and Codex's braille do: one of them, and the spaces after it, aren't part of its name.
const TITLE_MARKS: [char; 10] = ['·', '✢', '✳', '✶', '✻', '✽', '◐', '◓', '◑', '◒'];

/// The name an agent's own title gives it, with the glyph that says it's busy taken off the
/// front: one braille letter or one of `TITLE_MARKS`, and the spaces after it.
pub fn own_name(title: &str) -> Option<String> {
    let title = title.trim();
    let mut chars = title.chars();
    let rest = match chars.next() {
        Some(c) if ('\u{2800}'..='\u{28ff}').contains(&c) || TITLE_MARKS.contains(&c) => {
            chars.as_str()
        }
        _ => title,
    };
    let name = rest.trim();
    (!name.is_empty()).then(|| name.to_string())
}

/// Cellix's own mark for a program in the side pane, by what the pane calls it.
pub fn program_icon(name: &str, shell: bool, browser: bool) -> Icon {
    match name {
        _ if shell => Icon::Terminal,
        _ if browser => Icon::Globe,
        "Claude Code" => Icon::AgentClaude,
        "Codex" => Icon::AgentCodex,
        "Gemini CLI" | "Gemini" => Icon::AgentGemini,
        _ => Icon::Program,
    }
}

/// The colour a program's mark is drawn in: Claude Code's warm clay, Gemini CLI's blue, a
/// shell's and any other program's in the quieter text colour, Codex's in the text's own.
pub fn icon_colour(icon: Icon, theme: &Theme) -> Rgb {
    match icon {
        Icon::AgentClaude => theme.claude_mark(),
        Icon::AgentGemini => theme.gemini_mark(),
        Icon::AgentCodex => theme.text.fg,
        _ => theme.tab.fg,
    }
}

/// The colour of a tab's mark: the accent while it works, the attention colour when it asks,
/// and green when it's done.
pub fn mark_colour(mark: TabMark, theme: &Theme) -> Rgb {
    match mark {
        TabMark::Working => theme.accent,
        TabMark::NeedsYou => theme.attention,
        TabMark::Done => theme.git_added,
    }
}

/// Numbers sessions, for those with no host to name them.
static SERIAL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

fn pty_size((columns, rows): (u16, u16), (width, height): CellPixels) -> PtySize {
    PtySize {
        rows,
        cols: columns,
        pixel_width: columns.saturating_mul(width),
        pixel_height: rows.saturating_mul(height),
    }
}

impl Session {
    /// Starts `argv` in `folder`. `env` is `KEY=value` pairs on top of Cellix's own.
    pub fn start(
        name: &str,
        argv: &[String],
        env: &[String],
        folder: &Path,
        size: (u16, u16),
        cell: CellPixels,
    ) -> io::Result<Self> {
        let pair = native_pty_system()
            .openpty(pty_size(size, cell))
            .map_err(failed)?;
        let (program, args) = argv.split_first().ok_or(io::ErrorKind::InvalidInput)?;
        let mut command = CommandBuilder::new(program);
        command.args(args);
        command.cwd(folder);
        command.env("TERM", "xterm-256color");
        command.env("COLORTERM", "truecolor");
        for pair in env {
            if let Some((key, value)) = pair.split_once('=') {
                command.env(key, value);
            }
        }
        let mut child = pair.slave.spawn_command(command).map_err(failed)?;
        drop(pair.slave);
        let killer = child.clone_killer();
        let pid = child.process_id();
        let mut reader = pair.master.try_clone_reader().map_err(failed)?;
        // Written off the loop, so a program that isn't reading can't hold it up.
        let writer = pair.master.take_writer().map_err(failed)?;
        let writer: Box<dyn Write + Send> = Box::new(crate::outbox::Outbox::new(writer));
        let (sender, output) = mpsc::channel();
        std::thread::spawn(move || {
            let mut buffer = [0; 8192];
            while let Ok(count) = reader.read(&mut buffer) {
                if count == 0 || sender.send(buffer[..count].to_vec()).is_err() {
                    break;
                }
            }
        });
        let (done, finished) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = child.wait();
            let _ = done.send(());
        });
        let link = Link::Own {
            master: pair.master,
            killer,
            pid,
        };
        Ok(Self::around(
            name,
            link,
            writer,
            (output, finished),
            (size, cell),
        ))
    }

    /// Joins a program running under a host of its own.
    pub fn join(name: &str, held: Held, size: (u16, u16), cell: CellPixels) -> io::Result<Self> {
        let writer = Box::new(held.keys()?);
        let (sender, output) = mpsc::channel();
        let (done, finished) = mpsc::channel();
        held.listen(sender, done)?;
        held.resize(
            size,
            (size.0.saturating_mul(cell.0), size.1.saturating_mul(cell.1)),
        );
        let channels = (output, finished);
        let mut session = Self::around(name, Link::Held(held), writer, channels, (size, cell));
        session.keep = true;
        Ok(session)
    }

    fn around(
        name: &str,
        link: Link,
        writer: Box<dyn Write + Send>,
        (output, finished): (Receiver<Vec<u8>>, Receiver<()>),
        (size, cell): ((u16, u16), CellPixels),
    ) -> Self {
        let events = Events::default();
        let dimensions = Size {
            columns: size.0 as usize,
            lines: size.1 as usize,
        };
        Self {
            shell: name == crate::agents::SHELL.0,
            browser: name == BROWSER,
            name: name.to_string(),
            term: Term::new(Config::default(), &dimensions, events.clone()),
            parser: Processor::new(),
            events,
            writer,
            link,
            keep: false,
            output,
            finished,
            exited: false,
            born: std::time::Instant::now(),
            found: Vec::new(),
            found_at: None,
            wrote: None,
            refused: false,
            size,
            cell,
            selection: None,
            scanner: Scanner::default(),
            protocol: Protocol::default(),
            pictures: Vec::new(),
            stopped: None,
            copied: None,
            commands: Vec::new(),
            typing: None,
            printed: None,
            transcript: None,
            usage: None,
            reports: Vec::new(),
            replaying: false,
            serial: SERIAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            title: None,
            renamed: None,
            doing: Doing::Idle,
            done_unseen: false,
            member: None,
            typed: Typed::default(),
            prompted: None,
            running: None,
            shown_running: None,
            own_front: None,
        }
    }

    /// What its tab is called: what the person named it, or for an agent the name its own
    /// title gives it, or else the program's name.
    pub fn label(&self) -> String {
        if let Some(name) = &self.renamed {
            return name.clone();
        }
        // A shell's is the program running in it, as VS Code names a terminal's tab.
        if let Some(program) = self.running_program() {
            return program.to_string();
        }
        let own = (!self.shell && !self.browser)
            .then(|| self.title.as_deref().and_then(own_name))
            .flatten();
        own.unwrap_or_else(|| self.name.clone())
    }

    /// The mark its tab shows after its name, if any: nothing while idle or once it's ended.
    pub fn mark(&self) -> Option<TabMark> {
        if self.exited {
            return None;
        }
        match self.doing {
            Doing::Working => Some(TabMark::Working),
            Doing::NeedsYou => Some(TabMark::NeedsYou),
            // A shell running a command turns, as an agent at work does.
            Doing::Idle if self.running_program().is_some() => Some(TabMark::Working),
            Doing::Idle => self.done_unseen.then_some(TabMark::Done),
        }
    }

    /// The program a shell is running, by name, once it has run long enough to say so.
    pub fn running_program(&self) -> Option<&str> {
        let (name, since) = self.running.as_ref()?;
        (since.elapsed() >= RUN_SHOWN_AFTER).then_some(name.as_str())
    }

    /// The name of the program in front of a shell, empty while the shell is: as its host
    /// says, or for a shell Cellix runs itself, from the PTY. A console can't say, so on
    /// Windows, which has no host, it's always empty.
    fn foreground(&mut self) -> String {
        match &self.link {
            #[cfg(unix)]
            Link::Held(held) => held.foreground(),
            #[cfg(not(unix))]
            Link::Held(held) => match *held {},
            #[cfg(unix)]
            Link::Own { master, pid, .. } => {
                let leader = master.process_group_leader();
                let Some(leader) = leader.filter(|l| Some(*l as u32) != *pid) else {
                    self.own_front = None;
                    return String::new();
                };
                // Read again a few times a second rather than once per group: the group is in
                // front from the moment the shell forks its leader, which has the shell's own
                // name until it has started the command.
                let stale = self.own_front.as_ref().is_none_or(|(group, _, read)| {
                    *group != leader || read.elapsed() >= OWN_FRONT_EVERY
                });
                if stale {
                    let name = crate::hold::program_name(leader as u32).unwrap_or_default();
                    self.own_front = Some((leader, name, std::time::Instant::now()));
                }
                self.own_front
                    .as_ref()
                    .map(|(_, name, _)| name.clone())
                    .unwrap_or_default()
            }
            #[cfg(not(unix))]
            Link::Own { .. } => String::new(),
            Link::Gone => String::new(),
        }
    }

    /// Notes what runs in front of a shell. Returns true when its tab should change.
    fn follow_foreground(&mut self) -> bool {
        if !self.shell || self.exited {
            self.running = None;
        } else {
            let now = self.foreground();
            if now.is_empty() {
                self.running = None;
            } else if self.running.as_ref().is_none_or(|(name, _)| *name != now) {
                self.running = Some((now, std::time::Instant::now()));
            }
        }
        let shown = self.running_program().map(str::to_string);
        if shown == self.shown_running {
            return false;
        }
        self.shown_running = shown;
        true
    }

    /// Whether its tab keeps room for what it's doing: an agent's while it runs, whose state
    /// its hooks report, and a shell's while what runs in it says how it is (OSC 7501).
    pub fn marked(&self) -> bool {
        !self.browser && !self.exited && (!self.shell || self.mark().is_some())
    }

    /// Whether its tab has its program's mark before its name: an agent's or another
    /// program's, and a shell's while a command runs in it. A shell at its prompt and the
    /// browser are names alone, as they were.
    pub fn has_icon(&self) -> bool {
        !self.browser && (!self.shell || self.running_program().is_some())
    }

    /// Cellix's own mark for its program, as its tab and the Agents list show it.
    pub fn icon(&self) -> Icon {
        program_icon(&self.name, self.shell, self.browser)
    }

    /// A program that stopped without finishing, showing what it last showed.
    pub fn stopped(stopped: crate::hold::Stopped, size: (u16, u16), cell: CellPixels) -> Self {
        // Nothing can arrive: both senders are gone.
        let (_, output) = mpsc::channel();
        let (_, finished) = mpsc::channel();
        let name = stopped.job.name.clone();
        let channels = (output, finished);
        let writer = Box::new(io::sink());
        let mut session = Self::around(&name, Link::Gone, writer, channels, (size, cell));
        session.take_in(&stopped.output);
        // What it said of itself was said by a program that's gone.
        session.reports.clear();
        session.exited = true;
        session.stopped = Some(stopped);
        session
    }

    /// Whether it has started and gone quiet, so it's ready for what's typed at it.
    pub fn settled(&self, quiet: std::time::Duration) -> bool {
        self.wrote.is_some_and(|at| at.elapsed() >= quiet)
    }

    /// Whether this can be left running after Cellix goes.
    pub fn can_outlive(&self) -> bool {
        matches!(self.link, Link::Held(_)) && !self.exited
    }

    /// Whether comments can go here.
    /// Whether it runs a task, whose output is kept so its errors can be read once it ends.
    pub fn task(&self) -> bool {
        self.printed.is_some()
    }

    pub fn agent(&self) -> bool {
        !self.exited && !self.shell && !self.browser
    }

    /// What names it among the sessions here from one run to the next: its host's socket,
    /// or the record it left when it stopped, which is named the same.
    pub fn kept_as(&self) -> Option<String> {
        match &self.stopped {
            Some(stopped) => stopped.key(),
            None => Some(self.socket()?.file_stem()?.to_string_lossy().into_owned()),
        }
    }

    /// What names it while it runs: what it's kept as, or else its number here.
    pub fn tab_key(&self) -> String {
        self.kept_as()
            .unwrap_or_else(|| format!("tab-{}", self.serial))
    }

    /// The socket of the host it runs under, beside which its agent's hooks leave word.
    pub fn socket(&self) -> Option<&Path> {
        match &self.link {
            #[cfg(unix)]
            Link::Held(held) => Some(held.socket.as_path()),
            // Nothing is held off Unix: there `Held` has no values.
            #[cfg(not(unix))]
            Link::Held(held) => match *held {},
            _ => None,
        }
    }

    /// How full its agent's context is, as the tab says it: `62%`.
    pub fn context(&self) -> Option<String> {
        let percent = self.usage.as_ref()?.percent()?;
        Some(format!("{percent}%"))
    }

    /// Takes in what the program wrote, with its pictures lifted out in order, so a reply
    /// goes back ahead of anything the terminal says about what came after. Says whether
    /// any of it reached the screen: text inside a synchronized update waits in the parser
    /// until the update ends, and changes nothing until then.
    fn take_in(&mut self, bytes: &[u8]) -> bool {
        let mut shown = false;
        for piece in self.scanner.feed(bytes) {
            match piece {
                Piece::Text(text) => {
                    self.answer_about_cells(&text);
                    self.forget_on_clear(&text);
                    let (was, held) = (self.updating(), self.parser.sync_bytes_count());
                    self.parser.advance(&mut self.term, &text);
                    let kept = self.parser.sync_bytes_count();
                    shown |= match (was, self.updating()) {
                        // Not holding now: whatever came is on the screen.
                        (_, false) => true,
                        // All of it held, unless an update ended and another began.
                        (true, true) => kept < held + text.len(),
                        // An update began: anything besides its start went to the screen.
                        (false, true) => text.len() > kept + UPDATE_START.len(),
                    };
                }
                Piece::Graphics(command) => {
                    // Text inside a synchronized update waits in the parser, cursor moves
                    // included, and a picture goes wherever the cursor is.
                    self.parser.stop_sync(&mut self.term);
                    let outcome = self.protocol.command(&command);
                    self.follow(outcome);
                    shown = true;
                }
                Piece::Status(payload) => {
                    if let Some(report) = crate::program::parse(&payload) {
                        self.reports.push((report, self.replaying));
                    }
                }
            }
        }
        shown
    }

    /// Whether a synchronized update is under way, its text held in the parser.
    fn updating(&self) -> bool {
        self.parser.sync_timeout().sync_timeout().is_some()
    }

    /// When a synchronized update under way runs out of time, if one is.
    pub fn update_deadline(&self) -> Option<std::time::Instant> {
        self.parser.sync_timeout().sync_timeout()
    }

    /// Ends a synchronized update its program never ended, once its time is up, as a
    /// terminal does. The parser only checks the time when more comes, so a program killed
    /// halfway through one left its pane as it was until two megabytes more arrived. Says
    /// whether it ended one.
    fn end_overdue_update(&mut self, now: std::time::Instant) -> bool {
        if self
            .update_deadline()
            .is_some_and(|deadline| now >= deadline)
        {
            self.parser.stop_sync(&mut self.term);
            return true;
        }
        false
    }

    /// The one size question the terminal emulator leaves unanswered.
    fn answer_about_cells(&mut self, text: &[u8]) {
        if self.cell.0 > 0 && contains(text, b"\x1b[16t") {
            let reply = format!("\x1b[6;{};{}t", self.cell.1, self.cell.0);
            self.write(reply.as_bytes());
        }
    }

    /// Clearing a screen takes its pictures with it.
    fn forget_on_clear(&mut self, text: &[u8]) {
        if contains(text, b"\x1b[2J") || contains(text, b"\x1b[3J") || contains(text, b"\x1bc") {
            let alternate = self.term.mode().contains(TermMode::ALT_SCREEN);
            self.pictures.retain(|p| p.alternate != alternate);
        }
        // The scrollback goes, and with it the lines the commands were on.
        if contains(text, b"\x1b[3J") || contains(text, b"\x1bc") {
            self.commands.clear();
            self.typing = None;
        }
    }

    fn follow(&mut self, outcome: Outcome) {
        if let Some(reply) = outcome.reply {
            self.write(reply.as_bytes());
        }
        match outcome.delete {
            Some(Delete::All) => self.pictures.clear(),
            Some(Delete::Image(image)) => self.pictures.retain(|p| p.image != image),
            Some(Delete::Placement(image, placement)) => {
                let same = |p: &Picture| p.image == image && p.placement == placement;
                self.pictures.retain(|p| !same(p));
            }
            None => {}
        }
        if let Some(shown) = outcome.show {
            self.place(shown);
        }
    }

    /// Puts a picture at the cursor, in place of the one it updates.
    fn place(&mut self, shown: Shown) {
        let (width, height) = (self.cell.0.max(1) as f32, self.cell.1.max(1) as f32);
        let columns = shown
            .columns
            .map_or(shown.bitmap.width as f32 / width, f32::from);
        let rows = shown
            .rows
            .map_or(shown.bitmap.height as f32 / height, f32::from);
        let grid = self.term.grid();
        let cursor = grid.cursor.point;
        let line = grid.history_size() + cursor.line.0.max(0) as usize;
        let same = |p: &Picture| p.image == shown.image && p.placement == shown.placement;
        self.pictures.retain(|p| !same(p));
        if self.pictures.len() >= MAX_PICTURES {
            self.pictures.remove(0);
        }
        self.pictures.push(Picture {
            image: shown.image,
            placement: shown.placement,
            bitmap: shown.bitmap,
            line,
            column: cursor.column.0 as u16,
            columns,
            rows,
            offset: (
                f32::from(shown.offset.0) / width,
                f32::from(shown.offset.1) / height,
            ),
            layer: shown.layer,
            alternate: self.term.mode().contains(TermMode::ALT_SCREEN),
        });
        if !shown.cursor_stays {
            let down = "\r\n".repeat(rows.ceil() as usize);
            self.parser.advance(&mut self.term, down.as_bytes());
        }
    }

    fn draw_pictures(&self, grid: &mut Grid, area: Rect, theme: &Theme) {
        let alternate = self.term.mode().contains(TermMode::ALT_SCREEN);
        let top = self.term.grid().history_size() as i64 - self.term.grid().display_offset() as i64;
        let mut pictures: Vec<&Picture> = self.pictures.iter().collect();
        pictures.retain(|p| p.alternate == alternate);
        pictures.sort_by_key(|p| p.layer);
        for picture in pictures {
            let row = picture.line as i64 - top;
            if row >= area.h as i64 || row + (picture.rows.ceil() as i64) < 0 {
                continue;
            }
            grid.paint.push(Paint::Image {
                bounds: Bounds::cells(
                    (area.x + picture.column) as f32 + picture.offset.0,
                    area.y as f32 + row as f32 + picture.offset.1,
                    picture.columns,
                    picture.rows,
                ),
                clip: Bounds::inset(area, 0.0, 0.0),
                bitmap: picture.bitmap.clone(),
                color: theme.text.fg,
            });
        }
    }

    /// Whether closing this would stop something: an agent, or a shell running a command
    /// rather than waiting at its prompt.
    pub fn busy(&self) -> bool {
        if self.exited || self.browser {
            return false;
        }
        !self.shell || !self.waits_at_prompt()
    }

    /// Whether the program in front is the shell itself, waiting for a command.
    fn waits_at_prompt(&self) -> bool {
        match &self.link {
            #[cfg(unix)]
            Link::Own { master, pid, .. } => {
                let front = master.process_group_leader();
                front.is_some() && front.map(|p| p as u32) == *pid
            }
            // A console can't say what's in front of it, so a shell closes as Windows Terminal
            // closes one, without asking. An agent still asks.
            #[cfg(not(unix))]
            Link::Own { .. } => true,
            Link::Held(held) => held.at_prompt(),
            Link::Gone => true,
        }
    }

    /// Whether what's typed now goes to a shell's prompt, as a command.
    fn typing_a_command(&self) -> bool {
        let alternate = self.term.mode().contains(TermMode::ALT_SCREEN);
        self.shell && !self.exited && !alternate && self.waits_at_prompt()
    }

    /// Where the cursor is, counting lines from the top of the scrollback.
    fn cursor_place(&self) -> (usize, u16) {
        let grid = self.term.grid();
        let point = grid.cursor.point;
        let line = grid.history_size() as i32 + point.line.0;
        (line.max(0) as usize, point.column.0 as u16)
    }

    /// Notes where a command starts, from the first key typed at the prompt.
    fn note_typing(&mut self) {
        if self.typing.is_none() && self.typing_a_command() {
            self.typing = Some(self.cursor_place());
        }
    }

    /// The command typed at the prompt was entered.
    fn note_entered(&mut self) {
        if let Some(start) = self.typing.take() {
            self.commands.push(start);
        }
    }

    /// Sends the program `bytes`, in order with all else sent it, without waiting on it:
    /// what it hasn't read yet waits on a thread of its own (`outbox.rs`).
    pub fn write(&mut self, bytes: &[u8]) {
        self.refused |= self.writer.write_all(bytes).is_err();
        let _ = self.writer.flush();
    }

    /// Whether something sent since this was last asked was refused, unread.
    pub fn take_refused(&mut self) -> bool {
        std::mem::take(&mut self.refused)
    }

    /// Takes in what the program wrote. Returns true if the screen changed.
    pub fn poll(&mut self, theme: &Theme) -> bool {
        let mut changed = self.end_overdue_update(std::time::Instant::now());
        let mut came = false;
        while let Ok(bytes) = self.output.try_recv() {
            changed |= self.take_in(&bytes);
            if let Some(kept) = &mut self.printed {
                kept.extend_from_slice(&bytes);
                // A task that prints without end keeps its last few megabytes.
                if kept.len() > PRINTED_MOST {
                    kept.drain(..kept.len() - PRINTED_MOST / 2);
                }
            }
            self.wrote = Some(std::time::Instant::now());
            came = true;
        }
        // What a joined host kept comes first, all at once.
        self.replaying &= !came;
        let events = self.events.0.lock().map(|mut e| std::mem::take(&mut *e));
        for event in events.unwrap_or_default() {
            match event {
                TermEvent::PtyWrite(text) => self.write(text.as_bytes()),
                // A program that takes the mouse selects and copies for itself, through the
                // terminal's clipboard, which here is Cellix.
                TermEvent::ClipboardStore(_, text) => {
                    self.copied = Some(text);
                    changed = true;
                }
                TermEvent::ColorRequest(index, format) => {
                    let color = asked_colour(index, theme);
                    let reply = format(TermRgb {
                        r: color.0,
                        g: color.1,
                        b: color.2,
                    });
                    self.write(reply.as_bytes());
                }
                TermEvent::Title(title) => {
                    changed |= self.title.as_deref() != Some(title.as_str());
                    self.title = Some(title);
                }
                TermEvent::ResetTitle => {
                    changed |= self.title.take().is_some();
                }
                TermEvent::TextAreaSizeRequest(format) => {
                    let reply = format(WindowSize {
                        num_lines: self.size.1,
                        num_cols: self.size.0,
                        cell_width: self.cell.0.max(1),
                        cell_height: self.cell.1.max(1),
                    });
                    self.write(reply.as_bytes());
                }
                _ => {}
            }
        }
        if !self.exited && self.finished.try_recv().is_ok() {
            self.exited = true;
            changed = true;
        }
        changed | self.follow_foreground()
    }

    pub fn resize(&mut self, size: (u16, u16), cell: CellPixels) {
        if (size == self.size && cell == self.cell) || size.0 == 0 || size.1 == 0 {
            return;
        }
        self.size = size;
        self.cell = cell;
        self.selection = None;
        match &self.link {
            Link::Own { master, .. } => {
                let _ = master.resize(pty_size(size, cell));
            }
            Link::Held(held) => {
                let pixels = pty_size(size, cell);
                held.resize(size, (pixels.pixel_width, pixels.pixel_height));
            }
            Link::Gone => {}
        }
        self.term.resize(Size {
            columns: size.0 as usize,
            lines: size.1 as usize,
        });
    }

    pub fn key(&mut self, key: KeyEvent) {
        self.typed.key(key);
        let application = self.term.mode().contains(TermMode::APP_CURSOR);
        let bytes = key_bytes(key, application);
        self.term.scroll_display(Scroll::Bottom);
        self.selection = None;
        match key.key {
            Key::Enter => self.note_entered(),
            // The line is thrown away, and the next prompt starts a new one.
            Key::Char('c' | 'C') if key.mods.ctrl => self.typing = None,
            _ => self.note_typing(),
        }
        self.write(&bytes);
    }

    /// What the person pasted.
    pub fn paste(&mut self, text: &str) {
        self.typed.pasted(text);
        self.put(text);
    }

    /// Text as a paste: bracketed where the program asked for that.
    fn put(&mut self, text: &str) {
        self.note_typing();
        self.term.scroll_display(Scroll::Bottom);
        let text = &pasteable(text);
        if self.term.mode().contains(TermMode::BRACKETED_PASTE) {
            self.write(b"\x1b[200~");
            self.write(text.as_bytes());
            self.write(b"\x1b[201~");
        } else {
            self.write(text.replace('\n', "\r").as_bytes());
        }
    }

    /// Sends a whole message, as if typed and entered.
    pub fn send_line(&mut self, text: &str) {
        self.put(text);
        self.note_entered();
        self.write(b"\r");
    }

    /// Types a message from an agent in, as a paste, for `submit` to enter a moment later, so
    /// the program sees the two apart.
    pub fn deliver(&mut self, text: &str) {
        self.put(text);
    }

    /// Enters what `deliver` typed, and whatever the person had begun with it.
    pub fn submit(&mut self) {
        self.note_entered();
        self.write(b"\r");
        self.typed.draft = 0;
        self.prompted = Some(std::time::Instant::now());
    }

    /// The process its program runs as, where that can be known: what a hook's report names
    /// to say which session it's about.
    pub fn program_pid(&self) -> Option<u32> {
        match &self.link {
            Link::Own { pid, .. } => *pid,
            Link::Held(_) => {
                let text = std::fs::read_to_string(self.socket()?.with_extension("pid")).ok()?;
                text.trim().parse().ok()
            }
            Link::Gone => None,
        }
    }

    /// The name other agents reach it by.
    pub fn called(&self) -> Option<&str> {
        self.member.as_ref().map(|m| m.name.as_str())
    }

    fn wants_mouse(&self) -> bool {
        self.term.mode().intersects(TermMode::MOUSE_MODE)
    }

    /// Reports the mouse to a program that asked, in the SGR form they all read.
    fn report_mouse(&mut self, button: u8, (x, y): Spot, press: bool) {
        let end = if press { 'M' } else { 'm' };
        let report = format!("\x1b[<{button};{};{}{end}", x + 1, y + 1);
        self.write(report.as_bytes());
    }

    pub fn wheel(&mut self, up: bool, spot: Spot) {
        if self.wants_mouse() {
            self.report_mouse(if up { 64 } else { 65 }, spot, true);
        } else if self.term.mode().contains(TermMode::ALT_SCREEN) {
            let key = KeyEvent::plain(if up { Key::Up } else { Key::Down });
            for _ in 0..WHEEL_LINES {
                self.key(key);
            }
        } else {
            let lines = if up { WHEEL_LINES } else { -WHEEL_LINES };
            self.term.scroll_display(Scroll::Delta(lines));
        }
    }

    /// A press, drag or release at `spot`. Text selected here is returned on release, so
    /// the pane copies the way a terminal does.
    pub fn mouse(&mut self, kind: MouseKind, spot: Spot) -> Option<String> {
        if self.wants_mouse() {
            match kind {
                MouseKind::Left => self.report_mouse(0, spot, true),
                MouseKind::Up => self.report_mouse(0, spot, false),
                MouseKind::Drag => self.report_mouse(32, spot, true),
                MouseKind::Right => {
                    self.report_mouse(2, spot, true);
                    self.report_mouse(2, spot, false);
                }
                MouseKind::Move if self.term.mode().contains(TermMode::MOUSE_MOTION) => {
                    self.report_mouse(35, spot, true);
                }
                _ => {}
            }
            return None;
        }
        match kind {
            MouseKind::Left => self.selection = Some((spot, spot)),
            MouseKind::Drag => {
                if let Some(selection) = &mut self.selection {
                    selection.1 = spot;
                }
            }
            MouseKind::Up => {
                let text = self.selected_text();
                if text.is_none() {
                    self.selection = None;
                }
                return text;
            }
            _ => {}
        }
        None
    }

    /// The selection in reading order.
    fn ordered(&self) -> Option<(Spot, Spot)> {
        let (a, b) = self.selection?;
        Some(if (a.1, a.0) <= (b.1, b.0) {
            (a, b)
        } else {
            (b, a)
        })
    }

    fn selected_text(&self) -> Option<String> {
        let (from, to) = self.ordered()?;
        if from == to {
            return None;
        }
        let rows = self.rows();
        let mut lines = Vec::new();
        for y in from.1..=to.1 {
            let chars: Vec<char> = rows.get(y as usize)?.chars().collect();
            let start = if y == from.1 { from.0 as usize } else { 0 };
            let end = if y == to.1 {
                (to.0 as usize + 1).min(chars.len())
            } else {
                chars.len()
            };
            let line: String = chars.get(start..end).unwrap_or_default().iter().collect();
            lines.push(line.trim_end().to_string());
        }
        Some(lines.join("\n"))
    }

    /// What's selected and where it is, as the first and last places in reading order,
    /// while something is.
    pub fn selection(&self) -> Option<(String, Spot, Spot)> {
        let (from, to) = self.ordered()?;
        let text = self.selected_text().filter(|t| !t.trim().is_empty())?;
        Some((text, from, to))
    }

    pub fn clear_selection(&mut self) {
        self.selection = None;
    }

    /// The command entered at the shell's prompt that printed the visible row `row`, when
    /// the shell was at its prompt as it was typed.
    pub fn command_before(&self, row: u16) -> Option<String> {
        let grid = self.term.grid();
        let top = grid.history_size() as i32 - grid.display_offset() as i32;
        let line = (top + row as i32).max(0) as usize;
        let &(start, column) = self.commands.iter().rev().find(|(at, _)| *at <= line)?;
        let command = self.logical_line(start, column);
        let command = command.trim();
        (!command.is_empty()).then(|| command.to_string())
    }

    /// The text from a place to the end of its line, following it across rows it wrapped to.
    fn logical_line(&self, start: usize, column: u16) -> String {
        let grid = self.term.grid();
        let history = grid.history_size() as i32;
        let columns = self.term.columns();
        let last = self.term.screen_lines() as i32 - 1;
        let mut text = String::new();
        let mut line = start as i32 - history;
        let mut from = column as usize;
        while line <= last && line >= -history {
            let row = &grid[Line(line)];
            for x in from.min(columns)..columns {
                let cell = &row[Column(x)];
                if !cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
                    text.push(cell.c);
                }
            }
            let wrapped = row[Column(columns - 1)].flags.contains(Flags::WRAPLINE);
            if !wrapped {
                break;
            }
            line += 1;
            from = 0;
        }
        text.trim_end().to_string()
    }

    /// The `file:line` or file under a column of a visible row, as tracebacks and compilers
    /// print them.
    pub fn path_at(&self, (x, y): Spot) -> Option<(String, Option<usize>)> {
        let rows = self.rows();
        let chars: Vec<char> = rows.get(y as usize)?.chars().collect();
        let x = x as usize;
        if chars.get(x).is_none_or(path_edge) {
            return None;
        }
        let start = chars[..x].iter().rposition(path_edge).map_or(0, |i| i + 1);
        let end = chars[x..]
            .iter()
            .position(path_edge)
            .map_or(chars.len(), |i| x + i);
        let word: String = chars[start..end].iter().collect();
        place_of(&word)
    }

    /// Every line it holds as text, the scrollback's first and the screen's after them.
    pub fn lines(&self) -> Vec<String> {
        let grid = self.term.grid();
        let history = grid.history_size() as i32;
        let (columns, screen) = (self.term.columns(), self.term.screen_lines() as i32);
        (-history..screen)
            .map(|y| {
                let line = (0..columns)
                    .map(|x| grid[Point::new(Line(y), Column(x))].c)
                    .collect::<String>();
                line.trim_end().to_string()
            })
            .collect()
    }

    /// Whether the program keeps a screen of its own, as Claude Code does: then there's no
    /// scrollback here, only what it shows now.
    pub fn keeps_own_screen(&self) -> bool {
        self.term.mode().contains(TermMode::ALT_SCREEN)
    }

    /// Marks what Find found, and scrolls so the one it's on is in view.
    pub fn show_found(&mut self, found: Vec<(usize, u16, u16)>, at: Option<usize>) {
        self.found = found;
        self.found_at = at;
        let Some(&(line, ..)) = at.and_then(|at| self.found.get(at)) else {
            return;
        };
        let grid = self.term.grid();
        let history = grid.history_size() as i32;
        let top = history - grid.display_offset() as i32;
        let screen = self.term.screen_lines() as i32;
        let line = line as i32;
        if line < top || line >= top + screen {
            // A third of the way down, so what led to it shows too.
            let wanted = (line - screen / 3).clamp(0, history);
            self.term.scroll_display(Scroll::Delta(top - wanted));
        }
    }

    /// Lets go of what Find marked.
    pub fn clear_found(&mut self) {
        self.found.clear();
        self.found_at = None;
    }

    /// The screen as text, one string a row.
    pub fn rows(&self) -> Vec<String> {
        let offset = self.term.grid().display_offset() as i32;
        let (columns, lines) = (self.term.columns(), self.term.screen_lines());
        (0..lines as i32)
            .map(|y| {
                let line = (0..columns)
                    .map(|x| {
                        let point = Point::new(Line(y - offset), Column(x));
                        self.term.grid()[point].c
                    })
                    .collect::<String>();
                line.trim_end().to_string()
            })
            .collect()
    }

    fn color(&self, color: Color, theme: &Theme, default: Rgb) -> Rgb {
        let palette = palette_of(theme);
        match color {
            Color::Spec(c) => Rgb(c.r, c.g, c.b),
            Color::Indexed(index) => indexed(index, palette),
            Color::Named(NamedColor::Foreground | NamedColor::BrightForeground) => default,
            Color::Named(NamedColor::Background | NamedColor::Cursor) => default,
            Color::Named(NamedColor::DimForeground) => theme.gutter.fg,
            Color::Named(named) => palette.get(named as usize).copied().unwrap_or(default),
        }
    }

    pub fn draw(&self, grid: &mut Grid, area: Rect, theme: &Theme, focused: bool) {
        grid.fill(area, pane_of(theme));
        let selection = self.ordered().filter(|(a, b)| a != b);
        let picked = |x: u16, y: u16| {
            selection.is_some_and(|(from, to)| (y, x) >= (from.1, from.0) && (y, x) <= (to.1, to.0))
        };
        let content = self.term.renderable_content();
        let offset = content.display_offset as i32;
        // What Find found on a cell: whether it's the one Find is on.
        let top = self.term.grid().history_size() as i32 - offset;
        let found = |x: u16, y: u16| {
            let line = (top + i32::from(y)) as usize;
            let on = |(at, column, columns): &(usize, u16, u16)| {
                *at == line && x >= *column && x < column + columns
            };
            let index = self.found.iter().position(on)?;
            Some(Some(index) == self.found_at)
        };
        let cursor = content.cursor;
        let show_cursor = focused
            && content.mode.contains(TermMode::SHOW_CURSOR)
            && cursor.shape != CursorShape::Hidden
            && offset == 0;
        for indexed in content.display_iter {
            let (point, cell) = (indexed.point, indexed.cell);
            let (x, y) = (point.column.0 as u16, (point.line.0 + offset) as u16);
            if x >= area.w || y >= area.h {
                continue;
            }
            if cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
                continue;
            }
            let mut fg = self.color(cell.fg, theme, theme.text.fg);
            let mut bg = self.color(cell.bg, theme, theme.sidebar.bg);
            if cell.flags.contains(Flags::INVERSE) {
                std::mem::swap(&mut fg, &mut bg);
            }
            if cell.flags.contains(Flags::DIM) {
                fg = theme.gutter.fg;
            }
            if picked(x, y) {
                bg = theme.selection;
            }
            match found(x, y) {
                Some(true) => bg = theme.selection,
                Some(false) => bg = theme.found,
                None => {}
            }
            let at_cursor = show_cursor
                && point.line == cursor.point.line
                && point.column == cursor.point.column;
            if at_cursor {
                (fg, bg) = (theme.sidebar.bg, theme.text.fg);
            }
            let style = Style {
                fg,
                bg,
                bold: cell.flags.contains(Flags::BOLD),
                italic: cell.flags.contains(Flags::ITALIC),
                underline: cell.flags.intersects(Flags::ALL_UNDERLINES),
                painted: false,
            };
            let mut text = [0; 4];
            let shown: &str = if cell.c == '\0' {
                " "
            } else {
                cell.c.encode_utf8(&mut text)
            };
            if UnicodeWidthStr::width(shown) > 0 {
                grid.put_str(area.x + x, area.y + y, shown, style, area.right());
            }
        }
        self.draw_pictures(grid, area, theme);
    }

    /// Says a finished agent is finished, on the row under what it last showed.
    /// `plus` says whether the pane's + shows: out of room, it's in the tab's list instead.
    pub fn draw_ended(
        &self,
        grid: &mut Grid,
        area: Rect,
        another: &str,
        (theme, measure): (&Theme, &dyn cellix_render::Measure),
    ) {
        let row = Rect::new(area.x, area.bottom().saturating_sub(1), area.w, 1);
        let note = Style {
            fg: theme.gutter.fg,
            ..theme.tab
        };
        grid.fill(row, theme.tab);
        let whole = format!("{} has ended. Close this tab, or {another}.", self.name);
        // In a pane too narrow for all of it, what to do next is the part that stays.
        let room = f32::from(row.w.saturating_sub(2));
        let text = if measure.width(&whole, cellix_render::Font::default()) <= room {
            whole
        } else {
            let (first, rest) = another.split_at(1);
            format!("{} ended. {}{rest}.", self.name, first.to_uppercase())
        };
        grid.label(row.x + 1, row.y, &text, note, row.right());
    }

    pub fn stop(&mut self) {
        match &mut self.link {
            Link::Own { killer, .. } => {
                let _ = killer.kill();
            }
            Link::Held(held) => held.kill(),
            Link::Gone => {}
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        if !self.keep {
            self.stop();
        }
    }
}

fn palette_of(theme: &Theme) -> &'static [Rgb; 16] {
    if theme.is_light() { &LIGHT } else { &DARK }
}

/// What a session is drawn on: the pane's own colour, with the text's.
fn pane_of(theme: &Theme) -> Style {
    Style {
        fg: theme.text.fg,
        ..theme.sidebar
    }
}

/// The colour a program asks for. Programs read the background to choose their own look, so
/// these are the pane's, and a palette entry is what the pane really draws for that number.
fn asked_colour(index: usize, theme: &Theme) -> Rgb {
    match index {
        _ if index == NamedColor::Background as usize => theme.sidebar.bg,
        _ if index == NamedColor::Foreground as usize => theme.text.fg,
        _ if index == NamedColor::Cursor as usize => theme.text.fg,
        _ => indexed(index.min(255) as u8, palette_of(theme)),
    }
}

/// What ends a path in a line of output.
fn path_edge(c: &char) -> bool {
    c.is_whitespace() || "\"'`()[]<>,".contains(*c)
}

/// A word as a file and the line it names, as in `src/main.rs:12:5`.
fn place_of(word: &str) -> Option<(String, Option<usize>)> {
    let word = word.trim_end_matches(':');
    let mut parts = word.split(':');
    let file = parts.next().filter(|f| !f.is_empty())?.to_string();
    Some((file, parts.next().and_then(|n| n.parse().ok())))
}

/// Every `file:line` a piece of output names, in order, as tracebacks and compilers print
/// them.
pub fn places_in(text: &str) -> Vec<(String, usize)> {
    let mut places = Vec::new();
    for word in text.split(|c: char| path_edge(&c)) {
        // A sentence may end on one: `failed at src/main.rs:12.`
        if let Some((file, Some(line))) = place_of(word.trim_end_matches('.')) {
            places.push((file, line));
        }
    }
    places
}

fn contains(text: &[u8], wanted: &[u8]) -> bool {
    text.windows(wanted.len()).any(|w| w == wanted)
}

fn failed(error: impl std::fmt::Display) -> io::Error {
    io::Error::other(error.to_string())
}

/// The 256 colour table: the sixteen, a cube, and a run of greys.
fn indexed(index: u8, palette: &[Rgb; 16]) -> Rgb {
    match index {
        0..=15 => palette[index as usize],
        16..=231 => {
            let level = |n: u8| if n == 0 { 0 } else { 55 + n * 40 };
            let n = index - 16;
            Rgb(level(n / 36), level(n / 6 % 6), level(n % 6))
        }
        _ => {
            let grey = 8 + (index - 232) * 10;
            Rgb(grey, grey, grey)
        }
    }
}

/// The bytes a terminal sends for a key.
fn key_bytes(key: KeyEvent, application: bool) -> Vec<u8> {
    let KeyEvent { key, mods } = key;
    let number = 1 + u8::from(mods.shift) + 2 * u8::from(mods.alt) + 4 * u8::from(mods.ctrl);
    let csi = |end: char| {
        if number > 1 {
            format!("\x1b[1;{number}{end}")
        } else if application {
            format!("\x1bO{end}")
        } else {
            format!("\x1b[{end}")
        }
    };
    let tilde = |code: u8| {
        if number > 1 {
            format!("\x1b[{code};{number}~")
        } else {
            format!("\x1b[{code}~")
        }
    };
    let text = match key {
        Key::Char(c) if mods.ctrl && c.is_ascii() => {
            let code = (c.to_ascii_uppercase() as u8) & 0x1f;
            let mut bytes = if mods.alt { vec![0x1b] } else { Vec::new() };
            bytes.push(code);
            return bytes;
        }
        Key::Char(c) if mods.alt => format!("\x1b{c}"),
        Key::Char(c) => c.to_string(),
        Key::Enter if mods.alt || mods.shift => "\x1b\r".to_string(),
        Key::Enter => "\r".to_string(),
        Key::Tab => "\t".to_string(),
        Key::BackTab => "\x1b[Z".to_string(),
        Key::Backspace if mods.alt => "\x1b\x7f".to_string(),
        Key::Backspace => "\x7f".to_string(),
        Key::Esc => "\x1b".to_string(),
        Key::Up => csi('A'),
        Key::Down => csi('B'),
        Key::Right => csi('C'),
        Key::Left => csi('D'),
        Key::Home => csi('H'),
        Key::End => csi('F'),
        Key::Delete => tilde(3),
        Key::PageUp => tilde(5),
        Key::PageDown => tilde(6),
        Key::F(n @ 1..=4) => format!("\x1bO{}", (b'P' + n - 1) as char),
        Key::F(n) => {
            let codes = [15, 17, 18, 19, 20, 21, 23, 24];
            tilde(codes[(n.clamp(5, 12) - 5) as usize])
        }
    };
    text.into_bytes()
}

/// Where something at `index` ends up when one thing moves from `from` to `to`.
pub fn moved(index: usize, from: usize, to: usize) -> usize {
    if index == from {
        to
    } else if from < index && index <= to {
        index - 1
    } else if to <= index && index < from {
        index + 1
    } else {
        index
    }
}

/// A tab's width with a count after its name, when there's something to count.
fn counted_width(base: u16, count: usize) -> u16 {
    let digits = if count > 9 { COUNT_WIDTH } else { 1 };
    base + CLOSE_WIDTH + 1 + if count > 0 { digits + 1 } else { 0 }
}

/// What a click in the pane's title row lands on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TabHit {
    Tab(usize),
    Close(usize),
    New,
    /// The button that maximizes the pane, and restores it.
    Maximize,
    /// A view that comes and goes, and the × on its tab.
    View(Front),
    CloseView(Front),
    /// The pane's own close button, at the far end.
    Hide,
    /// The chevron on the tab in front, when the tabs don't all fit, which lists them all.
    Pick,
}

/// How the pane's tabs stand in their row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fit {
    /// Every tab whole, and the + after them.
    Whole,
    /// Every tab there: the one in front whole, those behind it cut short. The + gives way.
    Cut,
    /// Only the tab in front, with a chevron that lists them all.
    Front,
}

/// A tab of the side pane, as the list of every tab names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaneTab {
    View(Front),
    Session(usize),
}

/// What the pane shows besides its sessions.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Front {
    #[default]
    Session,
    Comments,
    Pdf,
    /// What the last build said.
    Log,
    /// Errors and warnings across the project.
    Problems,
    /// What can be started, and what's running elsewhere.
    Agents,
    /// What waits for the person, in every workspace.
    Inbox,
}

/// How far each view of the pane has scrolled, in its own rows. They're kept in one place and
/// turned one way, so a view can't be left out of scrolling, or scroll past its end.
#[derive(Default)]
pub struct Tops([usize; 7]);

impl Tops {
    pub fn of(&self, view: Front) -> usize {
        self.0[view as usize]
    }

    pub fn set(&mut self, view: Front, top: usize) {
        self.0[view as usize] = top;
    }

    /// A notch of the wheel: `step` rows, and never past `last`, the furthest the view goes
    /// with what's in it now.
    pub fn wheel(&mut self, view: Front, up: bool, (last, step): (usize, usize)) {
        let top = self.of(view).min(last);
        let top = if up {
            top.saturating_sub(step)
        } else {
            (top + step).min(last)
        };
        self.set(view, top);
    }
}

/// What a look at the sessions found.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Polled {
    /// Something on screen changed.
    pub shown: bool,
    /// Sessions out of sight printed.
    pub behind: bool,
}

/// The side pane: its tabs, and whether it's open and has the keyboard.
#[derive(Default)]
pub struct Dock {
    pub open: bool,
    pub focused: bool,
    pub sessions: Vec<Session>,
    /// The session in front. None shows the list of what can be started.
    pub active: Option<usize>,
    /// Set by dragging the pane's edge.
    pub width: Option<u16>,
    /// What stands in front of any session.
    pub front: Front,
    /// The PDF or image in the Preview.
    pub pdf: Option<PdfView>,
    /// The place on a page last pointed at, for a comment on it.
    pub pdf_spot: Option<(usize, f32, f32)>,
    /// How far each view has scrolled.
    pub tops: Tops,
    /// How many comments are open, which the tab shows.
    pub comment_count: Cell<usize>,
    /// A comment or reply being written.
    pub draft: Option<Draft>,
    /// What the last build said, and where its paths start.
    pub log: Vec<String>,
    /// Whether the problems have a tab.
    pub problems: bool,
    pub log_folder: std::path::PathBuf,
    /// Whether the list of agents has a tab.
    pub agents_tab: bool,
    /// Whether the comments have a tab.
    pub comments_tab: bool,
    /// Whether the inbox has a tab, and how much is in it.
    pub inbox_tab: bool,
    pub inbox_count: Cell<usize>,
    /// The column tabs stop at, for what stands past them.
    pub tabs_end: Cell<u16>,
    /// Columns kept clear between that and the icons past it, which names with no chip
    /// around them may use when the row is too short even for the button that maximizes the
    /// pane: an icon has air of its own inside its cell.
    pub tabs_slack: Cell<u16>,
    /// Whether the tabs start flush, under the file's, with no line beside them.
    pub tabs_flush: Cell<bool>,
    /// Rows the pane was dragged to, standing under the file.
    pub height: Option<u16>,
    /// When the hint on moving a page wider than the pane showed, and whether it's done
    /// with: it shows once a run.
    pub pan_hint: Option<std::time::Instant>,
    pub pan_hint_done: bool,
    /// Dragged under the file, where it stands even with room beside it. Kept while running.
    pub under: bool,
    /// The order of the views' tabs, once one has been dragged.
    pub view_order: Vec<Front>,
    /// Docked, icons at the start of the row stand for the comments and the agents, so
    /// those have no tabs of their own.
    pub docked: bool,
    /// Agents whose tabs closed as they ended, for the status bar to say.
    pub ended: Vec<String>,
    /// How the interface's text is measured, so a tab can be as wide as its name. Without
    /// one, a name is as many columns as it has letters, as on the cell tier.
    pub measure: Option<std::rc::Rc<dyn cellix_render::Measure>>,
    /// Agents whose tabs closed, with what they last showed, for the app to take.
    pub gone: Vec<Gone>,
}

/// Lines of what a closed agent showed that are kept for others to read.
const GONE_LINES: usize = 400;

/// An agent whose tab closed: who it was and what it last showed.
#[derive(Clone, Debug)]
pub struct Gone {
    pub member: crate::hold::Member,
    pub lines: Vec<String>,
    pub transcript: Option<std::path::PathBuf>,
    /// Whether its program had ended, rather than being closed.
    pub exited: bool,
}

/// An agent that ends this soon after starting didn't start: its tab stays, with what it said.
const FAILED_START: std::time::Duration = std::time::Duration::from_secs(5);

/// The views' tabs in the order they come in until one is dragged.
const VIEW_ORDER: [Front; 6] = [
    Front::Inbox,
    Front::Comments,
    Front::Agents,
    Front::Pdf,
    Front::Log,
    Front::Problems,
];

impl Dock {
    /// How wide the pane is in a window `room` columns wide past the sidebar.
    pub fn width_in(&self, room: u16) -> u16 {
        if !self.open || room < MIN_MAIN + MIN_WIDTH {
            return if self.open { room } else { 0 };
        }
        match self.width {
            // Dragged there: both sides keep enough to be read, and no more is asked.
            Some(dragged) => dragged.clamp(DRAGGED_LEAST, room - DRAGGED_LEAST),
            None => (room * SHARE / 100).clamp(MIN_WIDTH, room - MIN_MAIN),
        }
    }

    /// Whether there's too little room for the pane to stand beside the file.
    pub fn too_narrow(&self, room: u16) -> bool {
        self.open && room < MIN_MAIN + MIN_WIDTH
    }

    /// Whether this view is what the pane shows.
    pub fn showing(&self, view: Front) -> bool {
        self.open && (self.front == view || (view == Front::Agents && self.listing()))
    }

    pub fn session(&self) -> Option<&Session> {
        self.sessions.get(self.active?)
    }

    pub fn session_mut(&mut self) -> Option<&mut Session> {
        self.sessions.get_mut(self.active?)
    }

    /// The agent comments go to: the one in front, or the first still running. A shell would
    /// run a comment's words as commands, so it never counts.
    pub fn agent_mut(&mut self) -> Option<&mut Session> {
        let front = self.active.filter(|i| self.sessions[*i].agent());
        let index = front.or_else(|| self.sessions.iter().position(Session::agent))?;
        self.sessions.get_mut(index)
    }

    /// What to call the agent an answer will come from.
    pub fn agent_name(&self) -> String {
        let agent = self.sessions.iter().find(|s| s.agent());
        agent.map_or_else(|| "the agent".to_string(), |s| s.name.clone())
    }

    pub fn running(&self) -> bool {
        self.sessions.iter().any(|s| !s.exited)
    }

    /// The names of what would stop if the pane closed.
    /// What a program here last put on the clipboard, once.
    pub fn take_copied(&mut self) -> Option<String> {
        self.sessions.iter_mut().find_map(|s| s.copied.take())
    }

    pub fn busy(&self) -> Vec<String> {
        let busy = self.sessions.iter().filter(|s| s.busy());
        busy.map(|s| s.name.clone()).collect()
    }

    /// Takes in what every session wrote. Says whether anything on screen changed, and
    /// whether only sessions out of sight did: `shown` is the one the pane shows, if any.
    /// What a session behind another prints is parsed as it comes, but draws nothing until
    /// the tick, which brings what's drawn from it (its tab, the workspaces' tiles) up to
    /// date; its ending closes its tab, which is seen at once.
    pub fn poll(&mut self, theme: &Theme, shown: Option<usize>) -> Polled {
        let mut changed = self.pdf.as_mut().is_some_and(PdfView::poll);
        let mut behind = false;
        for (index, session) in self.sessions.iter_mut().enumerate() {
            let ended = session.exited;
            let printed = session.poll(theme);
            if Some(index) == shown {
                changed |= printed;
            } else {
                behind |= printed;
                changed |= session.exited != ended || session.copied.is_some();
            }
        }
        // A session that's finished has nothing left to show, so its tab closes, as when an
        // agent is told /exit. One that ended as it started failed to, and what it printed
        // says why, and one that stopped with the machine waits to be resumed.
        // A task's tab stays, whatever it took: its output is where its errors are read
        // from, and it's what the user looks at to see how the build went.
        let done = |s: &Session| {
            let agent = !s.shell && !s.browser && !s.task();
            let failed = agent && s.born.elapsed() < FAILED_START;
            s.exited && s.stopped.is_none() && !failed && (agent || s.shell || s.browser)
        };
        while let Some(index) = self.sessions.iter().position(done) {
            let session = &self.sessions[index];
            if !session.shell && !session.browser {
                self.ended.push(session.name.clone());
            }
            self.close(index);
            changed = true;
        }
        Polled {
            shown: changed,
            behind,
        }
    }

    /// Stops everything here, even what could have outlived Cellix.
    /// The soonest a synchronized update a program here left open runs out of time, for
    /// the loop to wake then and end it.
    pub fn update_deadline(&self) -> Option<std::time::Instant> {
        self.sessions
            .iter()
            .filter_map(Session::update_deadline)
            .min()
    }

    pub fn stop_all(&mut self) {
        for session in &mut self.sessions {
            session.stop();
        }
    }

    pub fn close(&mut self, index: usize) {
        if index >= self.sessions.len() {
            return;
        }
        let mut session = self.sessions.remove(index);
        // Other agents may still ask what it said, and its children end with it.
        if let Some(member) = session.member.clone() {
            let mut lines = session.lines();
            while lines.last().is_some_and(|l| l.trim().is_empty()) {
                lines.pop();
            }
            lines.drain(..lines.len().saturating_sub(GONE_LINES));
            self.gone.push(Gone {
                member,
                lines,
                transcript: session.transcript.as_ref().map(|(_, path)| path.clone()),
                exited: session.exited,
            });
        }
        session.stop();
        self.active = match self.active {
            _ if self.sessions.is_empty() => None,
            Some(active) if active >= index => Some(active.saturating_sub(1)),
            other => other,
        };
        self.focused &= self.active.is_some();
        if self.front == Front::Session && self.active.is_none() {
            self.next_tab();
        }
    }

    /// Goes to what's left in the pane, which closes when that's nothing.
    fn next_tab(&mut self) {
        let standing = [
            (self.inbox_tab, Front::Inbox),
            (self.comments_tab, Front::Comments),
            (self.agents_tab, Front::Agents),
            (self.pdf.is_some(), Front::Pdf),
            (!self.log.is_empty(), Front::Log),
            (self.problems, Front::Problems),
        ];
        let left = standing.into_iter().find(|(open, _)| *open);
        self.front = match (self.active, left) {
            (Some(_), _) | (None, None) => Front::Session,
            (None, Some((_, view))) => view,
        };
        if self.active.is_none() && left.is_none() {
            self.open = false;
            self.focused = false;
        }
    }

    fn comments_width(&self) -> u16 {
        counted_width(COMMENTS_WIDTH, self.comment_count.get())
    }

    fn inbox_width(&self) -> u16 {
        counted_width(INBOX_WIDTH, self.inbox_count.get())
    }

    /// Brings the inbox forward, in a tab of its own.
    pub fn show_inbox(&mut self) {
        self.inbox_tab = true;
        self.tops.set(Front::Inbox, 0);
        self.open = true;
        self.front = Front::Inbox;
        self.focused = false;
    }

    /// Whether the list of what can be started is showing. With nothing running, it's what
    /// the pane has to show.
    pub fn listing(&self) -> bool {
        self.front == Front::Agents || (self.active.is_none() && self.front == Front::Session)
    }

    /// Brings the comments forward, in a tab of their own.
    pub fn show_comments(&mut self) {
        self.comments_tab = true;
        self.front = Front::Comments;
    }

    /// Brings the list of agents forward, in a tab that stays until it's closed or used.
    pub fn show_list(&mut self) {
        self.agents_tab = true;
        self.front = Front::Agents;
        self.focused = false;
    }

    /// The views with a tab just now, in order, each with its name and width.
    fn views(&self) -> Vec<(Front, &'static str, u16)> {
        let mut views = Vec::new();
        // The inbox has no icon among the panel's, so it keeps its tab even docked.
        if self.inbox_tab || self.front == Front::Inbox {
            views.push((Front::Inbox, "Inbox", self.inbox_width()));
        }
        let tabbed = !self.docked;
        if tabbed && (self.comments_tab || self.front == Front::Comments) {
            views.push((Front::Comments, "Comments", self.comments_width()));
        }
        if tabbed && (self.agents_tab || self.listing()) {
            views.push((Front::Agents, "Agents", AGENTS_WIDTH));
        }
        if self.pdf.is_some() {
            views.push((Front::Pdf, "Preview", PREVIEW_WIDTH));
        }
        if !self.log.is_empty() {
            views.push((Front::Log, "Log", LOG_WIDTH));
        }
        if self.problems {
            views.push((Front::Problems, "Problems", PROBLEMS_WIDTH));
        }
        // A sort that keeps ties in place, so views never dragged keep their own order.
        let rank = |front: &Front| self.view_order.iter().position(|v| v == front);
        views.sort_by_key(|view| rank(&view.0).unwrap_or(usize::MAX));
        views
    }

    /// Where a tab dragged along the row would go, among the views' tabs or the sessions':
    /// the index it would take, and the column between tabs where it would stand.
    pub fn slot(&self, area: Rect, x: u16, views: bool) -> (usize, u16) {
        let spans: Vec<(u16, u16)> = if views {
            let tabs = self.view_tabs(area);
            tabs.iter().map(|t| (t.2.x, t.2.right())).collect()
        } else {
            self.tabs(area)
        };
        for (index, (start, end)) in spans.iter().enumerate() {
            if x < (start + end) / 2 {
                return (index, *start);
            }
        }
        let end = spans.last().map_or(self.first_tab(area), |t| t.1);
        (spans.len(), end)
    }

    /// Moves a view's tab to `to` among the views' tabs, counted before it moves.
    pub fn move_view(&mut self, view: Front, to: usize) {
        let mut order: Vec<Front> = self.views().iter().map(|v| v.0).collect();
        let Some(from) = order.iter().position(|v| *v == view) else {
            return;
        };
        order.remove(from);
        let to = if to > from { to - 1 } else { to };
        order.insert(to.min(order.len()), view);
        let rest = VIEW_ORDER
            .into_iter()
            .filter(|v| !order.contains(v))
            .collect::<Vec<_>>();
        order.extend(rest);
        self.view_order = order;
    }

    /// Moves a session's tab to `to` among the sessions' tabs, counted before it moves.
    pub fn move_session(&mut self, from: usize, to: usize) {
        if from >= self.sessions.len() {
            return;
        }
        let to = if to > from { to - 1 } else { to }.min(self.sessions.len() - 1);
        let session = self.sessions.remove(from);
        self.sessions.insert(to, session);
        self.active = self.active.map(|active| moved(active, from, to));
    }

    /// What a view's tab is called.
    pub fn view_name(&self, view: Front) -> &'static str {
        let views = self.views();
        views.iter().find(|v| v.0 == view).map_or("", |v| v.1)
    }

    /// How wide a session's tab would like to be: its mark, its name, how full its context
    /// is, room for what it's doing, and its ×.
    fn session_width(session: &Session) -> u16 {
        let context = session
            .context()
            .map_or(0, |c| c.chars().count() as u16 + CONTEXT_GAP);
        let label = session.label().chars().count() as u16;
        label + TAB_PADDING * 2 + CLOSE_WIDTH + context + Self::session_extra(session)
    }

    /// Columns a session's tab has besides its name: its mark, and room for what its agent is
    /// doing.
    fn session_extra(session: &Session) -> u16 {
        let icon = if session.has_icon() { ICON_ROOM } else { 0 };
        icon + if session.marked() { MARK_ROOM } else { 0 }
    }

    /// The same for any tab, views and then sessions: a view's tab has neither.
    fn extra(&self, index: usize) -> u16 {
        let views = self.views().len();
        index
            .checked_sub(views)
            .and_then(|at| self.sessions.get(at))
            .map_or(0, Self::session_extra)
    }

    pub fn tabs_start(&self, area: Rect) -> u16 {
        // Flush with the pane, as a file's first tab is with its sheet.
        area.x
    }

    /// The column the + after the tabs starts at, which its menu drops from.
    pub fn plus_x(&self, area: Rect) -> u16 {
        self.tabs(area).last().map_or(self.first_tab(area), |t| t.1)
    }

    /// How wide each tab would like to be, views and then sessions.
    fn wanted_widths(&self) -> Vec<u16> {
        let views = self.views().into_iter().map(|view| view.2);
        views
            .chain(self.sessions.iter().map(Self::session_width))
            .collect()
    }

    /// Whether only the tab in front shows, with a chevron that lists every tab: the row
    /// before the icons at the top right is too short even for tabs cut to a few letters.
    pub fn picking(&self, area: Rect) -> bool {
        self.fit(area) == Fit::Front
    }

    /// How the tabs stand in the row they have before the icons at the top right.
    pub fn fit(&self, area: Rect) -> Fit {
        self.fitted(area).1
    }

    /// What starts another agent or terminal as the tabs stand now, for the note under one
    /// that has ended: it never points at a + or a chevron that isn't drawn.
    pub fn another(&self, area: Rect) -> &'static str {
        match self.fit(area) {
            Fit::Whole => "press + for another",
            Fit::Cut => "start another above",
            Fit::Front => "use ⌄ for another",
        }
    }

    /// How wide `text` is in the interface's font, in cells.
    fn text_width(&self, text: &str) -> f32 {
        match &self.measure {
            Some(measure) => measure.width(text, cellix_render::Font::default()),
            None => text.chars().count() as f32,
        }
    }

    fn in_cells(&self) -> bool {
        self.measure.as_ref().is_none_or(|m| m.in_cells())
    }

    /// What each tab says, views and then sessions: a view's name with how many it holds.
    fn tab_texts(&self) -> Vec<String> {
        let views = self.views().into_iter().map(|(view, name, _)| {
            let counted = match view {
                Front::Comments => self.comment_count.get(),
                Front::Inbox => self.inbox_count.get(),
                _ => 0,
            };
            if counted > 0 {
                format!("{name} {counted}")
            } else {
                name.to_string()
            }
        });
        let sessions = self.sessions.iter().map(Session::label);
        views.chain(sessions).collect()
    }

    /// Columns the tab in front takes when the row is short: its mark, its name, what its
    /// agent is doing and its ×, with the air a chip has around them and none to spare.
    fn hugging(&self, text: &str, extra: u16) -> u16 {
        if self.in_cells() {
            // A column either side, and one between the name and the ×.
            text.chars().count() as u16 + 4 + extra
        } else {
            (self.text_width(text) + HUG + f32::from(extra)).ceil() as u16
        }
    }

    /// Columns a tab behind the one in front takes to show its whole name, with no × and
    /// half a column of air either side, and an agent's mark and state either end of it.
    fn stub(&self, text: &str, extra: u16) -> u16 {
        (self.text_width(text) + CUT_AIR * 2.0 + f32::from(extra)).ceil() as u16
    }

    /// What each tab gets, views and then sessions, and how they stand. When they don't all
    /// fit with the + after them, the one in front keeps its whole name, in a chip that
    /// hugs it, and those behind it stand as names alone: whole while there's room, and
    /// then cut short with a "…", the longest giving way first. When that would leave them
    /// under `CUT_LEAST` columns each, only the one in front shows, with a chevron that
    /// lists every tab.
    fn fitted(&self, area: Rect) -> (Vec<u16>, Fit) {
        let mut widths = self.wanted_widths();
        let end = self.tabs_end.get();
        // The button that maximizes the pane keeps its columns at the row's end: the tabs
        // and the + have what's before them. It used to go when the row was short, which is
        // every row of a pane 40 columns wide, as Cellix's is inside Claude Code.
        let start = self.tabs_start(area);
        let room = end.saturating_sub(start + MAXIMIZE_ROOM);
        if end == 0 || widths.iter().sum::<u16>() + NEW_WIDTH <= room {
            return (widths, Fit::Whole);
        }
        // Without the +, the button stands right at that end, with no column before it, and
        // the names run up to it. Every tab staying in sight comes before the button, though:
        // where the tabs fit at their least only without it, they have the row as they had,
        // up to the icons' own cells, and it goes.
        let front = self.front_index().unwrap_or(0);
        let behind = widths.len().saturating_sub(1) as u16;
        let texts = self.tab_texts();
        let kept = (texts.get(front)).map_or(0, |text| self.hugging(text, self.extra(front)));
        let room = end.saturating_sub(start + MAXIMIZE_SHORT);
        let short = if self.cut_tabs_leave_the_button(area) {
            room
        } else {
            end.saturating_sub(start) + self.tabs_slack.get()
        };
        if kept + behind * CUT_LEAST > short {
            for (index, width) in widths.iter_mut().enumerate() {
                *width = if index == front {
                    (*width + PICK_WIDTH).min(room)
                } else {
                    0
                };
            }
            return (widths, Fit::Front);
        }
        for (index, (width, text)) in widths.iter_mut().zip(&texts).enumerate() {
            *width = if index == front {
                kept
            } else {
                self.stub(text, self.extra(index))
            };
        }
        let rest = short - kept;
        let taken = |cap: u16, widths: &[u16]| -> u16 {
            let behind = widths
                .iter()
                .enumerate()
                .filter(|(index, _)| *index != front);
            behind.map(|(_, width)| (*width).min(cap)).sum()
        };
        let mut cap = widths.iter().copied().max().unwrap_or(0);
        while cap > CUT_LEAST && taken(cap, &widths) > rest {
            cap -= 1;
        }
        // What's left over goes a column each to the first of those cut.
        let mut spare = rest.saturating_sub(taken(cap, &widths));
        for (index, width) in widths.iter_mut().enumerate() {
            if index != front && *width > cap {
                let more = u16::from(spare > 0);
                spare -= more;
                *width = cap + more;
            }
        }
        (widths, Fit::Cut)
    }

    /// Whether the tabs, each cut to its least, leave the button that maximizes the pane its
    /// columns at the row's end.
    fn cut_tabs_leave_the_button(&self, area: Rect) -> bool {
        let front = self.front_index().unwrap_or(0);
        let texts = self.tab_texts();
        let kept = (texts.get(front)).map_or(0, |text| self.hugging(text, self.extra(front)));
        let behind = texts.len().saturating_sub(1) as u16;
        let room = (self.tabs_end.get()).saturating_sub(self.tabs_start(area) + MAXIMIZE_SHORT);
        kept + behind * CUT_LEAST <= room
    }

    /// Whether the tab in front stands in a chip that hugs its name, with its × two columns
    /// from its end rather than three.
    fn hugs(&self, area: Rect) -> bool {
        self.fit(area) == Fit::Cut
    }

    /// Which tabs stand as names alone, views and then sessions: every one behind the tab
    /// in front, when the row is too short for them all. One shows no ×, and a click
    /// anywhere on it brings it forward.
    fn cut_short(&self, area: Rect) -> Vec<bool> {
        let fit = self.fit(area);
        let front = self.front_index().unwrap_or(0);
        let count = self.wanted_widths().len();
        (0..count)
            .map(|index| fit == Fit::Cut && index != front)
            .collect()
    }

    /// Where the tab in front comes among them all, views and then sessions.
    fn front_index(&self) -> Option<usize> {
        let views = self.views();
        let shown = |view: &(Front, &str, u16)| {
            self.front == view.0 || (view.0 == Front::Agents && self.listing())
        };
        views.iter().position(shown).or_else(|| {
            let session = self.active.filter(|_| self.front == Front::Session)?;
            Some(views.len() + session)
        })
    }

    /// What each tab gets, views and then sessions.
    fn tab_widths(&self, area: Rect) -> Vec<u16> {
        self.fitted(area).0
    }

    /// Every tab, views and then sessions, with what it's called and whether it's in front,
    /// for the list the chevron opens.
    pub fn every_tab(&self) -> Vec<(PaneTab, String, bool)> {
        let front = self.front_index();
        let views = self.views().into_iter().map(|(view, name, _)| {
            let counted = match view {
                Front::Comments => self.comment_count.get(),
                Front::Inbox => self.inbox_count.get(),
                _ => 0,
            };
            let name = if counted > 0 {
                format!("{name} {counted}")
            } else {
                name.to_string()
            };
            (PaneTab::View(view), name)
        });
        let sessions = (self.sessions.iter().enumerate())
            .map(|(index, session)| (PaneTab::Session(index), session.label()));
        let all = views.chain(sessions).enumerate();
        all.map(|(index, (tab, name))| (tab, name, Some(index) == front))
            .collect()
    }

    /// Where the tab in front ends its name when it carries the chevron, and where the chevron
    /// stands, given where its × stands.
    fn pick_at(&self, area: Rect, close: u16) -> Option<u16> {
        self.picking(area).then(|| close.saturating_sub(PICK_WIDTH))
    }

    /// What an agent's tab says of its context, for a tooltip, with the pointer at `x`.
    pub fn context_tip(&self, area: Rect, x: u16) -> Option<String> {
        let (index, _) = self
            .tabs(area)
            .into_iter()
            .enumerate()
            .find(|(_, (start, end))| x >= *start && x < *end)?;
        let usage = self.sessions.get(index)?.usage.as_ref()?;
        usage.percent()?;
        Some(usage.describe())
    }

    /// The full name of a tab too narrow to show it, for a tooltip.
    pub fn cut_name(&self, area: Rect, x: u16) -> Option<String> {
        let wanted = self.wanted_widths();
        let widths = self.tab_widths(area);
        let views = self.views();
        let names = views
            .iter()
            .map(|view| view.1.to_string())
            .chain(self.sessions.iter().map(Session::label));
        let mut at = self.tabs_start(area);
        let pick = if self.picking(area) { PICK_WIDTH } else { 0 };
        let cuts = self.cut_short(area);
        let texts = self.tab_texts();
        let tabs = names
            .zip(widths)
            .zip(wanted)
            .zip(cuts)
            .zip(texts)
            .enumerate();
        for (index, ((((name, width), full), cut), text)) in tabs {
            if x >= at && x < at + width {
                // Standing as a name alone, it's cut when that name has no room.
                let short = cut && self.stub(&text, self.extra(index)) > width;
                let stub = pick > 0 && width < full + pick;
                return (short || stub).then_some(name);
            }
            at += width;
        }
        None
    }

    /// Each view's tab, where it stands.
    fn view_tabs(&self, area: Rect) -> Vec<(Front, &'static str, Rect)> {
        let mut x = self.tabs_start(area);
        let widths = self.tab_widths(area);
        let views = self.views().into_iter().zip(widths);
        let tabs = views.map(|((front, name, _), width)| {
            let tab = Rect::new(x, area.y, width, 1);
            x += width;
            (front, name, tab)
        });
        tabs.collect()
    }

    /// Where the button that maximizes the pane starts: after the +, or where the row is
    /// too short for every tab and the +, at the end of the pane's own part of it.
    pub fn maximize_button(&self, area: Rect) -> u16 {
        let end = self.tabs_end.get();
        if end == 0 || self.fit(area) == Fit::Whole {
            let plus = self.tabs(area).last().map_or(self.first_tab(area), |t| t.1);
            return plus + NEW_WIDTH + MAXIMIZE_GAP;
        }
        end.saturating_sub(MAXIMIZE_SHORT)
    }

    /// Whether that button shows. It does but in one case: a row whose tabs, each cut to its
    /// least, fit only without it. There a double click on a tab still maximizes the pane,
    /// and dragging the pane a little wider brings the button back.
    pub fn maximize_shows(&self, area: Rect) -> bool {
        let end = self.tabs_end.get();
        if end == 0 {
            return true;
        }
        match self.fit(area) {
            Fit::Whole => true,
            // One tab and its chevron: the rest of the row is free.
            Fit::Front => end >= self.tabs_start(area) + MAXIMIZE_SHORT,
            Fit::Cut => self.cut_tabs_leave_the_button(area),
        }
    }

    #[cfg(test)]
    pub fn new_button_for_test(&self, area: Rect) -> u16 {
        self.tabs(area).last().map_or(self.first_tab(area), |t| t.1)
    }

    #[cfg(test)]
    pub fn view_tabs_for_test(&self, area: Rect) -> Vec<(Front, &'static str, Rect)> {
        self.view_tabs(area)
    }

    /// Where sessions' tabs start, after the fixed ones.
    fn first_tab(&self, area: Rect) -> u16 {
        let views = self
            .view_tabs(area)
            .iter()
            .map(|view| view.2.w)
            .sum::<u16>();
        self.tabs_start(area) + views
    }

    /// Takes a view's tab away, and what it shows.
    pub fn close_view(&mut self, view: Front) {
        let showing = self.front == view || (view == Front::Agents && self.listing());
        match view {
            Front::Pdf => {
                self.pdf = None;
                self.pdf_spot = None;
            }
            Front::Log => self.log.clear(),
            Front::Problems => self.problems = false,
            Front::Agents => self.agents_tab = false,
            Front::Comments => self.comments_tab = false,
            Front::Inbox => self.inbox_tab = false,
            Front::Session => {}
        }
        if showing {
            self.next_tab();
        }
    }

    /// The furthest the log scrolls in a screen `height` rows tall: its last line at the bottom.
    pub fn log_last(&self, height: u16) -> usize {
        self.log.len().saturating_sub(height as usize)
    }

    fn log_top(&self, height: u16) -> usize {
        self.tops.of(Front::Log).min(self.log_last(height))
    }

    /// The log line at a row of the screen.
    pub fn log_line(&self, screen: Rect, y: u16) -> Option<String> {
        let row = y.checked_sub(screen.y)? as usize;
        self.log.get(self.log_top(screen.h) + row).cloned()
    }

    pub fn draw_log(
        &self,
        grid: &mut Grid,
        screen: Rect,
        hover: Option<(u16, u16)>,
        theme: &Theme,
    ) {
        grid.fill(screen, theme.sidebar);
        let palette = palette_of(theme);
        let top = self.log_top(screen.h);
        let lines = self.log.iter().skip(top).take(screen.h as usize);
        for (row, line) in lines.enumerate() {
            let y = screen.y + row as u16;
            let whole = Rect::new(screen.x, y, screen.w, 1);
            if hover.is_some_and(|h| whole.contains(h.0, h.1)) {
                grid.panel(
                    Bounds::inset(whole, 0.3, 0.06),
                    12,
                    Some(theme.row_hover),
                    None,
                );
            }
            let lowered = line.to_lowercase();
            let fg = if lowered.starts_with("error")
                || line.starts_with('!')
                || lowered.contains(": error")
            {
                palette[1]
            } else if lowered.contains("warning") || lowered.contains("overfull") {
                palette[3]
            } else {
                theme.sidebar.fg
            };
            let style = Style {
                fg,
                ..theme.sidebar
            };
            grid.put_str(screen.x + 1, y, line, style, screen.right());
        }
    }

    /// Each tab as (start, end) columns.
    fn tabs(&self, area: Rect) -> Vec<(u16, u16)> {
        let mut x = self.first_tab(area);
        let widths = self.tab_widths(area);
        self.sessions
            .iter()
            .zip(widths.into_iter().skip(self.views().len()))
            .map(|(_, width)| {
                let tab = (x, x + width);
                x += width;
                tab
            })
            .collect()
    }

    pub fn hit(&self, area: Rect, x: u16) -> Option<TabHit> {
        if x + HIDE_WIDTH >= area.right() {
            return Some(TabHit::Hide);
        }
        let fit = self.fit(area);
        let picking = fit == Fit::Front;
        let cuts = self.cut_short(area);
        // Columns at a tab's end that are its ×: one fewer in a chip that hugs its name.
        let reach = if fit == Fit::Cut { 2 } else { CLOSE_WIDTH + 1 };
        let views = self.view_tabs(area);
        let sessions_from = views.len();
        for (index, (view, _, tab)) in views.into_iter().enumerate() {
            if x >= tab.x && x < tab.right() {
                // Behind the one in front, a tab has no × to miss: all of it brings it forward.
                if cuts[index] {
                    return Some(TabHit::View(view));
                }
                let close = tab.right().saturating_sub(reach);
                if picking && x + PICK_WIDTH >= close && x < close {
                    return Some(TabHit::Pick);
                }
                let on_close = x >= close;
                return Some(if on_close {
                    TabHit::CloseView(view)
                } else {
                    TabHit::View(view)
                });
            }
        }
        let tabs = self.tabs(area);
        for (index, (start, end)) in tabs.iter().enumerate() {
            if x >= *start && x < *end {
                if cuts[sessions_from + index] {
                    return Some(TabHit::Tab(index));
                }
                let close = end.saturating_sub(reach);
                if picking && x + PICK_WIDTH >= close && x < close {
                    return Some(TabHit::Pick);
                }
                let on_close = x >= close;
                return Some(if on_close {
                    TabHit::Close(index)
                } else {
                    TabHit::Tab(index)
                });
            }
        }
        let end = tabs.last().map_or(self.first_tab(area), |t| t.1);
        // Out of room, the + gives way: the icons at the top right start the same things.
        if fit == Fit::Whole && x >= end && x < end + NEW_WIDTH {
            return Some(TabHit::New);
        }
        let maximize = self.maximize_button(area);
        let on_it = x >= maximize && x < maximize + MAXIMIZE_WIDTH;
        (on_it && self.maximize_shows(area)).then_some(TabHit::Maximize)
    }

    /// The title row, in the frame's colour, with tabs like the editor's.
    pub fn draw_tabs(
        &self,
        grid: &mut Grid,
        area: Rect,
        (hover, maximized): (Option<(u16, u16)>, bool),
        theme: &Theme,
        measure: &dyn cellix_render::Measure,
    ) {
        let count = self.comment_count.get();
        let row = Rect::new(area.x, area.y, area.w, 1);
        grid.fill(row, theme.tab);
        let whole = self.fit(area) == Fit::Whole;
        let hugs = self.hugs(area);
        let cuts = self.cut_short(area);
        let font = cellix_render::Font::default();
        // A tab behind the one in front: its name, or as much as fits and a "…", with no ×
        // to crowd it. Under the pointer its chip is that name and the air around it.
        let cut_name = |grid: &mut Grid, tab: Rect, name: &str, (style, hovered): (Style, bool)| {
            let room = f32::from(tab.w) - CUT_AIR * 2.0;
            let text = crate::context::elide(name, room, measure);
            if hovered {
                let (wide, high) = (measure.width(&text, font) + CUT_AIR * 2.0, 0.8);
                let top = f32::from(tab.y) + (1.0 - high) / 2.0;
                let chip = Bounds::cells(f32::from(tab.x), top, wide, high);
                grid.panel(chip, TAB_CHIP_RADIUS, Some(theme.row_hover), None);
            }
            let at = f32::from(tab.x) + CUT_AIR;
            grid.label_at(at, tab.x + 1, area.y, &text, style, tab.right(), 100);
        };
        // Where a whole tab's × stands, and the column its name stops before. In a chip that
        // hugs its name the × is 10 px from the chip's edge and 12 px after the name.
        let close_of = |tab: Rect| -> (f32, u16) {
            let right = tab.right();
            if !hugs {
                let close = right.saturating_sub(CLOSE_WIDTH + 1);
                (f32::from(close), close)
            } else if measure.in_cells() {
                (f32::from(right) - 2.0, right.saturating_sub(3))
            } else {
                (f32::from(right) - HUG_CLOSE, right.saturating_sub(2))
            }
        };
        // The cell a whole tab's name starts in: a column sooner where the chip hugs it.
        let name_cell = |tab: Rect| tab.x + if hugs { 1 } else { 2 };
        let chevron = |grid: &mut Grid, x: u16, style: Style| {
            let quiet = Style {
                bold: false,
                ..style
            };
            grid.glyph(x as f32, area.y, Icon::ChevronDown, quiet.fg, "▾", quiet);
        };
        let views = self.view_tabs(area);
        let sessions_from = views.len();
        for (index, (view, name, tab)) in views.into_iter().enumerate() {
            // Out of room, only the tab in front has columns; the rest are in its list.
            if tab.w == 0 {
                continue;
            }
            let on = self.front == view || (view == Front::Agents && self.listing());
            let hovered = hover.is_some_and(|h| tab.contains(h.0, h.1));
            let quiet = Style {
                fg: if on || hovered {
                    theme.text.fg
                } else {
                    theme.tab.fg
                },
                bg: if on { theme.sidebar.bg } else { theme.tab.bg },
                ..theme.tab
            };
            let (close_at, close) = close_of(tab);
            let counted = match view {
                Front::Comments => count,
                Front::Inbox => self.inbox_count.get(),
                _ => 0,
            };
            let text = if counted > 0 {
                format!("{name} {counted}")
            } else {
                name.to_string()
            };
            if cuts[index] {
                cut_name(grid, tab, &text, (quiet, hovered));
                continue;
            }
            // Where the views are icons, the one showing has a column and no tab to draw.
            if on && tab.w > CLOSE_WIDTH + 1 {
                front_tab(grid, tab, theme, measure.in_cells());
            }
            let at = tab.x as f32 + TAB_TEXT;
            let pick = self.pick_at(area, close);
            grid.label_at(
                at,
                name_cell(tab),
                area.y,
                &text,
                quiet,
                pick.unwrap_or(close),
                100,
            );
            if let Some(x) = pick {
                chevron(grid, x, quiet);
            }
            if on || hovered {
                let plain = Style {
                    bold: false,
                    ..quiet
                };
                grid.glyph(close_at, area.y, Icon::Close, plain.fg, "×", plain);
            }
        }
        for (index, (start, end)) in self.tabs(area).into_iter().enumerate() {
            if end == start {
                continue;
            }
            let session = &self.sessions[index];
            let label = session.label();
            let active = self.active == Some(index) && self.front == Front::Session;
            let tab = Rect::new(start, area.y, end - start, 1);
            let hovered = hover.is_some_and(|h| tab.contains(h.0, h.1));
            let style = if active {
                Style {
                    fg: theme.text.fg,
                    bg: theme.sidebar.bg,
                    ..theme.tab
                }
            } else if hovered {
                Style {
                    fg: theme.text.fg,
                    ..theme.tab
                }
            } else {
                theme.tab
            };
            let shown = if session.exited {
                Style {
                    fg: theme.gutter.fg,
                    ..style
                }
            } else {
                style
            };
            if cuts[sessions_from + index] {
                if session.has_icon() || session.marked() {
                    cut_session(grid, tab, session, (shown, hovered), (measure, theme));
                } else {
                    cut_name(grid, tab, &label, (shown, hovered));
                }
                continue;
            }
            if active {
                front_tab(grid, tab, theme, measure.in_cells());
            }
            // Its program's mark, where a file's tab has its icon, and its name after it.
            let lead = start as f32 + TAB_TEXT;
            let (at, first) = if session.has_icon() {
                let mark_at = (lead + ICON_MIDDLE, name_cell(tab));
                tab_icon(grid, session.icon(), mark_at, area.y, style, theme);
                (lead + ICON_SHIFT, name_cell(tab) + ICON_ROOM)
            } else {
                (lead, name_cell(tab))
            };
            let (close_at, close) = close_of(tab);
            let pick = self.pick_at(area, close);
            if let Some(x) = pick {
                chevron(grid, x, style);
            }
            // What stands at its end, the chevron or the ×, and what its agent is doing before
            // that: its name and context stop short of both.
            let (end_at, end_cell) = pick.map_or((close_at, close), |x| (f32::from(x), x));
            let stop = if session.marked() {
                end_cell.saturating_sub(MARK_ROOM)
            } else {
                end_cell
            };
            if let Some(mark) = session.mark() {
                draw_mark(grid, (end_at - MARK_SHIFT, area.y), mark, style, theme);
            }
            // How full its context is, when the tab has its whole width.
            let pick_room = if pick.is_some() { PICK_WIDTH } else { 0 };
            let whole = end - start >= Self::session_width(session) + pick_room;
            let context = session.context().filter(|_| whole);
            let name_end = match &context {
                Some(text) => {
                    // Right after the name, as the workspace's tile has it, not at the ×.
                    let after = at + measure.width(&label, font) + CONTEXT_AIR;
                    let x = after.ceil().min(stop.saturating_sub(1) as f32) as u16;
                    let percent = session.usage.as_ref().and_then(|u| u.percent());
                    let fg = match percent {
                        Some(p) if p >= crate::transcript::FULL => theme.error,
                        Some(p) if p >= crate::transcript::WARN => theme.attention,
                        _ => theme.gutter.fg,
                    };
                    let quiet = Style { fg, ..shown };
                    let px = after.min(x as f32);
                    grid.label_at(px, x, area.y, text, quiet, stop, CONTEXT_SIZE);
                    x
                }
                None => stop,
            };
            grid.label_at(at, first, area.y, &label, shown, name_end, 100);
            if hovered || active {
                let quiet = Style {
                    fg: theme.gutter.fg,
                    ..style
                };
                grid.glyph(close_at, area.y, Icon::Close, quiet.fg, "×", quiet);
            }
        }
        let plus = self.tabs(area).last().map_or(self.first_tab(area), |t| t.1);
        let button = Rect::new(plus, area.y, NEW_WIDTH, 1);
        let on_it = hover.is_some_and(|h| button.contains(h.0, h.1));
        // Out of room, the + gives way: the icons at the top right start the same things.
        if on_it && whole {
            grid.panel(
                Bounds::inset(button, 0.1, 0.1),
                16,
                Some(theme.row_hover),
                None,
            );
        }
        let style = Style {
            fg: if on_it {
                theme.text.fg
            } else {
                theme.gutter.fg
            },
            ..theme.tab
        };
        if whole {
            grid.label(plus + 1, area.y, "+", style, area.right());
        }
        // Maximized, it stands on the chip a selected icon has, so it's plain how to get back.
        // In a short row it keeps its place at the end of the pane's part of it, where it
        // gave way with the +: a pane that can't be maximized is stuck at its width.
        let button = Rect::new(self.maximize_button(area), area.y, MAXIMIZE_WIDTH, 1);
        if maximized || self.maximize_shows(area) {
            let on_it = hover.is_some_and(|h| button.contains(h.0, h.1));
            if maximized || on_it {
                let fill = if maximized {
                    theme.tint
                } else {
                    theme.row_hover
                };
                // The chip the icons beside it have, around its middle.
                let (wide, high) = ACTION_PILL;
                let middle = button.x as f32 + button.w as f32 / 2.0;
                let top = area.y as f32 + (1.0 - high) / 2.0;
                let chip = Bounds::cells(middle - wide / 2.0, top, wide, high);
                grid.panel(chip, HIDE_RADIUS, Some(fill), None);
            }
            let icon = if maximized {
                Icon::Minimize
            } else {
                Icon::Maximize
            };
            let fg = if maximized || on_it {
                theme.text.fg
            } else {
                theme.gutter.fg
            };
            // The size of the icons at the top right, which it stands among.
            crate::toolbar::draw_icon(grid, (icon, ACTION_ICON), button, fg, theme.tab);
        }
        // Drawn last, over any tabs that reach this far, so it can always be found.
        let hide = Rect::new(
            area.right().saturating_sub(HIDE_WIDTH),
            area.y,
            HIDE_WIDTH,
            1,
        );
        grid.fill(hide, theme.tab);
        let on_it = hover.is_some_and(|h| hide.contains(h.0, h.1));
        // Two columns from the end of the pane's row, which is a column short of the
        // window's edge, so its chip keeps two columns clear of that.
        let middle = hide.x as f32 + 1.0;
        if on_it {
            // The chip the icons beside it have on hover.
            let (wide, high) = HIDE_PILL;
            let top = area.y as f32 + (1.0 - high) / 2.0;
            let chip = Bounds::cells(middle - wide / 2.0, top, wide, high);
            grid.panel(chip, HIDE_RADIUS, Some(theme.row_hover), None);
        }
        let fg = if on_it {
            theme.text.fg
        } else {
            theme.gutter.fg
        };
        // A window with its right part filled, not an ×: the pane hides, and nothing in the
        // corner reads as closing more than that. The size of the icons beside it.
        let cell = Rect::new(hide.x, area.y, 2, 1);
        debug_assert_eq!(cell.x as f32 + cell.w as f32 / 2.0, middle);
        // Standing under the file, the window's foot is the part filled, as VS Code's panel
        // button has it; beside the file, its right part.
        let icon = if self.tabs_flush.get() {
            Icon::HidePanel
        } else {
            Icon::HidePane
        };
        crate::toolbar::draw_icon(grid, (icon, ACTION_ICON), cell, fg, theme.tab);
    }
}

/// The pane's tab in front, in the shape a file's has: the colour of the pane under it, the
/// accent's line along its top, and its top corners rounded into the strip. In cells the
/// colour alone says it.
fn front_tab(grid: &mut Grid, tab: Rect, theme: &Theme, in_cells: bool) {
    // Tabs stand on their sheet, so it's a line under the one in front that says which it
    // is: the accent's, or where there's no paint for that, an underline on its cells.
    let style = Style {
        bg: theme.sidebar.bg,
        underline: in_cells,
        ..theme.tab
    };
    grid.fill(tab, style);
    if in_cells {
        return;
    }
    let (x, y, w) = (f32::from(tab.x), f32::from(tab.y), f32::from(tab.w));
    grid.panel(
        Bounds::cells(x, y + 1.0 - FRONT_LINE, w, FRONT_LINE),
        0,
        Some(theme.accent),
        None,
    );
}

/// A program's mark on its tab: a shape whose middle is at `middle` where there are pictures,
/// and its letter in `cell` on cells.
fn tab_icon(
    grid: &mut Grid,
    icon: Icon,
    (middle, cell): (f32, u16),
    y: u16,
    style: Style,
    theme: &Theme,
) {
    let color = icon_colour(icon, theme);
    let across = ICON_SIZE * 2.0;
    let top = y as f32 + (1.0 - ICON_SIZE) / 2.0;
    grid.paint.push(Paint::Icon {
        bounds: Bounds::cells(middle - across / 2.0, top, across, ICON_SIZE),
        icon,
        color,
    });
    let shown = Style {
        fg: color,
        painted: true,
        ..style
    };
    grid.put_str(cell, y, crate::toolbar::fallback(icon), shown, cell + 1);
}

/// What an agent is doing, at `x`: turning while it works, a dot when it asks or is done.
pub fn draw_mark(grid: &mut Grid, (x, y): (f32, u16), mark: TabMark, style: Style, theme: &Theme) {
    let color = mark_colour(mark, theme);
    match mark {
        TabMark::Working => grid.spinner(x, y, MARK_SIZE, color, style),
        TabMark::NeedsYou | TabMark::Done => grid.dot(x, y, MARK_SIZE, color, style),
    }
}

/// A session's tab behind the one in front, in a row too short for every tab whole: its mark
/// and what its agent is doing, either end of it, and as much of its name between as fits.
/// Under the pointer its chip is the tab's width.
fn cut_session(
    grid: &mut Grid,
    tab: Rect,
    session: &Session,
    (style, hovered): (Style, bool),
    (measure, theme): (&dyn cellix_render::Measure, &Theme),
) {
    let (x, right) = (f32::from(tab.x), f32::from(tab.right()));
    if hovered {
        let high = 0.8;
        let top = f32::from(tab.y) + (1.0 - high) / 2.0;
        let chip = Bounds::cells(x, top, f32::from(tab.w), high);
        grid.panel(chip, TAB_CHIP_RADIUS, Some(theme.row_hover), None);
    }
    let icon = session.has_icon();
    let marked = session.marked();
    // The name, between the mark and the state, where three letters and a "…" have room.
    let from = if icon {
        x + CUT_ICON + ICON_SIZE + CUT_AIR
    } else {
        x + CUT_AIR
    };
    let to = if marked {
        right - CUT_MARK - CUT_AIR
    } else {
        right - CUT_AIR
    };
    let first = tab.x + if icon { 3 } else { 1 };
    let last = tab.right().saturating_sub(if marked { 3 } else { 1 });
    let room = if measure.in_cells() {
        f32::from(last.saturating_sub(first))
    } else {
        to - from
    };
    let label = session.label();
    let text = crate::context::elide(&label, room, measure);
    let named = text == label || text.chars().count() >= 4;
    if named {
        grid.label_at(from, first, tab.y, &text, style, last, 100);
    }
    // Either end of the name; with none, the two side by side in the tab's middle.
    let (icon_at, mark_at) = if named || !icon || measure.in_cells() {
        (x + CUT_ICON, right - CUT_MARK)
    } else {
        let pair = ICON_SIZE * 2.0 + CUT_AIR + MARK_WIDE;
        let start = x + (f32::from(tab.w) - pair) / 2.0;
        (start + ICON_SIZE, start + ICON_SIZE * 2.0 + CUT_AIR)
    };
    if icon {
        tab_icon(
            grid,
            session.icon(),
            (icon_at, tab.x + 1),
            tab.y,
            style,
            theme,
        );
    }
    if let Some(mark) = session.mark() {
        draw_mark(grid, (mark_at, tab.y), mark, style, theme);
    }
}

/// How tall the accent's line on the tab in front is, in rows: the file tabs' own.
const FRONT_LINE: f32 = 0.06;

const TAB_CHIP_RADIUS: u8 = 22;

/// Text as it's pasted into a program: what Cellix sends an agent holds file text, terminal
/// output and agents' replies, any of which can carry control characters. An escape could end
/// the paste early (`ESC[201~`) and type the rest as keys, a carriage return send the prompt
/// half written, and Ctrl+C stop the agent, so none of them go: line breaks stay as `\n`,
/// tabs stay, and the rest are dropped.
pub fn pasteable(text: &str) -> String {
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    text.chars()
        .filter(|c| !c.is_control() || matches!(c, '\n' | '\t'))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An agent's title, as Claude Code and Codex set it, is its name once the glyph that
    /// says it's busy is off the front: one of them, and the spaces after it, and no more.
    #[test]
    fn an_agents_title_is_its_name_without_its_busy_glyph() {
        assert_eq!(own_name("✳ pricing page").as_deref(), Some("pricing page"));
        assert_eq!(own_name("⠙  og tags").as_deref(), Some("og tags"));
        assert_eq!(own_name("· docs nav ").as_deref(), Some("docs nav"));
        assert_eq!(
            own_name("◐ ✳ both").as_deref(),
            Some("✳ both"),
            "one glyph only"
        );
        assert_eq!(own_name("refunds").as_deref(), Some("refunds"));
        assert_eq!(own_name("✶ ").as_deref(), None, "nothing left is no name");
        assert_eq!(own_name("").as_deref(), None);
    }
    use cellix_input::Mods;
    #[cfg(unix)]
    use cellix_render::UNIT;

    /// Runs something that writes `stream`, and waits for `count` pictures to arrive. Unix
    /// only: ConPTY keeps the graphics protocol from reaching Cellix.
    #[cfg(unix)]
    fn pictures_from(stream: &[u8], count: usize) -> (tempfile::TempDir, Session) {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("stream");
        std::fs::write(&file, stream).unwrap();
        let script = format!("cat '{}'; sleep 5", file.display());
        let argv = ["sh".to_string(), "-c".to_string(), script];
        let mut session =
            Session::start("Test", &argv, &[], dir.path(), (80, 24), (10, 20)).unwrap();
        let theme = Theme::named(Some("light"));
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while session.pictures.len() < count {
            assert!(
                std::time::Instant::now() < deadline,
                "the pictures never came"
            );
            session.poll(&theme);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        (dir, session)
    }

    /// A one pixel picture, as a program sends it.
    #[cfg(unix)]
    fn pixel(control: &str) -> Vec<u8> {
        format!("\x1b_Ga=T,f=24,s=1,v=1,C=1,q=2,{control};AQID\x1b\\").into_bytes()
    }

    #[test]
    #[cfg(unix)]
    fn a_picture_inside_a_synchronized_update_lands_where_the_update_moved_to() {
        let mut stream = b"\x1b[?2026h\x1b[3;5H".to_vec();
        stream.extend(pixel("i=1"));
        stream.extend(b"\x1b[7;9H");
        stream.extend(pixel("i=2"));
        stream.extend(b"\x1b[?2026l");
        let (_dir, session) = pictures_from(&stream, 2);
        let spots: Vec<_> = session
            .pictures
            .iter()
            .map(|p| (p.line, p.column))
            .collect();
        assert_eq!(spots, [(2, 4), (6, 8)]);
    }

    #[test]
    #[cfg(unix)]
    fn pictures_stand_in_their_cells_and_draw_in_their_layers() {
        let mut stream = b"\x1b[3;5H".to_vec();
        stream.extend(pixel("i=1,z=5,X=5,Y=10"));
        stream.extend(pixel("i=2,z=1"));
        let (_dir, session) = pictures_from(&stream, 2);
        let theme = Theme::named(Some("light"));
        let mut grid = Grid::new(80, 24, theme.text);
        session.draw_pictures(&mut grid, Rect::new(0, 0, 80, 24), &theme);
        let places: Vec<(f32, f32)> = grid
            .paint
            .iter()
            .filter_map(|p| match p {
                Paint::Image { bounds, .. } => Some(bounds),
                _ => None,
            })
            .map(|b| (b.x as f32 / UNIT as f32, b.y as f32 / UNIT as f32))
            .collect();
        assert_eq!(
            places,
            [(4.0, 2.0), (4.5, 2.5)],
            "the lower layer goes first"
        );
    }

    #[test]
    #[cfg(unix)]
    fn taking_one_placement_away_leaves_the_rest() {
        let mut stream = pixel("i=1,p=1");
        stream.extend(pixel("i=1,p=2"));
        stream.extend(pixel("i=2,p=1"));
        stream.extend(b"\x1b_Ga=d,d=i,i=1,p=2,q=2\x1b\\");
        stream.extend(pixel("i=3"));
        let (_dir, session) = pictures_from(&stream, 3);
        let left: Vec<_> = session
            .pictures
            .iter()
            .map(|p| (p.image, p.placement))
            .collect();
        assert_eq!(left, [(1, 1), (2, 1), (3, 0)]);
    }

    /// A session fed by the test, with nothing running.
    fn fed() -> (mpsc::Sender<Vec<u8>>, Session) {
        let (sender, output) = mpsc::channel();
        let (_, finished) = mpsc::channel();
        let writer = Box::new(io::sink());
        let channels = (output, finished);
        let session = Session::around("Test", Link::Gone, writer, channels, ((40, 5), (10, 20)));
        (sender, session)
    }

    /// A program that begins a synchronized update and never ends it, as one killed halfway
    /// through does: what it wrote is held, which changes nothing on screen, until the
    /// update's time is up. Then it shows, with nothing more coming.
    #[test]
    fn an_update_left_open_shows_once_its_time_is_up() {
        let theme = Theme::named(Some("light"));
        let (program, mut session) = fed();
        program.send(b"\x1b[?2026hheld back".to_vec()).unwrap();
        assert!(
            !session.poll(&theme),
            "only held, so the screen is as it was"
        );
        program.send(b" and more".to_vec()).unwrap();
        assert!(!session.poll(&theme));
        assert!(!session.rows().concat().contains("held back"));
        let deadline = session.update_deadline().expect("an update is under way");
        assert!(deadline <= std::time::Instant::now() + std::time::Duration::from_millis(150));
        std::thread::sleep(deadline.saturating_duration_since(std::time::Instant::now()));
        assert!(session.poll(&theme), "its time is up: it ends");
        assert!(session.rows().concat().contains("held back and more"));
        assert_eq!(session.update_deadline(), None);
        // One that ends on time shows when it ends, and text outside one shows at once.
        program.send(b"\x1b[?2026h\r\nnext".to_vec()).unwrap();
        assert!(!session.poll(&theme));
        program.send(b" frame\x1b[?2026l".to_vec()).unwrap();
        assert!(session.poll(&theme));
        assert!(session.rows().concat().contains("next frame"));
        program.send(b" plain".to_vec()).unwrap();
        assert!(session.poll(&theme));
    }

    /// A paste far larger than a terminal's buffer into a program that isn't reading, as one
    /// busy or paused: it's queued, and the paste comes back at once, where it waited for the
    /// program and held up every frame and key meanwhile.
    #[test]
    #[cfg(unix)]
    fn a_paste_into_a_program_not_reading_doesnt_wait_for_it() {
        let dir = tempfile::tempdir().unwrap();
        let argv = ["sh".to_string(), "-c".to_string(), "sleep 30".to_string()];
        let mut session =
            Session::start("Test", &argv, &[], dir.path(), (80, 24), (10, 20)).unwrap();
        let started = std::time::Instant::now();
        session.paste(&"pasted text\n".repeat(90_000));
        session.key(KeyEvent::plain(Key::Char('x')));
        let took = started.elapsed();
        assert!(took < std::time::Duration::from_millis(100), "{took:?}");
        assert!(!session.take_refused(), "a megabyte is queued");
        session.stop();
    }

    #[test]
    fn a_program_is_told_the_real_background() {
        let theme = Theme::named(Some("light"));
        let background = NamedColor::Background as usize;
        assert_eq!(asked_colour(background, &theme), theme.sidebar.bg);
        let foreground = NamedColor::Foreground as usize;
        assert_eq!(asked_colour(foreground, &theme), theme.text.fg);
        assert_eq!(asked_colour(11, &theme), LIGHT[11]);
    }

    #[test]
    fn keys_become_what_a_terminal_sends() {
        let ctrl = Mods {
            ctrl: true,
            ..Mods::default()
        };
        let plain = |key| KeyEvent::plain(key);
        assert_eq!(key_bytes(plain(Key::Char('é')), false), "é".as_bytes());
        assert_eq!(key_bytes(KeyEvent::ctrl('c'), false), [3]);
        assert_eq!(key_bytes(plain(Key::Up), false), b"\x1b[A");
        assert_eq!(key_bytes(plain(Key::Up), true), b"\x1bOA");
        assert_eq!(key_bytes(plain(Key::Backspace), false), [0x7f]);
        let word = KeyEvent {
            key: Key::Left,
            mods: ctrl,
        };
        assert_eq!(key_bytes(word, false), b"\x1b[1;5D");
        assert_eq!(key_bytes(plain(Key::F(5)), false), b"\x1b[15~");
        assert_eq!(key_bytes(plain(Key::F(1)), false), b"\x1bOP");
    }

    /// The pane opens at its share, the file keeping its 44 columns and the pane its 40.
    /// Dragged, the line goes further: each side keeps 24, where it was held to the widths
    /// the pane opens at and so, in a window of 90 columns, hardly moved.
    #[test]
    fn the_file_beside_the_pane_keeps_its_room() {
        let mut dock = Dock {
            open: true,
            ..Dock::default()
        };
        assert_eq!(dock.width_in(120), 55);
        assert_eq!(
            dock.width_in(86),
            MIN_WIDTH,
            "at its share it has its least"
        );
        assert_eq!(dock.width_in(84), 84 - MIN_MAIN);
        dock.width = Some(200);
        assert_eq!(dock.width_in(120), 120 - DRAGGED_LEAST);
        dock.width = Some(1);
        assert_eq!(dock.width_in(120), DRAGGED_LEAST);
        assert_eq!(dock.width_in(60), 60, "too narrow to share");
        dock.open = false;
        assert_eq!(dock.width_in(120), 0);
    }

    #[test]
    fn the_colour_table_has_its_three_parts() {
        assert_eq!(indexed(1, &DARK), DARK[1]);
        assert_eq!(indexed(16, &DARK), Rgb(0, 0, 0));
        assert_eq!(indexed(231, &DARK), Rgb(255, 255, 255));
        assert_eq!(indexed(232, &DARK), Rgb(8, 8, 8));
    }
}
