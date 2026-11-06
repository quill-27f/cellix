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
use crate::workspaces::{Workspace, Workspaces};

/// Set when the terminal hangs up or Cellix is asked to stop. Left to their default, those
/// signals end the process on the spot and leave the workspace socket behind.
static STOPPED: AtomicBool = AtomicBool::new(false);

#[cfg(unix)]
extern "C" fn stop(_: libc::c_int) {
    STOPPED.store(true, Ordering::Relaxed);
}

/// Hangs up and stops end the loop, so they leave the way Quit does.
///
/// A stop that lands while crossterm waits for input is retried inside crossterm, and if the
/// terminal then closes, crossterm reads its end over and over and never returns, so the loop
/// never sees the stop. A watch outside the loop ends Cellix after a moment in that case,
/// clearing `left` (the socket and the like) as the loop would have.
#[cfg(unix)]
fn catch_stops(left: Left) {
    for signal in [libc::SIGHUP, libc::SIGTERM] {
        // SAFETY: the handler only stores to an atomic, which is safe in a signal handler.
        unsafe {
            libc::signal(signal, stop as *const () as libc::sighandler_t);
        }
    }
    std::thread::spawn(move || {
        while !STOPPED.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(100));
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
/// Background file checks run about twice a second.
const TICK: Duration = Duration::from_millis(500);

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
            let (x, y, fine_x) = match pixels {
                Some((cw, ch)) => {
                    let fine = m.column as f32 / cw.max(1) as f32;
                    (fine as u16, m.row / ch.max(1) as u16, fine)
                }
                None => (m.column, m.row, m.column as f32),
            };
            Event::Mouse(MouseEvent {
                kind,
                x,
                y,
                fine_x,
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

/// Windows' console gives Cellix no pastes, only the keys that would type them, and typed,
/// a paste's brackets close themselves and its lines indent. So in one batch of input, keys
/// between a bracketed paste's markers become a paste, as do keys that type more after a line
/// break or a tab, which no one types faster than Cellix reads. Anything else stays keys.
fn pasted_keys(events: Vec<ct::Event>) -> Vec<ct::Event> {
    // Releases say nothing here, and would split a run of keys.
    let events: Vec<ct::Event> = events
        .into_iter()
        .filter(|e| !matches!(e, ct::Event::Key(k) if k.kind == KeyEventKind::Release))
        .collect();
    let text = |run: &[ct::Event]| run.iter().filter_map(typed).collect::<String>();
    let mut out = Vec::new();
    let mut i = 0;
    while i < events.len() {
        if marker(&events[i..], '0') {
            let start = i + 6;
            let end = (start..events.len())
                .find(|&j| marker(&events[j..], '1'))
                .unwrap_or(events.len());
            out.push(ct::Event::Paste(text(&events[start..end])));
            i = (end + 6).min(events.len());
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
        } else {
            out.extend_from_slice(&events[i..i + run.max(1)]);
        }
        i += run.max(1);
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
    pixel_mouse: bool,
    light: Option<bool>,
    background: Option<(u8, u8, u8)>,
    /// Where the colour is asked again when Cellix comes back into focus.
    multiplexer: cellix_term::Multiplexer,
    passthrough: bool,
    smooth: Option<Smooth>,
    bleed: Bleed,
}

impl Display {
    fn detect(setting: Option<&str>, smooth: bool) -> io::Result<Self> {
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
            placement,
            cell,
            told_cell: env.cell.is_some() && env.multiplexer == cellix_term::Multiplexer::Tmux,
            pixel_mouse: env.pixel_mouse,
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

    /// How to measure text for this display.
    fn measure(&self) -> Rc<dyn Measure> {
        match self.placement.zip(self.cell) {
            Some((_, cell)) => Rc::new(FontMeasure::new(cell.0, cell.1)),
            None => Rc::new(CellMeasure),
        }
    }

    /// Any failure on the way to pixels lands on cells, which always work.
    fn backend(&self) -> Box<dyn Backend> {
        let pixels = self.placement.zip(self.cell).and_then(|(placement, cell)| {
            let rasterizer = SoftwareRasterizer::new(cell.0, cell.1)?;
            let backend =
                PixelBackend::new(rasterizer, placement, DEFAULT_QUOTA).with_bleed(self.bleed);
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
    let mut display = Display::detect(settings.tier.as_deref(), smooth)?;
    let mut backend = display.backend();
    let mut out = io::stdout();
    let mut slow = crate::slow::Log::start();
    let mut phases = crate::slow::Phases::default();
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
    let mut spaces = Workspaces::new(size()?, crate::app::machine_name(), start);
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
    let mouse_cell = |display: &Display| display.cell.filter(|_| pixel_mouse);
    let mut redraw = true;
    let mut shape = None;
    let mut last_tick = std::time::Instant::now();
    let mut shown: Option<Grid> = None;
    let ghostty = std::env::var("TERM").is_ok_and(|t| t.contains("ghostty"));
    // The loop's errors still come back, but only after what Quit does: a console closing
    // under Cellix can fail its last frame.
    #[allow(clippy::redundant_closure_call)]
    let ended = (|| -> io::Result<()> {
        loop {
            // Polled as well as signalled: a resize during slow startup is otherwise lost.
            let now = size()?;
            if now != spaces.size() {
                spaces.handle(Event::Resize(now.0, now.1));
                let cell = window_cell();
                if display.placement.is_some()
                    && !display.told_cell
                    && cell.is_some()
                    && cell != display.cell
                {
                    display.cell = cell;
                    backend = display.backend();
                    let measure = display.measure();
                    look.borrow_mut().measure = measure.clone();
                    for workspace in &mut spaces.list {
                        workspace.app.set_measure(measure.clone());
                    }
                }
                backend.invalidate();
                redraw = true;
            }
            if spaces.take_switched() {
                // Another workspace is in front: none of what's on screen can be kept.
                backend.invalidate();
                shown = None;
                redraw = true;
            }
            if let Some(on) = spaces.app_mut().take_smooth_change() {
                display.set_smooth(on);
                backend = display.backend();
                redraw = true;
            }
            if redraw {
                let (width, height) = spaces.size();
                let mut grid = Grid::new(width, height, spaces.app().background());
                let started = Instant::now();
                spaces.draw(&mut grid);
                phases.draw = started.elapsed();
                let started = Instant::now();
                if let (Some((area, rows)), Some(from)) = (spaces.app_mut().take_scroll(), &shown) {
                    backend.scroll(from, &grid, area, rows, &mut out)?;
                }
                // Under a scroll the page can wait, so the time goes on moving it.
                if backend.gliding().is_none() || last_present.elapsed() >= SCROLL_PRESENT {
                    backend.present(&grid, &mut out)?;
                    last_present = Instant::now();
                }
                phases.present = started.elapsed();
                slow.note(&phases);
                phases = crate::slow::Phases::default();
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
            let wait = backend
                .gliding()
                .unwrap_or(if has_session { SESSION_POLL } else { POLL });
            // A signal interrupts the wait; the loop goes round and sees it.
            let ready = match ct::poll(wait) {
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                ready => ready?,
            };
            if ready {
                let started = Instant::now();
                // Drain everything queued, so a fast wheel doesn't redraw per event.
                let mut batch = Vec::new();
                let mut came_back = false;
                loop {
                    let event = ct::read()?;
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
                    // Coming back, or typing, can leave the terminal showing a pointer of its own.
                    if matches!(event, ct::Event::FocusGained | ct::Event::Key(_)) {
                        shape = None;
                    }
                    // Keys that type are taken whole on Windows, where a paste comes as them.
                    let typing = cfg!(windows) && typed(&event).is_some();
                    batch.push(event);
                    // A stream of input mustn't hold up the frames it's asking for.
                    if (started.elapsed() >= INPUT_SLICE && !typing) || !ct::poll(Duration::ZERO)? {
                        break;
                    }
                }
                if cfg!(windows) {
                    batch = pasted_keys(batch);
                }
                for event in batch {
                    match translate(event, mouse_cell(&display)).map(|e| cmd_left(e, ghostty)) {
                        // The size check above handles it, and compares against the old size.
                        Some(Event::Resize(..)) | None => {}
                        Some(event) => spaces.handle(event),
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
                redraw = true;
            } else if last_tick.elapsed() >= TICK {
                let started = Instant::now();
                last_tick = started;
                spaces.handle(Event::Tick);
                phases.events += started.elapsed();
                redraw = true;
            }
            let started = Instant::now();
            redraw |= spaces.poll();
            phases.poll += started.elapsed();
            if backend.gliding().is_some() {
                backend.glide(Instant::now(), &mut out)?;
                // Arrived, so the page underneath takes over.
                redraw |= backend.gliding().is_none();
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

    #[test]
    fn keys_that_paste_on_windows_become_a_paste() {
        // Between a bracketed paste's markers, whatever the text.
        let marked = pasted_keys(keys("\x1b[200~f(x)\x1b[201~"));
        assert_eq!(marked, vec![ct::Event::Paste("f(x)".into())]);
        let marked = pasted_keys(keys("\x1b[200~a(\n\tb\n\x1b[201~"));
        assert_eq!(pastes(&marked), ["a(\n\tb\n"]);
        // Without markers, lines typed faster than anyone types.
        let unmarked = pasted_keys(keys("pasted(\n\ttext\nmore\n"));
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
            assert_eq!(pasted_keys(events), presses, "{typed:?}");
        }
        let ctrl_s = ct::Event::Key(CtKey::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
        let mut events = keys("a\nb");
        events.insert(2, ctrl_s.clone());
        assert!(pastes(&pasted_keys(events)).is_empty());
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
