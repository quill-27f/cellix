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
/// What the mod and the pane say to each other. The mod sets `CELLIX_MOD` to the one it
/// speaks, and this goes up when the lines change, so an old mod left behind by an upgrade
/// says so instead of waiting for lines it can't read.
pub const PROTOCOL: &str = "2";

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
    /// The mod found it can't show pictures here, so the pane goes as cells.
    Tier {
        tier: String,
        /// Claude Code's reason, to say why.
        why: Option<String>,
    },
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
                fine_y: f32::from(y) + 0.5,
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
                    fine_y: 0.0,
                    mods: Mods::default(),
                })
            };
            (0..by.unsigned_abs()).map(wheel).collect()
        }
        Input::Resize { columns, rows } => vec![Event::Resize(columns.max(20), rows.max(5))],
        Input::Open { .. } | Input::Quiet | Input::Tier { .. } => Vec::new(),
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

/// Changed rows of more than this many make a frame a big one, as scrolling makes.
const BURST_ROWS: usize = 8;
/// Big frames closer together than this are a burst: their rows go at half resolution, about
/// a quarter of the bytes, so a scroll keeps up over a slow link.
const BURST_GAP: std::time::Duration = std::time::Duration::from_millis(250);
/// How long the pane stays still before rows sent at half resolution are sent sharp.
const SHARPEN_AFTER: std::time::Duration = std::time::Duration::from_millis(200);

/// The pane as one picture per row, sent by value, for a terminal that can't read this
/// machine's files: over SSH, or through tmux. Each row is named by what it shows, and its
/// picture crosses only when the mod doesn't hold that name yet. Claude Code uploads a picture
/// to the terminal once per name and moves it with the row, so a scroll costs the lines that
/// came into view, and a hover or a click a row or two; measured in real Claude Code, a row
/// moved by name uploads nothing.
struct RowPictures<R> {
    rasterizer: R,
    cell: (usize, usize),
    /// The name of what each row on screen shows, as last sent.
    shown: Vec<String>,
    /// The names whose pictures the mod holds: those on screen.
    held: std::collections::HashSet<String>,
    width: u16,
    last_big: Option<std::time::Instant>,
    last_change: std::time::Instant,
}

/// A row's name: what it holds, and whether its picture is the soft one.
fn row_name(key: u64, sharp: bool) -> String {
    if sharp {
        format!("{key:x}")
    } else {
        format!("{key:x}s")
    }
}

impl<R: Rasterizer> RowPictures<R> {
    fn new(rasterizer: R, cell: (usize, usize)) -> Self {
        Self {
            rasterizer,
            cell,
            shown: Vec::new(),
            held: std::collections::HashSet::new(),
            width: 0,
            last_big: None,
            last_change: std::time::Instant::now(),
        }
    }

    fn soft_on_screen(&self) -> bool {
        self.shown.iter().any(|name| name.ends_with('s'))
    }

    /// One row as a PNG, base64, at full or half resolution.
    fn picture(&mut self, grid: &Grid, y: u16, sharp: bool) -> String {
        let rgb = self.rasterizer.row(grid, y).rgb;
        let (mut width, mut height) = (grid.width() as usize * self.cell.0, self.cell.1);
        let rgb = if sharp {
            rgb
        } else {
            let half = halve(&rgb, width, height);
            (width, height) = (width / 2, height / 2);
            half
        };
        base64::engine::general_purpose::STANDARD.encode(encode_png(&rgb, width, height))
    }

    /// The names of every row and the pictures the mod lacks, in one line; None when nothing
    /// changed and nothing waits to be sharpened.
    fn line(&mut self, grid: &Grid, now: std::time::Instant) -> Option<String> {
        if grid.width() != self.width {
            self.held.clear();
            self.width = grid.width();
        }
        // The cell size is in the name: a row drawn at another size is another picture.
        let size = (self.cell.0 as u64) << 32 | self.cell.1 as u64;
        let keys: Vec<u64> = (0..grid.height())
            .map(|y| row_key(grid, y) ^ size.wrapping_mul(0x9E37_79B9_7F4A_7C15))
            .collect();
        // A row keeps the name it had, sharp or soft, when it's still on screen somewhere.
        let held = |key: u64| {
            let sharp = row_name(key, true);
            if self.held.contains(&sharp) {
                return Some(sharp);
            }
            let soft = row_name(key, false);
            self.held.contains(&soft).then_some(soft)
        };
        let new = keys.iter().filter(|&&key| held(key).is_none()).count();
        let mut sharp = true;
        if new > BURST_ROWS {
            sharp = self.last_big.is_none_or(|at| now - at > BURST_GAP);
            self.last_big = Some(now);
        }
        let still = new == 0 && now - self.last_change >= SHARPEN_AFTER;
        let names: Vec<String> = keys
            .iter()
            .map(|&key| match held(key) {
                // Still for long enough: what went soft in a burst goes sharp now.
                Some(name) if still && name.ends_with('s') => row_name(key, true),
                Some(name) => name,
                None => row_name(key, sharp),
            })
            .collect();
        if names == self.shown {
            return None;
        }
        if new > 0 {
            self.last_change = now;
        }
        let mut line = format!(
            "rows {} {} {}",
            grid.width(),
            grid.height(),
            names.join(",")
        );
        let mut sent = std::collections::HashSet::new();
        for (y, name) in names.iter().enumerate() {
            if self.held.contains(name) || !sent.insert(name.clone()) {
                continue;
            }
            let picture = self.picture(grid, y as u16, !name.ends_with('s'));
            line.push_str(&format!(" {name}:{picture}"));
        }
        // The mod keeps only what's on screen.
        self.held = names.iter().cloned().collect();
        self.shown = names;
        Some(line)
    }
}

/// Half the width and height, each pixel the top left of its four: averaging makes colours
/// the row didn't have, and a row past 256 of them loses its small palette PNG.
fn halve(rgb: &[u8], width: usize, height: usize) -> Vec<u8> {
    let (w, h) = (width / 2, height / 2);
    let mut out = Vec::with_capacity(w * h * 3);
    for y in 0..h {
        for x in 0..w {
            let at = (y * 2 * width + x * 2) * 3;
            out.extend_from_slice(&rgb[at..at + 3]);
        }
    }
    out
}

fn encode_png(rgb: &[u8], width: usize, height: usize) -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut encoder = png::Encoder::new(&mut bytes, width as u32, height as u32);
    encoder.set_compression(png::Compression::Balanced);
    // Filters cost more than they save on flat colour and text.
    encoder.set_filter(png::Filter::NoFilter);
    let palette = palette_of(rgb);
    let data = match &palette {
        Some((colours, indices)) => {
            encoder.set_color(png::ColorType::Indexed);
            encoder.set_depth(png::BitDepth::Eight);
            encoder.set_palette(colours.clone());
            indices.as_slice()
        }
        None => {
            encoder.set_color(png::ColorType::Rgb);
            rgb
        }
    };
    if let Ok(mut writer) = encoder.write_header() {
        let _ = writer.write_image_data(data);
    }
    bytes
}

/// The colours of a picture and each pixel's index into them, if there are 256 or fewer.
fn palette_of(rgb: &[u8]) -> Option<(Vec<u8>, Vec<u8>)> {
    let mut index: HashMap<[u8; 3], u8> = HashMap::new();
    let mut colours = Vec::new();
    let mut indices = Vec::with_capacity(rgb.len() / 3);
    for &key in rgb.as_chunks::<3>().0 {
        let at = match index.get(&key) {
            Some(&at) => at,
            None => {
                if index.len() == 256 {
                    return None;
                }
                let at = index.len() as u8;
                index.insert(key, at);
                colours.extend_from_slice(&key);
                at
            }
        };
        indices.push(at);
    }
    Some((colours, indices))
}

/// The tmux options Claude Code needs to show pictures inside tmux, as it says itself. It
/// reads them when it starts, so they must be on before then, server wide: a pane's own
/// option, or one set while it runs, isn't seen.
const TMUX_OPTIONS: [&str; 2] = ["allow-passthrough", "focus-events"];
const RESTART_FOR_PICTURES: &str = "tmux is ready for pictures: restart Claude Code to see them";

/// Gets the tmux this runs in ready for Claude Code's pictures. Sets the options on the
/// running server, never in the user's config, so every Claude Code started after shows
/// pictures. Returns what was found.
pub fn prepare_tmux(var: impl Fn(&str) -> Option<String>) -> Tmux {
    let (Some(socket), Some(pane)) = (var("TMUX").filter(|v| !v.is_empty()), var("TMUX_PANE"))
    else {
        return Tmux::Absent;
    };
    let tmux = |args: &[&str]| {
        let done = std::process::Command::new("tmux")
            .env("TMUX", &socket)
            .args(args)
            .stdin(std::process::Stdio::null())
            .output()
            .ok()?;
        done.status
            .success()
            .then(|| String::from_utf8_lossy(&done.stdout).trim().to_string())
    };
    // Pictures stand on placeholder characters, which tmux mangles unless it speaks UTF-8.
    let utf8 = tmux(&["display-message", "-p", "-t", &pane, "#{client_utf8}"]);
    if utf8.as_deref() == Some("0") {
        return Tmux::NoPictures(
            "tmux isn't using UTF-8, so the pane shows text. Set LANG to a UTF-8 locale and attach again for pictures".into(),
        );
    }
    let on = |option: &str| {
        let value = tmux(&["show-options", "-g", "-v", option]);
        matches!(value.as_deref(), Some("on" | "all"))
    };
    if TMUX_OPTIONS.iter().all(|option| on(option)) {
        return Tmux::Ready;
    }
    let set = TMUX_OPTIONS
        .iter()
        .all(|option| tmux(&["set-option", "-g", option, "on"]).is_some());
    if set {
        Tmux::JustSet
    } else {
        Tmux::NoPictures(
            "tmux won't pass pictures through, so the pane shows text. tmux 3.3 or later shows them".into(),
        )
    }
}

#[derive(Debug, PartialEq)]
pub enum Tmux {
    Absent,
    /// Set before this Claude Code started, so its pictures show.
    Ready,
    /// Set just now: Claude Code started without them, so pictures show from its next start.
    JustSet,
    NoPictures(String),
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

/// The process that started this one. When it goes, the pane passes to another, so a
/// different answer means Claude Code is gone. It can't be compared with 1: in a container
/// or a sandbox Claude Code may itself be process 1.
#[cfg(unix)]
fn parent() -> u32 {
    // SAFETY: getppid has no preconditions and cannot fail.
    unsafe { libc::getppid() as u32 }
}

/// Windows doesn't hand an orphan to another parent, so there's nothing to see here: the pane
/// ends when its output can't be written, or Claude Code ends it.
#[cfg(not(unix))]
fn parent() -> u32 {
    0
}

/// Whether Claude Code can show the pane from a picture file here. It reads one only in kitty
/// and Ghostty on this machine, never through tmux or Zellij, and over SSH the file isn't
/// where the terminal is. Everywhere else the pictures go by value, a row each.
pub fn pictures_show(var: impl Fn(&str) -> Option<String>) -> bool {
    let set = |name: &str| var(name).is_some_and(|v| !v.is_empty());
    match var("CELLIX_TIER").as_deref() {
        Some("cells") => return false,
        Some("pixels") => return true,
        _ => {}
    }
    if set("SSH_CONNECTION") || set("SSH_TTY") || set("TMUX") || var("ZELLIJ").is_some() {
        return false;
    }
    let program = var("TERM_PROGRAM").unwrap_or_default().to_lowercase();
    let term = var("TERM").unwrap_or_default();
    program == "ghostty"
        || term == "xterm-ghostty"
        || term == "xterm-kitty"
        || set("KITTY_WINDOW_ID")
}

/// Why an old mod can't run this pane, once the new one is written where Claude Code loads it.
fn mod_too_old() -> io::Error {
    let written = crate::plugin::install().is_some();
    let version = env!("CARGO_PKG_VERSION");
    io::Error::other(if written {
        format!("Cellix {version} updated its Claude Code plugin. Restart Claude Code to load it")
    } else {
        format!(
            "Cellix {version} needs a newer Claude Code plugin: run `cellix setup claude`, then restart Claude Code"
        )
    })
}

/// Runs until Claude Code goes away.
pub fn run(root: &Path) -> io::Result<()> {
    if std::env::var("CELLIX_MOD").as_deref() != Ok(PROTOCOL) {
        return Err(mod_too_old());
    }
    let started_by = parent();
    let mut app = App::new(root.to_path_buf(), DEFAULT_SIZE);
    app.compact_width = PANE_COMPACT_WIDTH;
    app.in_pane = true;
    let var = |name: &str| std::env::var(name).ok();
    let mut output = match (var("CELLIX_TIER").as_deref(), prepare_tmux(var)) {
        (Some("cells"), _) => Output::Cells,
        (_, Tmux::NoPictures(why)) => {
            app.say(why);
            Output::Cells
        }
        (_, Tmux::JustSet) => {
            app.say(RESTART_FOR_PICTURES);
            Output::Cells
        }
        (_, Tmux::Absent | Tmux::Ready) => {
            let cell = window_cell().unwrap_or(DEFAULT_CELL);
            app.set_measure(Rc::new(FontMeasure::new(cell.0, cell.1)));
            let rasterizer = SoftwareRasterizer::new(cell.0, cell.1)
                .ok_or_else(|| io::Error::other("no fonts"))?;
            if pictures_show(var) {
                let folder = cellix_agent::socket_path(root).with_extension("pane");
                fs::create_dir_all(&folder)?;
                Output::File(Frames {
                    rasterizer,
                    cell,
                    rows: HashMap::new(),
                    shown: Vec::new(),
                    folder,
                    count: 0,
                })
            } else {
                Output::Rows(RowPictures::new(rasterizer, cell))
            }
        }
    };
    let mut shown_cells = Vec::new();
    // `CELLIX_PANE_LOG=<file>` notes what came in and what went out, to see why a setup
    // sends more than it should.
    let mut log = std::env::var_os("CELLIX_PANE_LOG").and_then(|path| {
        fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .ok()
    });
    // Pictures cross the connection, so none go until the mod says how big the pane is:
    // a frame at the default size would cross only to be sent again.
    let mut sized = !matches!(output, Output::Rows(_));
    let server = cellix_agent::start(root, app.comments.clone())?;
    app.restore_unsent();
    app.ide = IdeServer::start(root).ok();
    let mut out = BufWriter::new(io::stdout());
    if let Some(ide) = &app.ide {
        writeln!(out, "ide {}", ide.port)?;
    }
    let mut redraw = true;
    let mut waits = 0;
    while !app.quit && parent() == started_by {
        if redraw {
            let (columns, rows) = app.size();
            let mut grid = Grid::new(columns, rows, app.background());
            app.draw(&mut grid);
            let line = match &mut output {
                Output::File(frames) => frames.line(&grid)?,
                Output::Rows(_) if !sized => None,
                Output::Rows(rows) => rows.line(&grid, std::time::Instant::now()),
                Output::Cells => changed(&mut shown_cells, &grid).then(|| cells_line(&grid)),
            };
            if let Some(line) = line {
                if let Some(log) = &mut log {
                    let _ = writeln!(log, "out {}", summary(&line));
                }
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
                if let Some(log) = &mut log {
                    for request in &waiting {
                        if let Request::Pane(text) = request {
                            let _ = writeln!(log, "in {text:.300}");
                        }
                    }
                }
                let (cells, resized) = take_requests(&mut app, waiting);
                sized |= resized;
                // A new font size in the terminal resizes the pane too: rows are drawn at the
                // cell size the terminal has now, or it would scale them and soften the text.
                if let (true, Output::Rows(rows)) = (resized, &mut output)
                    && let Some(cell) = window_cell().filter(|&cell| cell != rows.cell)
                    && let Some(rasterizer) = SoftwareRasterizer::new(cell.0, cell.1)
                {
                    app.set_measure(Rc::new(FontMeasure::new(cell.0, cell.1)));
                    *rows = RowPictures::new(rasterizer, cell);
                }
                if cells {
                    // Pictures can't be shown here after all.
                    app.set_measure(Rc::new(cellix_render::CellMeasure));
                    output = Output::Cells;
                    shown_cells.clear();
                }
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
        // Rows sent soft while scrolling are sent sharp once the pane is still.
        redraw |= matches!(&output, Output::Rows(rows) if rows.soft_on_screen());
    }
    app.finish()
}

/// Hands requests to the app in order, with the pointer's moves between other input cut to
/// the last. Returns whether the mod asked for cells, and whether it said the pane's size.
fn take_requests(app: &mut App, requests: Vec<Request>) -> (bool, bool) {
    let mut input = Vec::new();
    let (mut cells, mut resized) = (false, false);
    for request in requests {
        match request {
            Request::Pane(text) => input.extend(inputs(&text)),
            other => {
                let (c, r) = take_input(app, std::mem::take(&mut input));
                (cells, resized) = (cells | c, resized | r);
                app.serve(other);
            }
        }
    }
    let (c, r) = take_input(app, input);
    (cells | c, resized | r)
}

fn take_input(app: &mut App, input: Vec<Input>) -> (bool, bool) {
    let (mut cells, mut resized) = (false, false);
    for one in latest_moves(input) {
        resized |= matches!(one, Input::Resize { .. });
        if let Input::Tier { tier, why } = &one
            && tier == "cells"
        {
            cells = true;
            let why = why.as_deref().unwrap_or("this terminal doesn't show them");
            // Set up after this Claude Code started, as its session-start hook does: it
            // shows pictures from its next start.
            let tmux_ready = matches!(
                prepare_tmux(|name| std::env::var(name).ok()),
                Tmux::Ready | Tmux::JustSet
            );
            app.say(if tmux_ready && why.contains("tmux") {
                RESTART_FOR_PICTURES.to_string()
            } else {
                format!("Pictures can't show here ({why}), so the pane shows text")
            });
        }
        if let Input::Open { path } = &one {
            let _ = app.open(path);
        }
        for event in translate(one) {
            app.handle(event);
        }
    }
    (cells, resized)
}

/// A line for the log: its kind and size, and for rows, which ones.
fn summary(line: &str) -> String {
    let mut parts = line.split(' ');
    let head: Vec<&str> = parts.by_ref().take(3).collect();
    if head.first() == Some(&"rows") {
        let rows: Vec<&str> = parts
            .filter_map(|p| p.split_once(':').map(|(y, _)| y))
            .collect();
        format!(
            "{} [{}] {} bytes",
            head.join(" "),
            rows.join(","),
            line.len()
        )
    } else {
        format!("{} {} bytes", head.join(" "), line.len())
    }
}

/// How frames leave the pane.
enum Output {
    /// A file the terminal reads, on this machine.
    File(Frames<SoftwareRasterizer>),
    /// Pictures by value, a row each.
    Rows(RowPictures<SoftwareRasterizer>),
    /// Text, where pictures can't show.
    Cells,
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

    /// Claude Code shows a picture from a file only in kitty and Ghostty on the same machine.
    /// Elsewhere it showed nothing, as in tmux, so the pane goes as cells there.
    #[test]
    fn the_pane_is_a_picture_only_where_claude_code_can_show_one() {
        let shows = |pairs: &[(&str, &str)]| {
            let pairs: Vec<(String, String)> = pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect();
            pictures_show(move |name| {
                pairs
                    .iter()
                    .find(|(k, _)| k == name)
                    .map(|(_, v)| v.clone())
            })
        };
        assert!(shows(&[("TERM_PROGRAM", "ghostty")]));
        assert!(shows(&[("TERM", "xterm-kitty")]));
        assert!(!shows(&[
            ("TERM_PROGRAM", "ghostty"),
            ("TMUX", "/tmp/tmux-1/default,1,0")
        ]));
        assert!(!shows(&[
            ("TERM_PROGRAM", "ghostty"),
            ("SSH_CONNECTION", "1 2 3 4")
        ]));
        assert!(!shows(&[("TERM", "xterm-kitty"), ("ZELLIJ", "0")]));
        assert!(!shows(&[("TERM_PROGRAM", "iTerm.app")]));
        assert!(
            !shows(&[("TERM", "xterm-256color")]),
            "a plain Linux terminal, or SSM"
        );
        assert!(!shows(&[]));
        assert!(!shows(&[
            ("TERM_PROGRAM", "ghostty"),
            ("CELLIX_TIER", "cells")
        ]));
        assert!(shows(&[("TERM", "screen"), ("CELLIX_TIER", "pixels")]));
    }

    /// A fake that draws each cell as a 2 by 2 block, so a row can be halved.
    struct Blocks;

    impl Rasterizer for Blocks {
        fn row(&mut self, grid: &Grid, y: u16) -> cellix_render::Image {
            let width = grid.width() as usize * 2;
            let shade = grid.row_text(y).bytes().fold(0u8, |a, b| a.wrapping_add(b));
            cellix_render::Image {
                width,
                height: 2,
                rgb: vec![shade; width * 2 * 3],
            }
        }
    }

    /// A line's row names, and the pictures it carries with their sizes.
    fn sent(line: &str) -> (Vec<String>, Vec<(String, u32, u32)>) {
        let mut parts = line.split(' ');
        assert_eq!(parts.next(), Some("rows"));
        parts.nth(1);
        let names = parts
            .next()
            .unwrap()
            .split(',')
            .map(str::to_string)
            .collect();
        let pictures = parts
            .map(|part| {
                let (name, text) = part.split_once(':').unwrap();
                let png = base64::engine::general_purpose::STANDARD
                    .decode(text)
                    .unwrap();
                let size = |at: usize| u32::from_be_bytes(png[at..at + 4].try_into().unwrap());
                (name.to_string(), size(16), size(20))
            })
            .collect();
        (names, pictures)
    }

    /// A grid whose rows read `first`, `first + 1`, and on: a page of a scrolled file.
    fn page(width: u16, height: u16, first: usize) -> Grid {
        let style = cellix_render::Style::default();
        let mut grid = Grid::new(width, height, style);
        for y in 0..height {
            grid.put_str(0, y, &format!("{}", first + y as usize), style, width);
        }
        grid
    }

    /// Over SSH and in tmux the pane goes as a picture a row, named by what it shows, and
    /// only pictures the mod lacks cross: a click costs a row, where the whole pane used to.
    #[test]
    fn only_the_rows_that_changed_go_as_pictures() {
        let mut rows = RowPictures::new(Blocks, (2, 2));
        let now = std::time::Instant::now();
        let mut grid = page(4, 3, 0);
        let (names, pictures) = sent(&rows.line(&grid, now).unwrap());
        assert_eq!(names.len(), 3);
        assert_eq!(
            pictures.iter().map(|p| (p.1, p.2)).collect::<Vec<_>>(),
            [(8, 2); 3]
        );
        assert!(
            rows.line(&grid, now).is_none(),
            "an idle pane sends nothing"
        );
        grid.put_str(1, 1, "x", cellix_render::Style::default(), 4);
        let (later, pictures) = sent(&rows.line(&grid, now).unwrap());
        assert_eq!(pictures.len(), 1, "one row changed");
        assert_eq!(
            (&later[0], &later[2]),
            (&names[0], &names[2]),
            "the rest keep their names"
        );
    }

    /// Scrolling moves rows. A moved row keeps its name, so only the lines that came into
    /// view cross: Claude Code moves an uploaded picture with its name.
    #[test]
    fn a_scroll_sends_only_the_lines_that_came_into_view() {
        let mut rows = RowPictures::new(Blocks, (2, 2));
        let now = std::time::Instant::now();
        let (first, _) = sent(&rows.line(&page(4, 12, 0), now).unwrap());
        let (names, pictures) = sent(&rows.line(&page(4, 12, 3), now).unwrap());
        assert_eq!(names[..9], first[3..], "moved up, by name");
        assert_eq!(pictures.len(), 3, "three new lines");
        assert!(pictures.iter().all(|p| (p.1, p.2) == (8, 2)), "and sharp");
    }

    /// Pages that are all new, one after another, go at half resolution, about a quarter of
    /// the bytes, and sharp once the pane is still.
    #[test]
    fn whole_new_pages_in_a_burst_go_soft_and_sharpen_when_still() {
        let mut rows = RowPictures::new(Blocks, (2, 2));
        let start = std::time::Instant::now();
        let at = |ms: u64| start + std::time::Duration::from_millis(ms);
        let (_, first) = sent(&rows.line(&page(4, 12, 0), at(0)).unwrap());
        assert!(first.iter().all(|p| p.1 == 8));
        let (names, soft) = sent(&rows.line(&page(4, 12, 100), at(50)).unwrap());
        assert_eq!(soft.len(), 12);
        assert!(
            soft.iter().all(|p| (p.1, p.2) == (4, 1)),
            "half size in a burst: {soft:?}"
        );
        assert!(names.iter().all(|name| name.ends_with('s')));
        assert!(
            rows.line(&page(4, 12, 100), at(100)).is_none(),
            "not yet still"
        );
        let (names, sharp) = sent(&rows.line(&page(4, 12, 100), at(400)).unwrap());
        assert_eq!(sharp.len(), 12);
        assert!(
            sharp.iter().all(|p| (p.1, p.2) == (8, 2)),
            "sharp once still"
        );
        assert!(names.iter().all(|name| !name.ends_with('s')));
        assert!(rows.line(&page(4, 12, 100), at(800)).is_none());
    }

    /// A new font size in the terminal is a new cell size: the same rows get new names, so
    /// the mod doesn't keep the pictures drawn at the old size.
    #[test]
    fn rows_at_another_cell_size_are_new_pictures() {
        let now = std::time::Instant::now();
        let (small, _) = sent(
            &RowPictures::new(Blocks, (2, 2))
                .line(&page(4, 3, 0), now)
                .unwrap(),
        );
        let (large, _) = sent(
            &RowPictures::new(Blocks, (3, 4))
                .line(&page(4, 3, 0), now)
                .unwrap(),
        );
        assert!(small.iter().all(|name| !large.contains(name)));
    }

    /// Claude Code changes the pane's height as its prompt grows. Rows that still hold the
    /// same aren't sent again.
    #[test]
    fn a_new_height_sends_only_what_changed() {
        let mut rows = RowPictures::new(Blocks, (2, 2));
        let now = std::time::Instant::now();
        rows.line(&page(4, 6, 0), now).unwrap();
        let mut shorter = page(4, 4, 0);
        shorter.put_str(0, 3, "status", cellix_render::Style::default(), 4);
        let line = rows.line(&shorter, now).unwrap();
        assert!(line.starts_with("rows 4 4 "), "{line:.20}");
        assert_eq!(sent(&line).1.len(), 1, "only the status row is new");
        assert_eq!(
            sent(&rows.line(&page(5, 4, 0), now).unwrap()).1.len(),
            4,
            "a new width sends all"
        );
    }

    /// A row goes as a palette PNG when it fits in 256 colours, losslessly, and as RGB when
    /// it doesn't; either decodes to the same pixels.
    #[test]
    fn rows_go_as_small_as_they_losslessly_can() {
        let decode = |bytes: &[u8]| {
            let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes.to_vec()));
            decoder.set_transformations(png::Transformations::EXPAND);
            let mut reader = decoder.read_info().unwrap();
            let mut out = vec![0; reader.output_buffer_size().unwrap()];
            let info = reader.next_frame(&mut out).unwrap();
            (info.color_type, out[..info.buffer_size()].to_vec())
        };
        let (width, height) = (64, 4);
        let few: Vec<u8> = (0..width * height)
            .flat_map(|i| [(i % 7) as u8 * 30, 40, 50])
            .collect();
        let png = encode_png(&few, width, height);
        assert_eq!(png[25], 3, "indexed");
        assert_eq!(decode(&png).1, few);
        // 512 colours.
        let many: Vec<u8> = (0..width * 2 * height)
            .flat_map(|i| [i as u8, (i / 256) as u8, 7])
            .collect();
        let png = encode_png(&many, width * 2, height);
        assert_eq!(png[25], 2, "RGB");
        assert_eq!(decode(&png), (png::ColorType::Rgb, many));
    }

    /// In tmux, Cellix turns on what Claude Code needs for pictures, server wide, since
    /// Claude Code reads it as it starts. Runs against a real tmux server where installed.
    #[test]
    fn tmux_is_set_up_for_claude_codes_pictures() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("tmux");
        let tmux = |args: &[&str]| {
            std::process::Command::new("tmux")
                .arg("-S")
                .arg(&socket)
                .args(["-f", "/dev/null"])
                .args(args)
                .output()
        };
        let Ok(started) = tmux(&["new-session", "-d", "-s", "t", "sleep 30"]) else {
            eprintln!("skipped: tmux isn't installed");
            return;
        };
        assert!(started.status.success());
        let pane = tmux(&["display-message", "-p", "-t", "t", "#{pane_id}"]).unwrap();
        let pane = String::from_utf8(pane.stdout).unwrap().trim().to_string();
        let value = format!("{},1,0", socket.display());
        let var = |name: &str| match name {
            "TMUX" => Some(value.clone()),
            "TMUX_PANE" => Some(pane.clone()),
            _ => None,
        };
        assert_eq!(prepare_tmux(var), Tmux::JustSet);
        for option in TMUX_OPTIONS {
            let value = tmux(&["show-options", "-g", "-v", option]).unwrap();
            assert_eq!(
                String::from_utf8_lossy(&value.stdout).trim(),
                "on",
                "{option}"
            );
        }
        assert_eq!(
            prepare_tmux(var),
            Tmux::Ready,
            "the next Claude Code finds it ready"
        );
        let _ = tmux(&["kill-server"]);
        assert_eq!(prepare_tmux(|_| None), Tmux::Absent);
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
