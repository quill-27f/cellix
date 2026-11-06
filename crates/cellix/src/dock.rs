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

/// Columns the file beside the pane always keeps.
const MIN_MAIN: u16 = 44;
const MIN_WIDTH: u16 = 40;
/// Share of the room right of the sidebar, in percent.
const SHARE: u16 = 46;
/// Lines a wheel notch moves through scrollback.
const WHEEL_LINES: i32 = 3;
const TAB_PADDING: u16 = 2;
const CLOSE_WIDTH: u16 = 2;
/// The Comments tab at the far end, and what a count adds to it.
const COMMENTS_WIDTH: u16 = 10;
const COUNT_WIDTH: u16 = 2;
/// The button that starts something, and the tab it becomes while that list shows.
const NEW_WIDTH: u16 = 3;
/// The button after it that maximizes the pane.
const MAXIMIZE_WIDTH: u16 = 3;
/// The column between it and the +, so the row keeps one step between its tabs and buttons.
const MAXIMIZE_GAP: u16 = 1;
/// Columns it keeps clear of the icons at the top right, so it isn't read as one of them.
const MAXIMIZE_CLEAR: u16 = 2;
const AGENTS_WIDTH: u16 = 11;
const PREVIEW_WIDTH: u16 = 12;
const LOG_WIDTH: u16 = 8;
const PROBLEMS_WIDTH: u16 = 13;
/// Where a tab's name starts, as on the file's tabs.
const TAB_TEXT: f32 = 1.5;
/// How narrow a session's tab can get, sharing the row with others.
const SMALLEST_TAB: u16 = 6;
/// The button that closes the pane.
const HIDE_WIDTH: u16 = 3;
/// Its hover chip, the size and roundness of the one the icons at the top right have.
const HIDE_PILL: (f32, f32) = (1.95, 0.89);
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
    size: (u16, u16),
    cell: CellPixels,
    selection: Option<(Spot, Spot)>,
    scanner: Scanner,
    protocol: Protocol,
    pictures: Vec<Picture>,
    /// What it was, when it stopped without finishing, so it can be resumed.
    pub stopped: Option<crate::hold::Stopped>,
}

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
        let writer = pair.master.take_writer().map_err(failed)?;
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
            wrote: None,
            size,
            cell,
            selection: None,
            scanner: Scanner::default(),
            protocol: Protocol::default(),
            pictures: Vec::new(),
            stopped: None,
        }
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
    pub fn agent(&self) -> bool {
        !self.exited && !self.shell && !self.browser
    }

    /// Takes in what the program wrote, with its pictures lifted out in order, so a reply
    /// goes back ahead of anything the terminal says about what came after.
    fn take_in(&mut self, bytes: &[u8]) {
        for piece in self.scanner.feed(bytes) {
            match piece {
                Piece::Text(text) => {
                    self.answer_about_cells(&text);
                    self.forget_on_clear(&text);
                    self.parser.advance(&mut self.term, &text);
                }
                Piece::Graphics(command) => {
                    // Text inside a synchronized update waits in the parser, cursor moves
                    // included, and a picture goes wherever the cursor is.
                    self.parser.stop_sync(&mut self.term);
                    let outcome = self.protocol.command(&command);
                    self.follow(outcome);
                }
            }
        }
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
        let at_prompt = match &self.link {
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
        };
        !self.shell || !at_prompt
    }

    pub fn write(&mut self, bytes: &[u8]) {
        let _ = self.writer.write_all(bytes);
        let _ = self.writer.flush();
    }

    /// Takes in what the program wrote. Returns true if the screen changed.
    pub fn poll(&mut self, theme: &Theme) -> bool {
        let mut changed = false;
        while let Ok(bytes) = self.output.try_recv() {
            self.take_in(&bytes);
            self.wrote = Some(std::time::Instant::now());
            changed = true;
        }
        let events = self.events.0.lock().map(|mut e| std::mem::take(&mut *e));
        for event in events.unwrap_or_default() {
            match event {
                TermEvent::PtyWrite(text) => self.write(text.as_bytes()),
                TermEvent::ColorRequest(index, format) => {
                    let color = asked_colour(index, theme);
                    let reply = format(TermRgb {
                        r: color.0,
                        g: color.1,
                        b: color.2,
                    });
                    self.write(reply.as_bytes());
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
        changed
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
        let application = self.term.mode().contains(TermMode::APP_CURSOR);
        let bytes = key_bytes(key, application);
        self.term.scroll_display(Scroll::Bottom);
        self.selection = None;
        self.write(&bytes);
    }

    pub fn paste(&mut self, text: &str) {
        self.term.scroll_display(Scroll::Bottom);
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
        self.paste(text);
        self.write(b"\r");
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

    /// The `file:line` or file under a column of a visible row, as tracebacks and compilers
    /// print them.
    pub fn path_at(&self, (x, y): Spot) -> Option<(String, Option<usize>)> {
        let rows = self.rows();
        let chars: Vec<char> = rows.get(y as usize)?.chars().collect();
        let x = x as usize;
        let edge = |c: &char| c.is_whitespace() || "\"'`()[]<>,".contains(*c);
        if chars.get(x).is_none_or(edge) {
            return None;
        }
        let start = chars[..x].iter().rposition(edge).map_or(0, |i| i + 1);
        let end = chars[x..]
            .iter()
            .position(edge)
            .map_or(chars.len(), |i| x + i);
        let word: String = chars[start..end].iter().collect();
        let word = word.trim_end_matches(':');
        let mut parts = word.split(':');
        let file = parts.next()?.to_string();
        Some((file, parts.next().and_then(|n| n.parse().ok())))
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
        grid.fill(area, theme.text);
        let selection = self.ordered().filter(|(a, b)| a != b);
        let picked = |x: u16, y: u16| {
            selection.is_some_and(|(from, to)| (y, x) >= (from.1, from.0) && (y, x) <= (to.1, to.0))
        };
        let content = self.term.renderable_content();
        let offset = content.display_offset as i32;
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
            let mut bg = self.color(cell.bg, theme, theme.text.bg);
            if cell.flags.contains(Flags::INVERSE) {
                std::mem::swap(&mut fg, &mut bg);
            }
            if cell.flags.contains(Flags::DIM) {
                fg = theme.gutter.fg;
            }
            if picked(x, y) {
                bg = theme.selection;
            }
            let at_cursor = show_cursor
                && point.line == cursor.point.line
                && point.column == cursor.point.column;
            if at_cursor {
                (fg, bg) = (theme.text.bg, theme.text.fg);
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
    pub fn draw_ended(&self, grid: &mut Grid, area: Rect, theme: &Theme) {
        let row = Rect::new(area.x, area.bottom().saturating_sub(1), area.w, 1);
        let note = Style {
            fg: theme.gutter.fg,
            ..theme.tab
        };
        grid.fill(row, theme.tab);
        let text = format!(
            "{} has ended. Close this tab, or press + for another.",
            self.name
        );
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

/// The colour a program asks for. Programs read the background to choose their own look, so
/// these are the sheet's, and a palette entry is what the pane really draws for that number.
fn asked_colour(index: usize, theme: &Theme) -> Rgb {
    match index {
        _ if index == NamedColor::Background as usize => theme.text.bg,
        _ if index == NamedColor::Foreground as usize => theme.text.fg,
        _ if index == NamedColor::Cursor as usize => theme.text.fg,
        _ => indexed(index.min(255) as u8, palette_of(theme)),
    }
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
    /// How far the comments have scrolled.
    pub comments_top: usize,
    /// How many comments are open, which the tab shows.
    pub comment_count: Cell<usize>,
    /// A comment or reply being written.
    pub draft: Option<Draft>,
    /// What the last build said, how far it has scrolled, and where its paths start.
    pub log: Vec<String>,
    pub log_top: usize,
    /// Whether the problems have a tab, and how far they've scrolled.
    pub problems: bool,
    pub problems_top: usize,
    pub log_folder: std::path::PathBuf,
    /// Whether the list of agents has a tab.
    pub agents_tab: bool,
    /// Whether the comments have a tab.
    pub comments_tab: bool,
    /// The column tabs stop at, for what stands past them.
    pub tabs_end: Cell<u16>,
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
}

/// The views' tabs in the order they come in until one is dragged.
const VIEW_ORDER: [Front; 5] = [
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
        let wanted = self.width.unwrap_or(room * SHARE / 100);
        wanted.clamp(MIN_WIDTH, room - MIN_MAIN)
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
    pub fn busy(&self) -> Vec<String> {
        let busy = self.sessions.iter().filter(|s| s.busy());
        busy.map(|s| s.name.clone()).collect()
    }

    pub fn poll(&mut self, theme: &Theme) -> bool {
        let mut changed = self.pdf.as_mut().is_some_and(PdfView::poll);
        for session in &mut self.sessions {
            changed |= session.poll(theme);
        }
        // A shell that's finished has nothing left to show. An agent may have said why, and
        // one that stopped with the machine waits to be resumed.
        let done = |s: &Session| s.exited && s.stopped.is_none() && (s.shell || s.browser);
        while let Some(index) = self.sessions.iter().position(done) {
            self.close(index);
            changed = true;
        }
        changed
    }

    /// Stops everything here, even what could have outlived Cellix.
    pub fn stop_all(&mut self) {
        for session in &mut self.sessions {
            session.stop();
        }
    }

    pub fn close(&mut self, index: usize) {
        if index >= self.sessions.len() {
            return;
        }
        self.sessions.remove(index).stop();
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
        let count = self.comment_count.get();
        let digits = if count > 9 { COUNT_WIDTH } else { 1 };
        COMMENTS_WIDTH + CLOSE_WIDTH + 1 + if count > 0 { digits + 1 } else { 0 }
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
        if self.comments_tab || self.front == Front::Comments {
            views.push((Front::Comments, "Comments", self.comments_width()));
        }
        if self.agents_tab || self.listing() {
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

    /// How wide a session's tab would like to be.
    fn session_width(session: &Session) -> u16 {
        session.name.chars().count() as u16 + TAB_PADDING * 2 + CLOSE_WIDTH
    }

    fn tabs_start(&self, area: Rect) -> u16 {
        area.x + u16::from(!self.tabs_flush.get())
    }

    /// How wide each tab would like to be, views and then sessions.
    fn wanted_widths(&self) -> Vec<u16> {
        let views = self.views().into_iter().map(|view| view.2);
        views
            .chain(self.sessions.iter().map(Self::session_width))
            .collect()
    }

    /// What each tab gets, views and then sessions. Out of room, the one in front keeps its
    /// whole name and the rest share what's left, so none falls off the end.
    fn tab_widths(&self, area: Rect) -> Vec<u16> {
        let mut widths = self.wanted_widths();
        let room = self
            .tabs_end
            .get()
            .saturating_sub(self.tabs_start(area) + NEW_WIDTH + MAXIMIZE_GAP + MAXIMIZE_WIDTH);
        if self.tabs_end.get() == 0 || widths.iter().sum::<u16>() <= room {
            return widths;
        }
        let views = self.views();
        let shown = |view: &(Front, &str, u16)| {
            self.front == view.0 || (view.0 == Front::Agents && self.listing())
        };
        let front = views.iter().position(shown).or_else(|| {
            let session = self.active.filter(|_| self.front == Front::Session)?;
            Some(views.len() + session)
        });
        let kept = front.map_or(0, |index| widths[index]);
        let others = (widths.len() - usize::from(front.is_some())).max(1) as u16;
        let each = (room.saturating_sub(kept) / others).max(SMALLEST_TAB);
        for (index, width) in widths.iter_mut().enumerate() {
            if Some(index) != front {
                *width = (*width).min(each);
            }
        }
        widths
    }

    /// The full name of a tab too narrow to show it, for a tooltip.
    pub fn cut_name(&self, area: Rect, x: u16) -> Option<String> {
        let wanted = self.wanted_widths();
        let widths = self.tab_widths(area);
        let views = self.views();
        let names = views
            .iter()
            .map(|view| view.1)
            .chain(self.sessions.iter().map(|s| s.name.as_str()));
        let mut at = self.tabs_start(area);
        for ((name, width), full) in names.zip(widths).zip(wanted) {
            if x >= at && x < at + width {
                return (width < full).then(|| name.to_string());
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

    /// Where the button that maximizes the pane starts.
    pub fn maximize_button(&self, area: Rect) -> u16 {
        self.tabs(area).last().map_or(self.first_tab(area), |t| t.1) + NEW_WIDTH + MAXIMIZE_GAP
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
            Front::Session => {}
        }
        if showing {
            self.next_tab();
        }
    }

    /// The log line at a row of the screen.
    pub fn log_line(&self, screen: Rect, y: u16) -> Option<String> {
        let row = y.checked_sub(screen.y)? as usize;
        self.log.get(self.log_top + row).cloned()
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
        let lines = self.log.iter().skip(self.log_top).take(screen.h as usize);
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
        for (view, _, tab) in self.view_tabs(area) {
            if x >= tab.x && x < tab.right() {
                let on_close = x + CLOSE_WIDTH + 1 >= tab.right();
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
                let on_close = x >= end.saturating_sub(CLOSE_WIDTH + 1);
                return Some(if on_close {
                    TabHit::Close(index)
                } else {
                    TabHit::Tab(index)
                });
            }
        }
        let end = tabs.last().map_or(self.first_tab(area), |t| t.1);
        if x >= end && x < end + NEW_WIDTH {
            return Some(TabHit::New);
        }
        let maximize = end + NEW_WIDTH + MAXIMIZE_GAP;
        let on_it = x >= maximize && x < maximize + MAXIMIZE_WIDTH;
        (on_it && self.maximize_fits(end)).then_some(TabHit::Maximize)
    }

    /// Whether the maximize button fits before the icons at the top right, after a + that
    /// ends at `plus`. In a narrow pane it goes: a double click on a tab does the same.
    fn maximize_fits(&self, plus: u16) -> bool {
        let end = self.tabs_end.get();
        end == 0 || plus + NEW_WIDTH + MAXIMIZE_GAP + MAXIMIZE_WIDTH + MAXIMIZE_CLEAR <= end
    }

    /// The title row, in the frame's colour, with tabs like the editor's.
    pub fn draw_tabs(
        &self,
        grid: &mut Grid,
        area: Rect,
        (hover, maximized): (Option<(u16, u16)>, bool),
        theme: &Theme,
    ) {
        let count = self.comment_count.get();
        let row = Rect::new(area.x, area.y, area.w, 1);
        grid.fill(row, theme.tab);
        for (view, name, tab) in self.view_tabs(area) {
            let on = self.front == view || (view == Front::Agents && self.listing());
            let hovered = hover.is_some_and(|h| tab.contains(h.0, h.1));
            if on {
                grid.fill(tab, theme.sidebar);
            } else if hovered {
                let bounds = Bounds::inset(tab, 0.1, 0.1);
                grid.panel(bounds, 16, Some(theme.row_hover), None);
            }
            let quiet = Style {
                fg: if on { theme.text.fg } else { theme.gutter.fg },
                ..theme.tab
            };
            let close = tab.right().saturating_sub(CLOSE_WIDTH + 1);
            let text = if view == Front::Comments && count > 0 {
                format!("{name} {count}")
            } else {
                name.to_string()
            };
            let at = tab.x as f32 + TAB_TEXT;
            grid.label_at(at, tab.x + 2, area.y, &text, quiet, close, 100);
            if on || hovered {
                let plain = Style {
                    bold: false,
                    ..quiet
                };
                grid.glyph(close as f32, area.y, Icon::Close, plain.fg, "×", plain);
            }
        }
        for (index, (start, end)) in self.tabs(area).into_iter().enumerate() {
            let session = &self.sessions[index];
            let active = self.active == Some(index) && self.front == Front::Session;
            let tab = Rect::new(start, area.y, end - start, 1);
            let hovered = hover.is_some_and(|h| tab.contains(h.0, h.1));
            let style = if active {
                theme.text
            } else {
                Style {
                    fg: theme.gutter.fg,
                    ..theme.tab
                }
            };
            if active {
                grid.fill(tab, theme.text);
            } else if hovered {
                grid.panel(
                    Bounds::inset(tab, 0.1, 0.1),
                    16,
                    Some(theme.row_hover),
                    None,
                );
            }
            let shown = if session.exited {
                Style {
                    fg: theme.gutter.fg,
                    ..style
                }
            } else {
                style
            };
            let close = end.saturating_sub(CLOSE_WIDTH + 1);
            let at = start as f32 + TAB_TEXT;
            grid.label_at(
                at,
                start + TAB_PADDING,
                area.y,
                &session.name,
                shown,
                close,
                100,
            );
            if hovered || active {
                let quiet = Style {
                    fg: theme.gutter.fg,
                    ..style
                };
                grid.glyph(close as f32, area.y, Icon::Close, quiet.fg, "×", quiet);
            }
        }
        let plus = self.tabs(area).last().map_or(self.first_tab(area), |t| t.1);
        let button = Rect::new(plus, area.y, NEW_WIDTH, 1);
        let on_it = hover.is_some_and(|h| button.contains(h.0, h.1));
        if on_it {
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
        grid.label(plus + 1, area.y, "+", style, area.right());
        // Maximized, it stands on the chip a selected icon has, so it's plain how to get back.
        let button = Rect::new(plus + NEW_WIDTH + MAXIMIZE_GAP, area.y, MAXIMIZE_WIDTH, 1);
        if maximized || self.maximize_fits(plus) {
            let on_it = hover.is_some_and(|h| button.contains(h.0, h.1));
            if maximized || on_it {
                let fill = if maximized {
                    theme.tint
                } else {
                    theme.row_hover
                };
                // Two cells around the icon: about square, as the rail's and the strip's chips are.
                let chip = Bounds::cells(button.x as f32 + 0.5, area.y as f32 + 0.12, 2.0, 0.76);
                grid.panel(chip, 22, Some(fill), None);
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
            crate::toolbar::draw_icon(grid, (icon, 0.62), button, fg, theme.tab);
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
        let middle = hide.x as f32 + hide.w as f32 / 2.0;
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
        // The same × as a tab's, so closing looks one way across the row.
        let style = Style {
            fg,
            painted: true,
            ..theme.tab
        };
        grid.put_str(hide.x + 1, area.y, "×", style, hide.right());
        grid.paint.push(cellix_render::Paint::Icon {
            bounds: Bounds::cells(middle - 1.0, area.y as f32 + 0.14, 2.0, 0.72),
            icon: Icon::Close,
            color: fg,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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

    #[test]
    fn a_program_is_told_the_real_background() {
        let theme = Theme::named(Some("light"));
        let background = NamedColor::Background as usize;
        assert_eq!(asked_colour(background, &theme), theme.text.bg);
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

    #[test]
    fn the_file_beside_the_pane_keeps_its_room() {
        let mut dock = Dock {
            open: true,
            ..Dock::default()
        };
        assert_eq!(dock.width_in(120), 55);
        dock.width = Some(200);
        assert_eq!(dock.width_in(120), 120 - MIN_MAIN);
        dock.width = Some(1);
        assert_eq!(dock.width_in(120), MIN_WIDTH);
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
