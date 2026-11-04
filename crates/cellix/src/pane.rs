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

/// What one `pane-send` carries: a batch the mod gathered, or one input.
fn inputs(text: &str) -> Vec<Input> {
    serde_json::from_str::<Vec<Input>>(text)
        .or_else(|_| serde_json::from_str::<Input>(text).map(|one| vec![one]))
        .unwrap_or_default()
}

/// Keeps only the last of a run of moves. Each move can change what's hovered and so cost a
/// frame, and only where the pointer ended up matters; downs, ups and keys all stay, in order.
fn latest_moves(inputs: Vec<Input>) -> Vec<Input> {
    let moving = |input: &Input| match input {
        Input::Pointer { kind, button, .. } if kind == "move" => Some(button.is_some()),
        _ => None,
    };
    let mut kept: Vec<Input> = Vec::with_capacity(inputs.len());
    for input in inputs {
        let held = moving(&input);
        if held.is_some() && kept.last().map(moving) == Some(held) {
            kept.pop();
        }
        kept.push(input);
    }
    kept
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
#[cfg(unix)]
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

    /// Describes a frame in one line for the mod, as a file the terminal reads on this machine.
    /// Returns None if nothing changed: an idle pane must not keep reuploading an image.
    fn line(&mut self, grid: &Grid) -> io::Result<Option<String>> {
        if !changed(&mut self.shown, grid) {
            return Ok(None);
        }
        let (rgb, width, height) = self.pixels(grid);
        self.count += 1;
        let path = self.folder.join(format!("frame-{}.rgb", self.count % 2));
        fs::write(&path, &rgb)?;
        let line = format!("frame {} {width} {height} {}", path.display(), self.count);
        Ok(Some(line))
    }
}

/// What a cell shows in Claude Code's Raster, which takes one printable width-1 character
/// from the Basic Multilingual Plane: an icon is its cell-tier drawing, as in a terminal, the
/// right half of a wide character is a space, and a character it can't show stands as `?`.
fn raster_char(symbol: &str) -> u32 {
    use unicode_width::UnicodeWidthChar;
    if let Some(piece) = cellix_render::Piece::decode(symbol) {
        return raster_char(piece.fallback());
    }
    match symbol.chars().next() {
        None => ' ' as u32,
        // The narrow layout's sidebar button, wide in Unicode; the same bars fit one cell.
        Some('☰') => '≡' as u32,
        Some(c) if (c as u32) <= 0xFFFF && !c.is_control() && c.width() == Some(1) => c as u32,
        Some(_) => '?' as u32,
    }
}

/// A frame as cells, for Claude Code to draw as text: the cell tier, as Cellix draws in any
/// terminal without pictures. Over SSH this is what keeps the pane quick, because only the
/// cells that changed cross the connection, where a picture of the pane crossed it whole
/// each time the pointer moved.
fn cells_line(grid: &Grid) -> String {
    let colour = |c: cellix_render::Rgb| u32::from_be_bytes([0, c.0, c.1, c.2]);
    let mut bytes = Vec::with_capacity(grid.width() as usize * grid.height() as usize * 12);
    for y in 0..grid.height() {
        for (x, cell) in grid.row(y).iter().enumerate() {
            // A Raster has no cursor of its own, so the cursor's cell is drawn inverted, as a
            // terminal's block cursor is.
            let (fg, bg) = if grid.cursor == Some((x as u16, y)) {
                (cell.style.bg, cell.style.fg)
            } else {
                (cell.style.fg, cell.style.bg)
            };
            for word in [raster_char(&cell.symbol), colour(fg), colour(bg)] {
                bytes.extend_from_slice(&word.to_le_bytes());
            }
        }
    }
    let text = base64::engine::general_purpose::STANDARD.encode(&bytes);
    format!("cells {} {} {text}", grid.width(), grid.height())
}

/// Whether a grid differs from the one last sent, remembering it if so.
fn changed(shown: &mut Vec<u64>, grid: &Grid) -> bool {
    let keys: Vec<u64> = (0..grid.height()).map(|y| row_key(grid, y)).collect();
    if keys == *shown {
        return false;
    }
    *shown = keys;
    true
}

/// A console doesn't give its size in pixels, so the default cell is used.
#[cfg(not(unix))]
fn window_cell() -> Option<(usize, usize)> {
    None
}

#[cfg(unix)]
fn parent_gone() -> bool {
    // SAFETY: getppid has no preconditions and cannot fail.
    unsafe { libc::getppid() == 1 }
}

/// Windows doesn't hand an orphan to another parent, so there's nothing to see here: the pane
/// ends when its output can't be written, or Claude Code ends it.
#[cfg(not(unix))]
fn parent_gone() -> bool {
    false
}

/// Runs until Claude Code goes away.
pub fn run(root: &Path) -> io::Result<()> {
    let mut app = App::new(root.to_path_buf(), DEFAULT_SIZE);
    app.compact_width = PANE_COMPACT_WIDTH;
    app.in_pane = true;
    // Over SSH a picture of the pane would cross the connection every time it changed, so
    // the pane is drawn as cells there, as it is in a terminal without pictures.
    let cells = std::env::var_os("SSH_CONNECTION").is_some()
        || std::env::var("CELLIX_TIER").is_ok_and(|tier| tier == "cells");
    let mut frames = None;
    if !cells {
        let cell = window_cell().unwrap_or(DEFAULT_CELL);
        app.set_measure(Rc::new(FontMeasure::new(cell.0, cell.1)));
        let rasterizer =
            SoftwareRasterizer::new(cell.0, cell.1).ok_or_else(|| io::Error::other("no fonts"))?;
        let folder = cellix_agent::socket_path(root).with_extension("pane");
        fs::create_dir_all(&folder)?;
        frames = Some(Frames {
            rasterizer,
            cell,
            rows: HashMap::new(),
            shown: Vec::new(),
            folder,
            count: 0,
        });
    }
    let mut shown_cells = Vec::new();
    let server = cellix_agent::start(root, app.comments.clone())?;
    app.restore_unsent();
    app.ide = IdeServer::start(root).ok();
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
            let line = match &mut frames {
                Some(frames) => frames.line(&grid)?,
                None => changed(&mut shown_cells, &grid).then(|| cells_line(&grid)),
            };
            if let Some(line) = line {
                writeln!(out, "{line}")?;
            }
            if let Some(prompt) = app.take_outgoing_prompt() {
                writeln!(
                    out,
                    "prompt {}",
                    serde_json::to_string(&prompt).unwrap_or_default()
                )?;
            }
            if let Some(text) = app.take_outgoing_fill() {
                writeln!(
                    out,
                    "fill {}",
                    serde_json::to_string(&text).unwrap_or_default()
                )?;
            }
            if let Some(reason) = app.take_attention() {
                writeln!(out, "attention {reason}")?;
            }
            out.flush()?;
            redraw = false;
        }
        match server.requests.recv_timeout(WAIT) {
            Ok(first) => {
                // Everything that came in meanwhile is taken before the next frame, so a burst
                // of input costs one frame, not one each.
                let waiting: Vec<Request> = std::iter::once(first)
                    .chain(server.requests.try_iter())
                    .collect();
                take_requests(&mut app, waiting);
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

/// Hands requests to the app in order, with the pointer's moves between other input cut to
/// the last.
fn take_requests(app: &mut App, requests: Vec<Request>) {
    let mut input = Vec::new();
    for request in requests {
        match request {
            Request::Pane(text) => input.extend(inputs(&text)),
            other => {
                take_input(app, std::mem::take(&mut input));
                app.serve(other);
            }
        }
    }
    take_input(app, input);
}

fn take_input(app: &mut App, input: Vec<Input>) {
    for one in latest_moves(input) {
        if let Input::Open { path } = &one {
            let _ = app.open(path);
        }
        for event in translate(one) {
            app.handle(event);
        }
    }
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
        assert!(frames.line(&grid).unwrap().is_some());
        assert!(frames.line(&grid).unwrap().is_none());
        grid.put_str(0, 1, "x", cellix_render::Style::default(), 4);
        assert!(frames.line(&grid).unwrap().is_some());
    }

    /// Over SSH a frame is the cell grid, as Claude Code's Raster reads it: a character, then
    /// the foreground and background, each a little-endian u32.
    #[test]
    fn a_remote_frame_travels_as_cells() {
        let mut grid = Grid::new(3, 1, cellix_render::Style::default());
        let style = cellix_render::Style {
            fg: cellix_render::Rgb(0x11, 0x22, 0x33),
            bg: cellix_render::Rgb(0xaa, 0xbb, 0xcc),
            ..cellix_render::Style::default()
        };
        grid.put_str(0, 0, "a界", style, 3);
        let line = cells_line(&grid);
        let [kind, columns, rows, data] = line.split(' ').collect::<Vec<_>>()[..] else {
            panic!("{line}");
        };
        assert_eq!((kind, columns, rows), ("cells", "3", "1"));
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(data)
            .unwrap();
        let words: Vec<u32> = bytes
            .chunks(4)
            .map(|w| u32::from_le_bytes(w.try_into().unwrap()))
            .collect();
        assert_eq!(&words[..3], [u32::from('a'), 0x112233, 0xaabbcc]);
        // A wide character can't stand in one cell of a Raster, and its right half is blank.
        assert_eq!(words[3], u32::from('?'));
        assert_eq!(words[6], u32::from(' '));
    }

    /// Icons live in the private plane, outside what a Raster takes, so they go as the cell
    /// tier draws them; and the cursor, which a Raster has none of, is its cell inverted.
    #[test]
    fn icons_and_the_cursor_survive_as_cells() {
        let mut grid = Grid::new(2, 1, cellix_render::Style::default());
        let style = cellix_render::Style {
            fg: cellix_render::Rgb(1, 1, 1),
            bg: cellix_render::Rgb(2, 2, 2),
            ..cellix_render::Style::default()
        };
        let piece = cellix_render::Piece {
            icon: cellix_render::Icon::Files,
            width: 1,
            height: 1,
            column: 0,
            row: 0,
        };
        grid.put_str(0, 0, &piece.symbol().to_string(), style, 1);
        grid.put_str(1, 0, "x", style, 2);
        grid.cursor = Some((1, 0));
        let line = cells_line(&grid);
        let data = line.rsplit(' ').next().unwrap();
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(data)
            .unwrap();
        let word = |at: usize| u32::from_le_bytes(bytes[at * 4..at * 4 + 4].try_into().unwrap());
        assert_eq!(word(0), u32::from('▤'));
        assert_eq!(raster_char("☰"), u32::from('≡'), "the sidebar button");
        assert_eq!((word(1), word(2)), (0x010101, 0x020202));
        assert_eq!(word(3), u32::from('x'));
        assert_eq!(
            (word(4), word(5)),
            (0x020202, 0x010101),
            "the cursor's cell inverted"
        );
    }

    #[test]
    fn a_batch_of_input_crosses_in_one_send() {
        let batch = r#"[{"type":"key","key":"a"},{"type":"pointer","kind":"down","x":1,"y":2}]"#;
        assert_eq!(inputs(batch).len(), 2);
        assert_eq!(inputs(r#"{"type":"key","key":"a"}"#).len(), 1);
        assert!(inputs("not json").is_empty());
    }

    /// Every move used to cost a frame, and over SSH the moves queued up ahead of a click.
    #[test]
    fn a_run_of_moves_keeps_only_where_the_pointer_ended() {
        let at = |kind: &str, x: u16, button: Option<&str>| Input::Pointer {
            kind: kind.to_string(),
            x,
            y: 0,
            fine: None,
            button: button.map(str::to_string),
        };
        let kept = latest_moves(vec![
            at("move", 1, None),
            at("move", 2, None),
            at("move", 3, None),
            at("down", 3, Some("left")),
            at("move", 4, Some("left")),
            at("move", 5, Some("left")),
            at("up", 5, Some("left")),
            at("move", 6, None),
        ]);
        let shown: Vec<(String, u16)> = kept
            .iter()
            .map(|input| match input {
                Input::Pointer { kind, x, .. } => (kind.clone(), *x),
                _ => panic!("only pointers"),
            })
            .collect();
        let expected = [
            ("move", 3),
            ("down", 3),
            ("move", 5),
            ("up", 5),
            ("move", 6),
        ];
        assert_eq!(
            shown,
            expected.map(|(kind, x)| (kind.to_string(), x)).to_vec()
        );
    }
}
