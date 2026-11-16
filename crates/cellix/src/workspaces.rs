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
use crate::inbox::Notifier;
use crate::theme::Theme;

/// The bar's size, relative to the interface font, as the status bar's.
const BAR_SIZE: u8 = 90;
/// Room inside a tile, in cells: before its mark, between its parts, and at its end.
const PAD: f32 = 1.5;
const GAP: f32 = 0.7;
/// Between a tile's name and what it says.
const WORD_GAP: f32 = 0.8;
/// Between one tile's chip and the next.
const BETWEEN: f32 = 2.0;
/// Kept on every tile for its close button, so tiles don't move as it comes and goes.
const CLOSE: f32 = 3.0;
const MARK: f32 = 1.0;
/// How far a chip stands in from the top and bottom of its row: three quarters of a row
/// tall. At two thirds, the tails of words of 12 pt stood on its lower edge.
const CHIP_INSET: f32 = 0.12;
/// How far the bar's chips and words are raised in their row on the pixel tiers, so 8 px of
/// air are left under them at the window's edge, and none crosses into the row above, whose
/// picture is made apart from this one's.
pub const BAR_LIFT: f32 = 0.12;
/// How much further the bar's words and icons are raised than its chips, in rows: a pixel
/// and a half.
/// Words stand with their capitals in the middle of their row, so their tails reach further
/// down than their heads reach up; raised, a tail has 2 px inside its chip and a head 3.
const WORD_LIFT: f32 = 0.045;
/// As a percentage of a row, about 9 pixels.
const CHIP_RADIUS: u8 = 26;
/// Columns the status bar keeps for its own words where the workspaces lead its row: what
/// it gives a message, and a branch before it. With fewer than that left between the
/// workspaces and the machine's name, the bar has a row to itself under the status bar.
const STATUS_ROOM: f32 = 72.0;
/// Between the button that opens a workspace and what the status bar says after it, where
/// the two share a row. A rule stands in the middle of it.
const AFTER: f32 = 2.0;
/// The button that opens another folder beside this one, and what its tip says.
const NEW_WORKSPACE: &str = "New Workspace";
/// The first screen's way out, beside Open: its word, and how far it stands from the
/// column's left edge.
const QUIT: &str = "Quit";
const QUIT_FROM: u16 = 8;
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
    /// Whether the terminal says Cellix is the window in front. Without word from it, it is.
    focused: bool,
    notifier: Notifier,
    /// Notifications for the terminal to show, as title and body, waiting to be written.
    pub notifications: Vec<(String, String)>,
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
            focused: true,
            notifier: Notifier::default(),
            notifications: Vec::new(),
        }
    }

    /// The terminal said Cellix came to the front, or went behind another window.
    pub fn set_focused(&mut self, focused: bool) {
        self.focused = focused;
        // Back in front, what wasn't told yet can be seen.
        if focused {
            self.notifier.clear();
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

    /// How wide the button that opens a workspace is: with its words where the bar has a
    /// row to itself, and its mark alone where the workspaces lead the status bar's.
    fn new_width(&self, words: bool) -> f32 {
        let said = if words {
            self.room(GAP) + self.text_width(NEW_WORKSPACE)
        } else {
            0.0
        };
        self.room(PAD) + self.room(MARK) + said + self.room(PAD)
    }

    /// Where the New Workspace button goes, and where it ends.
    fn new_button(&self) -> (f32, f32) {
        let start = self.spans().1;
        (start, start + self.new_width(!self.one_bar()))
    }

    /// Whether there's one bar at the foot: the workspaces lead the status bar's own row, the
    /// branch and what was last done follow them, and the machine's name stands before the
    /// bell. That's wherever both fit. A window too narrow, or with too many workspaces,
    /// keeps the bar a row of its own under the status bar.
    fn one_bar(&self) -> bool {
        if !self.shows_bar() {
            return false;
        }
        let lead = self.spans().1 + self.new_width(false) + AFTER;
        // The machine's name, and after the bell the button that quits.
        let quit = f32::from(crate::app::quit_room(self.size));
        let tail = self.text_width(&self.machine) + 2.0 + quit;
        f32::from(self.size.0) - lead - tail >= STATUS_ROOM
    }

    /// The columns of the status bar's row the bar has, where they share one: at its start,
    /// and before the bell.
    fn shared(&self) -> (u16, u16) {
        if !self.one_bar() {
            return (0, 0);
        }
        let lead = (self.new_button().1 + AFTER).ceil() as u16;
        let machine = self
            .machine_x()
            .map_or(0.0, |_| self.text_width(&self.machine) + 2.0);
        let resume = self
            .resume_label()
            .map_or(0.0, |label| self.resume_width(&label) + 1.5);
        (lead, (machine + resume).ceil() as u16)
    }

    /// Tells each app what of its status bar's row the bar has.
    fn share_row(&self) {
        let room = self.shared();
        for workspace in &self.list {
            workspace.app.status_room.set(room);
        }
    }

    /// Whether a place on the bar's row is the bar's: all of it where the bar has the row,
    /// and where it shares the status bar's, the workspaces at its start and Resume.
    fn bar_owns(&self, mouse: &MouseEvent) -> bool {
        !self.one_bar() || mouse.x < self.shared().0 || self.bar_hit(mouse).is_some()
    }

    /// The size each app has: the window, less the bar's row when it has one of its own.
    fn app_size(&self) -> (u16, u16) {
        let bar = u16::from(self.shows_bar() && !self.one_bar());
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
        // Made to about the size it has once it's in the list, and fitted to it after: with
        // one more workspace the bar may need a row of its own.
        let bar = u16::from(self.bar && !self.choosing);
        let size = (self.size.0, self.size.1.saturating_sub(bar));
        // Noted as open from now, and not only as Cellix leaves: a dropped connection, a
        // kill or a power cut never gets that far, and the next Cellix then offered only what
        // had something running. Tests run on whatever machine they're on, whose own list
        // this is.
        if self.bar && !cfg!(test) {
            crate::hold::remember_open(&root);
        }
        let workspace = (self.start)(root, size);
        self.list.push(workspace);
        self.fit();
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
        // An app can't see the others. It's told how many there are, for the commands that
        // move between them.
        let count = self.list.len();
        if let Some(front) = self.list.get_mut(self.active) {
            front.app.workspaces = count;
        }
        // What a workspace says in the bar changes its width, and with it whether the bar
        // fits in the status bar's row.
        self.fit();
        self.share_row();
        // The last row is drawn down into the window's bottom padding, and a click there
        // counts rows past the grid. It's on that row all the same.
        let event = match event {
            Event::Mouse(mut mouse) => {
                mouse.y = mouse.y.min(self.size.1.saturating_sub(1));
                Event::Mouse(mouse)
            }
            event => event,
        };
        match event {
            Event::Resize(width, height) => {
                self.size = (width, height);
                self.fit();
                self.share_row();
            }
            Event::Tick => {
                for workspace in &mut self.list {
                    workspace.app.handle(Event::Tick);
                }
            }
            Event::Mouse(mouse) if self.choosing => self.choose_mouse(mouse),
            Event::Mouse(mouse) if self.on_bar(mouse.y) && self.bar_owns(&mouse) => {
                self.bar_mouse(mouse)
            }
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
        changed |= self.notify();
        let count = self.list.len();
        let active = self.active;
        self.settle();
        changed || count != self.list.len() || active != self.active
    }

    /// Gives the workspace in front what waits in all of them, for its inbox.
    fn share_inbox(&mut self) {
        let active = self.active;
        let mut rows = Vec::new();
        for (index, workspace) in self.list.iter().enumerate() {
            let root = workspace.app.workspace.root().to_path_buf();
            let place = name_of(&workspace.app);
            for item in &workspace.app.inbox.items {
                rows.push(crate::inbox::Listed {
                    root: root.clone(),
                    place: place.clone(),
                    here: index == active,
                    item: item.clone(),
                });
            }
        }
        let rows = crate::inbox::sorted(rows);
        for (index, workspace) in self.list.iter_mut().enumerate() {
            workspace.app.inbox_shared = (index == active).then(|| rows.clone());
        }
    }

    /// Passes what's new in any workspace to the notifier while Cellix is behind another
    /// window, and takes a notification from it when one is due.
    fn notify(&mut self) -> bool {
        let on = |app: &App| app.settings.notify != Some(false);
        let front_on = self.list.get(self.active).is_none_or(|w| on(&w.app));
        for workspace in &mut self.list {
            let fresh = workspace.app.inbox.take_fresh();
            if self.focused || !front_on || !on(&workspace.app) {
                continue;
            }
            let name = name_of(&workspace.app);
            for item in fresh {
                self.notifier.push(&name, &item.title);
            }
        }
        let due = self.notifier.due(Instant::now());
        let any = due.is_some();
        self.notifications.extend(due);
        any
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
            Some(WorkspaceRequest::Attend(root, key)) => {
                if let Some(index) = self.find(&root) {
                    self.switch(index);
                    self.list[index].app.attend(&key);
                }
            }
            Some(WorkspaceRequest::Dismiss(root, key)) => {
                if let Some(index) = self.find(&root) {
                    self.list[index].app.inbox.drop_key(&key);
                }
            }
            Some(WorkspaceRequest::ClearFinished) => {
                for workspace in &mut self.list {
                    workspace.app.inbox.drop_kind(crate::inbox::Kind::Finished);
                }
            }
            None => {}
        }
        self.share_inbox();
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
            // A workspace closed from its tile is let go; one that goes as Cellix quits opens
            // again next time, whether or not anything is left running in it.
            if closed {
                crate::hold::forget_open(&root);
            } else if self.bar && !cfg!(test) {
                // Tests run on whatever machine they're on, whose own list this is.
                crate::hold::remember_open(&root);
            }
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
            let (_, word, _) = self.word(&workspace.app);
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

    /// Where the machine's name starts: ending where the status bar's last words end. None
    /// when it would reach the tiles, New Workspace or Resume: a long name, as some machines
    /// have, gives way rather than covering them.
    fn machine_x(&self) -> Option<f32> {
        // In the status bar's row it stands before the bell.
        let edge = f32::from(self.size.0) - 1.0;
        let end = match self.app().status_bell() {
            Some((bell, _)) if self.one_bar() => f32::from(bell) - 1.0,
            _ => edge,
        };
        let x = end - self.text_width(&self.machine);
        let resume = self
            .resume_label()
            .map_or(0.0, |l| self.resume_width(&l) + 1.5);
        let (_, new_end) = self.new_button();
        (x - resume >= new_end + 1.5).then_some(x)
    }

    /// What the Resume button says, when something stopped.
    fn resume_label(&self) -> Option<String> {
        let stopped = self.stopped();
        if stopped == 0 {
            return None;
        }
        let what = if stopped == 1 { "Session" } else { "Sessions" };
        Some(format!("Resume {stopped} {what}"))
    }

    fn resume_width(&self, label: &str) -> f32 {
        self.room(PAD) + self.room(MARK) + self.room(GAP) + self.text_width(label) + self.room(PAD)
    }

    /// The Resume button's place at the right, when something stopped: its start and label.
    fn resume_button(&self) -> Option<(f32, String)> {
        let label = self.resume_label()?;
        let width = self.resume_width(&label);
        let right = self
            .machine_x()
            .map_or(f32::from(self.size.0) - 1.0, |x| x - 1.5);
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

    /// The mark, word and their colour for a tile: what its agents are doing, and how full
    /// the fullest one's context is.
    fn word(&self, app: &App) -> (Mark, String, Rgb) {
        let (mark, mut word, mut color) = self.state_word(app.tile_state());
        if let Some(percent) = app.context_percent() {
            if !word.is_empty() {
                word.push_str(" · ");
            }
            word.push_str(&format!("{percent}%"));
            let theme = self.theme();
            if percent >= crate::transcript::FULL {
                color = theme.error;
            } else if percent >= crate::transcript::WARN {
                color = theme.attention;
            }
        }
        (mark, word, color)
    }

    /// The mark, word and their colour for a tile's state.
    fn state_word(&self, state: TileState) -> (Mark, String, Rgb) {
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
        self.share_row();
        self.app().draw(grid);
        if self.shows_bar() {
            // Lifted within its row, for air between the bar and the window's edge, where
            // the pixel tiers can place it. On a row with room, a chip is tall enough for its
            // words to stand in its middle and the row has air under it: nothing is lifted.
            let from = grid.painted_so_far();
            self.draw_bar(grid);
            if crate::measures::roomy(self.measure().as_ref()) {
                return;
            }
            let raise = |grid: &mut Grid, (from, to): (usize, usize)| {
                use cellix_render::Paint;
                let to = to.min(grid.paint.len());
                for paint in grid.paint.iter_mut().take(to).skip(from) {
                    paint.lift(cellix_render::units(BAR_LIFT));
                    // A mark a whole row tall, as the branch's is, has its ink at the row's
                    // top once the bar is raised. Raised with the words as well, its tip
                    // was in the row above, whose picture isn't moved down with the bar's:
                    // a sliver of it showed over the bar.
                    let with_words = match paint {
                        Paint::Text { .. } => true,
                        Paint::Icon { bounds, .. } => bounds.h < cellix_render::UNIT,
                        _ => false,
                    };
                    if with_words {
                        paint.lift(cellix_render::units(WORD_LIFT));
                    }
                }
            };
            let to = grid.painted_so_far();
            raise(grid, (from, to));
            if self.one_bar() {
                // The status bar's own words are in the row too, and stand level with the
                // workspaces' beside them.
                raise(grid, self.app().status_paint.get());
            }
        }
    }

    fn draw_bar(&self, grid: &mut Grid) {
        let theme = self.theme();
        let y = self.size.1.saturating_sub(1);
        let base = theme.workspaces;
        let shared = self.one_bar();
        let pictures = !self.measure().in_cells();
        if !shared {
            grid.fill(Rect::new(0, y, self.size.0, 1), base);
        }
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
            // The workspace in front is in the accent, with everything on it in the colour
            // that reads there. Without pictures there's no chip, and its name is brighter.
            let ink = (active && pictures).then_some(theme.button_text);
            if active {
                let fill = if pictures { theme.accent } else { theme.tint };
                grid.panel(chip, CHIP_RADIUS, Some(fill), None);
            } else if hovered {
                grid.panel(chip, CHIP_RADIUS, Some(theme.row_hover), None);
            }
            let (mark, word, color) = self.word(&workspace.app);
            let mut x = span.start + self.room(PAD);
            match ink {
                Some(ink) => {
                    let inked = Theme {
                        gutter: Style {
                            fg: ink,
                            ..theme.gutter
                        },
                        git_added: ink,
                        ..*theme
                    };
                    let mark = match mark {
                        Mark::Dot(_) => Mark::Dot(ink),
                        other => other,
                    };
                    draw_mark(grid, (x, y), mark, &inked, base);
                }
                None => draw_mark(grid, (x, y), mark, theme, base),
            }
            x += self.room(MARK) + self.room(GAP);
            let name = name_of(&workspace.app);
            let bright = if active { theme.text.fg } else { base.fg };
            let name_style = Style {
                fg: ink.unwrap_or(bright),
                ..base
            };
            label(grid, (x, y), &name, name_style, right);
            x += self.text_width(&name);
            if !word.is_empty() {
                x += self.room(WORD_GAP);
                let said = Style {
                    fg: ink.unwrap_or(color),
                    ..base
                };
                label(grid, (x, y), &word, said, right);
            }
            if active {
                let close = span.end - self.room(CLOSE) + 0.3;
                let quiet = Style {
                    fg: ink.unwrap_or(theme.gutter.fg),
                    ..base
                };
                grid.glyph(close, y, Icon::Close, quiet.fg, "×", quiet);
            }
        }
        self.draw_new_button(grid, y, base);
        if shared {
            // A rule between the workspaces and what the status bar says after them.
            let x = self.new_button().1 + AFTER / 2.0;
            let rule = Bounds::cells(x, f32::from(y) + 0.28, 0.0, 0.44);
            grid.hairline(rule, theme.border);
        }
        // The machine's name at the right, so it's plain which one this is.
        let machine_style = Style {
            fg: theme.gutter.fg,
            ..base
        };
        if let Some(x) = self.machine_x() {
            label(grid, (x, y), &self.machine, machine_style, right);
        }
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
            grid.small_glyph(
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
        // Its mark alone where the workspaces lead the status bar's row: the tip names it.
        if !self.one_bar() {
            label(grid, (x, y), NEW_WORKSPACE, text, right);
        }
        // The tip stays out of the way of what the button opened.
        if on_it && !self.measure().in_cells() && !self.app().overlay_is_open() {
            let font = Font {
                size: TIP_SIZE,
                ..Font::default()
            };
            let wide = self.measure().width(NEW_WORKSPACE_TIP, font) + 2.0;
            let left = start.min(f32::from(self.size.0) - wide - 1.0).max(0.0);
            // The row above the bar: a tip covers a row whole, never part of one, and on a
            // taller row stands in the middle of it.
            let top = f32::from(y) - 1.0;
            let inset = crate::app::TIP_INSET;
            let bounds = Bounds::cells(left, top + inset, wide, 1.0 - inset * 2.0);
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
        if self.on_quit(mouse.x, mouse.y) {
            self.quit = true;
            return;
        }
        let picked = self
            .choice_at(mouse.y)
            .or_else(|| (mouse.y == self.open_all_row()).then_some(self.active));
        if let Some(index) = picked {
            self.choose(index);
        }
    }

    /// Where Quit stands on the first screen, beside Open.
    fn quit_link(&self) -> Rect {
        let (left, ..) = self.choice_rows();
        let width = QUIT.len() as u16;
        Rect::new(left + QUIT_FROM, self.open_all_row(), width, 1)
    }

    /// The button at the end of the first screen's foot, which quits as it does on every
    /// other screen, from the same place.
    fn quit_button(&self) -> Option<Rect> {
        crate::app::quit_cell(self.size)
    }

    /// Whether a place on the first screen is one of its ways out.
    fn on_quit(&self, x: u16, y: u16) -> bool {
        let link = self.quit_link();
        // A column either side of the word counts, so it isn't a pixel hunt.
        let near = y == link.y && x + 1 >= link.x && x <= link.right();
        near || self.quit_button().is_some_and(|cell| cell.contains(x, y))
    }

    /// Up and Down pick a workspace on the first screen, and Enter opens it. The key that
    /// quits Cellix quits here too: every workspace stays as it is, for next time.
    fn choose_key(&mut self, key: KeyEvent) {
        if self.app().command_for(key) == Some(Command::Quit) {
            self.quit = true;
            return;
        }
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
        grid.roomy = crate::measures::roomy(self.measure().as_ref());
        grid.fill(Rect::new(0, 0, width, height), base);
        // The frame round it, as round any page: the rail where there is one, and the bar at
        // the foot. What's between is a sheet, as the file is, so the terminal's window
        // padding meets the frame's colour here too.
        let rail = self.app().rail_width();
        grid.fill(Rect::new(0, 0, rail, height), theme.activity);
        grid.edge = Some(theme.activity.bg);
        let bottom = height.saturating_sub(1);
        grid.fill(Rect::new(0, bottom, width, 1), theme.status);
        label(grid, (1.5, bottom), &self.machine, theme.status, width);
        if !self.measure().in_cells() {
            let sheet = Rect::new(rail, 0, width.saturating_sub(rail), bottom);
            let bounds = Bounds::inset(sheet, 0.0, 0.0);
            crate::app::sheet_edge(grid, bounds, theme.activity.bg, theme.border);
        }

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
            let (mark, word, color) = self.word(&workspace.app);
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
        // The way out, beside it and at the end of the foot, where it is on every other
        // screen.
        let leaving = self.hover.is_some_and(|(x, y)| self.on_quit(x, y));
        let quit = self.quit_link();
        let style = if leaving { base } else { dim };
        grid.label(quit.x, quit.y, QUIT, style, right);
        if let Some(cell) = self.quit_button() {
            let on_it = self.hover.is_some_and(|(x, y)| cell.contains(x, y));
            let measure = self.measure();
            crate::app::draw_quit(grid, cell, on_it || leaving, (theme, measure.as_ref()));
        }
        // What the pointer is on says what it does, where the keys are named otherwise.
        let hint = if leaving {
            "Quits Cellix. Every workspace stays as it is, for next time"
        } else {
            "↑ ↓ to pick, Ctrl+Q to quit, or run cellix in a folder"
        };
        let hint_x = (f32::from(right) - self.text_width(hint)).max(f32::from(quit.right() + 2));
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

    /// An app can't see the other workspaces, and its palette offers Next Workspace only
    /// when there's one to go to: it's told how many the window holds.
    #[test]
    fn an_app_is_told_how_many_workspaces_the_window_holds() {
        use cellix_input::{Command, Key, KeyEvent};
        let (_dir, first, second) = projects();
        let mut spaces = spaces();
        spaces.open(first);
        spaces.handle(Event::Key(KeyEvent::plain(Key::Esc)));
        assert_eq!(spaces.app().workspaces, 1);
        assert!(!spaces.app().can_run(Command::NextWorkspace));
        spaces.open(second);
        spaces.handle(Event::Key(KeyEvent::plain(Key::Esc)));
        assert_eq!(spaces.app().workspaces, 2);
        assert!(spaces.app().can_run(Command::PreviousWorkspace));
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
            fine_y: f32::from(y) + 0.5,
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
            pid: None,
            transcript: String::new(),
        })
    }

    /// The bar keeps air under it at the window's edge, as it's drawn with pixels, and stays
    /// in its own row.
    #[test]
    fn the_bar_leaves_air_at_the_windows_edge() {
        let (_dir, first, _) = projects();
        let mut spaces = spaces();
        spaces.open(first);
        let grid = screen(&spaces);
        let row = f32::from(spaces.size().1 - 1);
        let unit = cellix_render::UNIT as f32;
        let tint = spaces.theme().tint;
        let chip = grid
            .paint
            .iter()
            .find_map(|p| match p {
                cellix_render::Paint::Panel {
                    bounds,
                    fill: Some(fill),
                    ..
                } if *fill == tint && bounds.y as f32 >= (row - 1.0) * unit => Some(*bounds),
                _ => None,
            })
            .expect("the open workspace's chip");
        let (top, bottom) = (chip.y as f32 / unit, (chip.y + chip.h) as f32 / unit);
        assert!(top >= row, "within its row: {top} {row}");
        // 8 px where a row is 36, which is the least anything keeps from an edge. It was a
        // third of a row while the chips were two thirds of one; they're three quarters now,
        // for words of 12 pt.
        let air = (row + 1.0 - bottom) * 36.0;
        assert!(air >= 8.0, "air under it: {air} px");
    }

    #[test]
    fn a_tile_says_how_full_its_agents_context_is() {
        let (dir, first, _) = projects();
        let log = dir.path().join("session.jsonl");
        fs::write(&log, crate::transcript::tests::CLAUDE).unwrap();
        let mut spaces = spaces();
        spaces.open(first);
        spaces.app_mut().start_stand_in("Claude Code", "sleep 30");
        let Request::Agent(mut said) = event(AgentState::Working, "Working") else {
            unreachable!()
        };
        said.session = "abc-123".into();
        said.transcript = log.display().to_string();
        spaces.app_mut().serve(Request::Agent(said));
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while !bar(&spaces).contains("working · 8%") {
            assert!(std::time::Instant::now() < deadline, "{}", bar(&spaces));
            spaces.handle(Event::Tick);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    /// One bar at the foot, where both fit. The workspaces lead the status bar's own row, what
    /// the status bar says follows them, and the machine's name stands before the bell. A
    /// press on a workspace is the bar's and one on the rest of the row is the status bar's.
    /// With pictures the workspace in front is in the accent. A window too narrow for both
    /// gives the bar a row of its own again.
    #[test]
    fn the_workspaces_lead_the_status_bars_own_row() {
        let (_dir, first, second) = projects();
        let mut spaces = spaces();
        spaces.open(first);
        spaces.open(second);
        spaces.handle(Event::Resize(140, 30));
        assert!(spaces.one_bar());
        let (lead, tail) = spaces.shared();
        assert!(lead > 0 && tail > 0, "{lead} {tail}");
        assert_eq!(
            spaces.app().status_room.get(),
            (lead, tail),
            "the app is told"
        );
        let row = bar(&spaces);
        let at = |text: &str| {
            let found = row
                .find(text)
                .unwrap_or_else(|| panic!("no {text} in {row}"));
            row[..found].chars().count() as u16
        };
        let (bell, _) = spaces.app().status_bell().expect("a bell");
        assert!(
            at("api-server") < at("infra") && at("infra") < lead,
            "{row}"
        );
        assert!(
            at("Ctrl+P") >= lead,
            "the status bar's words start after them: {row}"
        );
        assert!(
            at("dev-box") + 7 <= bell,
            "the machine's name, before the bell: {row}"
        );
        assert!(
            at("dev-box") >= bell - tail - 1,
            "in the room kept for it: {row}"
        );
        // A press on the bell is the status bar's, and one on a workspace is the bar's.
        assert_eq!(spaces.active, 0);
        click(&mut spaces, MouseKind::Left, (bell + 1, 29));
        assert!(spaces.hover.is_none(), "the bar didn't take it");
        assert_eq!(spaces.active, 0);
        click(&mut spaces, MouseKind::Left, (at("infra"), 29));
        assert_eq!(spaces.active, 1, "a press on a workspace switches to it");
        assert!(spaces.hover.is_some());
        // With pictures the one in front is in the accent, as a chip in the last row.
        let measure: Rc<dyn Measure> = Rc::new(cellix_render::FontMeasure::new(17, 37));
        for workspace in &mut spaces.list {
            workspace.app.set_measure(measure.clone());
        }
        spaces.handle(Event::Tick);
        assert!(spaces.one_bar(), "still, with words measured");
        let grid = screen(&spaces);
        let accent = spaces.theme().accent;
        let last = 29 * cellix_render::UNIT;
        let chips = grid.paint.iter().filter(|p| {
            matches!(p, cellix_render::Paint::Panel { bounds, fill: Some(fill), .. }
                if *fill == accent && bounds.y >= last - cellix_render::UNIT / 2 && bounds.w > 4 * cellix_render::UNIT)
        });
        assert_eq!(chips.count(), 1, "one workspace is in front");
        // Too narrow for both, the bar has a row of its own and the status bar all of its.
        spaces.handle(Event::Resize(90, 30));
        assert!(!spaces.one_bar());
        assert_eq!(spaces.shared(), (0, 0));
        assert_eq!(spaces.app().size(), (90, 29));
        let _ = screen(&spaces);
        assert_eq!(spaces.app().status_room.get(), (0, 0));
    }

    #[test]
    fn the_bar_shows_every_workspace_and_a_click_switches() {
        let (_dir, first, second) = projects();
        let mut spaces = spaces();
        spaces.open(first.clone());
        // Even with one, the bar is there, with a button that says what it does.
        assert!(spaces.shows_bar());
        assert_eq!(
            spaces.app().size(),
            (100, 29),
            "in a window this narrow the bar takes a row"
        );
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
        assert!(row.contains("+ New Workspace"), "its button says so: {row}");
        assert!(!row.contains("Ctrl+P"), "{row}");
        // The status bar is above it.
        assert!(screen(&spaces).row_text(28).contains("Ctrl+P"));
        // A wider window has room for both in one row: the workspaces lead the status bar's,
        // their button its mark alone. Narrowed again, the bar has its own row back.
        spaces.handle(Event::Resize(140, 30));
        assert_eq!(spaces.app().size(), (140, 30));
        let row = bar(&spaces);
        assert!(
            !row.contains("New Workspace") && row.contains(" + "),
            "{row}"
        );
        let (names, machine) = (row.find("infra").unwrap(), row.find("dev-box").unwrap());
        let hint = row.find("Ctrl+P").expect("the status bar's words");
        assert!(names < hint && hint < machine, "in that order: {row}");
        spaces.handle(Event::Resize(100, 30));
        assert_eq!(spaces.app().size(), (100, 29));

        assert_eq!(spaces.active, 0);
        let grid = screen(&spaces);
        click(&mut spaces, MouseKind::Left, spot(&grid, 29, "infra"));
        assert_eq!(spaces.active, 1);
        assert!(spaces.take_switched(), "the whole screen is sent again");
        assert_eq!(spaces.app().workspace.root(), second);

        // Ghostty draws the last row down into its bottom padding, so the lower half of a
        // tile is past the grid's last row. A click there still lands on the tile.
        let (x, _) = spot(&grid, 29, "api-server");
        click(&mut spaces, MouseKind::Left, (x, 30));
        assert_eq!(spaces.active, 0, "a click under the last row is on the bar");
        let (x, _) = spot(&grid, 29, "infra");
        click(&mut spaces, MouseKind::Left, (x, 31));
        assert_eq!(spaces.active, 1);
    }

    /// Some machines have long names, as CI's Macs do: the name gives way rather than
    /// covering New Workspace, which a click needs to find.
    #[test]
    fn a_long_machine_name_leaves_the_bar_its_buttons() {
        let (_dir, first, second) = projects();
        let start: Starter = Box::new(|root, size| Workspace {
            app: App::new(root, size),
            server: None,
        });
        let long = "Mac-1762900000-iad20-gt1021-0ace7a96-99fb-47be-b5dc-271f74ee0e80".to_string();
        let mut spaces = Workspaces::new((100, 30), long.clone(), start);
        spaces.open(first);
        spaces.open(second);
        let row = bar(&spaces);
        assert!(row.contains("+ New Workspace"), "{row}");
        assert!(!row.contains("iad20"), "the name gives way: {row}");
        // A short one, with room, still shows.
        assert!(spaces_named("dev-box").contains("dev-box"));
    }

    /// The window the TeX tests have on CI: 140 columns, one workspace, a machine with a
    /// long name. What BibTeX said ends with the file it couldn't open, and the bar the
    /// workspaces share cut that off to keep the cursor's place.
    #[test]
    fn a_long_message_in_the_shared_bar_keeps_its_end() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("paper");
        fs::create_dir(&root).unwrap();
        fs::write(root.join("notes.txt"), "As Knuth shows.\n").unwrap();
        let start: Starter = Box::new(|root, size| Workspace {
            app: App::new(root, size),
            server: None,
        });
        let mut spaces = Workspaces::new((140, 30), "ip-10-0-129-183".to_string(), start);
        spaces.open(root.clone());
        spaces.app_mut().open(&root.join("notes.txt")).unwrap();
        // Five letters longer than BibTeX's, for the count of problems the bar had there.
        let said =
            "Built in 0.3 s, but BibTeX failed: I couldn't open database file missing-refs.bib";
        spaces.app().say(said);
        let row = bar(&spaces);
        assert!(row.contains("paper"), "{row}");
        assert!(row.contains("missing-refs.bib"), "{row}");
        assert!(row.contains("ip-10-0-129-183"), "{row}");
    }

    /// On a row with room a workspace's chip is tall enough for its words to stand in its
    /// middle, and nothing in the bar is raised: raised, as it is on the terminal's line for
    /// air at the window's edge, the words stood high in their chips.
    #[test]
    fn on_a_tall_row_nothing_in_the_bar_is_raised() {
        use cellix_render::{FontMeasure, Icon, Paint, UNIT, units};
        let (_dir, first, _) = projects();
        let mut spaces = spaces();
        spaces.open(first);
        spaces.handle(Event::Resize(140, 30));
        spaces.app_mut().set_branch("main");
        let row = i32::from(spaces.size().1 - 1) * UNIT;
        // Anything of the bar's starts after this, raised or not.
        let foot = row - UNIT / 2;
        // The chip in front, the branch's mark and a word of the bar's, from the row's top.
        let places = |spaces: &Workspaces| {
            let grid = screen(spaces);
            let accent = spaces.theme().accent;
            let chip = grid.paint.iter().find_map(|p| match p {
                Paint::Panel {
                    bounds,
                    fill: Some(fill),
                    ..
                } if *fill == accent && bounds.y > foot => Some(bounds.y - row),
                _ => None,
            });
            let mark = grid.paint.iter().find_map(|p| match p {
                Paint::Icon {
                    bounds,
                    icon: Icon::Git,
                    ..
                } if bounds.y > foot => Some(bounds.y - row),
                _ => None,
            });
            let word = grid.paint.iter().find_map(|p| match p {
                Paint::Text { text, y, .. } if text == "main" && *y > foot => Some(*y - row),
                _ => None,
            });
            (
                chip.expect("a chip"),
                mark.expect("a mark"),
                word.expect("a word"),
            )
        };
        let tall = Rc::new(FontMeasure::new(17, 50).with_type_row(36));
        spaces.app_mut().set_measure(tall);
        let (chip, mark, word) = places(&spaces);
        assert_eq!((chip, word), (units(CHIP_INSET), 0));
        assert!(mark >= 0, "{mark}");
        // On the terminal's own line the bar is raised, its words a little more. The
        // branch's mark, a whole row tall, goes no further than the bar: further, a sliver of
        // it was drawn in the row above.
        let line = Rc::new(FontMeasure::new(17, 36));
        spaces.app_mut().set_measure(line);
        let (chip, mark, word) = places(&spaces);
        assert_eq!(chip, units(CHIP_INSET) - units(BAR_LIFT));
        assert!(
            mark >= -units(BAR_LIFT),
            "raised with the bar and no further: {mark}"
        );
        assert_eq!(word, -units(BAR_LIFT) - units(WORD_LIFT));
    }

    fn spaces_named(machine: &str) -> String {
        let (_dir, first, _) = projects();
        let start: Starter = Box::new(|root, size| Workspace {
            app: App::new(root, size),
            server: None,
        });
        let mut spaces = Workspaces::new((100, 30), machine.to_string(), start);
        spaces.open(first);
        bar(&spaces)
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
    fn what_needs_you_anywhere_reaches_the_terminal_only_while_cellix_is_behind() {
        let (_dir, first, second) = projects();
        let mut spaces = spaces();
        spaces.open(first);
        spaces.open(second);
        // In front, the person sees it: the inbox has it, the terminal isn't told.
        spaces.list[1]
            .app
            .serve(event(AgentState::NeedsYou, "Allow sqlx migrate run?"));
        spaces.poll();
        assert!(spaces.notifications.is_empty());
        assert!(spaces.list[1].app.inbox.has("ask:s1"));

        // Behind another window, news from a workspace in the back is told, once.
        spaces.set_focused(false);
        spaces.list[1]
            .app
            .serve(event(AgentState::Working, "Working"));
        assert!(!spaces.list[1].app.inbox.has("ask:s1"), "answered");
        spaces.list[1]
            .app
            .serve(event(AgentState::NeedsYou, "Allow cargo publish?"));
        spaces.poll();
        assert_eq!(
            std::mem::take(&mut spaces.notifications),
            vec![(
                "Cellix · infra".to_string(),
                "Allow cargo publish?".to_string()
            )]
        );
        spaces.poll();
        assert!(spaces.notifications.is_empty(), "one per event");

        // More within the gap waits, and goes as one when it's over.
        spaces.list[1]
            .app
            .serve(event(AgentState::Idle, "Finished"));
        spaces.list[0]
            .app
            .serve(Request::Test(cellix_agent::TestRun {
                command: "cargo test".to_string(),
                passed: false,
                output: "test refunds ... FAILED".to_string(),
            }));
        spaces.poll();
        assert!(spaces.notifications.is_empty(), "rate-limited");
        let inbox = &spaces.list[1].app.inbox.items;
        assert_eq!(inbox.len(), 1, "finished replaces asking: {inbox:?}");
        assert_eq!(inbox[0].title, "Claude Code finished its turn");
        assert_eq!(spaces.list[0].app.inbox.items[0].title, "cargo test failed");

        // Coming back, what wasn't told yet isn't told at all.
        spaces.set_focused(true);
        spaces.set_focused(false);
        spaces.list[1]
            .app
            .serve(event(AgentState::Working, "Working"));
        spaces.poll();
        assert!(spaces.notifications.is_empty());

        // Turned off in Settings, nothing goes.
        spaces.list[0].app.settings.notify = Some(false);
        spaces.list[1]
            .app
            .serve(event(AgentState::NeedsYou, "Allow rm -rf target?"));
        spaces.notifier = Notifier::default();
        spaces.poll();
        assert!(spaces.notifications.is_empty());
        assert!(
            spaces.list[1].app.inbox.has("ask:s1"),
            "the inbox still has it"
        );
    }

    /// The row on screen holding `text`, and the column it starts at.
    fn find_text(grid: &Grid, text: &str) -> (u16, u16) {
        (0..grid.height())
            .find_map(|y| {
                let row = grid.row_text(y);
                let at = row.find(text)?;
                Some((row[..at].chars().count() as u16, y))
            })
            .unwrap_or_else(|| panic!("no {text} in\n{}", grid.text()))
    }

    #[test]
    fn the_inbox_lists_what_waits_everywhere_and_a_click_goes_there() {
        let (_dir, first, second) = projects();
        let mut spaces = spaces();
        spaces.open(first);
        spaces.open(second);
        spaces.handle(Event::Resize(140, 36));
        // The bell at the right end of the status bar opens it. Empty, it says what would
        // fill it, and what to do meanwhile.
        // The workspaces lead the status bar's row, and the bell still ends what it says,
        // before the button that quits.
        let status = spaces.size().1 - 1;
        let row = screen(&spaces).row_text(status);
        let said = row
            .trim_end()
            .strip_suffix("Quit")
            .expect("quit ends the row");
        assert!(said.trim_end().ends_with('◬'), "{row}");
        let bell = (spaces.app().status_bell().unwrap().0, status);
        click(&mut spaces, MouseKind::Left, bell);
        spaces.poll();
        let text = screen(&spaces).text();
        assert!(text.contains("Nothing is waiting for you"), "{text}");
        assert!(text.contains("A plan is ready for you to review"), "{text}");
        assert!(text.contains("Notification Settings"), "{text}");

        // An agent asks in the workspace behind, then tests fail in this one.
        spaces.list[1]
            .app
            .serve(event(AgentState::NeedsYou, "Allow sqlx migrate run?"));
        std::thread::sleep(std::time::Duration::from_millis(5));
        spaces.list[0]
            .app
            .serve(Request::Test(cellix_agent::TestRun {
                command: "cargo test".to_string(),
                passed: false,
                output: "test refunds ... FAILED".to_string(),
            }));
        spaces.poll();
        let grid = screen(&spaces);
        let text = grid.text();
        assert!(text.contains("Inbox 2"), "the tab counts them: {text}");
        let bell = grid.row_text(status);
        let said = bell.trim_end().strip_suffix("Quit").unwrap_or(&bell);
        assert!(said.trim_end().ends_with("◬ 2"), "and the bell: {bell}");
        assert!(text.contains("2 waiting in 2 workspaces"), "{text}");
        let (_, failed) = find_text(&grid, "cargo test failed");
        let (x, asking) = find_text(&grid, "Allow sqlx migrate run?");
        assert!(failed < asking, "newest first");
        assert!(
            grid.row_text(asking + 1).contains("infra"),
            "where it waits"
        );

        // Under the pointer, a row says what a click does.
        spaces.handle(Event::Mouse(MouseEvent {
            kind: MouseKind::Move,
            x,
            y: asking,
            fine_x: f32::from(x),
            fine_y: f32::from(asking) + 0.5,
            mods: cellix_input::Mods::default(),
        }));
        assert!(screen(&spaces).row_text(asking).contains("Show Prompt"));

        // A click goes to its workspace, and with no session in the pane, to the agents list.
        click(&mut spaces, MouseKind::Left, (x, asking));
        assert_eq!(spaces.active, 1);
        assert!(spaces.app().showing_agents_for_test());
        assert!(
            spaces.list[1].app.inbox.has("ask:s1"),
            "it waits until answered"
        );

        // From there, the × on the other workspace's row lets it go.
        spaces.app_mut().run(Command::ShowInbox);
        spaces.poll();
        let grid = screen(&spaces);
        let (_, failed) = find_text(&grid, "cargo test failed");
        click(&mut spaces, MouseKind::Move, (grid.width() - 4, failed));
        click(&mut spaces, MouseKind::Left, (grid.width() - 4, failed));
        assert!(spaces.list[0].app.inbox.items.is_empty());
        assert!(!screen(&spaces).text().contains("cargo test failed"));
        assert!(screen(&spaces).text().contains("Inbox 1"));
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

    /// The empty field's hint says Tab goes into a folder, and the hint goes with the first
    /// letter typed: someone who typed a name saw the folder they meant in the list, and
    /// nothing saying how to carry on into it. The field's far end says what the keys do
    /// with the folder picked, for as long as something is typed.
    #[test]
    fn the_folder_finder_says_what_tab_and_enter_do_while_a_name_is_typed() {
        let (dir, first, _second) = projects();
        let long = "a".repeat(45);
        fs::create_dir_all(dir.path().join("workplace/app").join(&long)).unwrap();
        let mut spaces = spaces();
        spaces.open(first);
        spaces.app_mut().home = Some(dir.path().to_path_buf());
        spaces.app_mut().run(Command::OpenWorkspace);
        let goes_in = crate::palette::GOES_IN;
        // Nothing typed: the hint says it, and the field's end doesn't say it again.
        let text = screen(&spaces).text();
        assert!(text.contains("Tab goes into it"), "{text}");
        assert!(!text.contains(goes_in), "{text}");
        // A name finds the folder among the suggestions. The hint has gone, and the field
        // says what the keys do with the folder picked.
        type_text(&mut spaces, "work");
        let text = screen(&spaces).text();
        assert!(text.contains("~/workplace"), "{text}");
        assert!(text.contains(goes_in), "{text}");
        // Tab goes in. The folder itself is picked now, and Tab has nowhere to take it.
        key(&mut spaces, Key::Tab, Mods::default());
        let text = screen(&spaces).text();
        assert!(text.contains("~/workplace/app"), "{text}");
        assert!(text.contains("Enter opens it"), "{text}");
        assert!(!text.contains("Tab goes into it"), "{text}");
        // A folder inside it, picked, can be gone into in turn.
        key(&mut spaces, Key::Down, Mods::default());
        assert!(screen(&spaces).text().contains(goes_in));
        key(&mut spaces, Key::Tab, Mods::default());
        let text = screen(&spaces).text();
        assert!(text.contains("~/workplace/app/"), "{text}");
        assert!(text.contains("Enter opens it"), "{text}");
        // What's typed comes first: a path as long as the field, with its folder picked
        // under it, leaves no room to say anything.
        type_text(&mut spaces, &long[..40]);
        let text = screen(&spaces).text();
        assert!(text.contains(&long[..20]), "{text}");
        assert!(!text.contains("opens it"), "{text}");
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
        // The button and the tile's word are gone. The status bar's own words are in the
        // row now, and may say what was resumed.
        let row = bar(&spaces);
        assert!(!row.contains("Resume 1 Session"), "{row}");
        assert!(!row.contains("infra stopped"), "{row}");
        let _ = fs::remove_dir_all(crate::hold::folder_of(&second));
    }

    /// The first screen had no way out at all: no button, and no key but those that pick.
    #[test]
    fn the_first_screen_can_be_quit_and_leaves_every_workspace_as_it_is() {
        let (_dir, first, second) = projects();
        let mut spaces = spaces();
        spaces.open(first);
        spaces.open(second);
        spaces.choosing = true;
        spaces.fit();
        let grid = screen(&spaces);
        let row = (0..30)
            .find(|&y| grid.row_text(y).contains("Open"))
            .expect("the row under the list");
        let quit = spot(&grid, row, "Quit");
        let foot = spaces.size.1 - 1;
        assert!(
            grid.row_text(foot).trim_end().ends_with("Quit"),
            "and at the end of the foot, where every other screen has it"
        );
        assert!(!grid.row_text(0).contains('×'), "nothing in the corner");
        assert!(grid.text().contains("Ctrl+Q to quit"), "{}", grid.text());

        click(&mut spaces, MouseKind::Left, quit);
        assert!(spaces.quit, "the word quits");
        assert_eq!(spaces.list.len(), 2, "and closes nothing");
        assert!(spaces.choosing);

        spaces.quit = false;
        let button = spaces.quit_button().expect("a window this wide has it");
        let corner = (button.x + 1, button.y);
        assert_eq!(button.y, foot);
        click(&mut spaces, MouseKind::Move, corner);
        assert!(screen(&spaces).text().contains("stays as it is"));
        click(&mut spaces, MouseKind::Left, corner);
        assert!(spaces.quit, "the button at the foot quits");

        spaces.quit = false;
        let ctrl = Mods {
            ctrl: true,
            ..Mods::default()
        };
        key(&mut spaces, Key::Char('q'), ctrl);
        assert!(
            spaces.quit,
            "and so does the key that quits everywhere else"
        );

        // Opening one still works from the row's other end.
        spaces.quit = false;
        click(&mut spaces, MouseKind::Left, spot(&grid, row, "Open"));
        assert!(!spaces.choosing && !spaces.quit);
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
