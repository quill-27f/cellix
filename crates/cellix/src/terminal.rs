use std::io::{self, Write};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use base64::Engine;
use cellix_input::{Event, Key, KeyEvent, Mods, MouseEvent, MouseKind};
use cellix_render::{CellMeasure, FontMeasure, Grid, Measure, SoftwareRasterizer};
use cellix_term::{
    Backend, CellBackend, DEFAULT_QUOTA, PixelBackend, Placement, Smooth, Tier, detect, select_tier,
};
use crossterm::event::{
    self as ct, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste,
    EnableMouseCapture, KeyCode, KeyEventKind, KeyModifiers, KeyboardEnhancementFlags, MouseButton,
    MouseEventKind, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use crossterm::execute;
use crossterm::style::Print;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode, size,
    window_size,
};

use crate::App;

/// Steps in a wheel notch's animation, which together take about 70ms.
const SCROLL_FRAMES: u32 = 6;
const REMOTE_SCROLL_FRAMES: u32 = 3;
const SCROLL_PAUSE: Duration = Duration::from_millis(12);
/// How long to wait for input before checking on language servers.
const POLL: Duration = Duration::from_millis(50);
/// Polls between ticks, so background file checks run about twice a second.
const POLLS_PER_TICK: u32 = 10;

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
            // Terminals without the Kitty keyboard protocol ignore this, and Ctrl+Shift+P
            // and Cmd keys then can't be told apart: F1 and Alt+P still open commands.
            PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
        )?;
        Ok(Self)
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = execute!(
            io::stdout(),
            PopKeyboardEnhancementFlags,
            Print("\x1b[?1016l"),
            DisableBracketedPaste,
            DisableMouseCapture,
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

/// How the terminal was found to work, kept so the backend can be rebuilt on a font zoom.
struct Display {
    placement: Option<Placement>,
    cell: Option<(usize, usize)>,
    pixel_mouse: bool,
    light: Option<bool>,
    background: Option<(u8, u8, u8)>,
    smooth: Option<Smooth>,
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
        Ok(Self {
            placement,
            cell,
            pixel_mouse: env.pixel_mouse,
            light: env.light,
            background: env.background,
            // Over a network each frame costs a round of placements, so send fewer.
            smooth: smooth.then(|| Smooth {
                frames: if std::env::var_os("SSH_CONNECTION").is_some() {
                    REMOTE_SCROLL_FRAMES
                } else {
                    SCROLL_FRAMES
                },
                pause: SCROLL_PAUSE,
            }),
        })
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
            let backend = PixelBackend::new(rasterizer, placement, DEFAULT_QUOTA);
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
    app.ide = server
        .as_ref()
        .and_then(|_| cellix_agent::ide::IdeServer::start(app.workspace.root()).ok());

    let _session = Session::start()?;
    let smooth = app.settings.smooth_scroll != Some(false);
    let mut display = Display::detect(app.settings.tier.as_deref(), smooth)?;
    let mut backend = display.backend();
    let mut out = io::stdout();
    app.set_measure(display.measure());
    app.follow_terminal_theme(display.light);
    app.match_terminal_background(display.background);
    // Whole cells can't place a caret in proportional text, so ask for pixels where that's
    // on offer. It's only used with a known cell size to divide by.
    let pixel_mouse = display.pixel_mouse && display.placement.is_some() && display.cell.is_some();
    if pixel_mouse {
        write!(out, "\x1b[?1016h")?;
    }
    let mouse_cell = |display: &Display| display.cell.filter(|_| pixel_mouse);
    let mut redraw = true;
    let mut polls = 0;
    let mut shown: Option<Grid> = None;
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
        if redraw {
            let (width, height) = app.size();
            let mut grid = Grid::new(width, height, app.background());
            app.draw(&mut grid);
            if let (Some((area, rows)), Some(from)) = (app.take_scroll(), &shown) {
                backend.scroll(from, &grid, area, rows, &mut out)?;
            }
            backend.present(&grid, &mut out)?;
            shown = Some(grid);
            if let Some(text) = app.outgoing_clipboard.take() {
                let encoded = base64::engine::general_purpose::STANDARD.encode(text);
                write!(out, "\x1b]52;c;{encoded}\x07")?;
            }
            out.flush()?;
            redraw = false;
        }
        if app.quit {
            break;
        }
        if ct::poll(POLL)? {
            // Drain everything queued, so a fast wheel doesn't redraw per event.
            loop {
                match translate(ct::read()?, mouse_cell(&display)) {
                    // The size check above handles it, and compares against the old size.
                    Some(Event::Resize(..)) | None => {}
                    Some(event) => app.handle(event),
                }
                if !ct::poll(Duration::ZERO)? {
                    break;
                }
            }
            redraw = true;
        } else {
            polls += 1;
            if polls >= POLLS_PER_TICK {
                polls = 0;
                app.handle(Event::Tick);
                redraw = true;
            }
        }
        redraw |= app.poll();
        if let Some(server) = &server {
            while let Ok(request) = server.requests.try_recv() {
                app.serve(request);
                redraw = true;
            }
        }
    }
    drop(_session);
    app.finish()
}
