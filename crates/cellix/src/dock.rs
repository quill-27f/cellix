//! The side pane. Agents and shells run here in a real terminal, beside the file, so working
//! with one doesn't mean leaving the other. The program is the real one, on a real PTY, and
//! it updates itself: nothing here knows what Claude Code looks like.

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
use cellix_render::{Bounds, Grid, Rect, Rgb, Style};
use portable_pty::{ChildKiller, CommandBuilder, MasterPty, PtySize, native_pty_system};
use unicode_width::UnicodeWidthStr;

use crate::pdfview::PdfView;
use crate::theme::Theme;

/// Columns the file beside the pane always keeps.
const MIN_MAIN: u16 = 44;
const MIN_WIDTH: u16 = 40;
/// Share of the room right of the sidebar, in percent.
const SHARE: u16 = 46;
/// Lines a wheel notch moves through scrollback.
const WHEEL_LINES: i32 = 3;
const TAB_PADDING: u16 = 2;
const CLOSE_WIDTH: u16 = 2;
/// The Comments tab at the far end.
const COMMENTS_WIDTH: u16 = 13;
const PDF_WIDTH: u16 = 5;

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

/// A place in the visible text, as (column, row).
type Spot = (u16, u16);

/// A program running on a PTY, and the screen it has drawn.
pub struct Session {
    pub name: String,
    /// A plain shell, which runs what it's sent. Comments must never reach one.
    pub shell: bool,
    term: Term<Events>,
    parser: Processor,
    events: Events,
    writer: Box<dyn Write + Send>,
    master: Box<dyn MasterPty + Send>,
    killer: Box<dyn ChildKiller + Send + Sync>,
    pid: Option<u32>,
    output: Receiver<Vec<u8>>,
    finished: Receiver<()>,
    pub exited: bool,
    size: (u16, u16),
    selection: Option<(Spot, Spot)>,
}

fn pty_size((columns, rows): (u16, u16)) -> PtySize {
    PtySize {
        rows,
        cols: columns,
        pixel_width: 0,
        pixel_height: 0,
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
    ) -> io::Result<Self> {
        let pair = native_pty_system()
            .openpty(pty_size(size))
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
        let events = Events::default();
        let dimensions = Size {
            columns: size.0 as usize,
            lines: size.1 as usize,
        };
        Ok(Self {
            shell: name == crate::agents::SHELL.0,
            name: name.to_string(),
            term: Term::new(Config::default(), &dimensions, events.clone()),
            parser: Processor::new(),
            events,
            writer,
            master: pair.master,
            killer,
            pid,
            output,
            finished,
            exited: false,
            size,
            selection: None,
        })
    }

    /// Whether closing this would stop something: an agent, or a shell running a command
    /// rather than waiting at its prompt.
    pub fn busy(&self) -> bool {
        if self.exited {
            return false;
        }
        let front = self.master.process_group_leader();
        !self.shell || front.is_none() || front.map(|p| p as u32) != self.pid
    }

    pub fn write(&mut self, bytes: &[u8]) {
        let _ = self.writer.write_all(bytes);
        let _ = self.writer.flush();
    }

    /// Takes in what the program wrote. Returns true if the screen changed.
    pub fn poll(&mut self, theme: &Theme) -> bool {
        let mut changed = false;
        while let Ok(bytes) = self.output.try_recv() {
            self.parser.advance(&mut self.term, &bytes);
            changed = true;
        }
        let events = self.events.0.lock().map(|mut e| std::mem::take(&mut *e));
        for event in events.unwrap_or_default() {
            match event {
                TermEvent::PtyWrite(text) => self.write(text.as_bytes()),
                TermEvent::ColorRequest(index, format) => {
                    let color = if index == 11 {
                        theme.text.bg
                    } else {
                        theme.text.fg
                    };
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
                        cell_width: 1,
                        cell_height: 1,
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

    pub fn resize(&mut self, size: (u16, u16)) {
        if size == self.size || size.0 == 0 || size.1 == 0 {
            return;
        }
        self.size = size;
        self.selection = None;
        let _ = self.master.resize(pty_size(size));
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
        let palette = if theme.is_light() { &LIGHT } else { &DARK };
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
        let _ = self.killer.kill();
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.stop();
    }
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

/// What a click in the pane's title row lands on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TabHit {
    Tab(usize),
    Close(usize),
    New,
    Comments,
    Pdf,
}

/// What the pane shows besides its sessions.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Front {
    #[default]
    Session,
    Comments,
    Pdf,
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
    /// The PDF being worked on.
    pub pdf: Option<PdfView>,
    /// How far the comments have scrolled.
    pub comments_top: usize,
}

impl Dock {
    /// How wide the pane is in a window `room` columns wide past the sidebar.
    pub fn width_in(&self, room: u16) -> u16 {
        if !self.open || room < MIN_MAIN + MIN_WIDTH {
            return if self.open { room } else { 0 };
        }
        let wanted = self.width.unwrap_or(room * SHARE / 100);
        wanted.clamp(MIN_WIDTH, room - MIN_MAIN)
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
        let usable = |s: &Session| !s.exited && !s.shell;
        let front = self.active.filter(|i| usable(&self.sessions[*i]));
        let index = front.or_else(|| self.sessions.iter().position(usable))?;
        self.sessions.get_mut(index)
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
        // A shell that's finished has nothing left to show. An agent may have said why.
        while let Some(index) = self.sessions.iter().position(|s| s.exited && s.shell) {
            self.close(index);
            changed = true;
        }
        changed
    }

    pub fn close(&mut self, index: usize) {
        if index >= self.sessions.len() {
            return;
        }
        self.sessions.remove(index);
        self.active = match self.active {
            _ if self.sessions.is_empty() => None,
            Some(active) if active >= index => Some(active.saturating_sub(1)),
            other => other,
        };
        self.focused &= self.active.is_some();
        self.front = match (self.front, self.active) {
            (Front::Session, None) => Front::Session,
            (front, _) => front,
        };
    }

    /// Each tab as (start, end) columns.
    fn tabs(&self, area: Rect) -> Vec<(u16, u16)> {
        let mut x = area.x + 1;
        self.sessions
            .iter()
            .map(|s| {
                let width = s.name.chars().count() as u16 + TAB_PADDING * 2 + CLOSE_WIDTH;
                let tab = (x, x + width);
                x += width;
                tab
            })
            .collect()
    }

    pub fn hit(&self, area: Rect, x: u16) -> Option<TabHit> {
        if x + COMMENTS_WIDTH >= area.right() {
            return Some(TabHit::Comments);
        }
        if self.pdf.is_some() && x + COMMENTS_WIDTH + PDF_WIDTH >= area.right() {
            return Some(TabHit::Pdf);
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
        let end = tabs.last().map_or(area.x + 1, |t| t.1);
        (x >= end && x < end + 3).then_some(TabHit::New)
    }

    /// The title row, in the frame's colour, with tabs like the editor's.
    pub fn draw_tabs(
        &self,
        grid: &mut Grid,
        area: Rect,
        (hover, count): (Option<(u16, u16)>, usize),
        theme: &Theme,
    ) {
        let row = Rect::new(area.x, area.y, area.w, 1);
        grid.fill(row, theme.tab);
        let comments = Rect::new(
            area.right().saturating_sub(COMMENTS_WIDTH),
            area.y,
            COMMENTS_WIDTH,
            1,
        );
        if self.pdf.is_some() {
            let tab = Rect::new(comments.x.saturating_sub(PDF_WIDTH), area.y, PDF_WIDTH, 1);
            let on = self.front == Front::Pdf;
            if on {
                grid.fill(tab, theme.sidebar);
            } else if hover.is_some_and(|h| tab.contains(h.0, h.1)) {
                let bounds = Bounds::inset(tab, 0.1, 0.1);
                grid.panel(bounds, 16, Some(theme.row_hover), None);
            }
            let quiet = Style {
                fg: if on { theme.text.fg } else { theme.gutter.fg },
                bold: on,
                ..theme.tab
            };
            grid.label(tab.x + 1, area.y, "PDF", quiet, comments.x);
        }
        let on_comments = self.front == Front::Comments;
        if on_comments {
            grid.fill(comments, theme.sidebar);
        } else if hover.is_some_and(|h| comments.contains(h.0, h.1)) {
            grid.panel(
                Bounds::inset(comments, 0.1, 0.1),
                16,
                Some(theme.row_hover),
                None,
            );
        }
        let name = if count > 0 {
            format!("Comments {count}")
        } else {
            "Comments".to_string()
        };
        let quiet = Style {
            fg: if on_comments {
                theme.text.fg
            } else {
                theme.gutter.fg
            },
            bold: on_comments,
            ..theme.tab
        };
        grid.label(comments.x + 1, area.y, &name, quiet, area.right());
        for (index, (start, end)) in self.tabs(area).into_iter().enumerate() {
            let session = &self.sessions[index];
            let active = self.active == Some(index) && self.front == Front::Session;
            let tab = Rect::new(start, area.y, end - start, 1);
            let hovered = hover.is_some_and(|h| tab.contains(h.0, h.1));
            let style = if active {
                Style {
                    bold: true,
                    ..theme.text
                }
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
            grid.label(start + TAB_PADDING, area.y, &session.name, shown, close);
            if hovered || active {
                let quiet = Style {
                    fg: theme.gutter.fg,
                    ..style
                };
                grid.label(close, area.y, "×", quiet, end);
            }
        }
        let plus = self.tabs(area).last().map_or(area.x + 1, |t| t.1);
        let button = Rect::new(plus, area.y, 3, 1);
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
            fg: if (self.active.is_none() && self.front == Front::Session) || on_it {
                theme.text.fg
            } else {
                theme.gutter.fg
            },
            ..theme.tab
        };
        grid.label(plus + 1, area.y, "+", style, area.right());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cellix_input::Mods;

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
