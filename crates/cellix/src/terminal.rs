use std::io::{self, Write};
use std::path::PathBuf;
use std::time::Duration;

use cellix_input::{Event, Key, KeyEvent, Mods, MouseEvent, MouseKind};
use cellix_render::Grid;
use cellix_term::{Backend, CellBackend};
use crossterm::event::{
    self as ct, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste,
    EnableMouseCapture, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind,
};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode, size,
};

use crate::App;

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
            EnableBracketedPaste
        )?;
        Ok(Self)
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = execute!(
            io::stdout(),
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
        ctrl: m.contains(KeyModifiers::CONTROL),
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

/// Opens `paths` and runs until the user quits.
pub fn run(paths: Vec<PathBuf>) -> io::Result<()> {
    let root = paths
        .iter()
        .find(|p| p.is_dir())
        .cloned()
        .map_or_else(std::env::current_dir, Ok)?
        .canonicalize()?;
    let mut app = App::new(root, size()?);
    for path in paths.iter().filter(|p| p.is_file()) {
        app.open(&path.canonicalize()?)?;
    }

    let _session = Session::start()?;
    let mut backend = CellBackend::default();
    let mut out = io::stdout();
    loop {
        let (width, height) = app.size();
        let mut grid = Grid::new(width, height, app.background());
        app.draw(&mut grid);
        backend.present(&grid, &mut out)?;
        out.flush()?;
        if app.quit {
            return Ok(());
        }
        if ct::poll(TICK)? {
            if let Some(event) = translate(ct::read()?) {
                if matches!(event, Event::Resize(..)) {
                    backend.invalidate();
                }
                app.handle(event);
            }
            // Drain everything queued, so a fast wheel doesn't redraw per event.
            while ct::poll(Duration::ZERO)? {
                if let Some(event) = translate(ct::read()?) {
                    if matches!(event, Event::Resize(..)) {
                        backend.invalidate();
                    }
                    app.handle(event);
                }
            }
        } else {
            app.handle(Event::Tick);
        }
    }
}
