//! Several folders in one Cellix, each a workspace with its own files, agents and sessions,
//! and a bar along the bottom to move between them. With sessions held by hosts and tabs
//! kept on disk, this does for work over SSH what a multiplexer did: close the laptop, and
//! the next Cellix on the machine finds everything as it was.

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Instant;

use cellix_input::{Command, Event, Key, KeyEvent, MouseEvent, MouseKind};
use cellix_render::{Bounds, Font, Grid, Icon, Measure, Rect, Rgb, Style};

use crate::app::{App, TileState, WorkspaceRequest};
use crate::theme::Theme;

/// The bar's size, relative to the interface font, as the status bar's.
const BAR_SIZE: u8 = 90;
/// Room inside a tile, in cells: before its mark, between its parts, and at its end.
const PAD: f32 = 0.8;
const GAP: f32 = 0.7;
/// Between a tile's name and what it says.
const WORD_GAP: f32 = 0.8;
/// Between one tile's chip and the next.
const BETWEEN: f32 = 0.7;
/// Kept on every tile for its close button, so tiles don't move as it comes and goes.
const CLOSE: f32 = 2.2;
const MARK: f32 = 1.0;
/// The chip behind a tile stands this far inside its row, above and below.
const CHIP_INSET: f32 = 0.06;
/// As a percentage of a row, about 9 pixels.
const CHIP_RADIUS: u8 = 26;
/// The button that opens another folder beside this one, and what its tip says.
const NEW_WORKSPACE: &str = "New Workspace";
const NEW_WORKSPACE_TIP: &str = "Open another folder beside this one";
/// The tip's size, relative to the interface font.
const TIP_SIZE: u8 = 85;

/// One folder's workspace: its app, and the socket agents reach it on.
pub struct Workspace {
    pub app: App,
    pub server: Option<cellix_agent::Server>,
}

/// Makes the workspace for a folder at a size, ready to show: in the terminal that means
/// its socket, its look, and what it left running or unsaved last time.
pub type Starter = Box<dyn FnMut(PathBuf, (u16, u16)) -> Workspace>;

/// What a click in the bar lands on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BarHit {
    Tile(usize),
    Close(usize),
    New,
    ResumeAll,
}

/// A tile's place in the bar, in cells.
struct Span {
    start: f32,
    end: f32,
}

pub struct Workspaces {
    pub list: Vec<Workspace>,
    pub active: usize,
    size: (u16, u16),
    hover: Option<(u16, u16)>,
    /// Quitting goes through each workspace in turn, so each asks about its own work. It
    /// holds the folder of the one asked last.
    quitting: Option<PathBuf>,
    /// A workspace being closed from its tile, by its folder, while it asks what to do.
    closing: Option<PathBuf>,
    pub quit: bool,
    /// Cellix started away from any project, with work running: what's running is offered
    /// first, to pick from.
    pub choosing: bool,
    /// Set when another workspace came to the front, so the whole screen is sent again.
    switched: bool,
    /// Whether the bar shows at all: not when Cellix is only editing a file for another
    /// program, as for a commit message.
    pub bar: bool,
    machine: String,
    start: Starter,
}

impl Workspaces {
    pub fn new(size: (u16, u16), machine: String, start: Starter) -> Self {
        Self {
            list: Vec::new(),
            active: 0,
            size,
            hover: None,
            quitting: None,
            closing: None,
            quit: false,
            choosing: false,
            switched: false,
            bar: true,
            machine,
            start,
        }
    }

    pub fn size(&self) -> (u16, u16) {
        self.size
    }

    /// Whether the bar shows: always, so there's somewhere to open another folder, except over
    /// the first screen.
    pub fn shows_bar(&self) -> bool {
        self.bar && !self.list.is_empty() && !self.choosing
    }

    /// Where the New Workspace button goes, and where it ends.
    fn new_button(&self) -> (f32, f32) {
        let start = self.spans().1;
        let width = self.room(PAD)
            + self.room(MARK)
            + self.room(GAP)
            + self.text_width(NEW_WORKSPACE)
            + self.room(PAD);
        (start, start + width)
    }

    /// The size each app has: the window, less the bar's row when it shows.
    fn app_size(&self) -> (u16, u16) {
        let bar = u16::from(self.shows_bar());
        (self.size.0, self.size.1.saturating_sub(bar))
    }

    pub fn app(&self) -> &App {
        &self.list[self.active].app
    }

    pub fn app_mut(&mut self) -> &mut App {
        &mut self.list[self.active].app
    }

    /// Opens `root` as a workspace, or finds it open, and gives its place in the list.
    pub fn open(&mut self, root: PathBuf) -> usize {
        if let Some(index) = self
            .list
            .iter()
            .position(|w| w.app.workspace.root() == root)
        {
            return index;
        }
        let had_bar = self.shows_bar();
        // Made to the size it has once it's in the list, with the bar if the bar shows.
        let bar = u16::from(self.bar && !self.choosing);
        let size = (self.size.0, self.size.1.saturating_sub(bar));
        let workspace = (self.start)(root, size);
        self.list.push(workspace);
        if had_bar != self.shows_bar() {
            self.fit();
        }
        self.list.len() - 1
    }

    /// Opens each folder with work running on this machine that no other Cellix has open,
    /// behind the ones already here.
    pub fn open_running(&mut self) {
        for (root, _) in crate::hold::projects_with_activity() {
            if !open_elsewhere(&root) {
                self.open(root);
            }
        }
    }

    /// Brings workspace `index` to the front.
    pub fn switch(&mut self, index: usize) {
        if index < self.list.len() && index != self.active {
            self.active = index;
            self.switched = true;
        }
    }

    /// Whether the front workspace changed since last asked, so all of it must be sent.
    pub fn take_switched(&mut self) -> bool {
        std::mem::take(&mut self.switched)
    }

    /// Gives every app the size it has now.
    fn fit(&mut self) {
        let (width, height) = self.app_size();
        for workspace in &mut self.list {
            if workspace.app.size() != (width, height) {
                workspace.app.handle(Event::Resize(width, height));
            }
        }
    }

    pub fn handle(&mut self, event: Event) {
        match event {
            Event::Resize(width, height) => {
                self.size = (width, height);
                self.fit();
            }
            Event::Tick => {
                for workspace in &mut self.list {
                    workspace.app.handle(Event::Tick);
                }
            }
            Event::Mouse(mouse) if self.choosing => self.choose_mouse(mouse),
            Event::Mouse(mouse) if self.on_bar(mouse.y) => self.bar_mouse(mouse),
            Event::Mouse(mouse) => {
                // Leaving the bar, nothing there stays lit.
                self.hover = None;
                self.app_mut().handle(Event::Mouse(mouse));
            }
            Event::Key(key) if self.choosing => self.choose_key(key),
            _ if self.choosing => {}
            event => self.app_mut().handle(event),
        }
        self.settle();
    }

    /// Lets every app take what came in, and answers the requests that brings. Returns
    /// whether anything changed.
    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        for workspace in &mut self.list {
            changed |= workspace.app.poll();
            if let Some(server) = &workspace.server {
                while let Ok(request) = server.requests.try_recv() {
                    workspace.app.serve(request);
                    changed = true;
                }
            }
        }
        let count = self.list.len();
        let active = self.active;
        self.settle();
        changed || count != self.list.len() || active != self.active
    }

    /// Acts on what the apps asked for, and lets go of those that quit.
    fn settle(&mut self) {
        if self.list.is_empty() {
            return;
        }
        match self.app_mut().workspace_request.take() {
            Some(WorkspaceRequest::Open(root)) => {
                let index = self.open(root);
                self.switch(index);
            }
            Some(WorkspaceRequest::Next) => {
                let next = (self.active + 1) % self.list.len();
                self.switch(next);
            }
            Some(WorkspaceRequest::Previous) => {
                let count = self.list.len();
                self.switch((self.active + count - 1) % count);
            }
            None => {}
        }
        // A quit chain stops if the one asked said Cancel.
        let asked = self.quitting.as_deref();
        let cancelled = asked.and_then(|root| self.find(root)).is_some_and(|index| {
            let app = &self.list[index].app;
            !app.quit && !app.asking()
        });
        if cancelled {
            self.quitting = None;
        }
        let closing = self.closing.as_deref();
        if closing
            .and_then(|root| self.find(root))
            .is_some_and(|index| !self.list[index].app.quit && !self.list[index].app.asking())
        {
            self.closing = None;
        }
        while let Some(index) = self.list.iter().position(|w| w.app.quit) {
            let gone = self.list.remove(index);
            let root = gone.app.workspace.root().to_path_buf();
            // Kept for the next run, as when Cellix quits; any error went to the app already.
            let _ = gone.app.finish();
            drop(gone);
            let closed = self.closing.as_deref() == Some(root.as_path());
            if closed {
                self.closing = None;
            } else if self.quitting.is_none() {
                // Quitting one quits Cellix: the others are asked in turn.
                self.quitting = Some(root.clone());
            }
            if self.list.is_empty() {
                self.quit = true;
                return;
            }
            if self.active > index || self.active >= self.list.len() {
                self.active = self.active.saturating_sub(1);
            }
            self.switched = true;
            if self.list.len() == 1 {
                self.fit();
            }
        }
        if self
            .quitting
            .as_ref()
            .is_some_and(|asked| self.find(asked).is_none())
        {
            self.active = 0;
            let next = self.list[0].app.workspace.root().to_path_buf();
            self.quitting = Some(next);
            self.list[0].app.run(Command::Quit);
            // It may have quit at once, with nothing to ask.
            self.settle();
        }
    }

    fn find(&self, root: &Path) -> Option<usize> {
        self.list
            .iter()
            .position(|w| w.app.workspace.root() == root)
    }

    /// Closes one workspace as quitting closes Cellix: it asks about unsaved files and
    /// what's running first.
    fn close(&mut self, index: usize) {
        self.switch(index);
        self.closing = Some(self.app().workspace.root().to_path_buf());
        self.app_mut().run(Command::Quit);
    }

    /// Resumes everything that stopped with the machine, in every workspace.
    fn resume_all(&mut self) {
        let mut count = 0;
        for workspace in &mut self.list {
            count += workspace.app.revive_all();
        }
        let agents = if count == 1 { "session" } else { "sessions" };
        self.app().say(format!("Resumed {count} {agents}"));
    }

    fn stopped(&self) -> usize {
        self.list.iter().map(|w| w.app.stopped_count()).sum()
    }

    fn on_bar(&self, y: u16) -> bool {
        self.shows_bar() && y + 1 == self.size.1
    }

    fn measure(&self) -> Rc<dyn Measure> {
        self.app().measure()
    }

    fn theme(&self) -> &Theme {
        self.app().theme()
    }

    /// A distance in the bar: as given where text is measured, whole cells where it isn't,
    /// so nothing rounds onto its neighbour.
    fn room(&self, cells: f32) -> f32 {
        if self.measure().in_cells() {
            cells.ceil()
        } else {
            cells
        }
    }

    fn font() -> Font {
        Font {
            size: BAR_SIZE,
            ..Font::default()
        }
    }

    fn text_width(&self, text: &str) -> f32 {
        if text.is_empty() {
            return 0.0;
        }
        let measure = self.measure();
        measure.width(text, Self::font())
    }

    /// Where each tile goes, and the button for a new one after them.
    fn spans(&self) -> (Vec<Span>, f32) {
        let plain = self.measure().in_cells();
        let round = |x: f32| if plain { x.ceil() } else { x };
        let mut x = 1.0;
        let mut spans = Vec::new();
        for (index, workspace) in self.list.iter().enumerate() {
            let (_, word, _) = self.word(workspace.app.tile_state());
            let mut width = self.room(PAD)
                + self.room(MARK)
                + self.room(GAP)
                + self.text_width(&name_of(&workspace.app));
            if !word.is_empty() {
                width += self.room(WORD_GAP) + self.text_width(&word);
            }
            // Only the tile in front has its ×, so the gaps between the others stay even;
            // a middle click closes any of them.
            width += if index == self.active {
                self.room(CLOSE)
            } else {
                self.room(PAD)
            };
            let start = x;
            x = round(x + width);
            spans.push(Span { start, end: x });
            x += round(BETWEEN);
        }
        (spans, x)
    }

    /// Where the machine's name starts: ending where the status bar's last words end.
    fn machine_x(&self) -> f32 {
        f32::from(self.size.0) - 1.0 - self.text_width(&self.machine)
    }

    /// The Resume button's place at the right, when something stopped: its start and label.
    fn resume_button(&self) -> Option<(f32, String)> {
        let stopped = self.stopped();
        if stopped == 0 {
            return None;
        }
        let what = if stopped == 1 { "Session" } else { "Sessions" };
        let label = format!("Resume {stopped} {what}");
        let width = self.room(PAD)
            + self.room(MARK)
            + self.room(GAP)
            + self.text_width(&label)
            + self.room(PAD);
        let right = self.machine_x() - 1.5;
        Some(((right - width).floor(), label))
    }

    fn bar_hit(&self, mouse: &MouseEvent) -> Option<BarHit> {
        let x = if self.measure().in_cells() {
            f32::from(mouse.x) + 0.5
        } else {
            mouse.fine_x
        };
        let (spans, _) = self.spans();
        for (index, span) in spans.iter().enumerate() {
            if x >= span.start && x < span.end {
                let on_close = x >= span.end - self.room(CLOSE);
                let shows_close = index == self.active;
                return Some(if on_close && shows_close {
                    BarHit::Close(index)
                } else {
                    BarHit::Tile(index)
                });
            }
        }
        let (start, end) = self.new_button();
        if x >= start && x < end {
            return Some(BarHit::New);
        }
        let (start, label) = self.resume_button()?;
        let end = start
            + self.room(PAD)
            + self.room(MARK)
            + self.room(GAP)
            + self.text_width(&label)
            + self.room(PAD);
        (x >= start && x < end).then_some(BarHit::ResumeAll)
    }

    fn hovered(&self, span: &Span) -> bool {
        self.hover.is_some_and(|(x, y)| {
            self.on_bar(y) && f32::from(x) + 0.5 >= span.start && f32::from(x) + 0.5 < span.end
        })
    }

    fn bar_mouse(&mut self, mouse: MouseEvent) {
        self.hover = Some((mouse.x, mouse.y));
        let hit = self.bar_hit(&mouse);
        match (mouse.kind, hit) {
            (MouseKind::Left, Some(BarHit::Tile(index))) => self.switch(index),
            (MouseKind::Left, Some(BarHit::Close(index)))
            | (MouseKind::Middle, Some(BarHit::Tile(index) | BarHit::Close(index))) => {
                self.close(index);
            }
            (MouseKind::Left, Some(BarHit::New)) => self.app_mut().run(Command::OpenWorkspace),
            (MouseKind::Left, Some(BarHit::ResumeAll)) => self.resume_all(),
            _ => {}
        }
    }

    /// The mark, word and their colour for a tile's state.
    fn word(&self, state: TileState) -> (Mark, String, Rgb) {
        let theme = self.theme();
        let dim = theme.gutter.fg;
        match state {
            TileState::NeedsYou => (
                Mark::Dot(theme.attention),
                "needs you".into(),
                theme.attention,
            ),
            TileState::Working => (Mark::Dot(theme.accent), "working".into(), dim),
            TileState::Done(at) => (Mark::Check, format!("done {}", ago(at)), dim),
            TileState::Stopped(_) => (Mark::Stopped, "stopped".into(), dim),
            TileState::Idle => (Mark::Dot(dim), String::new(), dim),
            TileState::Nothing => (Mark::Ring, String::new(), dim),
        }
    }

    pub fn draw(&self, grid: &mut Grid) {
        if self.choosing {
            self.draw_choice(grid);
            return;
        }
        self.app().draw(grid);
        if self.shows_bar() {
            self.draw_bar(grid);
        }
    }

    fn draw_bar(&self, grid: &mut Grid) {
        let theme = self.theme();
        let y = self.size.1.saturating_sub(1);
        let base = theme.workspaces;
        grid.fill(Rect::new(0, y, self.size.0, 1), base);
        let (spans, _) = self.spans();
        let right = self.size.0.saturating_sub(1);
        for (index, (workspace, span)) in self.list.iter().zip(&spans).enumerate() {
            let active = index == self.active;
            let hovered = self.hovered(span);
            let chip = Bounds::cells(
                span.start,
                f32::from(y) + CHIP_INSET,
                span.end - span.start,
                1.0 - 2.0 * CHIP_INSET,
            );
            if active {
                grid.panel(chip, CHIP_RADIUS, Some(theme.tint), None);
            } else if hovered {
                grid.panel(chip, CHIP_RADIUS, Some(theme.row_hover), None);
            }
            let (mark, word, color) = self.word(workspace.app.tile_state());
            let mut x = span.start + self.room(PAD);
            draw_mark(grid, (x, y), mark, theme, base);
            x += self.room(MARK) + self.room(GAP);
            let name = name_of(&workspace.app);
            let name_style = Style {
                fg: if active { theme.text.fg } else { base.fg },
                ..base
            };
            label(grid, (x, y), &name, name_style, right);
            x += self.text_width(&name);
            if !word.is_empty() {
                x += self.room(WORD_GAP);
                label(grid, (x, y), &word, Style { fg: color, ..base }, right);
            }
            if active {
                let close = span.end - self.room(CLOSE) + 0.3;
                let quiet = Style {
                    fg: theme.gutter.fg,
                    ..base
                };
                grid.glyph(close, y, Icon::Close, quiet.fg, "×", quiet);
            }
        }
        self.draw_new_button(grid, y, base);
        // The machine's name at the right, so it's plain which one this is.
        let machine_style = Style {
            fg: theme.gutter.fg,
            ..base
        };
        label(
            grid,
            (self.machine_x(), y),
            &self.machine,
            machine_style,
            right,
        );
        if let Some((start, text)) = self.resume_button() {
            let width = self.room(PAD)
                + self.room(MARK)
                + self.room(GAP)
                + self.text_width(&text)
                + self.room(PAD);
            let chip = Bounds::cells(
                start,
                f32::from(y) + CHIP_INSET,
                width,
                1.0 - 2.0 * CHIP_INSET,
            );
            let on_it = self.hover.is_some_and(|(x, hy)| {
                self.on_bar(hy) && f32::from(x) + 0.5 >= start && f32::from(x) + 0.5 < start + width
            });
            let fill = if on_it {
                theme.popup_hover
            } else {
                theme.row_active
            };
            grid.panel(chip, CHIP_RADIUS, Some(fill), None);
            let style = Style {
                fg: theme.text.fg,
                ..base
            };
            grid.glyph(
                start + self.room(PAD),
                y,
                Icon::Play,
                theme.accent,
                "▶",
                Style {
                    fg: theme.accent,
                    ..base
                },
            );
            label(
                grid,
                (start + self.room(PAD) + self.room(MARK) + self.room(GAP), y),
                &text,
                style,
                right,
            );
        }
    }

    /// The button that opens another folder as a workspace: labelled, so it says what it
    /// does, with a tip above it on hover.
    fn draw_new_button(&self, grid: &mut Grid, y: u16, base: Style) {
        let theme = self.theme();
        let (start, end) = self.new_button();
        let on_it = self.hover.is_some_and(|(x, hy)| {
            let x = f32::from(x) + 0.5;
            self.on_bar(hy) && x >= start && x < end
        });
        let chip = Bounds::cells(
            start,
            f32::from(y) + CHIP_INSET,
            end - start,
            1.0 - 2.0 * CHIP_INSET,
        );
        let fill = if on_it {
            theme.popup_hover
        } else {
            theme.row_active
        };
        grid.panel(chip, CHIP_RADIUS, Some(fill), None);
        let right = self.size.0.saturating_sub(1);
        let mark = start + self.room(PAD);
        let plus = Style {
            fg: theme.accent,
            ..base
        };
        grid.label_at(mark + 0.1, mark as u16, y, "+", plus, right, 120);
        let text = Style {
            fg: theme.text.fg,
            ..base
        };
        let x = mark + self.room(MARK) + self.room(GAP);
        label(grid, (x, y), NEW_WORKSPACE, text, right);
        // The tip stays out of the way of what the button opened.
        if on_it && !self.measure().in_cells() && !self.app().overlay_is_open() {
            let font = Font {
                size: TIP_SIZE,
                ..Font::default()
            };
            let wide = self.measure().width(NEW_WORKSPACE_TIP, font) + 2.0;
            let left = start.min(f32::from(self.size.0) - wide - 1.0).max(0.0);
            let top = f32::from(y) - 1.25;
            let bounds = Bounds::cells(left, top, wide, 1.0);
            grid.popup(bounds, CHIP_RADIUS, theme.popup, theme.popup_border);
            let colour = (theme.popup_dim, false);
            grid.text_run(
                (left + 1.0, top),
                left + wide,
                NEW_WORKSPACE_TIP,
                colour,
                font,
            );
        }
    }

    /// The first screen's column: its left edge, width and top row.
    fn choice_rows(&self) -> (u16, u16, u16) {
        let rail = self.app().rail_width();
        let room = self.size.0.saturating_sub(rail);
        let width = 76.min(room.saturating_sub(6));
        let left = rail + (room.saturating_sub(width)) / 2;
        let top = (self.size.1 / 5).max(2);
        (left, width, top)
    }

    /// The row of the first workspace on the first screen; each takes two.
    fn first_choice(&self) -> u16 {
        self.choice_rows().2 + 6
    }

    fn choice_at(&self, y: u16) -> Option<usize> {
        let offset = y.checked_sub(self.first_choice())?;
        let index = usize::from(offset / 2);
        (offset % 2 == 0 && index < self.list.len()).then_some(index)
    }

    fn open_all_row(&self) -> u16 {
        self.first_choice() + 2 * self.list.len() as u16 + 1
    }

    fn choose(&mut self, index: usize) {
        self.choosing = false;
        self.active = index;
        self.switched = true;
        self.fit();
    }

    fn choose_mouse(&mut self, mouse: MouseEvent) {
        self.hover = Some((mouse.x, mouse.y));
        if mouse.kind != MouseKind::Left {
            return;
        }
        let picked = self
            .choice_at(mouse.y)
            .or_else(|| (mouse.y == self.open_all_row()).then_some(self.active));
        if let Some(index) = picked {
            self.choose(index);
        }
    }

    /// Up and Down pick a workspace on the first screen, and Enter opens it.
    fn choose_key(&mut self, key: KeyEvent) {
        let count = self.list.len();
        match key.key {
            Key::Up => self.active = (self.active + count - 1) % count,
            Key::Down => self.active = (self.active + 1) % count,
            Key::Enter => self.choose(self.active),
            _ => {}
        }
    }

    /// The first screen, when Cellix started away from a project with work running here: the
    /// app's frame, with what's running to pick from.
    fn draw_choice(&self, grid: &mut Grid) {
        let theme = self.theme();
        let base = theme.text;
        let (width, height) = self.size;
        grid.fill(Rect::new(0, 0, width, height), base);
        // The rail, the tab row and the status bar, as around any page, so the terminal's
        // window padding meets the same colours it does there.
        let rail = self.app().rail_width();
        grid.fill(Rect::new(0, 0, rail, height), theme.activity);
        grid.fill(Rect::new(rail, 0, width.saturating_sub(rail), 1), theme.tab);
        let bottom = height.saturating_sub(1);
        grid.fill(Rect::new(0, bottom, width, 1), theme.status);
        label(grid, (1.5, bottom), &self.machine, theme.status, width);

        let (left, column, top) = self.choice_rows();
        let right = left + column;
        let heading = Style {
            bold: true,
            fg: theme.heading,
            ..base
        };
        grid.label_at(f32::from(left), left, top, "Cellix", heading, right, 180);
        let dim = Style {
            fg: theme.gutter.fg,
            ..base
        };
        let count = self.list.len();
        let what = if count == 1 {
            "workspace is"
        } else {
            "workspaces are"
        };
        let line = format!(
            "{count} {what} open on {}. Pick one to carry on.",
            self.machine
        );
        grid.label(left, top + 2, &line, dim, right);
        // The section's name sits half a row nearer its list than the line above it.
        let section = format!("RUNNING ON {}", self.machine.to_uppercase());
        if self.measure().in_cells() {
            grid.label_at(f32::from(left), left, top + 4, &section, dim, right, 80);
        } else {
            let font = Font {
                size: 80,
                bold: true,
                ..Font::default()
            };
            let at = (f32::from(left), f32::from(top) + 4.45);
            grid.text_run(at, f32::from(right), &section, (dim.fg, false), font);
        }
        // Names, what each is doing, and where, each in a column of its own.
        let word_x = f32::from(left) + 20.0;
        let path_x = f32::from(left) + 42.0;
        for (index, workspace) in self.list.iter().enumerate() {
            let y = self.first_choice() + 2 * index as u16;
            let row = Rect::new(left, y, column, 1);
            let hovered = self.hover.is_some_and(|(x, hy)| row.contains(x, hy));
            let chip = Bounds::cells(
                f32::from(left) - 1.0,
                f32::from(y) - 0.28,
                f32::from(column) + 2.0,
                1.56,
            );
            if index == self.active {
                grid.panel(chip, CHIP_RADIUS, Some(theme.tint), None);
            } else if hovered {
                grid.panel(chip, CHIP_RADIUS, Some(theme.row_hover), None);
            }
            let (mark, word, color) = self.word(workspace.app.tile_state());
            draw_mark(grid, (f32::from(left), y), mark, theme, base);
            let strong = Style { bold: true, ..base };
            label(
                grid,
                (f32::from(left) + 1.8, y),
                &name_of(&workspace.app),
                strong,
                right,
            );
            let word = if word.is_empty() {
                "open".to_string()
            } else {
                word
            };
            label(grid, (word_x, y), &word, Style { fg: color, ..base }, right);
            let room = f32::from(right) - path_x;
            let path = self.shortened(&tilde(workspace.app.workspace.root()), room);
            label(grid, (path_x, y), &path, dim, right);
        }
        let last = self.first_choice() + 2 * count.saturating_sub(1) as u16;
        let rule = Bounds::cells(
            f32::from(left),
            f32::from(last) + 1.95,
            f32::from(column),
            0.0,
        );
        grid.hairline(rule, theme.border);
        let link = Style {
            fg: theme.accent,
            ..base
        };
        let y = self.open_all_row();
        grid.label(left, y, "Open", link, right);
        let hint = "↑ ↓ to pick, or run cellix in a project's folder";
        let hint_x = f32::from(right) - self.text_width(hint);
        label(grid, (hint_x, y), hint, dim, right);
    }

    /// `text` with its middle taken out to fit `room` cells.
    fn shortened(&self, text: &str, room: f32) -> String {
        if self.text_width(text) <= room {
            return text.to_string();
        }
        let chars: Vec<char> = text.chars().collect();
        let mut keep = chars.len();
        while keep > 4 {
            keep -= 1;
            let head = keep / 3;
            let tail = keep - head;
            let tried: String = chars[..head]
                .iter()
                .chain(std::iter::once(&'…'))
                .chain(&chars[chars.len() - tail..])
                .collect();
            if self.text_width(&tried) <= room {
                return tried;
            }
        }
        "…".to_string()
    }
}

/// How a tile marks its state.
#[derive(Clone, Copy)]
enum Mark {
    Dot(Rgb),
    Ring,
    Check,
    Stopped,
}

fn draw_mark(grid: &mut Grid, (x, y): (f32, u16), mark: Mark, theme: &Theme, base: Style) {
    let right = grid.width();
    match mark {
        Mark::Dot(color) => label(grid, (x, y), "●", Style { fg: color, ..base }, right),
        Mark::Ring => label(
            grid,
            (x, y),
            "○",
            Style {
                fg: theme.gutter.fg,
                ..base
            },
            right,
        ),
        Mark::Check => {
            let style = Style {
                fg: theme.git_added,
                ..base
            };
            // The check is wider than the dots, so it starts a little earlier to end with them.
            grid.glyph(x - 0.2, y, Icon::Check, theme.git_added, "✓", style);
        }
        Mark::Stopped => {
            let style = Style {
                fg: theme.gutter.fg,
                ..base
            };
            grid.glyph(x, y, Icon::Stop, theme.gutter.fg, "■", style);
        }
    }
}

fn label(grid: &mut Grid, (x, y): (f32, u16), text: &str, style: Style, right: u16) {
    grid.label_at(x, x.round() as u16, y, text, style, right, BAR_SIZE);
}

/// What a workspace is called: its folder's name.
fn name_of(app: &App) -> String {
    let root = app.workspace.root();
    root.file_name().map_or_else(
        || root.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}

/// A path from home written with `~`.
fn tilde(path: &Path) -> String {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    match home.and_then(|home| path.strip_prefix(home).ok().map(Path::to_path_buf)) {
        Some(rest) => {
            let parts: Vec<String> = rest
                .components()
                .map(|part| part.as_os_str().to_string_lossy().into_owned())
                .collect();
            format!("~/{}", parts.join("/"))
        }
        None => path.display().to_string(),
    }
}

/// How long ago, in a word or two.
fn ago(at: Instant) -> String {
    let seconds = at.elapsed().as_secs();
    match seconds {
        0..60 => "just now".to_string(),
        60..3600 => format!("{}m ago", seconds / 60),
        _ => format!("{}h ago", seconds / 3600),
    }
}

/// Whether another Cellix has `root` open: its socket answers.
pub fn open_elsewhere(root: &Path) -> bool {
    cellix_agent::request(root, "comments").is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use cellix_agent::{AgentEvent, AgentState, Request};
    use cellix_input::Mods;
    use std::fs;

    /// Two projects side by side, as sibling folders.
    fn projects() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let first = dir.path().join("api-server");
        let second = dir.path().join("infra");
        for root in [&first, &second] {
            fs::create_dir(root).unwrap();
            fs::write(root.join("notes.txt"), "some notes\n").unwrap();
        }
        (dir, first, second)
    }

    fn spaces() -> Workspaces {
        let start: Starter = Box::new(|root, size| Workspace {
            app: App::new(root, size),
            server: None,
        });
        Workspaces::new((100, 30), "dev-box".to_string(), start)
    }

    fn screen(spaces: &Workspaces) -> Grid {
        let (width, height) = spaces.size();
        let mut grid = Grid::new(width, height, spaces.app().background());
        spaces.draw(&mut grid);
        grid
    }

    fn bar(spaces: &Workspaces) -> String {
        screen(spaces).row_text(spaces.size().1 - 1)
    }

    /// Where `text` starts on row `y`, in cells.
    fn spot(grid: &Grid, y: u16, text: &str) -> (u16, u16) {
        let row = grid.row_text(y);
        let at = row
            .find(text)
            .unwrap_or_else(|| panic!("no {text} in {row:?}"));
        (row[..at].chars().count() as u16, y)
    }

    fn click(spaces: &mut Workspaces, kind: MouseKind, (x, y): (u16, u16)) {
        spaces.handle(Event::Mouse(MouseEvent {
            kind,
            x,
            y,
            fine_x: f32::from(x),
            mods: Mods::default(),
        }));
    }

    fn type_text(spaces: &mut Workspaces, text: &str) {
        for c in text.chars() {
            spaces.handle(Event::Key(KeyEvent {
                key: Key::Char(c),
                mods: Mods::default(),
            }));
        }
    }

    fn key(spaces: &mut Workspaces, key: Key, mods: Mods) {
        spaces.handle(Event::Key(KeyEvent { key, mods }));
    }

    fn event(state: AgentState, detail: &str) -> Request {
        Request::Agent(AgentEvent {
            session: "s1".to_string(),
            agent: "Claude Code".to_string(),
            state,
            detail: detail.to_string(),
            folder: String::new(),
        })
    }

    #[test]
    fn the_bar_shows_every_workspace_and_a_click_switches() {
        let (_dir, first, second) = projects();
        let mut spaces = spaces();
        spaces.open(first.clone());
        // Even with one, the bar is there, with a button that says what it does.
        assert!(spaces.shows_bar());
        assert_eq!(spaces.app().size(), (100, 29), "the bar takes a row");
        let row = bar(&spaces);
        assert!(
            row.contains("api-server") && row.contains("+ New Workspace"),
            "{row}"
        );

        spaces.open(second.clone());
        assert!(spaces.shows_bar());
        for workspace in &spaces.list {
            assert_eq!(
                workspace.app.size(),
                (100, 29),
                "the bar takes a row from each"
            );
        }
        let row = bar(&spaces);
        assert!(row.contains("api-server") && row.contains("infra"), "{row}");
        assert!(row.contains("dev-box"), "the machine is named: {row}");
        assert!(row.contains("+ New Workspace"), "{row}");
        // The status bar is still there, above it.
        assert!(screen(&spaces).row_text(28).contains("Ctrl+P"));

        assert_eq!(spaces.active, 0);
        let grid = screen(&spaces);
        click(&mut spaces, MouseKind::Left, spot(&grid, 29, "infra"));
        assert_eq!(spaces.active, 1);
        assert!(spaces.take_switched(), "the whole screen is sent again");
        assert_eq!(spaces.app().workspace.root(), second);
    }

    #[test]
    fn a_tile_says_what_its_agent_is_doing() {
        let (_dir, first, second) = projects();
        let mut spaces = spaces();
        spaces.open(first);
        spaces.open(second);
        // The workspace behind still hears from its agents, and its tile says so.
        spaces.list[1]
            .app
            .serve(event(AgentState::Working, "Working"));
        assert!(bar(&spaces).contains("infra working"), "{}", bar(&spaces));
        spaces.list[1]
            .app
            .serve(event(AgentState::NeedsYou, "Allow sqlx migrate run?"));
        assert!(bar(&spaces).contains("infra needs you"), "{}", bar(&spaces));
        spaces.list[1]
            .app
            .serve(event(AgentState::Idle, "Finished"));
        assert!(
            bar(&spaces).contains("infra done just now"),
            "{}",
            bar(&spaces)
        );
        spaces.list[1]
            .app
            .serve(event(AgentState::Working, "Working"));
        assert!(bar(&spaces).contains("infra working"), "{}", bar(&spaces));
    }

    #[test]
    fn closing_a_tile_asks_as_quitting_does_and_the_rest_carry_on() {
        let (_dir, first, second) = projects();
        let mut spaces = spaces();
        spaces.open(first.clone());
        spaces.open(second.clone());
        spaces.list[1].app.open(&second.join("notes.txt")).unwrap();
        spaces.switch(1);
        type_text(&mut spaces, "draft ");
        spaces.switch(0);

        // Middle-click on its tile, as on a tab: it comes to the front and asks.
        let grid = screen(&spaces);
        click(&mut spaces, MouseKind::Middle, spot(&grid, 29, "infra"));
        assert_eq!(spaces.active, 1);
        let text = screen(&spaces).text();
        assert!(text.contains("Save changes to notes.txt?"), "{text}");
        let grid = screen(&spaces);
        let at = spot(
            &grid,
            (0..30)
                .find(|&y| grid.row_text(y).contains("Don't Save"))
                .unwrap(),
            "Don't Save",
        );
        click(&mut spaces, MouseKind::Left, at);

        assert_eq!(spaces.list.len(), 1);
        assert!(!spaces.quit, "Cellix goes on with the other");
        assert_eq!(spaces.app().workspace.root(), first);
        assert!(!bar(&spaces).contains("infra"), "{}", bar(&spaces));
        assert_eq!(spaces.app().size(), (100, 29));
        let _ = fs::remove_dir_all(crate::hold::folder_of(&second));
    }

    #[test]
    fn quitting_asks_each_workspace_in_turn_and_cancel_stops_it() {
        let (_dir, first, second) = projects();
        let mut spaces = spaces();
        spaces.open(first.clone());
        spaces.open(second.clone());
        for (index, root) in [(0, &first), (1, &second)] {
            spaces.switch(index);
            spaces.app_mut().open(&root.join("notes.txt")).unwrap();
            type_text(&mut spaces, "draft ");
        }
        spaces.switch(0);
        let ctrl = Mods {
            ctrl: true,
            ..Mods::default()
        };
        let answer = |spaces: &mut Workspaces, button: &str| {
            let grid = screen(spaces);
            let y = (0..30)
                .find(|&y| grid.row_text(y).contains(button))
                .unwrap();
            click(spaces, MouseKind::Left, spot(&grid, y, button));
        };

        key(&mut spaces, Key::Char('q'), ctrl);
        answer(&mut spaces, "Don't Save");
        // The first is gone, and the second asks about its own file.
        assert_eq!(spaces.list.len(), 1);
        assert!(
            screen(&spaces)
                .text()
                .contains("Save changes to notes.txt?")
        );
        answer(&mut spaces, "Cancel");
        assert!(!spaces.quit, "Cancel keeps Cellix open");
        assert_eq!(spaces.list.len(), 1);
        // Nothing asks again unbidden.
        spaces.poll();
        assert!(!spaces.app().asking());

        key(&mut spaces, Key::Char('q'), ctrl);
        answer(&mut spaces, "Don't Save");
        assert!(spaces.quit);
        for root in [&first, &second] {
            let _ = fs::remove_dir_all(crate::hold::folder_of(root));
        }
    }

    #[test]
    fn a_folder_opens_beside_this_one_and_keys_move_between_them() {
        let (_dir, first, second) = projects();
        let mut spaces = spaces();
        spaces.open(first);
        // A home of its own, so what the finder suggests is only what's here.
        spaces.app_mut().home = Some(_dir.path().to_path_buf());
        // The bar's button asks for the folder.
        let grid = screen(&spaces);
        click(
            &mut spaces,
            MouseKind::Left,
            spot(&grid, 29, "New Workspace"),
        );
        let text = screen(&spaces).text();
        assert!(text.contains("Folder to open, or a path"), "{text}");
        assert!(
            text.contains("infra") && text.contains("beside this one"),
            "{text}"
        );
        type_text(&mut spaces, "infra");
        key(&mut spaces, Key::Enter, Mods::default());
        assert_eq!(spaces.list.len(), 2);
        assert_eq!(spaces.active, 1, "the new one comes to the front");
        assert_eq!(
            spaces.app().workspace.root(),
            dunce::canonicalize(&second).unwrap()
        );

        let both = Mods {
            ctrl: true,
            alt: true,
            ..Mods::default()
        };
        key(&mut spaces, Key::PageDown, both);
        assert_eq!(spaces.active, 0, "round to the first");
        key(&mut spaces, Key::PageUp, both);
        assert_eq!(spaces.active, 1);

        // A name that's no folder says so. The palette asks too.
        spaces.app_mut().home = Some(_dir.path().to_path_buf());
        spaces.app_mut().run(Command::OpenWorkspace);
        type_text(&mut spaces, "nowhere");
        key(&mut spaces, Key::Enter, Mods::default());
        assert_eq!(spaces.list.len(), 2);
        assert!(
            screen(&spaces)
                .text()
                .contains("There's no folder at nowhere")
        );
    }

    #[test]
    fn the_folder_finder_walks_down_a_path_with_tab() {
        let (dir, first, _second) = projects();
        fs::create_dir_all(dir.path().join("work/deep/inside")).unwrap();
        let mut spaces = spaces();
        spaces.open(first);
        spaces.app_mut().home = Some(dir.path().to_path_buf());
        spaces.app_mut().run(Command::OpenWorkspace);
        // A path lists the folders in it as it's typed.
        type_text(&mut spaces, "~/wo");
        let text = screen(&spaces).text();
        assert!(text.contains("~/work"), "{text}");
        // Tab goes into the picked one, and lists what's inside.
        key(&mut spaces, Key::Tab, Mods::default());
        let text = screen(&spaces).text();
        assert!(text.contains("~/work/deep"), "{text}");
        assert!(text.contains("this folder"), "{text}");
        key(&mut spaces, Key::Down, Mods::default());
        key(&mut spaces, Key::Tab, Mods::default());
        assert!(screen(&spaces).text().contains("~/work/deep/inside"));
        // Enter on the folder itself opens it.
        key(&mut spaces, Key::Enter, Mods::default());
        assert_eq!(spaces.list.len(), 2);
        let opened = dunce::canonicalize(dir.path().join("work/deep")).unwrap();
        assert_eq!(spaces.app().workspace.root(), opened);
    }

    /// Unix only: sessions outlive Cellix only there.
    #[test]
    #[cfg(unix)]
    fn what_stopped_anywhere_resumes_from_the_bar() {
        let (_dir, first, second) = projects();
        let aside = crate::hold::folder_of(&second).join("stopped");
        fs::create_dir_all(&aside).unwrap();
        let job = crate::hold::Job {
            name: "Terminal".to_string(),
            argv: vec!["sh".to_string()],
            env: Vec::new(),
            folder: second.clone(),
            size: (80, 24),
            pixels: (0, 0),
        };
        fs::write(aside.join("1-1.json"), serde_json::to_vec(&job).unwrap()).unwrap();
        fs::write(aside.join("1-1.log"), "$ cargo watch\r\n").unwrap();
        let mut spaces = spaces();
        spaces.open(first);
        spaces.open(second.clone());
        spaces.list[1].app.rejoin_sessions();
        spaces.poll();
        let row = bar(&spaces);
        assert!(row.contains("infra stopped"), "{row}");
        assert!(row.contains("Resume 1 Session"), "{row}");

        let grid = screen(&spaces);
        click(
            &mut spaces,
            MouseKind::Left,
            spot(&grid, 29, "Resume 1 Session"),
        );
        assert_eq!(spaces.list[1].app.stopped_count(), 0);
        assert!(!bar(&spaces).contains("Resume"), "{}", bar(&spaces));
        assert!(!bar(&spaces).contains("stopped"));
        let _ = fs::remove_dir_all(crate::hold::folder_of(&second));
    }

    #[test]
    fn started_away_from_a_project_what_is_running_comes_first() {
        let (_dir, first, second) = projects();
        let mut spaces = spaces();
        spaces.open(first.clone());
        spaces.open(second.clone());
        spaces.choosing = true;
        spaces.fit();
        assert!(!spaces.shows_bar());
        let grid = screen(&spaces);
        let text = grid.text();
        assert!(text.contains("2 workspaces are open on dev-box"), "{text}");
        assert!(text.contains("RUNNING ON DEV-BOX"), "{text}");
        let y = (0..30)
            .find(|&y| grid.row_text(y).contains("infra"))
            .unwrap();
        click(&mut spaces, MouseKind::Left, spot(&grid, y, "infra"));
        assert!(!spaces.choosing);
        assert_eq!(spaces.app().workspace.root(), second);
        assert!(spaces.shows_bar());
        assert_eq!(spaces.app().size(), (100, 29));

        // The keys pick one too.
        spaces.choosing = true;
        spaces.active = 0;
        key(&mut spaces, Key::Down, Mods::default());
        assert_eq!(spaces.active, 1);
        key(&mut spaces, Key::Down, Mods::default());
        assert_eq!(spaces.active, 0, "round to the first");
        key(&mut spaces, Key::Enter, Mods::default());
        assert!(!spaces.choosing);
        assert_eq!(spaces.app().workspace.root(), first);
    }
}
