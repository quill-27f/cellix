use std::io::{self, Write};
use std::path::PathBuf;
use std::time::Duration;

use base64::Engine;
use cellix_input::{Event, Key, KeyEvent, Mods, MouseEvent, MouseKind};
use cellix_render::{Grid, SoftwareRasterizer};
use cellix_term::{
    Backend, CellBackend, DEFAULT_QUOTA, PixelBackend, Placement, Tier, detect, select_tier,
};
use crossterm::event::{
    self as ct, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste,
    EnableMouseCapture, KeyCode, KeyEventKind, KeyModifiers, KeyboardEnhancementFlags, MouseButton,
    MouseEventKind, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode, size,
    window_size,
};

use crate::App;

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

fn translate(event: ct::Event) -> Option<Event> {
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
                MouseEventKind::ScrollUp => MouseKind::ScrollUp,
                MouseEventKind::ScrollDown => MouseKind::ScrollDown,
                _ => return None,
            };
            Event::Mouse(MouseEvent {
                kind,
                x: m.column,
                y: m.row,
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
}

impl Display {
    fn detect(setting: Option<&str>) -> io::Result<Self> {
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
        Ok(Self { placement, cell })
    }

    /// Any failure on the way to pixels lands on cells, which always work.
    fn backend(&self) -> Box<dyn Backend> {
        let pixels = self.placement.zip(self.cell).and_then(|(placement, cell)| {
            let rasterizer = SoftwareRasterizer::new(cell.0, cell.1)?;
            Some(PixelBackend::new(rasterizer, placement, DEFAULT_QUOTA))
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

    let _session = Session::start()?;
    let mut display = Display::detect(app.settings.tier.as_deref())?;
    let mut backend = display.backend();
    let mut out = io::stdout();
    let mut redraw = true;
    let mut polls = 0;
    loop {
        // Polled as well as signalled: a resize during slow startup is otherwise lost.
        let now = size()?;
        if now != app.size() {
            app.handle(Event::Resize(now.0, now.1));
            let cell = window_cell();
            if display.placement.is_some() && cell.is_some() && cell != display.cell {
                display.cell = cell;
                backend = display.backend();
            }
            backend.invalidate();
            redraw = true;
        }
        if redraw {
            let (width, height) = app.size();
            let mut grid = Grid::new(width, height, app.background());
            app.draw(&mut grid);
            backend.present(&grid, &mut out)?;
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
                match translate(ct::read()?) {
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
