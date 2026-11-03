use std::io::{self, Write};
use std::path::PathBuf;
use std::rc::Rc;
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
    KeyModifiers, KeyboardEnhancementFlags, MouseButton, MouseEventKind,
    PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use crossterm::execute;
use crossterm::style::Print;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode, size,
    window_size,
};

use crate::App;

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
            // Terminals without the Kitty keyboard protocol ignore this, and Ctrl+Shift+P
            // and Cmd keys then can't be told apart: F1 and Alt+P still open commands.
            PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
        )?;
        // tmux ignores kitty's flags but passes modified keys on with its extended-keys, once
        // asked this way, so Ctrl+Shift+P stops arriving as Ctrl+P.
        if std::env::var_os("TMUX").is_some() {
            execute!(io::stdout(), Print("\x1b[>4;2m"))?;
        }
        Ok(Self)
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = execute!(
            io::stdout(),
            PopKeyboardEnhancementFlags,
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
                _ => return None,
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
    pixel_mouse: bool,
    light: Option<bool>,
    background: Option<(u8, u8, u8)>,
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
            pixel_mouse: env.pixel_mouse,
            light: env.light,
            background: env.background,
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
    let root = paths
        .iter()
        .find(|p| p.is_dir())
        .cloned()
        .map_or_else(std::env::current_dir, Ok)?
        .canonicalize()?;
    let mut app = App::new(root, size()?);
    for path in paths.iter().filter(|p| p.is_file()) {
        let path = path.canonicalize()?;
        app.open(&path)?;
        if options.wait {
            app.wait_file = Some(path);
        }
    }
    // Agents are optional: a second window on this workspace still edits normally.
    let server = cellix_agent::start(app.workspace.root(), app.comments.clone()).ok();
    app.ide = server.as_ref().and_then(|_| {
        let root = app.workspace.root();
        let kept = crate::hold::folder_of(root).join("ide.json");
        cellix_agent::ide::IdeServer::start_kept(root, &kept).ok()
    });

    let _session = Session::start()?;
    let smooth = app.settings.smooth_scroll != Some(false);
    let mut display = Display::detect(app.settings.tier.as_deref(), smooth)?;
    let mut backend = display.backend();
    let mut out = io::stdout();
    let mut slow = crate::slow::Log::start();
    let mut phases = crate::slow::Phases::default();
    let mut last_present = Instant::now();
    app.set_measure(display.measure());
    // Only the direct tier moves by less than a line, so elsewhere a notch stays short.
    app.wheel_lines = if display.placement == Some(Placement::Direct) {
        3
    } else {
        1
    };
    app.follow_terminal_theme(display.light);
    app.rejoin_sessions();
    app.match_terminal_background(display.background);
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
    loop {
        // Polled as well as signalled: a resize during slow startup is otherwise lost.
        let now = size()?;
        if now != app.size() {
            app.handle(Event::Resize(now.0, now.1));
            let cell = window_cell();
            if display.placement.is_some() && cell.is_some() && cell != display.cell {
                display.cell = cell;
                backend = display.backend();
                app.set_measure(display.measure());
            }
            backend.invalidate();
            redraw = true;
        }
        if let Some(on) = app.take_smooth_change() {
            display.set_smooth(on);
            backend = display.backend();
            redraw = true;
        }
        if redraw {
            let (width, height) = app.size();
            let mut grid = Grid::new(width, height, app.background());
            let started = Instant::now();
            app.draw(&mut grid);
            phases.draw = started.elapsed();
            let started = Instant::now();
            if let (Some((area, rows)), Some(from)) = (app.take_scroll(), &shown) {
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
            if let Some(text) = app.outgoing_clipboard.take() {
                let encoded = base64::engine::general_purpose::STANDARD.encode(text);
                write!(out, "\x1b]52;c;{encoded}\x07")?;
            }
            // The terminal's own idea of the pointer drifts, so it's told each time.
            let pointer = app.pointer();
            if shape != Some(pointer) {
                write!(out, "\x1b]22;{pointer}\x1b\\")?;
                shape = Some(pointer);
            }
            out.flush()?;
            redraw = false;
        }
        if app.quit {
            break;
        }
        let wait = backend.gliding().unwrap_or(if app.has_session() {
            SESSION_POLL
        } else {
            POLL
        });
        if ct::poll(wait)? {
            let started = Instant::now();
            // Drain everything queued, so a fast wheel doesn't redraw per event.
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
                // Coming back, or typing, can leave the terminal showing a pointer of its own.
                if matches!(event, ct::Event::FocusGained | ct::Event::Key(_)) {
                    shape = None;
                }
                match translate(event, mouse_cell(&display)).map(|e| cmd_left(e, ghostty)) {
                    // The size check above handles it, and compares against the old size.
                    Some(Event::Resize(..)) | None => {}
                    Some(event) => app.handle(event),
                }
                // A stream of input mustn't hold up the frames it's asking for.
                if started.elapsed() >= INPUT_SLICE || !ct::poll(Duration::ZERO)? {
                    break;
                }
            }
            phases.events += started.elapsed();
            redraw = true;
        } else if last_tick.elapsed() >= TICK {
            let started = Instant::now();
            last_tick = started;
            app.handle(Event::Tick);
            phases.events += started.elapsed();
            redraw = true;
        }
        let started = Instant::now();
        redraw |= app.poll();
        phases.poll += started.elapsed();
        if backend.gliding().is_some() {
            backend.glide(Instant::now(), &mut out)?;
            // Arrived, so the page underneath takes over.
            redraw |= backend.gliding().is_none();
        }
        if let Some(server) = &server {
            while let Ok(request) = server.requests.try_recv() {
                app.serve(request);
                redraw = true;
            }
        }
    }
    write!(out, "\x1b]22;default\x1b\\")?;
    out.flush()?;
    drop(_session);
    app.finish()
}
