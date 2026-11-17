use std::io::{self, Write};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use base64::Engine;
use cellix_input::{Event, Key, KeyEvent, Mods, MouseEvent, MouseKind};
use cellix_render::{CellMeasure, FontMeasure, Grid, Measure, SoftwareRasterizer};
use cellix_term::{
    Backend, Bleed, CellBackend, DEFAULT_QUOTA, PixelBackend, Placement, Smooth, Tier, detect,
    select_tier,
};
use crossterm::event::{
    self as ct, DisableBracketedPaste, DisableFocusChange, DisableMouseCapture,
    EnableBracketedPaste, EnableFocusChange, EnableMouseCapture, KeyCode, KeyEventKind,
    KeyModifiers, MouseButton, MouseEventKind,
};
#[cfg(not(windows))]
use crossterm::event::{
    KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use crossterm::execute;
use crossterm::style::Print;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode, size,
    window_size,
};

use crate::App;
use crate::prof::{Cause, Causes, Phase};
use crate::workspaces::{Workspace, Workspaces};

/// Set when the terminal hangs up or Cellix is asked to stop. Left to their default, those
/// signals end the process on the spot and leave the workspace socket behind.
static STOPPED: AtomicBool = AtomicBool::new(false);

#[cfg(unix)]
extern "C" fn stop(_: libc::c_int) {
    STOPPED.store(true, Ordering::Relaxed);
}

/// How often the watch looks at whether the terminal is still there.
#[cfg(unix)]
const TERMINAL_LOOK: Duration = Duration::from_secs(1);

/// Whether the terminal Cellix reads has gone: closed at its other end, or taken away. It
/// still answers as a terminal then, so it's asked the way a reader finds out, without
/// reading anything.
#[cfg(unix)]
fn terminal_gone() -> bool {
    let mut input = libc::pollfd {
        fd: libc::STDIN_FILENO,
        events: libc::POLLIN,
        revents: 0,
    };
    // SAFETY: one descriptor, in a struct that outlives the call, and no waiting.
    let found = unsafe { libc::poll(&mut input, 1, 0) };
    found > 0 && input.revents & (libc::POLLHUP | libc::POLLNVAL | libc::POLLERR) != 0
}

/// Hangs up and stops end the loop, so they leave the way Quit does.
///
/// A stop that lands while crossterm waits for input is retried inside crossterm, and if the
/// terminal then closes, crossterm reads its end over and over and never returns, so the loop
/// never sees the stop. A watch outside the loop ends Cellix after a moment in that case,
/// clearing `left` (the socket and the like) as the loop would have.
///
/// A terminal can also go with no hang-up reaching Cellix at all, when Cellix isn't in the
/// terminal's own session. Its input then reads as ended for ever and crossterm reads that for
/// ever: one Cellix was found a week later, still using a whole core. So the watch also looks
/// at whether the terminal is still there, and counts its going as a stop.
#[cfg(unix)]
fn catch_stops(left: Left) {
    use std::io::IsTerminal;
    for signal in [libc::SIGHUP, libc::SIGTERM] {
        // SAFETY: the handler only stores to an atomic, which is safe in a signal handler.
        unsafe {
            libc::signal(signal, stop as *const () as libc::sighandler_t);
        }
    }
    // Started on a terminal, so one that's gone can be told from one that never was.
    let on_terminal = io::stdin().is_terminal();
    std::thread::spawn(move || {
        let mut looked = Instant::now();
        while !STOPPED.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(100));
            if on_terminal && looked.elapsed() >= TERMINAL_LOOK {
                looked = Instant::now();
                if terminal_gone() {
                    STOPPED.store(true, Ordering::Relaxed);
                }
            }
        }
        // The loop sees a stop within a frame; leaving normally ends this thread with it.
        std::thread::sleep(Duration::from_secs(2));
        // In case the terminal is still there after all, it gets itself back.
        let _ = disable_raw_mode();
        let _ = write!(io::stdout(), "\x1b[?1049l\x1b[?25h");
        let _ = io::stdout().flush();
        let paths = left.lock().map(|l| l.clone()).unwrap_or_default();
        for path in paths {
            let _ = std::fs::remove_file(&path);
            let _ = std::fs::remove_dir_all(&path);
        }
        std::process::exit(0);
    });
}

/// Set once Cellix has kept what it must and cleared what it leaves, so a closing console
/// can let Windows end it.
#[cfg(windows)]
static FINISHED: AtomicBool = AtomicBool::new(false);

/// How long a closing console waits for Cellix to finish. Windows ends the process at about
/// five seconds whatever the handler is doing.
#[cfg(windows)]
const CLOSE_GRACE: Duration = Duration::from_millis(4500);

/// Closing the window, logging off or shutting down ends the loop as Quit does, and Windows
/// waits while it finishes, since it ends the process as soon as this returns.
#[cfg(windows)]
unsafe extern "system" fn console_closing(kind: u32) -> windows_sys::core::BOOL {
    use windows_sys::Win32::System::Console::{
        CTRL_CLOSE_EVENT, CTRL_LOGOFF_EVENT, CTRL_SHUTDOWN_EVENT,
    };
    if !matches!(
        kind,
        CTRL_CLOSE_EVENT | CTRL_LOGOFF_EVENT | CTRL_SHUTDOWN_EVENT
    ) {
        return 0;
    }
    STOPPED.store(true, Ordering::Relaxed);
    let started = Instant::now();
    while !FINISHED.load(Ordering::Relaxed) && started.elapsed() < CLOSE_GRACE {
        std::thread::sleep(Duration::from_millis(20));
    }
    1
}

/// Windows has no hang-up signal: a closing console calls a handler instead. The loop finishes
/// as it would on Quit, so unsent comments are kept and the socket and lock files go.
#[cfg(windows)]
fn catch_stops(_left: Left) {
    // SAFETY: the handler only touches atomics and sleeps, which any thread may do.
    unsafe {
        windows_sys::Win32::System::Console::SetConsoleCtrlHandler(Some(console_closing), 1);
    }
}

#[cfg(not(any(unix, windows)))]
fn catch_stops(_left: Left) {}

/// What a stop must clear if the loop can't: each workspace's socket and the like.
type Left = std::sync::Arc<std::sync::Mutex<Vec<PathBuf>>>;

/// How each new workspace is made to look, as the terminal said, and kept up as it changes.
struct Look {
    measure: Rc<dyn Measure>,
    wheel_lines: isize,
    light: Option<bool>,
    background: Option<(u8, u8, u8)>,
}

/// Makes a folder's workspace: its socket for agents and the IDE, what it left unsaved and
/// running last time, and the terminal's look.
fn start_workspace(
    root: PathBuf,
    size: (u16, u16),
    (look, left): (&Look, &Left),
    wait: bool,
) -> Workspace {
    let mut app = App::new(root, size);
    // As an editor for another program, only the file it asked for opens.
    if !wait {
        app.restore_state();
    }
    // Agents are optional: a second window on this workspace still edits normally. Anything
    // else that stops agents reaching this one is said, not swallowed.
    let server = match cellix_agent::start(app.workspace.root(), app.comments.clone()) {
        Ok(server) => Some(server),
        Err(e) if e.kind() == io::ErrorKind::AddrInUse => {
            app.say("Another Cellix has this folder open, so agents reach that one");
            None
        }
        Err(e) => {
            app.say(format!("Agents can't reach this Cellix: {e}"));
            None
        }
    };
    if server.is_some() {
        app.restore_unsent();
    }
    app.ide = server.as_ref().and_then(|_| {
        let root = app.workspace.root();
        let kept = crate::hold::folder_of(root).join("ide.json");
        cellix_agent::ide::IdeServer::start_kept(root, &kept).ok()
    });
    if let Ok(mut left) = left.lock() {
        if server.is_some() {
            let socket = cellix_agent::socket_path(app.workspace.root());
            left.push(socket.with_extension(""));
            left.push(socket);
        }
        left.extend(app.ide.as_ref().map(|ide| ide.lock().to_path_buf()));
    }
    app.set_measure(look.measure.clone());
    app.wheel_lines = look.wheel_lines;
    app.follow_terminal_theme(look.light);
    app.rejoin_sessions();
    app.match_terminal_background(look.background);
    Workspace { app, server }
}

/// How often a scroll moves. Over a network each frame costs a round of placements.
const SCROLL_FRAME: Duration = Duration::from_millis(8);
const REMOTE_SCROLL_FRAME: Duration = Duration::from_millis(33);
/// How quickly a scroll closes on where it's going. It only blends one line into the next: any
/// longer and the page trails the hand, which reads as lag and not as smoothness.
const SCROLL_EASE: Duration = Duration::from_millis(14);
/// The longest to spend on queued input before drawing.
const INPUT_SLICE: Duration = Duration::from_millis(4);
/// How often the page under a scroll is brought up to date while it moves.
const SCROLL_PRESENT: Duration = Duration::from_millis(120);
/// How long to wait for input before checking on language servers.
const POLL: Duration = Duration::from_millis(50);
/// How long to wait with a session in the side pane, whose output should feel typed.
const SESSION_POLL: Duration = Duration::from_millis(8);
/// The same once nothing has been typed or printed for a while. A session at its prompt, or
/// an agent thinking, is looked at less often; the first key or line brings the quick wait
/// back.
const SESSION_REST: Duration = Duration::from_millis(40);
/// How long after the last key or output a session still counts as lively.
const LIVELY: Duration = Duration::from_millis(300);

/// How long the loop waits for input before it looks at the sessions again, `quiet` after
/// the last key or change.
fn session_wait(quiet: Duration) -> Duration {
    if quiet < LIVELY {
        SESSION_POLL
    } else {
        SESSION_REST
    }
}
/// Background file checks run about twice a second.
const TICK: Duration = Duration::from_millis(500);

/// How long the loop waits for input: what it waits anyway, as little as a scroll under way
/// asks, and no longer than until the next step of a spinner that's turning, which is the only
/// time one asks anything. Nothing turning, it changes nothing.
fn wait_for(
    rest: Duration,
    glide: Option<Duration>,
    turning: Option<Instant>,
    now: Instant,
) -> Duration {
    let wait = glide.unwrap_or(rest);
    turning.map_or(wait, |at| wait.min(at.saturating_duration_since(now)))
}

/// What the loop shows this time round.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Frame {
    /// Something changed: everything is drawn again.
    Whole,
    /// Only a spinner's step is due: the frame shown, with its spinners a step on.
    Step,
    /// Nothing.
    Same,
}

/// Which, from whether anything asked for a frame, when a spinner's next step is due, and
/// whether there's a frame shown to take it on.
fn frame_now(redraw: bool, turning: Option<Instant>, shown: bool, now: Instant) -> Frame {
    let due = turning.is_some_and(|at| now >= at);
    if redraw || (due && !shown) {
        Frame::Whole
    } else if due {
        Frame::Step
    } else {
        Frame::Same
    }
}

/// When a grid just drawn wants drawing again: at the next step of its spinners' turn, if
/// anything on it turns.
fn next_turn(grid: &Grid, now: Instant) -> Option<Instant> {
    use cellix_render::spinner;
    grid.spinning
        .then(|| now + spinner::until_next(spinner::clock()))
}

/// Restores the terminal however the loop ends, including on panic.
struct Session;

impl Session {
    fn start() -> io::Result<Self> {
        enable_raw_mode()?;
        execute!(
            io::stdout(),
            EnterAlternateScreen,
            EnableMouseCapture,
            EnableBracketedPaste,
            EnableFocusChange,
        )?;
        // Terminals without the Kitty keyboard protocol ignore this, and Ctrl+Shift+P
        // and Cmd keys then can't be told apart: F1 and Alt+P still open commands. On Windows
        // keys arrive as console events, which have no such protocol, and crossterm refuses it.
        #[cfg(not(windows))]
        execute!(
            io::stdout(),
            PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
        )?;
        // tmux ignores kitty's flags but passes modified keys on with its extended-keys, once
        // asked this way, so Ctrl+Shift+P stops arriving as Ctrl+P. Only in the csi-u form:
        // the xterm form, tmux's default, is one crossterm can't read, and every Ctrl key
        // would be lost.
        if std::env::var_os("TMUX").is_some() && tmux_keys_readable() {
            execute!(io::stdout(), Print("\x1b[>4;2m"))?;
        }
        Ok(Self)
    }
}

/// Whether tmux sends extended keys in the form crossterm reads.
fn tmux_keys_readable() -> bool {
    let output = std::process::Command::new("tmux")
        .args(["show", "-sv", "extended-keys-format"])
        .output();
    output.is_ok_and(|o| String::from_utf8_lossy(&o.stdout).trim() == "csi-u")
}

impl Drop for Session {
    fn drop(&mut self) {
        #[cfg(not(windows))]
        let _ = execute!(io::stdout(), PopKeyboardEnhancementFlags);
        let _ = execute!(
            io::stdout(),
            Print("\x1b[?1016l"),
            Print(cellix_term::clipboard::PASTE_EVENTS_OFF),
            Print(if std::env::var_os("TMUX").is_some() {
                "\x1b[>4;0m"
            } else {
                ""
            }),
            DisableBracketedPaste,
            DisableMouseCapture,
            DisableFocusChange,
            LeaveAlternateScreen,
            crossterm::cursor::Show
        );
        let _ = disable_raw_mode();
    }
}

fn mods(m: KeyModifiers) -> Mods {
    Mods {
        // Cmd on macOS behaves as Ctrl, so VS Code habits work on either key.
        ctrl: m.intersects(KeyModifiers::CONTROL | KeyModifiers::SUPER),
        alt: m.contains(KeyModifiers::ALT),
        shift: m.contains(KeyModifiers::SHIFT),
    }
}

/// `pixels` is the cell size when the mouse reports pixels instead of cells.
fn translate(event: ct::Event, pixels: Option<(usize, usize)>) -> Option<Event> {
    Some(match event {
        ct::Event::Key(k) if k.kind != KeyEventKind::Release => {
            let key = match k.code {
                KeyCode::Char(c) => Key::Char(c),
                KeyCode::Enter => Key::Enter,
                KeyCode::Tab => Key::Tab,
                KeyCode::BackTab => Key::BackTab,
                KeyCode::Backspace => Key::Backspace,
                KeyCode::Delete => Key::Delete,
                KeyCode::Esc => Key::Esc,
                KeyCode::Left => Key::Left,
                KeyCode::Right => Key::Right,
                KeyCode::Up => Key::Up,
                KeyCode::Down => Key::Down,
                KeyCode::Home => Key::Home,
                KeyCode::End => Key::End,
                KeyCode::PageUp => Key::PageUp,
                KeyCode::PageDown => Key::PageDown,
                KeyCode::F(n) => Key::F(n),
                _ => return None,
            };
            Event::Key(KeyEvent {
                key,
                mods: mods(k.modifiers),
            })
        }
        ct::Event::Mouse(m) => {
            let kind = match m.kind {
                MouseEventKind::Down(MouseButton::Left) => MouseKind::Left,
                MouseEventKind::Down(MouseButton::Middle) => MouseKind::Middle,
                MouseEventKind::Down(MouseButton::Right) => MouseKind::Right,
                MouseEventKind::Up(_) => MouseKind::Up,
                MouseEventKind::Drag(_) => MouseKind::Drag,
                MouseEventKind::Moved => MouseKind::Move,
                MouseEventKind::ScrollUp => MouseKind::ScrollUp,
                MouseEventKind::ScrollDown => MouseKind::ScrollDown,
                MouseEventKind::ScrollLeft => MouseKind::ScrollLeft,
                MouseEventKind::ScrollRight => MouseKind::ScrollRight,
            };
            let (x, y, fine_x, fine_y) = match pixels {
                Some((cw, ch)) => {
                    let fine = m.column as f32 / cw.max(1) as f32;
                    let fine_y = m.row as f32 / ch.max(1) as f32;
                    (fine as u16, m.row / ch.max(1) as u16, fine, fine_y)
                }
                None => (m.column, m.row, m.column as f32, m.row as f32 + 0.5),
            };
            Event::Mouse(MouseEvent {
                kind,
                x,
                y,
                fine_x,
                fine_y,
                mods: mods(m.modifiers),
            })
        }
        ct::Event::Paste(text) => Event::Paste(text),
        ct::Event::Resize(w, h) => Event::Resize(w, h),
        _ => return None,
    })
}

/// The character a key types into text, if it's a key that types and isn't a shortcut.
fn typed(event: &ct::Event) -> Option<char> {
    let ct::Event::Key(key) = event else {
        return None;
    };
    if key.kind == KeyEventKind::Release {
        return None;
    }
    let plain = key.modifiers.difference(KeyModifiers::SHIFT).is_empty();
    match key.code {
        KeyCode::Char(c) if plain => Some(c),
        KeyCode::Enter if key.modifiers.is_empty() => Some('\n'),
        KeyCode::Tab if key.modifiers.is_empty() => Some('\t'),
        _ => None,
    }
}

/// What the terminal says of its clipboard (OSC 5522), put back together. crossterm doesn't
/// know the sequence and hands it over as keys: Alt+] for `ESC ]`, or Esc and then `]` when a
/// read ends between them, a key for each character, and Alt+\ (or Esc, `\`) for `ESC \`, or
/// Ctrl+G for BEL, at its end. It's only done where paste events are on, so in Ghostty or kitty,
/// whose Kitty keyboard protocol sends a real Esc or Alt+] as a CSI u key: what spells
/// `ESC ] 5522 ;` there is the terminal's. Keys held back while it isn't yet clear go on as
/// they were.
#[derive(Default)]
struct Osc {
    /// Paste events are on, so messages can come.
    on: bool,
    /// Keys held while they spell the start of a message, and how much of it.
    held: Vec<ct::Event>,
    matched: usize,
    /// The message so far, once it's certain, and whether an `ESC` just came in it.
    body: Option<String>,
    escaped: bool,
}

/// What a key comes to, read by `Osc`.
enum Took {
    /// Kept back, as part of a message.
    Held,
    /// Keys for Cellix as usual.
    Keys(Vec<ct::Event>),
    /// A whole message's body, after `5522;`.
    Message(String),
}

/// A key as the bytes the terminal sent for it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Sent {
    Esc,
    Bell,
    Char(char),
}

/// How a message starts: `ESC ] 5522 ;`.
const OSC_START: [Sent; 7] = [
    Sent::Esc,
    Sent::Char(']'),
    Sent::Char('5'),
    Sent::Char('5'),
    Sent::Char('2'),
    Sent::Char('2'),
    Sent::Char(';'),
];

/// How long to wait for the rest of a message under way before drawing a frame.
const OSC_PATIENCE: Duration = Duration::from_millis(100);
/// The most a message is let grow to: a picture over that isn't worth the memory.
const OSC_LIMIT: usize = 96 << 20;

/// The bytes behind a key, where it's one a message can be made of.
fn sent(event: &ct::Event) -> Option<Vec<Sent>> {
    let ct::Event::Key(key) = event else {
        return None;
    };
    if key.kind != KeyEventKind::Press {
        return None;
    }
    let mods = key.modifiers.difference(KeyModifiers::SHIFT);
    match key.code {
        KeyCode::Esc if mods.is_empty() => Some(vec![Sent::Esc]),
        KeyCode::Char(c) if mods.is_empty() => Some(vec![Sent::Char(c)]),
        KeyCode::Char(c) if mods == KeyModifiers::ALT => Some(vec![Sent::Esc, Sent::Char(c)]),
        KeyCode::Char('g') if mods == KeyModifiers::CONTROL => Some(vec![Sent::Bell]),
        _ => None,
    }
}

impl Osc {
    fn new(on: bool) -> Self {
        Self {
            on,
            ..Self::default()
        }
    }

    /// Whether a message may have started and not ended, so the input should be read on. The
    /// start of one counts, since the terminal's write can be split anywhere.
    fn reading(&self) -> bool {
        self.body.is_some() || self.matched > 0
    }

    fn take(&mut self, event: ct::Event) -> Took {
        if !self.on {
            return Took::Keys(vec![event]);
        }
        let bytes = sent(&event);
        if let Some(body) = &mut self.body {
            let Some(bytes) = bytes else {
                // Anything else ends it unfinished: what came of it is dropped.
                self.body = None;
                self.escaped = false;
                return Took::Keys(vec![event]);
            };
            for byte in bytes {
                match (self.escaped, byte) {
                    (true, Sent::Char('\\')) | (false, Sent::Bell) => {
                        self.escaped = false;
                        return Took::Message(self.body.take().unwrap_or_default());
                    }
                    (false, Sent::Esc) => self.escaped = true,
                    (false, Sent::Char(c)) if body.len() < OSC_LIMIT => body.push(c),
                    _ => {
                        self.body = None;
                        self.escaped = false;
                        return Took::Keys(vec![event]);
                    }
                }
            }
            return Took::Held;
        }
        let spells = bytes.as_ref().is_some_and(|bytes| {
            let rest = &OSC_START[self.matched..];
            rest.len() >= bytes.len() && rest[..bytes.len()] == bytes[..]
        });
        if !spells {
            let mut keys = self.release();
            keys.push(event);
            return Took::Keys(keys);
        }
        self.matched += bytes.map_or(0, |b| b.len());
        if self.matched == OSC_START.len() {
            self.release();
            self.body = Some(String::new());
        } else {
            self.held.push(event);
        }
        Took::Held
    }

    /// Gives back what's held when no more input came: an Alt+] on its own was a key.
    fn release(&mut self) -> Vec<ct::Event> {
        self.matched = 0;
        std::mem::take(&mut self.held)
    }
}

/// Whether `events` starts with a bracketed paste's marker, `Esc [ 2 0 <last> ~`, sent as keys.
fn marker(events: &[ct::Event], last: char) -> bool {
    let is_esc = matches!(
        events.first(),
        Some(ct::Event::Key(k)) if k.code == KeyCode::Esc && k.kind != KeyEventKind::Release
    );
    is_esc
        && events.len() >= 6
        && events[1..6]
            .iter()
            .map(typed)
            .eq(['[', '2', '0', last, '~'].map(Some))
}

/// How long after a paste read as keys more of it may still come, in the next read. The
/// console hands a long paste over in parts, and no one types again this soon after one.
const PASTE_TAIL: Duration = Duration::from_millis(50);

/// A paste read as keys that may go on in the next read: one whose closing marker hasn't
/// come yet, or one that ran to the end of what was read.
#[derive(Default)]
struct Pasting {
    open: bool,
    until: Option<Instant>,
}

/// Windows' console gives Cellix no pastes, only the keys that would type them, and typed,
/// a paste's brackets close themselves and its lines indent. So in one batch of input, keys
/// between a bracketed paste's markers become a paste, as do keys that type more after a line
/// break or a tab, which no one types faster than Cellix reads. Anything else stays keys.
/// The console can hand a paste over in two reads, and its last line break, read alone as
/// Enter, would accept a suggestion: so a paste still open, or one that ran to the end of the
/// last read moments ago, takes the keys that start this one.
fn pasted_keys(events: Vec<ct::Event>, pasting: &mut Pasting, now: Instant) -> Vec<ct::Event> {
    // Releases say nothing here, and would split a run of keys.
    let events: Vec<ct::Event> = events
        .into_iter()
        .filter(|e| !matches!(e, ct::Event::Key(k) if k.kind == KeyEventKind::Release))
        .collect();
    let text = |run: &[ct::Event]| run.iter().filter_map(typed).collect::<String>();
    let mut out = Vec::new();
    let mut i = 0;
    let closing = |from: usize| (from..events.len()).find(|&j| marker(&events[j..], '1'));
    let carried_on = pasting.until.take().is_some_and(|until| now <= until);
    if std::mem::take(&mut pasting.open) {
        let end = closing(0);
        let stop = end.unwrap_or(events.len());
        out.push(ct::Event::Paste(text(&events[..stop])));
        pasting.open = end.is_none();
        i = end.map_or(events.len(), |end| end + 6);
    } else if carried_on {
        let run = events.iter().take_while(|e| typed(e).is_some()).count();
        if run > 0 {
            out.push(ct::Event::Paste(text(&events[..run])));
            i = run;
        }
        // Its closing marker, when it had one.
        if marker(&events[i..], '1') {
            i += 6;
        }
    }
    // Whether what was read last went into a paste, up to its end.
    let mut tail = !out.is_empty() && i >= events.len();
    while i < events.len() {
        if marker(&events[i..], '0') {
            let start = i + 6;
            let end = closing(start);
            out.push(ct::Event::Paste(text(
                &events[start..end.unwrap_or(events.len())],
            )));
            pasting.open = end.is_none();
            i = end.map_or(events.len(), |end| end + 6);
            tail = end.is_none();
            continue;
        }
        let run = events[i..]
            .iter()
            .take_while(|e| typed(e).is_some())
            .count();
        let typed_run = &events[i..i + run];
        let breaks_then_types = typed_run
            .iter()
            .take(run.saturating_sub(1))
            .any(|e| matches!(typed(e), Some('\n' | '\t')));
        if run >= 3 && breaks_then_types {
            out.push(ct::Event::Paste(text(typed_run)));
            tail = i + run >= events.len();
        } else {
            out.extend_from_slice(&events[i..i + run.max(1)]);
            tail = false;
        }
        i += run.max(1);
    }
    if tail && !pasting.open {
        pasting.until = Some(now + PASTE_TAIL);
    }
    out
}

/// Ghostty sends Ctrl+A for Cmd+Left, and keeps Cmd+A for itself, so there Ctrl+A means
/// the start of the line. Select All stays in the palette.
fn cmd_left(event: Event, ghostty: bool) -> Event {
    match event {
        Event::Key(key) if ghostty && key == KeyEvent::ctrl('a') => {
            Event::Key(KeyEvent::plain(Key::Home))
        }
        other => other,
    }
}

/// How the terminal was found to work, kept so the backend can be rebuilt on a font zoom.
struct Display {
    placement: Option<Placement>,
    cell: Option<(usize, usize)>,
    /// The terminal said its cell size itself. Under tmux the window's own size can be one
    /// tmux made up, so a resize doesn't overrule it.
    told_cell: bool,
    /// The terminal said its cell size when it was asked at the start, so it can be asked
    /// again. One that doesn't answer would only be waited on, with what's typed meanwhile
    /// taken for its answer.
    answers_cell: bool,
    pixel_mouse: bool,
    /// The terminal sends paste events, so pictures can be pasted.
    paste_events: bool,
    light: Option<bool>,
    background: Option<(u8, u8, u8)>,
    /// Where the colour is asked again when Cellix comes back into focus.
    multiplexer: cellix_term::Multiplexer,
    passthrough: bool,
    smooth: Option<Smooth>,
    bleed: Bleed,
    /// How many times the terminal's line a row of Cellix's own is tall, where it's asked
    /// for. See `row`.
    own_rows: Option<f32>,
    /// The terminal's lines, which a row's height is fitted to. See `fitted`.
    lines: u16,
    /// Whether it's fitted: not where `CELLIX_ROW_FIT` is 0, which is how the harness keeps
    /// rows the height it was told they are.
    fit: bool,
}

/// The height of a row, near `nominal` pixels, that leaves the least of a window of `lines`
/// lines of `line` pixels over under its last row. Rows of one fixed height left up to a
/// whole row over, all of it under the bar at the foot, where it read as a window with too
/// much space at its bottom. A row a twentieth taller or shorter isn't seen, and leaves a
/// few pixels in a window of up to some 45 rows. (One that holds about as many rows as a row
/// has pixels gains nothing from a pixel more or less on each, and keeps what it leaves.)
/// The nearest to `nominal` wins among heights that leave as little.
fn fitted(nominal: usize, line: usize, lines: u16) -> usize {
    let tall = lines as usize * line;
    let reach = ((nominal as f32 * 0.05).round() as usize).max(1);
    let from = nominal.saturating_sub(reach).max(line + 1);
    (from..=nominal + reach)
        .min_by_key(|row| (tall % row, row.abs_diff(nominal)))
        .unwrap_or(nominal)
}

/// A press under the last row is on it. The bar at the foot is drawn down into what the
/// window has left under its rows, and into the terminal's padding where that's reached.
fn on_the_grid(event: Event, rows: u16) -> Event {
    match event {
        Event::Mouse(mut mouse) if rows > 0 && mouse.y >= rows => {
            mouse.y = rows - 1;
            mouse.fine_y = f32::from(rows) - 0.01;
            Event::Mouse(mouse)
        }
        event => event,
    }
}

/// A press at the foot is on the row drawn where it lands. A foot of two rows stands them
/// below their own places, evenly in what's under the sheets, so the row drawn at a place
/// isn't always the one that begins there: `drops` is how far down each stands, in pixels,
/// under rows `row` pixels tall. Between two of them a press is on the nearer. One row at
/// the foot needs none of this: whatever is under the sheets is that row.
fn on_the_foot(event: Event, rows: u16, row: usize, drops: &[usize]) -> Event {
    let Event::Mouse(mut mouse) = event else {
        return event;
    };
    let first = rows.saturating_sub(drops.len() as u16);
    if drops.len() < 2 || row == 0 || mouse.fine_y < f32::from(first) {
        return Event::Mouse(mouse);
    }
    let tall = row as f32;
    // How far down the foot the press is, in pixels.
    let at = (mouse.fine_y - f32::from(first)) * tall;
    // The first row whose picture, with half of what's between it and the next, reaches it.
    let reaches = |index: usize| {
        let ends = (index + 1) as f32 * tall + drops[index] as f32;
        let between = drops.get(index + 1).map_or(f32::INFINITY, |next| {
            next.saturating_sub(drops[index]) as f32 / 2.0
        });
        at < ends + between
    };
    let on = (0..drops.len())
        .find(|index| reaches(*index))
        .unwrap_or(drops.len() - 1);
    let into = (at - on as f32 * tall - drops[on] as f32) / tall;
    mouse.y = first + on as u16;
    mouse.fine_y = f32::from(mouse.y) + into.clamp(0.0, 0.99);
    Event::Mouse(mouse)
}

/// How far down the window the middle of each row is, in pixels, for the harness to press
/// on a row where it's drawn: the rows at the foot stand below where they begin.
fn row_middles(rows: u16, row: usize, drops: &[usize]) -> String {
    let first = rows.saturating_sub(drops.len() as u16);
    let middle = |y: u16| {
        let drop = y.checked_sub(first).map_or(0, |at| drops[at as usize]);
        y as usize * row + row / 2 + drop
    };
    let middles: Vec<String> = (0..rows).map(|y| middle(y).to_string()).collect();
    middles.join("\n")
}

/// How tall a row of Cellix's own is, in lines of the terminal's: what Settings' Row height
/// says, unless `CELLIX_ROW_HEIGHT` says otherwise, which is how the harness chooses. A row
/// no taller than a line is a line, and only heights that leave a window some rows count.
fn own_rows(asked: Option<&str>, setting: f32) -> Option<f32> {
    let asked = asked.and_then(|asked| asked.trim().parse().ok());
    let times: f32 = asked.unwrap_or(setting);
    (1.05..=2.0).contains(&times).then_some(times)
}

/// What `CELLIX_ROW_HEIGHT` says, if it's set.
fn asked_rows() -> Option<String> {
    std::env::var("CELLIX_ROW_HEIGHT").ok()
}

/// How many rows `row` pixels tall fit in `lines` lines of `line` pixels.
fn rows_in(lines: u16, line: usize, row: usize) -> u16 {
    (lines as usize * line / row.max(1)).clamp(1, u16::MAX as usize) as u16
}

impl Display {
    fn detect(setting: Option<&str>, smooth: bool, rows: f32) -> io::Result<Self> {
        let forced = std::env::var("CELLIX_TIER")
            .ok()
            .as_deref()
            .or(setting)
            .and_then(|v| v.parse().ok());
        let env = detect(|k| std::env::var(k).ok())?;
        let placement = match select_tier(env, forced) {
            Tier::PixelsDirect => Some(Placement::Direct),
            Tier::PixelsPlaceholder => Some(Placement::Placeholder),
            Tier::Cells => None,
        };
        let cell = env.cell.or_else(window_cell);
        let bleed = cell.map_or_else(Bleed::default, |cell| {
            crate::integrations::window_bleed(|k| std::env::var(k).ok(), cell.1)
        });
        Ok(Self {
            bleed,
            own_rows: own_rows(asked_rows().as_deref(), rows),
            lines: 0,
            fit: std::env::var("CELLIX_ROW_FIT").map_or(true, |fit| fit.trim() != "0"),
            placement,
            cell,
            told_cell: env.cell.is_some() && env.multiplexer == cellix_term::Multiplexer::Tmux,
            answers_cell: env.cell.is_some(),
            pixel_mouse: env.pixel_mouse,
            // Set to 0, it leaves pastes as plain bracketed text, should paste events misbehave.
            paste_events: env.paste_events
                && std::env::var("CELLIX_PASTE_EVENTS")
                    .ok()
                    .is_none_or(|v| v != "0"),
            light: env.light,
            background: env.background,
            multiplexer: env.multiplexer,
            passthrough: env.tmux_passthrough,
            smooth: smooth.then(|| Smooth {
                frame: if std::env::var_os("SSH_CONNECTION").is_some() {
                    REMOTE_SCROLL_FRAME
                } else {
                    SCROLL_FRAME
                },
                ease: SCROLL_EASE,
            }),
        })
    }

    /// Turns the wheel's animation on or off.
    fn set_smooth(&mut self, on: bool) {
        self.smooth = on.then_some(Smooth {
            frame: SCROLL_FRAME,
            ease: SCROLL_EASE,
        });
    }

    /// A row's height in pixels where Cellix lays out rows of its own, taller than the
    /// terminal's line, so 13 pt words have air round them whatever the terminal's line
    /// height. Only where a tile can stand between two lines and the terminal says where the
    /// mouse is in pixels: direct placement, with a known cell. Everywhere else a row is a
    /// line.
    fn row(&self) -> Option<usize> {
        let (_, line) = self.cell?;
        let times = self.own_rows?;
        let can = self.placement == Some(Placement::Direct) && self.pixel_mouse;
        let nominal = (line as f32 * times).round() as usize;
        if !can || nominal <= line {
            return None;
        }
        let fits = self.fit && self.lines > 0;
        Some(if fits {
            fitted(nominal, line, self.lines)
        } else {
            nominal
        })
    }

    /// Takes the terminal's lines as they are now, and says whether a row's height changed
    /// with them: everything is then measured and drawn again.
    fn fit_to(&mut self, lines: u16) -> bool {
        let before = self.row();
        self.lines = lines;
        self.row() != before
    }

    /// The grid's size in a terminal of so many columns and lines.
    fn grid_size(&self, (cols, lines): (u16, u16)) -> (u16, u16) {
        match (self.row(), self.cell) {
            (Some(row), Some((_, line))) => (cols, rows_in(lines, line, row)),
            _ => (cols, lines),
        }
    }

    /// How to measure text for this display.
    fn measure(&self) -> Rc<dyn Measure> {
        match self.placement.zip(self.cell) {
            Some((_, cell)) => {
                let row = self.row().unwrap_or(cell.1);
                Rc::new(FontMeasure::new(cell.0, row).with_type_row(cell.1))
            }
            None => Rc::new(CellMeasure),
        }
    }

    /// Any failure on the way to pixels lands on cells, which always work.
    fn backend(&self) -> Box<dyn Backend> {
        let pixels = self.placement.zip(self.cell).and_then(|(placement, cell)| {
            let row = self.row();
            let rasterizer =
                SoftwareRasterizer::new(cell.0, row.unwrap_or(cell.1))?.with_type_row(cell.1);
            let backend =
                PixelBackend::new(rasterizer, placement, DEFAULT_QUOTA).with_bleed(self.bleed);
            let backend = match row {
                Some(row) => backend.with_own_rows(cellix_term::OwnRows { row, line: cell.1 }),
                None => backend,
            };
            Some(match self.smooth {
                Some(smooth) => backend.with_smooth(smooth),
                None => backend,
            })
        });
        match pixels {
            Some(backend) => Box::new(backend),
            None => Box::new(CellBackend::default()),
        }
    }
}

/// Cell size from the window's pixel size, or None when the terminal reports no pixels.
/// How long the window's size has to hold still before the terminal is asked its cell size:
/// a window being dragged changes many times a second, and each question is a round trip.
const CELL_SETTLE: Duration = Duration::from_millis(200);

/// The window's size in pixels, as the terminal reports it, to tell when it changes.
fn window_pixels() -> Option<(u16, u16)> {
    let window = window_size().ok()?;
    Some((window.width, window.height))
}

fn window_cell() -> Option<(usize, usize)> {
    let window = window_size().ok()?;
    let cell = (
        window.width as usize / window.columns.max(1) as usize,
        window.height as usize / window.rows.max(1) as usize,
    );
    (cell.0 > 0 && cell.1 > 0).then_some(cell)
}

/// What the command line asked for.
#[derive(Debug, Default)]
pub struct Options {
    pub paths: Vec<PathBuf>,
    /// Behave as `$VISUAL`: write review comments into the file before exiting.
    pub wait: bool,
}

/// Opens the requested files and runs until the user quits.
pub fn run(options: Options) -> io::Result<()> {
    let paths = options.paths;
    let named = paths.iter().find(|p| p.is_dir()).cloned();
    let root = dunce::canonicalize(named.clone().map_or_else(std::env::current_dir, Ok)?)?;
    // Started at home with nothing named, there's no project here: what's running on the
    // machine comes first, if anything is.
    let home = std::env::var_os("HOME").and_then(|h| dunce::canonicalize(h).ok());
    let away = named.is_none() && paths.is_empty() && home.as_deref() == Some(root.as_path());
    let settings = crate::settings::Settings::load(&root);
    let left: Left = Default::default();
    catch_stops(left.clone());
    let _session = Session::start()?;
    let smooth = settings.smooth_scroll != Some(false);
    let rows = crate::settings::row_height_times(settings.row_height.as_deref());
    let mut display = Display::detect(settings.tier.as_deref(), smooth, rows)?;
    display.fit_to(size()?.1);
    let mut backend = display.backend();
    // Counted, for `CELLIX_PROF`: an addition a write, whether or not it's on.
    let mut out = crate::prof::Counted::new(io::stdout());
    let mut slow = crate::slow::Log::start();
    let mut phases = crate::slow::Phases::default();
    let mut prof = crate::prof::Prof::start();
    // Why the next frame is drawn, for the counters.
    let mut causes = Causes::default();
    causes.add(Cause::Resize);
    let mut last_present = Instant::now();
    let look = Rc::new(std::cell::RefCell::new(Look {
        measure: display.measure(),
        // Only the direct tier moves by less than a line, so elsewhere a notch stays short.
        wheel_lines: if display.placement == Some(Placement::Direct) {
            3
        } else {
            1
        },
        light: display.light,
        background: display.background,
    }));
    let wait = options.wait;
    let starting = (look.clone(), left.clone());
    let start = Box::new(move |root: PathBuf, size: (u16, u16)| {
        start_workspace(root, size, (&starting.0.borrow(), &starting.1), wait)
    });
    let mut spaces = Workspaces::new(
        display.grid_size(size()?),
        crate::app::machine_name(),
        start,
    );
    spaces.bar = !wait;
    if away {
        spaces.open_running();
        spaces.choosing = !spaces.list.is_empty();
    }
    if spaces.list.is_empty() {
        spaces.open(root);
        for path in paths.iter().filter(|p| p.is_file()) {
            let path = dunce::canonicalize(path)?;
            spaces.app_mut().open(&path)?;
            if wait {
                spaces.app_mut().wait_file = Some(path);
            }
        }
        // What else is running on the machine is a click away, as in tmux, unless Cellix is
        // only here to edit a file for another program.
        if !wait {
            spaces.open_running();
        }
    }
    // Whole cells can't place a caret in proportional text, so ask for pixels where that's
    // on offer. It's only used with a known cell size to divide by.
    let pixel_mouse = display.pixel_mouse && display.placement.is_some() && display.cell.is_some();
    if pixel_mouse {
        write!(out, "\x1b[?1016h")?;
    }
    // Under rows of Cellix's own a press is so many of those down, not so many lines.
    let mouse_cell = |display: &Display| {
        let cell = display.cell.filter(|_| pixel_mouse)?;
        Some((cell.0, display.row().unwrap_or(cell.1)))
    };
    // A paste then says what the clipboard holds, so a picture can come over SSH as well as
    // text: OSC 52 carries only text.
    if display.paste_events {
        write!(out, "{}", cellix_term::clipboard::PASTE_EVENTS_ON)?;
    }
    let mut osc = Osc::new(display.paste_events);
    let mut clipboard = cellix_term::clipboard::Reader::default();
    let mut redraw = true;
    let mut shape = None;
    // A paste on Windows that the console handed over in parts.
    let mut pasting = Pasting::default();
    let mut last_tick = std::time::Instant::now();
    // When a key last came or anything last changed, which sets how often sessions are read.
    let mut lively = Instant::now();
    let mut shown: Option<Grid> = None;
    // When something on screen turns, when its next step is due.
    let mut turning: Option<Instant> = None;
    // How far below its own place each row at the foot was last drawn. See `on_the_foot`.
    let mut foot: Vec<usize> = Vec::new();
    let ghostty = std::env::var("TERM").is_ok_and(|t| t.contains("ghostty"));
    // Where the screen is also written as text, for the harness: its terminal gets pictures,
    // and the tour has to know a scene reached its surface before taking one of it.
    let screen_text = std::env::var_os("CELLIX_SCREEN_TEXT").map(PathBuf::from);
    // What the terminal is decides which notification sequence it reads.
    let notice =
        cellix_term::Notice::for_terminal(&cellix_term::terminal_names(|n| std::env::var(n).ok()));
    let inside_tmux = display.multiplexer == cellix_term::Multiplexer::Tmux;
    // When the terminal is next asked how big a cell is, and the window's pixels as last seen.
    let mut cell_asked: Option<Instant> = None;
    let mut pixels = window_pixels();
    // The loop's errors still come back, but only after what Quit does: a console closing
    // under Cellix can fail its last frame.
    #[allow(clippy::redundant_closure_call)]
    let ended = (|| -> io::Result<()> {
        loop {
            if let Some(prof) = &mut prof {
                prof.wake();
                prof.written(out.total());
                prof.worked(backend.take_work());
                prof.flush(Instant::now());
            }
            // Polled as well as signalled: a resize during slow startup is otherwise lost.
            let lines = size()?;
            if display.fit_to(lines.1) {
                // Rows of another height hold these lines with less left over.
                backend = display.backend();
                let measure = display.measure();
                look.borrow_mut().measure = measure.clone();
                for workspace in &mut spaces.list {
                    workspace.app.set_measure(measure.clone());
                }
                backend.invalidate();
                shown = None;
                redraw = true;
                causes.add(Cause::Resize);
            }
            let now = display.grid_size(lines);
            if now != spaces.size() {
                spaces.handle(Event::Resize(now.0, now.1));
                // The cell's size is asked of the terminal once the window has settled. The
                // window's pixels over its cells was taken at once here, and is a guess:
                // padding, or a tab bar the terminal counts, made each row two or three
                // pixels taller than it is, and a press in pixels then landed three or four
                // lines above the pointer, low in the window.
                cell_asked = Some(Instant::now() + CELL_SETTLE);
                backend.invalidate();
                redraw = true;
                causes.add(Cause::Resize);
            }
            // Its pixels can change with its cells as they were: a font size that keeps the
            // grid, a move to a display of another density, a size passed on late over SSH.
            let seen = window_pixels();
            if seen != pixels {
                pixels = seen;
                cell_asked = Some(Instant::now() + CELL_SETTLE);
            }
            if cell_asked.is_some_and(|at| Instant::now() >= at) {
                cell_asked = None;
                if display.placement.is_some() && !display.told_cell {
                    // What the terminal says, and the guess only where it says nothing.
                    let told = display
                        .answers_cell
                        .then(|| cellix_term::cell_size(display.multiplexer, display.passthrough))
                        .flatten();
                    let cell = told.or_else(window_cell);
                    if cell.is_some() && cell != display.cell {
                        display.cell = cell;
                        backend = display.backend();
                        let measure = display.measure();
                        look.borrow_mut().measure = measure.clone();
                        for workspace in &mut spaces.list {
                            workspace.app.set_measure(measure.clone());
                        }
                        backend.invalidate();
                        shown = None;
                        redraw = true;
                        causes.add(Cause::Resize);
                    }
                }
            }
            if spaces.take_switched() {
                // Another workspace is in front: none of what's on screen can be kept.
                backend.invalidate();
                shown = None;
                redraw = true;
                causes.add(Cause::Resize);
            }
            if let Some(on) = spaces.app_mut().take_smooth_change() {
                display.set_smooth(on);
                backend = display.backend();
                redraw = true;
                causes.add(Cause::Resize);
            }
            if let Some(times) = spaces.app_mut().take_row_height_change() {
                // Rows of another height: everything is measured and drawn again, in a
                // grid of as many of them as the window holds.
                display.own_rows = own_rows(asked_rows().as_deref(), times);
                backend = display.backend();
                let measure = display.measure();
                look.borrow_mut().measure = measure.clone();
                for workspace in &mut spaces.list {
                    workspace.app.set_measure(measure.clone());
                }
                let now = display.grid_size(lines);
                if now != spaces.size() {
                    spaces.handle(Event::Resize(now.0, now.1));
                }
                backend.invalidate();
                shown = None;
                redraw = true;
                causes.add(Cause::Resize);
            }
            let frame = frame_now(redraw, turning, shown.is_some(), Instant::now());
            if let (Frame::Step, Some(grid)) = (frame, shown.as_mut()) {
                // Nothing but a spinner's step: the frame shown, a step on, and nothing laid
                // out or drawn again. Under a glide the step waits for the glide's frames.
                if backend.gliding().is_none() {
                    let now = cellix_render::spinner::clock();
                    grid.turn_to(cellix_render::spinner::step_at(now));
                    backend.present(grid, &mut out)?;
                    out.flush()?;
                }
                turning = next_turn(grid, Instant::now());
            }
            if frame == Frame::Whole {
                let (width, height) = spaces.size();
                let mut grid = Grid::new(width, height, spaces.app().background());
                let started = Instant::now();
                spaces.draw(&mut grid);
                phases.draw = started.elapsed();
                if let Some(prof) = &mut prof {
                    prof.drew(causes);
                    prof.took(Phase::Draw, phases.draw);
                }
                causes = Causes::default();
                let started = Instant::now();
                if let (Some((area, rows)), Some(from)) = (spaces.app_mut().take_scroll(), &shown) {
                    backend.scroll(from, &grid, area, rows, &mut out)?;
                }
                // Under a scroll the page can wait, so the time goes on moving it.
                if backend.gliding().is_none() || last_present.elapsed() >= SCROLL_PRESENT {
                    backend.set_lines(lines.1);
                    let mark = out.total();
                    backend.present(&grid, &mut out)?;
                    if let Some(prof) = &mut prof {
                        prof.presented(out.total() - mark);
                    }
                    last_present = Instant::now();
                }
                phases.present = started.elapsed();
                if let Some(prof) = &mut prof {
                    prof.took(Phase::Present, phases.present);
                }
                slow.note(&phases);
                phases = crate::slow::Phases::default();
                foot = backend.foot_drops(grid.height(), grid.foot);
                turning = next_turn(&grid, Instant::now());
                if let Some(path) = &screen_text {
                    // Where its rows stand first, so whoever reads the text can press on what
                    // it says: only where rows are Cellix's own, and can stand off their places.
                    if let Some(row) = display.row() {
                        let middles = row_middles(grid.height(), row, &foot);
                        let _ = std::fs::write(path.with_extension("rows"), middles);
                    }
                    // Whole or not at all: it's read while Cellix writes.
                    let fresh = path.with_extension("new");
                    if std::fs::write(&fresh, grid.text()).is_ok() {
                        let _ = std::fs::rename(&fresh, path);
                    }
                }
                shown = Some(grid);
                if let Some(text) = spaces.app_mut().outgoing_clipboard.take() {
                    let encoded = base64::engine::general_purpose::STANDARD.encode(text);
                    write!(out, "\x1b]52;c;{encoded}\x07")?;
                }
                // The terminal's own idea of the pointer drifts, so it's told each time.
                let pointer = spaces.app().pointer();
                if shape != Some(pointer) {
                    write!(out, "\x1b]22;{pointer}\x1b\\")?;
                    shape = Some(pointer);
                }
                out.flush()?;
                redraw = false;
            }
            if spaces.quit || STOPPED.load(Ordering::Relaxed) {
                break;
            }
            let has_session = spaces.list.iter().any(|w| w.app.has_session());
            let rest = if has_session {
                session_wait(lively.elapsed())
            } else {
                POLL
            };
            let wait = wait_for(rest, backend.gliding(), turning, Instant::now());
            // A signal interrupts the wait; the loop goes round and sees it.
            let ready = match ct::poll(wait) {
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                ready => ready?,
            };
            if ready {
                let started = Instant::now();
                lively = started;
                // Drain everything queued, so a fast wheel doesn't redraw per event.
                let mut batch = Vec::new();
                let mut came_back = false;
                let mut left = false;
                let mut messages = Vec::new();
                loop {
                    let read = ct::read()?;
                    let events = match osc.take(read) {
                        Took::Held => Vec::new(),
                        Took::Message(body) => {
                            messages.push(body);
                            Vec::new()
                        }
                        Took::Keys(keys) => keys,
                    };
                    let mut typing = false;
                    for event in events {
                        // Anything but the wheel or a passing pointer may change what's under a scroll.
                        // A trackpad wanders sideways as it goes, so those count as the wheel too.
                        let passing = matches!(
                            &event,
                            ct::Event::Mouse(mouse) if matches!(
                                mouse.kind,
                                ct::MouseEventKind::Moved
                                    | ct::MouseEventKind::ScrollUp
                                    | ct::MouseEventKind::ScrollDown
                                    | ct::MouseEventKind::ScrollLeft
                                    | ct::MouseEventKind::ScrollRight
                            )
                        );
                        if !passing {
                            backend.settle();
                        }
                        // A multiplexer forgets images when a client leaves, so a new one starts
                        // with an empty window until everything is sent again.
                        if matches!(event, ct::Event::FocusGained)
                            && display.placement == Some(Placement::Placeholder)
                        {
                            backend.invalidate();
                        }
                        came_back |= matches!(event, ct::Event::FocusGained);
                        left |= matches!(event, ct::Event::FocusLost);
                        match event {
                            ct::Event::FocusGained => spaces.set_focused(true),
                            ct::Event::FocusLost => spaces.set_focused(false),
                            _ => {}
                        }
                        // Coming back, or typing, can leave the terminal showing a pointer of its own.
                        if matches!(event, ct::Event::FocusGained | ct::Event::Key(_)) {
                            shape = None;
                        }
                        // Keys that type are taken whole on Windows, where a paste comes as them.
                        typing |= cfg!(windows) && typed(&event).is_some();
                        batch.push(event);
                    }
                    // Inside a message the rest is on its way: it's waited for, rather than
                    // drawing a frame each time the terminal's pipe runs dry.
                    let patience = if osc.reading() {
                        OSC_PATIENCE
                    } else {
                        Duration::ZERO
                    };
                    let more = ct::poll(patience)?;
                    // An Alt+] with nothing after it, even after waiting, was only a key.
                    if !more {
                        batch.extend(osc.release());
                    }
                    // A stream of input mustn't hold up the frames it's asking for, but a
                    // pasted picture is read whole first.
                    let slice = started.elapsed() >= INPUT_SLICE && !typing && !osc.reading();
                    if slice || !more {
                        break;
                    }
                }
                for body in messages {
                    use cellix_term::clipboard::{Pasted, choose, read_request};
                    match clipboard.take(&body) {
                        // A paste: what to read of it is asked for with its password.
                        Some(Pasted::Offer { types, password }) => {
                            if let Some(mime) = choose(&types) {
                                write!(out, "{}", read_request(mime, password.as_deref()))?;
                                out.flush()?;
                            }
                        }
                        Some(Pasted::Data { mime, bytes }) if mime.starts_with("text/") => {
                            let text = String::from_utf8_lossy(&bytes).into_owned();
                            batch.push(ct::Event::Paste(text));
                        }
                        Some(Pasted::Data { mime, bytes }) => {
                            spaces.app_mut().paste_picture(&bytes, &mime);
                        }
                        // Nothing pastes then, so how to have plain pastes back is said.
                        Some(Pasted::Refused(why)) => {
                            spaces.app_mut().say(format!(
                                "The terminal didn't hand over the clipboard ({why}): \
                                 CELLIX_PASTE_EVENTS=0 pastes as before"
                            ));
                        }
                        None => {}
                    }
                }
                if cfg!(windows) {
                    batch = pasted_keys(batch, &mut pasting, Instant::now());
                }
                for event in batch {
                    match translate(event, mouse_cell(&display)).map(|e| cmd_left(e, ghostty)) {
                        // The size check above handles it, and compares against the old size.
                        Some(Event::Resize(..)) | None => {}
                        Some(event) => {
                            let rows = spaces.size().1;
                            let event = match display.row() {
                                Some(row) => on_the_foot(event, rows, row, &foot),
                                None => event,
                            };
                            spaces.handle(on_the_grid(event, rows));
                        }
                    }
                }
                // Saving on focus change saves as the window is left, as VS Code does.
                if left {
                    for workspace in &mut spaces.list {
                        workspace.app.window_left();
                    }
                }
                // The terminal may have turned dark or light while Cellix was away, as a Mac
                // does at sunset, or a reattach finds it changed: its colour is asked again.
                if came_back
                    && let Some((light, background)) =
                        cellix_term::background(display.multiplexer, display.passthrough)
                    && (display.light != Some(light) || display.background != Some(background))
                {
                    display.light = Some(light);
                    display.background = Some(background);
                    let mut look = look.borrow_mut();
                    look.light = Some(light);
                    look.background = Some(background);
                    for workspace in &mut spaces.list {
                        workspace.app.follow_terminal_theme(Some(light));
                        workspace.app.match_terminal_background(Some(background));
                    }
                    backend.invalidate();
                    shown = None;
                }
                phases.events += started.elapsed();
                if let Some(prof) = &mut prof {
                    prof.took(Phase::Events, started.elapsed());
                }
                redraw = true;
                causes.add(Cause::Input);
            } else if last_tick.elapsed() >= TICK {
                let started = Instant::now();
                last_tick = started;
                spaces.handle(Event::Tick);
                phases.events += started.elapsed();
                if let Some(prof) = &mut prof {
                    prof.took(Phase::Events, started.elapsed());
                }
                redraw = true;
                causes.add(Cause::Tick);
            }
            let started = Instant::now();
            if spaces.poll() {
                lively = started;
                redraw = true;
                causes.add(Cause::Poll);
            }
            phases.poll += started.elapsed();
            if let Some(prof) = &mut prof {
                prof.took(Phase::Poll, started.elapsed());
            }
            if !spaces.notifications.is_empty() {
                for (title, body) in spaces.notifications.drain(..) {
                    let bytes = cellix_term::notification(notice, &title, &body, inside_tmux);
                    write!(out, "{bytes}")?;
                }
                out.flush()?;
            }
            if backend.gliding().is_some() {
                backend.glide(Instant::now(), &mut out)?;
                // Arrived, so the page underneath takes over.
                if backend.gliding().is_none() {
                    redraw = true;
                    causes.add(Cause::Scroll);
                }
            }
        }
        Ok(())
    })();
    // After a hang-up there's no terminal to tell, and the rest must still happen.
    let _ = write!(out, "\x1b]22;default\x1b\\");
    let _ = out.flush();
    drop(_session);
    // Each workspace keeps what it must: comments, tabs and unsaved edits.
    let mut finished = Ok(());
    for mut workspace in spaces.list.drain(..) {
        // Hung up rather than quit: every workspace still here opens again next time.
        if spaces.bar {
            crate::hold::remember_open(workspace.app.workspace.root());
        }
        let result = workspace.app.finish();
        if finished.is_ok() {
            finished = result;
        }
        // Before Windows ends the process, which it does as soon as a closing console is let go.
        drop(workspace.app.ide.take());
        drop(workspace.server);
    }
    #[cfg(windows)]
    FINISHED.store(true, Ordering::Relaxed);
    ended.and(finished)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyEvent as CtKey, KeyEventState};

    /// A row's height is fitted to the window, a twentieth either side of what Settings
    /// asks, so little is left over under the last row: all of what was left stood under the
    /// bar at the foot, up to a whole row of it. A press in what's left, where the bar is
    /// drawn down to stand in its middle, is a press on the bar.
    #[test]
    fn a_rows_height_is_fitted_to_the_window() {
        // A row's height is fitted to the window: of the heights near 50 px, the one that
        // leaves least of 57 lines of 36 px over, which 50 itself does, with 2.
        assert_eq!(fitted(50, 36, 57), 50);
        // 56 lines are 42 rows of 48 exactly, where rows of 50 left 16 px under the bar.
        assert_eq!(fitted(50, 36, 56), 48);
        // In a window of up to 63 lines, 45 rows, a quarter of a row is left at most, where
        // it was up to a whole one. A taller window can hold about as many rows as a row
        // has pixels, and then no near height leaves less: the bar stands in the middle of
        // what's left.
        for lines in 12..64u16 {
            let row = fitted(50, 36, lines);
            assert!((47..=53).contains(&row), "{lines} lines: {row}");
            assert!(
                lines as usize * 36 % row <= 12,
                "{lines} lines leave too much"
            );
        }
        // Never a row no taller than the line.
        assert!(fitted(37, 36, 40) > 36);
        // A press under the last row, where the bar is drawn down into what the window has
        // left, is a press on the bar's row. One anywhere else is where it was.
        let press = |y: u16| {
            Event::Mouse(MouseEvent {
                kind: MouseKind::Left,
                x: 3,
                y,
                fine_x: 3.5,
                fine_y: f32::from(y) + 0.2,
                mods: Mods::default(),
            })
        };
        let Event::Mouse(under) = on_the_grid(press(40), 40) else {
            panic!("a press stays a press");
        };
        assert_eq!((under.x, under.y), (3, 39));
        assert!(under.fine_y > 39.0 && under.fine_y < 40.0);
        let Event::Mouse(inside) = on_the_grid(press(12), 40) else {
            panic!("a press stays a press");
        };
        assert_eq!((inside.y, inside.fine_y), (12, 12.2));
    }

    /// A foot of two rows stands them evenly in what's under the sheets, so each is drawn
    /// below where it begins, and a press is on the row drawn under it. Rows of 50 px, forty
    /// of them: the status bar's picture is 20 px down and the workspaces' 40, so a press
    /// 1955 px down the window is on the status bar, though the fortieth row begins at 1950.
    #[test]
    fn a_press_at_a_foot_of_two_rows_is_on_the_row_drawn_there() {
        let press = |pixel: usize, drops: &[usize]| {
            let event = Event::Mouse(MouseEvent {
                kind: MouseKind::Left,
                x: 3,
                y: (pixel / 50) as u16,
                fine_x: 3.5,
                fine_y: pixel as f32 / 50.0,
                mods: Mods::default(),
            });
            match on_the_grid(on_the_foot(event, 40, 50, drops), 40) {
                Event::Mouse(mouse) => (mouse.y, (mouse.fine_y * 100.0).round() as u32),
                _ => panic!("a press stays a press"),
            }
        };
        let two = [20, 40];
        assert_eq!(press(1899, &two), (37, 3798), "over the foot, as it was");
        assert_eq!(
            press(1905, &two),
            (38, 3800),
            "over the status bar's picture"
        );
        assert_eq!(
            press(1955, &two),
            (38, 3870),
            "on it, past where its row ends"
        );
        assert_eq!(press(1975, &two), (38, 3899), "under it, and nearer to it");
        assert_eq!(press(1985, &two), (39, 3900), "nearer to the workspaces");
        assert_eq!(press(2010, &two), (39, 3940), "on them, under the last row");
        assert_eq!(press(2070, &two), (39, 3999), "under them, in the padding");
        // One row at the foot is whatever is under the sheets, as before.
        assert_eq!(press(1955, &[30]), (39, 3910));
        assert_eq!(press(2010, &[30]), (39, 3999));
        // Each row's middle, where the harness presses: the foot's two are lower.
        let middles = row_middles(40, 50, &two);
        let middles: Vec<&str> = middles.lines().collect();
        assert_eq!(middles.len(), 40);
        assert_eq!(middles[37..], ["1875", "1945", "2015"]);
        assert_eq!(press(1945, &two).0, 38);
        assert_eq!(press(2015, &two).0, 39);
    }

    /// A row of Cellix's own is as many of the terminal's lines tall as Settings says, and
    /// `CELLIX_ROW_HEIGHT` says otherwise for the harness. One no taller than a line is a
    /// line, whichever said so.
    #[test]
    fn a_row_of_cellixs_own_is_as_tall_as_settings_says() {
        assert_eq!(own_rows(None, 1.4), Some(1.4), "airy");
        assert_eq!(own_rows(None, 1.2), Some(1.2), "comfortable");
        assert_eq!(own_rows(None, 1.0), None, "the terminal's own line");
        assert_eq!(own_rows(Some("1"), 1.4), None, "the variable turns it off");
        assert_eq!(own_rows(Some(" 1.25 "), 1.0), Some(1.25), "or on");
        assert_eq!(
            own_rows(Some("tall"), 1.2),
            Some(1.2),
            "nonsense is nothing said"
        );
        assert_eq!(
            own_rows(Some("9"), 1.4),
            None,
            "a window has to keep some rows"
        );
    }

    /// Forty-five lines of 37 pixels hold thirty-two rows of 52, and a press 130 pixels down
    /// is on the third of those, which is the fourth of the terminal's lines.
    #[test]
    fn a_press_under_taller_rows_lands_on_the_row_it_is_on() {
        assert_eq!(rows_in(45, 37, 52), 32);
        assert_eq!(rows_in(1, 37, 52), 1, "a window always has a row");
        let press = |pixels| {
            let event = ct::Event::Mouse(crossterm::event::MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: 40,
                row: 130,
                modifiers: KeyModifiers::NONE,
            });
            match translate(event, Some(pixels)) {
                Some(Event::Mouse(mouse)) => (mouse.x, mouse.y),
                _ => panic!("a press is a mouse event"),
            }
        };
        assert_eq!(press((17, 37)), (2, 3), "on the terminal's own lines");
        assert_eq!(press((17, 52)), (2, 2), "on rows of 52 pixels");
    }

    fn key(code: KeyCode, kind: KeyEventKind) -> ct::Event {
        ct::Event::Key(CtKey {
            code,
            modifiers: if matches!(code, KeyCode::Char('~' | '(' | 'A')) {
                KeyModifiers::SHIFT
            } else {
                KeyModifiers::NONE
            },
            kind,
            state: KeyEventState::NONE,
        })
    }

    /// Text as a Windows console delivers it: each key pressed, then released.
    fn keys(text: &str) -> Vec<ct::Event> {
        text.chars()
            .flat_map(|c| {
                let code = match c {
                    '\n' => KeyCode::Enter,
                    '\t' => KeyCode::Tab,
                    '\x1b' => KeyCode::Esc,
                    c => KeyCode::Char(c),
                };
                [
                    key(code, KeyEventKind::Press),
                    key(code, KeyEventKind::Release),
                ]
            })
            .collect()
    }

    fn pastes(events: &[ct::Event]) -> Vec<String> {
        events
            .iter()
            .filter_map(|e| match e {
                ct::Event::Paste(text) => Some(text.clone()),
                _ => None,
            })
            .collect()
    }

    /// The loop waits as long as it always did while nothing turns; with a spinner on screen,
    /// no longer than its next step, and a step that's due is drawn at once.
    #[test]
    fn a_spinner_wakes_the_loop_only_for_its_next_step() {
        let now = Instant::now();
        let ms = Duration::from_millis;
        assert_eq!(wait_for(POLL, None, None, now), POLL, "nothing turning");
        assert_eq!(wait_for(POLL, Some(ms(8)), None, now), ms(8), "a glide");
        assert_eq!(wait_for(POLL, None, Some(now + ms(20)), now), ms(20));
        assert_eq!(wait_for(ms(8), None, Some(now + ms(20)), now), ms(8));
        assert_eq!(wait_for(POLL, None, Some(now - ms(5)), now), Duration::ZERO);
        // A grid with nothing turning on it asks for nothing; one with a spinner, its next
        // step, which is never further off than a step.
        let mut grid = Grid::new(20, 3, cellix_render::Style::default());
        assert_eq!(next_turn(&grid, now), None);
        grid.dot(
            2.0,
            1,
            0.5,
            cellix_render::Rgb(1, 2, 3),
            cellix_render::Style::default(),
        );
        assert_eq!(next_turn(&grid, now), None, "a dot is still");
        grid.turn = None;
        grid.spinner(
            2.0,
            1,
            0.5,
            cellix_render::Rgb(1, 2, 3),
            cellix_render::Style::default(),
        );
        assert_eq!(
            next_turn(&grid, now),
            None,
            "nor does a spinner with motion off"
        );
        grid.turn = Some(3);
        grid.spinner(
            2.0,
            1,
            0.5,
            cellix_render::Rgb(1, 2, 3),
            cellix_render::Style::default(),
        );
        let due = next_turn(&grid, now).expect("turning");
        assert!(due > now && due <= now + cellix_render::spinner::STEP);
    }

    /// A second of a spinner turning with nothing else changing draws the interface once, at
    /// the start, and takes every step after on the frame shown; a change while it turns
    /// draws everything again, and the steps go on from that frame.
    #[test]
    fn a_spinner_step_alone_draws_nothing_again() {
        use cellix_render::spinner::{self, STEP};
        let start = Instant::now();
        // The interface: a spinner on its first row, and a count of what's changed on its
        // second, which only a whole draw shows.
        let draw = |step: u8, changes: usize| {
            let mut grid = Grid::new(20, 2, cellix_render::Style::default());
            grid.turn = Some(step);
            grid.spinner(
                1.0,
                0,
                0.65,
                cellix_render::Rgb(1, 2, 3),
                Default::default(),
            );
            grid.put_str(0, 1, &changes.to_string(), Default::default(), 20);
            grid
        };
        let (mut whole, mut steps, mut changes) = (0, 0, 0);
        let mut shown: Option<Grid> = None;
        let mut turning = None;
        let mut redraw = true;
        let mut clock = Duration::ZERO;
        while clock < Duration::from_secs(1) {
            let now = start + clock;
            // Half way, something else changes.
            if clock == Duration::from_millis(500) {
                changes += 1;
                redraw = true;
            }
            let step = spinner::step_at(clock);
            match frame_now(redraw, turning, shown.is_some(), now) {
                Frame::Whole => {
                    whole += 1;
                    let grid = draw(step, changes);
                    turning = grid.spinning.then(|| now + spinner::until_next(clock));
                    shown = Some(grid);
                    redraw = false;
                }
                Frame::Step => {
                    steps += 1;
                    let grid = shown.as_mut().unwrap();
                    grid.turn_to(step);
                    assert_eq!(*grid, draw(step, changes), "the frame a whole draw makes");
                    turning = grid.spinning.then(|| now + spinner::until_next(clock));
                }
                Frame::Same => {}
            }
            clock += Duration::from_millis(5);
        }
        assert_eq!(whole, 2, "the first frame, and the change");
        assert!(
            (9..=11).contains(&steps),
            "{steps} steps in a second of {STEP:?} each"
        );
        assert_eq!(
            shown.unwrap().row_text(1).trim(),
            "1",
            "the change is on screen"
        );
        // Nothing turning, nothing is due.
        assert_eq!(frame_now(false, None, true, start), Frame::Same);
        assert_eq!(
            frame_now(false, Some(start), false, start),
            Frame::Whole,
            "nothing to step"
        );
    }

    #[test]
    fn a_quiet_session_is_read_less_often_until_something_happens() {
        // A key or a line of output just came: read as often as typing needs.
        assert_eq!(session_wait(Duration::ZERO), SESSION_POLL);
        assert_eq!(session_wait(Duration::from_millis(200)), SESSION_POLL);
        // At its prompt for a while: a fifth as many wake-ups, and still no slower than a
        // window with no session at all.
        let resting = session_wait(Duration::from_secs(2));
        assert!(resting >= SESSION_POLL * 5, "{resting:?}");
        assert!(resting <= POLL, "{resting:?}");
        assert_eq!(session_wait(Duration::from_secs(3600)), resting);
    }

    /// One read of input, with nothing before it.
    fn once(events: Vec<ct::Event>) -> Vec<ct::Event> {
        pasted_keys(events, &mut Pasting::default(), Instant::now())
    }

    /// Keys as crossterm makes them of what a terminal sent, each byte one key.
    fn sent_as_keys(bytes: &str, split_escapes: bool) -> Vec<ct::Event> {
        let mut out = Vec::new();
        let mut chars = bytes.chars().peekable();
        while let Some(c) = chars.next() {
            let (code, modifiers) = match c {
                '\x1b' if !split_escapes && chars.peek().is_some() => {
                    (KeyCode::Char(chars.next().unwrap()), KeyModifiers::ALT)
                }
                '\x1b' => (KeyCode::Esc, KeyModifiers::NONE),
                '\x07' => (KeyCode::Char('g'), KeyModifiers::CONTROL),
                c if c.is_ascii_uppercase() => (KeyCode::Char(c), KeyModifiers::SHIFT),
                c => (KeyCode::Char(c), KeyModifiers::NONE),
            };
            out.push(ct::Event::Key(CtKey {
                code,
                modifiers,
                kind: KeyEventKind::Press,
                state: KeyEventState::NONE,
            }));
        }
        out
    }

    fn osc_messages(osc: &mut Osc, events: Vec<ct::Event>) -> (Vec<String>, usize) {
        let (mut messages, mut keys) = (Vec::new(), 0);
        for event in events {
            match osc.take(event) {
                Took::Held => {}
                Took::Keys(k) => keys += k.len(),
                Took::Message(m) => messages.push(m),
            }
        }
        keys += osc.release().len();
        (messages, keys)
    }

    #[test]
    fn a_clipboard_message_comes_back_whole_from_keys() {
        let two = "\x1b]5522;type=read:status=OK\x1b\\\x1b]5522;type=read:status=DONE\x07";
        // Alt+] and Alt+\ as crossterm makes them, or Esc and the character when a read ends
        // between the two bytes: either way, two messages and no keys.
        for split in [false, true] {
            let (messages, keys) = osc_messages(&mut Osc::new(true), sent_as_keys(two, split));
            assert_eq!(
                messages,
                vec!["type=read:status=OK", "type=read:status=DONE"],
                "split {split}"
            );
            assert_eq!(keys, 0, "split {split}");
        }
        // Typing Alt+] and then text is keys, held only until it's clear.
        let (messages, keys) = osc_messages(&mut Osc::new(true), sent_as_keys("\x1b]55x", false));
        assert!(messages.is_empty());
        assert_eq!(keys, 4, "Alt+], 5, 5 and x go on as typed");
        // Where paste events are off, nothing is held at all.
        let (messages, keys) = osc_messages(&mut Osc::new(false), sent_as_keys(two, false));
        assert!(messages.is_empty());
        assert!(keys > 0);
    }

    #[test]
    fn a_paste_the_console_hands_over_in_two_reads_stays_one_paste() {
        let now = Instant::now();
        // Without markers: its last line break, read alone, would be Enter and take a
        // suggestion. It's the paste's.
        let mut pasting = Pasting::default();
        let first = pasted_keys(keys("pasted(\n\ttext\nmore"), &mut pasting, now);
        assert_eq!(pastes(&first), ["pasted(\n\ttext\nmore"]);
        let rest = pasted_keys(keys("\n"), &mut pasting, now + Duration::from_millis(5));
        assert_eq!(rest, vec![ct::Event::Paste("\n".into())]);
        // Typed a moment after the paste's last part, Enter is a key again.
        let typed = now + Duration::from_millis(5) + PASTE_TAIL * 2;
        let later = pasted_keys(keys("\n"), &mut pasting, typed);
        assert!(pastes(&later).is_empty(), "{later:?}");
        let mut pasting = Pasting::default();
        let _ = pasted_keys(keys("a(\n\tb"), &mut pasting, now);
        let after = pasted_keys(keys("\n"), &mut pasting, now + PASTE_TAIL * 2);
        assert!(pastes(&after).is_empty(), "{after:?}");
        // Between markers, the rest is the paste's however late, up to the closing marker.
        let mut pasting = Pasting::default();
        let first = pasted_keys(keys("\x1b[200~a(\n"), &mut pasting, now);
        assert_eq!(first, vec![ct::Event::Paste("a(\n".into())]);
        let later = now + Duration::from_secs(1);
        let rest = pasted_keys(keys("\tb\n\x1b[201~x"), &mut pasting, later);
        assert_eq!(pastes(&rest), ["\tb\n"]);
        assert_eq!(rest.len(), 2, "what follows the marker is typed: {rest:?}");
        // A closing marker that comes on its own isn't typed.
        let mut pasting = Pasting::default();
        let _ = pasted_keys(keys("x(\n\ty\n"), &mut pasting, now);
        let rest = pasted_keys(keys("\x1b[201~"), &mut pasting, now);
        assert!(
            rest.iter().all(|e| matches!(e, ct::Event::Paste(_))),
            "{rest:?}"
        );
    }

    #[test]
    fn keys_that_paste_on_windows_become_a_paste() {
        // Between a bracketed paste's markers, whatever the text.
        let marked = once(keys("\x1b[200~f(x)\x1b[201~"));
        assert_eq!(marked, vec![ct::Event::Paste("f(x)".into())]);
        let marked = once(keys("\x1b[200~a(\n\tb\n\x1b[201~"));
        assert_eq!(pastes(&marked), ["a(\n\tb\n"]);
        // Without markers, lines typed faster than anyone types.
        let unmarked = once(keys("pasted(\n\ttext\nmore\n"));
        assert_eq!(
            unmarked,
            vec![ct::Event::Paste("pasted(\n\ttext\nmore\n".into())]
        );
        // Typing stays typing: a word, a word and Enter, a shortcut, an Escape.
        for typed in ["new ", "from a human\n", "n\n", "\x1b", "A"] {
            let events = keys(typed);
            let presses: Vec<_> = events
                .iter()
                .filter(|e| !matches!(e, ct::Event::Key(k) if k.kind == KeyEventKind::Release))
                .cloned()
                .collect();
            assert_eq!(once(events), presses, "{typed:?}");
        }
        let ctrl_s = ct::Event::Key(CtKey::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
        let mut events = keys("a\nb");
        events.insert(2, ctrl_s.clone());
        assert!(pastes(&once(events)).is_empty());
    }

    #[test]
    fn the_wheel_tipped_sideways_arrives_as_itself() {
        let sideways = |kind| {
            let event = ct::Event::Mouse(ct::MouseEvent {
                kind,
                column: 5,
                row: 3,
                modifiers: KeyModifiers::SHIFT,
            });
            translate(event, None)
        };
        let Some(Event::Mouse(left)) = sideways(MouseEventKind::ScrollLeft) else {
            panic!("dropped");
        };
        assert_eq!((left.kind, left.x, left.y), (MouseKind::ScrollLeft, 5, 3));
        let Some(Event::Mouse(right)) = sideways(MouseEventKind::ScrollRight) else {
            panic!("dropped");
        };
        assert_eq!(right.kind, MouseKind::ScrollRight);
        assert!(right.mods.shift);
    }
}
