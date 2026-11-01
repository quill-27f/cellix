//! Cellix with no terminal of its own, drawn inside another program. The Claude Code mod
//! reads frames from stdout, one line each, and sends input through the workspace socket.

use std::collections::HashMap;
use std::fs;
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use base64::Engine;
use cellix_agent::Request;
use cellix_agent::ide::IdeServer;
use cellix_input::{Event, Key, KeyEvent, Mods, MouseEvent, MouseKind};
use cellix_render::{FontMeasure, Grid, Rasterizer, SoftwareRasterizer};
use cellix_term::row_key;
use serde::Deserialize;

use crate::App;

/// How long to wait for input before looking at language servers and agents.
const WAIT: Duration = Duration::from_millis(30);
const TICKS_EVERY: u32 = 16;
/// A pane sits beside a conversation, so the code gets the room until the pane is wide.
const PANE_COMPACT_WIDTH: u16 = 110;
const DEFAULT_SIZE: (u16, u16) = (100, 40);
/// A cell on a Retina display, when the terminal won't say.
const DEFAULT_CELL: (usize, usize) = (16, 36);

/// One thing the mod passes on.
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Input {
    Key {
        key: String,
        #[serde(default)]
        ctrl: bool,
        #[serde(default)]
        shift: bool,
        #[serde(default)]
        meta: bool,
    },
    Pointer {
        kind: String,
        x: u16,
        y: u16,
        fine: Option<f32>,
        button: Option<String>,
    },
    Scroll {
        by: i32,
    },
    Resize {
        columns: u16,
        rows: u16,
    },
    Open {
        path: PathBuf,
    },
    Quiet,
}

fn key_of(name: &str) -> Option<Key> {
    Some(match name {
        "up" => Key::Up,
        "down" => Key::Down,
        "left" => Key::Left,
        "right" => Key::Right,
        "return" | "enter" => Key::Enter,
        "tab" => Key::Tab,
        "backspace" => Key::Backspace,
        "delete" => Key::Delete,
        "pageup" => Key::PageUp,
        "pagedown" => Key::PageDown,
        "home" => Key::Home,
        "end" => Key::End,
        "escape" => Key::Esc,
        "space" => Key::Char(' '),
        one => {
            let mut chars = one.chars();
            let c = chars.next()?;
            if chars.next().is_some() {
                return None;
            }
            Key::Char(c)
        }
    })
}

/// Turns input into the app's events. Takes no terminal, which keeps it testable.
pub fn translate(input: Input) -> Vec<Event> {
    match input {
        Input::Key {
            key,
            ctrl,
            shift,
            meta,
        } => key_of(&key)
            .map(|key| {
                let mods = Mods {
                    ctrl: ctrl || meta,
                    alt: false,
                    shift,
                };
                Event::Key(KeyEvent { key, mods })
            })
            .into_iter()
            .collect(),
        Input::Pointer {
            kind,
            x,
            y,
            fine,
            button,
        } => {
            let kind = match (kind.as_str(), button.as_deref()) {
                ("down", Some("right")) => MouseKind::Right,
                ("down", Some("middle")) => MouseKind::Middle,
                ("down", _) => MouseKind::Left,
                ("up", _) => MouseKind::Up,
                ("move", Some(_)) => MouseKind::Drag,
                ("move", None) => MouseKind::Move,
                _ => return Vec::new(),
            };
            let fine_x = fine.unwrap_or(x as f32);
            let mods = Mods::default();
            vec![Event::Mouse(MouseEvent {
                kind,
                x,
                y,
                fine_x,
                mods,
            })]
        }
        Input::Scroll { by } => {
            let kind = if by < 0 {
                MouseKind::ScrollUp
            } else {
                MouseKind::ScrollDown
            };
            let wheel = |_| {
                Event::Mouse(MouseEvent {
                    kind,
                    x: u16::MAX,
                    y: u16::MAX,
                    fine_x: 0.0,
                    mods: Mods::default(),
                })
            };
            (0..by.unsigned_abs()).map(wheel).collect()
        }
        Input::Resize { columns, rows } => vec![Event::Resize(columns.max(20), rows.max(5))],
        Input::Open { .. } | Input::Quiet => Vec::new(),
    }
}

/// Cell size from the window Claude Code is drawn in. The child has no replies to read,
/// because Claude Code owns the terminal's input, but the window size needs none.
fn window_cell() -> Option<(usize, usize)> {
    let tty = fs::File::open("/dev/tty").ok()?;
    let mut size = libc::winsize {
        ws_row: 0,
        ws_col: 0,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    // SAFETY: TIOCGWINSZ writes one winsize into memory this function owns.
    let done = unsafe {
        libc::ioctl(
            std::os::fd::AsRawFd::as_raw_fd(&tty),
            libc::TIOCGWINSZ,
            &mut size,
        )
    };
    let usable = done == 0 && size.ws_xpixel > 0 && size.ws_col > 0 && size.ws_row > 0;
    usable.then(|| {
        (
            size.ws_xpixel as usize / size.ws_col as usize,
            size.ws_ypixel as usize / size.ws_row as usize,
        )
    })
}

/// Rows of a frame, rastered once each and kept until they change.
struct Frames<R> {
    rasterizer: R,
    cell: (usize, usize),
    rows: HashMap<u64, Rc<Vec<u8>>>,
    /// What the last frame was made of.
    shown: Vec<u64>,
    folder: PathBuf,
    /// Frames take turns between two files, so one is never read while it's written.
    count: u64,
}

impl<R: Rasterizer> Frames<R> {
    /// All of a grid as RGB, and its size.
    fn pixels(&mut self, grid: &Grid) -> (Vec<u8>, usize, usize) {
        let mut rgb = Vec::new();
        let mut live = HashMap::new();
        for y in 0..grid.height() {
            let key = row_key(grid, y);
            let row = self
                .rows
                .get(&key)
                .cloned()
                .unwrap_or_else(|| Rc::new(self.rasterizer.row(grid, y).rgb));
            rgb.extend_from_slice(&row);
            live.insert(key, row);
        }
        self.rows = live;
        let width = grid.width() as usize * self.cell.0;
        let height = grid.height() as usize * self.cell.1;
        (rgb, width, height)
    }

    /// Describes a frame in one line for the mod, as a file where it's read on this machine.
    /// Returns None if nothing changed: an idle pane must not keep reuploading an image.
    fn line(&mut self, grid: &Grid, remote: bool) -> io::Result<Option<String>> {
        let keys: Vec<u64> = (0..grid.height()).map(|y| row_key(grid, y)).collect();
        if keys == self.shown {
            return Ok(None);
        }
        self.shown = keys;
        let (rgb, width, height) = self.pixels(grid);
        self.count += 1;
        if remote {
            return png_line(&rgb, width, height).map(Some);
        }
        let path = self.folder.join(format!("frame-{}.rgb", self.count % 2));
        fs::write(&path, &rgb)?;
        let line = format!("frame {} {width} {height} {}", path.display(), self.count);
        Ok(Some(line))
    }
}

/// A frame by value. A file can't cross SSH, so remote sessions pay for a PNG.
fn png_line(rgb: &[u8], width: usize, height: usize) -> io::Result<String> {
    let mut bytes = Vec::new();
    let mut encoder = png::Encoder::new(&mut bytes, width as u32, height as u32);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_compression(png::Compression::Fast);
    let mut writer = encoder.write_header().map_err(io::Error::other)?;
    writer.write_image_data(rgb).map_err(io::Error::other)?;
    writer.finish().map_err(io::Error::other)?;
    let text = base64::engine::general_purpose::STANDARD.encode(&bytes);
    Ok(format!("png {text}"))
}

fn parent_gone() -> bool {
    // SAFETY: getppid has no preconditions and cannot fail.
    unsafe { libc::getppid() == 1 }
}

/// Runs until Claude Code goes away.
pub fn run(root: &Path) -> io::Result<()> {
    let mut app = App::new(root.to_path_buf(), DEFAULT_SIZE);
    app.compact_width = PANE_COMPACT_WIDTH;
    app.in_pane = true;
    let cell = window_cell().unwrap_or(DEFAULT_CELL);
    app.set_measure(Rc::new(FontMeasure::new(cell.0, cell.1)));
    let server = cellix_agent::start(root, app.comments.clone())?;
    app.ide = IdeServer::start(root).ok();
    let folder = cellix_agent::socket_path(root).with_extension("pane");
    fs::create_dir_all(&folder)?;
    let rasterizer =
        SoftwareRasterizer::new(cell.0, cell.1).ok_or_else(|| io::Error::other("no fonts"))?;
    let mut frames = Frames {
        rasterizer,
        cell,
        rows: HashMap::new(),
        shown: Vec::new(),
        folder,
        count: 0,
    };
    let remote = std::env::var_os("SSH_CONNECTION").is_some();
    let mut out = BufWriter::new(io::stdout());
    if let Some(ide) = &app.ide {
        writeln!(out, "ide {}", ide.port)?;
    }
    let mut redraw = true;
    let mut waits = 0;
    while !app.quit && !parent_gone() {
        if redraw {
            let (columns, rows) = app.size();
            let mut grid = Grid::new(columns, rows, app.background());
            app.draw(&mut grid);
            if let Some(line) = frames.line(&grid, remote)? {
                writeln!(out, "{line}")?;
            }
            if let Some(reason) = app.take_attention() {
                writeln!(out, "attention {reason}")?;
            }
            out.flush()?;
            redraw = false;
        }
        match server.requests.recv_timeout(WAIT) {
            Ok(Request::Pane(text)) => {
                if let Ok(input) = serde_json::from_str::<Input>(&text) {
                    if let Input::Open { path } = &input {
                        let _ = app.open(path);
                    }
                    for event in translate(input) {
                        app.handle(event);
                    }
                    redraw = true;
                }
            }
            Ok(request) => {
                app.serve(request);
                redraw = true;
            }
            Err(_) => {
                waits += 1;
                if waits >= TICKS_EVERY {
                    waits = 0;
                    app.handle(Event::Tick);
                    redraw = true;
                }
            }
        }
        redraw |= app.poll();
    }
    app.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_cross_from_the_mod() {
        let input: Input = serde_json::from_str(r#"{"type":"key","key":"s","meta":true}"#).unwrap();
        assert_eq!(translate(input), [Event::Key(KeyEvent::ctrl('s'))]);
        let input: Input = serde_json::from_str(r#"{"type":"key","key":"return"}"#).unwrap();
        assert_eq!(translate(input), [Event::Key(KeyEvent::plain(Key::Enter))]);
    }

    #[test]
    fn the_space_bar_crosses_from_the_mod() {
        for name in ["space", " "] {
            let text = format!(r#"{{"type":"key","key":"{name}"}}"#);
            let input: Input = serde_json::from_str(&text).unwrap();
            assert_eq!(
                translate(input),
                [Event::Key(KeyEvent::plain(Key::Char(' ')))]
            );
        }
    }

    #[test]
    fn clicks_and_the_wheel_cross_from_the_mod() {
        let click = r#"{"type":"pointer","kind":"down","x":3,"y":4,"button":"middle"}"#;
        let input: Input = serde_json::from_str(click).unwrap();
        let [Event::Mouse(mouse)] = translate(input)[..] else {
            panic!("no event");
        };
        assert_eq!((mouse.kind, mouse.x, mouse.y), (MouseKind::Middle, 3, 4));
        let input: Input = serde_json::from_str(r#"{"type":"scroll","by":-2}"#).unwrap();
        assert_eq!(translate(input).len(), 2);
    }

    #[test]
    fn a_size_never_goes_below_what_can_be_drawn() {
        let input: Input =
            serde_json::from_str(r#"{"type":"resize","columns":3,"rows":1}"#).unwrap();
        assert_eq!(translate(input), [Event::Resize(20, 5)]);
    }

    struct Flat;

    impl Rasterizer for Flat {
        fn row(&mut self, grid: &Grid, _y: u16) -> cellix_render::Image {
            cellix_render::Image {
                width: grid.width() as usize,
                height: 1,
                rgb: vec![9; grid.width() as usize * 3],
            }
        }
    }

    #[test]
    fn an_idle_pane_sends_no_frames() {
        let dir = tempfile::tempdir().unwrap();
        let mut frames = Frames {
            rasterizer: Flat,
            cell: (1, 1),
            rows: HashMap::new(),
            shown: Vec::new(),
            folder: dir.path().to_path_buf(),
            count: 0,
        };
        let mut grid = Grid::new(4, 2, cellix_render::Style::default());
        assert!(frames.line(&grid, false).unwrap().is_some());
        assert!(frames.line(&grid, false).unwrap().is_none());
        grid.put_str(0, 1, "x", cellix_render::Style::default(), 4);
        assert!(frames.line(&grid, false).unwrap().is_some());
    }

    #[test]
    fn a_remote_frame_travels_as_a_png() {
        let line = png_line(&[9; 12], 2, 2).unwrap();
        assert!(line.starts_with("png iVBOR"), "{line}");
    }
}
