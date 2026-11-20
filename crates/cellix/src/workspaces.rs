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
/// How far before the mark's room a spinner or dot starts, in cells: it's wider than the
/// room, and stands centred on it.
const MARK_LEAD: f32 = 0.15;
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
/// workspaces and the machine's name, or fewer than its words take at their fullest
/// (`App::status_full_room`), the bar has a row to itself under the status bar.
const STATUS_ROOM: f32 = 72.0;
/// Between the button that opens a workspace and what the status bar says after it, where
/// the two share a row. A rule stands in the middle of it.
const AFTER: f32 = 2.0;
/// The button that opens another folder beside this one, and what its tip says.
/// How far the status bar's words are lowered in their row, and the workspaces raised in
/// theirs, where the foot is the two of them on rows with room, in rows. The rows stand
/// evenly apart, but what's drawn in the status bar is 30 px of its 50 and a chip is 37, so
/// there were 7 to 9 px more between the two than over the first or under the second.
const PAIR: (f32, f32) = (0.04, 0.06);
const NEW_WORKSPACE: &str = "New Workspace";
/// The first screen's buttons: the way out, beside the one that opens the picked workspace.
const QUIT: &str = "Quit";
const OPEN: &str = "Open";
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
    /// A session on screen printed, and something out of sight changed, since the loop asked.
    printed: bool,
    behind: bool,
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
            printed: false,
            behind: false,
        }
    }

    /// The terminal said Cellix came to the front, or went behind another window.
    pub fn set_focused(&mut self, focused: bool) {
        self.focused = focused;
        // Behind another window, a caret stays shown and nothing wakes for its blink.
        for workspace in &mut self.list {
            workspace.app.set_window_focused(focused);
        }
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
    /// bell. That's wherever both fit: the status bar keeps a message's room, and the room its
    /// words take at their fullest, a server's wait and a file's details, as they're drawn. A
    /// window too narrow, or with too many workspaces, keeps the bar a row of its own under
    /// the status bar. With the 72 columns alone, tiles narrower once agents outside Cellix
    /// weren't counted gave the bar the row, and "Spaces: 4" went while a server fetched.
    fn one_bar(&self) -> bool {
        if !self.shows_bar() {
            return false;
        }
        let lead = self.spans().1 + self.new_width(false) + AFTER;
        // The machine's name and Resume, as `shared` takes them from the status bar, and
        // after the bell the button that quits. Resume wasn't counted, and took the room the
        // file's details had.
        let quit = f32::from(crate::app::quit_room(self.size));
        let resume = self
            .resume_label()
            .map_or(0.0, |label| self.resume_width(&label) + 1.5);
        let tail = self.text_width(&self.machine) + 2.0 + resume + quit;
        let full = f32::from(self.app().status_full_room());
        f32::from(self.size.0) - lead - tail >= STATUS_ROOM.max(full)
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
        let mut workspace = (self.start)(root, size);
        workspace.app.set_window_focused(self.focused);
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

    /// Gives every app the size it has now. Says whether any changed.
    fn fit(&mut self) -> bool {
        let (width, height) = self.app_size();
        let mut changed = false;
        for workspace in &mut self.list {
            if workspace.app.size() != (width, height) {
                workspace.app.handle(Event::Resize(width, height));
                changed = true;
            }
        }
        changed
    }

    pub fn handle(&mut self, event: Event) {
        // Once the last workspace has quit there's no app to hand anything to: what came in
        // the same read as the quit, as a click's release after the press that quit, goes
        // nowhere. It went to the app in front, which was gone, and Cellix panicked.
        if self.list.is_empty() {
            return;
        }
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

    /// What the tick can change on screen in every workspace, as one number: the bar shows
    /// each of them. See `App::tick_mark`.
    pub fn tick_mark(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut mark = std::collections::hash_map::DefaultHasher::new();
        (self.active, self.list.len(), self.choosing).hash(&mut mark);
        for workspace in &self.list {
            workspace.app.tick_mark().hash(&mut mark);
        }
        mark.finish()
    }

    /// Whether a session on screen printed since this was last asked, and whether anything
    /// out of sight changed: sessions behind others, and workspaces not in front.
    pub fn take_printed(&mut self) -> (bool, bool) {
        (
            std::mem::take(&mut self.printed),
            std::mem::take(&mut self.behind),
        )
    }

    /// The soonest a program's synchronized update runs out of time, in any workspace.
    pub fn update_deadline(&self) -> Option<std::time::Instant> {
        self.list
            .iter()
            .filter_map(|w| w.app.update_deadline())
            .min()
    }

    /// Whether anything in any workspace is timed from a moment just past (`App::soon`), or
    /// a notification waits for its gap: the loop looks as often as it did while one is.
    pub fn soon(&self) -> bool {
        self.notifier.waiting() || self.list.iter().any(|w| w.app.soon())
    }

    /// Lets every app take what came in, and answers the requests that brings. Returns
    /// whether anything changed.
    /// What changes in a workspace not in front is out of sight but for its tile in the bar,
    /// which the tick brings up to date.
    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        for (index, workspace) in self.list.iter_mut().enumerate() {
            workspace.app.in_front = index == self.active;
            let mut here = workspace.app.poll();
            if let Some(server) = &workspace.server {
                while let Ok(request) = server.requests.try_recv() {
                    workspace.app.serve(request);
                    here = true;
                }
            }
            let (printed, behind) = workspace.app.take_printed();
            if index == self.active {
                changed |= here;
                self.printed |= printed;
                self.behind |= behind;
            } else {
                self.behind |= here || printed || behind;
            }
        }
        changed |= self.notify();
        let count = self.list.len();
        let active = self.active;
        self.settle();
        // News changes what the tiles say, and with it whether the bar shares the status bar's
        // row: fitted only on the next key or tick, a frame drew the tiles' own row over the
        // status bar.
        if !self.list.is_empty() {
            changed |= self.fit();
        }
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
            Some(WorkspaceRequest::OpenAndStart(root, program)) => {
                let index = self.open(root);
                self.switch(index);
                self.list[index].app.open_agent(program);
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
            let (mark, word, _) = self.word(&workspace.app);
            let mut width =
                self.room(PAD) + self.mark_room(mark) + self.text_width(&name_of(&workspace.app));
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
            TileState::Working => (Mark::Spin(theme.accent), "working".into(), dim),
            // The marks a tab shows and only those: green for a finished turn, where the tile
            // had a check; and nothing where a tab has nothing to say, where an idle terminal's
            // tile had a dark dot and an empty one a ring, which no tab has.
            TileState::Done(at) => (Mark::Dot(theme.git_added), format!("done {}", ago(at)), dim),
            // Stopped, its dot is dimmed, as an ended program's tab dims its mark: with none,
            // it stood beside chips that each had one (0.11.0's final review).
            TileState::Stopped(_) => (Mark::Dot(theme.disabled), "stopped".into(), dim),
            TileState::Idle | TileState::Nothing => (Mark::None, String::new(), dim),
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
            // With a row of its own under the status bar, the foot is those two rows, and
            // what's left of the window under them is shared out round both.
            grid.foot = if self.one_bar() { 1 } else { 2 };
            // Lifted within its row, for air between the bar and the window's edge, where
            // the pixel tiers can place it. On a row with room, a chip is tall enough for its
            // words to stand in its middle and the row has air under it: nothing is lifted.
            let from = grid.painted_so_far();
            self.draw_bar(grid);
            if crate::measures::roomy(self.measure().as_ref()) {
                if !self.one_bar() {
                    // Two rows at the foot read as a pair: as much between what's drawn in
                    // them as over the first and under the second.
                    let to = grid.painted_so_far();
                    let lower = |grid: &mut Grid, (from, to): (usize, usize), by: f32| {
                        let to = to.min(grid.paint.len());
                        for paint in grid.paint.iter_mut().take(to).skip(from) {
                            paint.lift(-cellix_render::units(by));
                        }
                    };
                    lower(grid, self.app().status_paint.get(), PAIR.0);
                    lower(grid, (from, to), -PAIR.1);
                }
                self.draw_new_tip(grid, self.size.1.saturating_sub(1));
                return;
            }
            self.draw_new_tip(grid, self.size.1.saturating_sub(1));
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
                        Paint::Spinner { .. } => true,
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
            // On the accent the spinner is in the chip's ink, and every dot keeps its own
            // colour in a ring of the ink 2 px wide, so it reads as what it says: plain, the
            // needs-you dot stood on the accent at 1.23:1 (1.09:1 light), and the others were
            // the ink's, telling nothing apart (0.11.0's release review).
            let shown = match (ink, mark) {
                (Some(ink), Mark::Dot(color)) => Mark::Ringed(color, ink),
                (Some(ink), Mark::Spin(_)) => Mark::Spin(ink),
                (_, mark) => mark,
            };
            self.draw_mark(grid, (x, y), shown, base);
            x += self.mark_room(mark);
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
            // A quiet button, as every one that isn't the main action is.
            let fill = theme.secondary(base.bg, on_it);
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
        // A quiet button, with a hover that shows: it took a picked row's wash, and a menu's
        // hover under the pointer, which on the frame were 1.07 to 1 apart.
        let fill = theme.secondary(base.bg, on_it);
        grid.panel(chip, CHIP_RADIUS, Some(fill), None);
        let right = self.size.0.saturating_sub(1);
        let mark = start + self.room(PAD);
        let plus = Style {
            fg: theme.accent,
            ..base
        };
        // In the middle of its room, which alone is the middle of the chip: a tenth of a
        // column in, it stood 31 px from the chip's left and 24 from its right.
        let font = cellix_render::Font {
            size: 120,
            ..cellix_render::Font::default()
        };
        let wide = self.measure().width("+", font);
        let at = if self.measure().in_cells() {
            mark
        } else {
            mark + (self.room(MARK) - wide) / 2.0
        };
        grid.label_at(at, mark as u16, y, "+", plus, right, 120);
        let text = Style {
            fg: theme.text.fg,
            ..base
        };
        let x = mark + self.room(MARK) + self.room(GAP);
        // Its mark alone where the workspaces lead the status bar's row: the tip names it.
        if !self.one_bar() {
            label(grid, (x, y), NEW_WORKSPACE, text, right);
        }
    }

    /// The tip over the button that opens a workspace, while the pointer is on it. Drawn
    /// after the bar, and over it.
    fn draw_new_tip(&self, grid: &mut Grid, y: u16) {
        let theme = self.theme();
        let (start, end) = self.new_button();
        let on_it = self.hover.is_some_and(|(x, hy)| {
            let x = f32::from(x) + 0.5;
            self.on_bar(hy) && x >= start && x < end
        });
        // The tip stays out of the way of what the button opened.
        if !on_it || self.measure().in_cells() || self.app().overlay_is_open() {
            return;
        }
        let font = Font {
            size: TIP_SIZE,
            ..Font::default()
        };
        let wide = self.measure().width(NEW_WORKSPACE_TIP, font) + 2.0;
        let left = start.min(f32::from(self.size.0) - wide - 1.0).max(0.0);
        // The row above the bar: a tip covers a row whole, never part of one. On a row with
        // room it's as tall as its words' ink and `LINE_AIR` over and under it make it
        // (`measures::tip_box`), at the top of its row with its words, so its shadow ends
        // inside the row, as the app's tips over the bar stand.
        let roomy = crate::measures::roomy(self.measure().as_ref());
        let row = f32::from(y) - 1.0;
        let tip = crate::measures::tip_box(self.measure().as_ref(), TIP_SIZE);
        let pixels = self.measure().cell_pixels().map(|(_, row)| row as f32);
        let painted = grid.painted_so_far();
        let top = match (tip, pixels) {
            (Some((_, tall)), Some(pixels)) => {
                // Where the foot is one row, the row it's on is a sheet's, with the sheets' line
                // at its foot: it's no taller than keeps it `LINE_AIR` over that line.
                let measure = self.measure();
                let line = crate::measures::line_pixels(measure.as_ref());
                let room = pixels - line - crate::measures::LINE_AIR;
                let tall = if self.one_bar() { tall.min(room) } else { tall };
                let mut bounds = Bounds::cells(left, row, wide, 0.0);
                let unit = cellix_render::UNIT as f32;
                bounds.h = (tall * unit / pixels).round() as i32;
                grid.exact_popup(bounds, CHIP_RADIUS, theme.popup, theme.popup_border);
                // Its words in its middle.
                row - ((pixels - tall) / 2.0).round() / pixels
            }
            _ => {
                let inset = crate::app::TIP_INSET;
                let bounds = Bounds::cells(left, row + inset, wide, 1.0 - inset * 2.0);
                grid.popup(bounds, CHIP_RADIUS, theme.popup, theme.popup_border);
                row
            }
        };
        // Kept to the rows over the bar: see the tips the app draws there. Raised, it's kept
        // to its own row: the row over that one may not be drawn where this one is.
        let first = if roomy { y.saturating_sub(1) } else { 0 };
        grid.keep_to_rows(painted, first, y);
        let colour = (theme.popup_dim, false);
        grid.text_run(
            (left + 1.0, top),
            left + wide,
            NEW_WORKSPACE_TIP,
            colour,
            font,
        );
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
        let measure = self.measure();
        let open = crate::measures::button_cells(OPEN, measure.as_ref());
        let quit = crate::measures::button_cells(QUIT, measure.as_ref());
        let x = left + open.chars().count() as u16 + crate::measures::BUTTON_GAP;
        Rect::new(x, self.open_all_row(), quit.chars().count() as u16, 1)
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
        grid.row_air = (self.measure().row_air() * 100.0).round() as u16;
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
            // The marks a tile and a tab show: a spinner while it works, where this was a
            // still blue dot.
            let (mark, word, color) = self.word(&workspace.app);
            self.draw_mark(grid, (f32::from(left), y), mark, base);
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
            let root = tilde(workspace.app.workspace.root());
            let path = crate::palette::path_cut_at(&root, room, BAR_SIZE, &*self.measure());
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
        // Open the picked one, and the way out beside it: the buttons every screen has, the
        // main one filled and the other quiet, where they were words in the accent and grey.
        let y = self.open_all_row();
        let measure = self.measure();
        // Where there are pictures the cells keep the ground and the chip is painted round
        // over them: in its fill they stood as a square block behind it.
        let cells = measure.in_cells();
        let ground = |fill| if cells { fill } else { base.bg };
        let open = crate::measures::button_cells(OPEN, measure.as_ref());
        let filled = Style {
            fg: theme.button_text,
            bg: ground(theme.button),
            ..base
        };
        grid.button(left, y, &open, OPEN, theme.button, filled);
        // The way out, beside it and at the end of the foot, where it is on every other
        // screen.
        let leaving = self.hover.is_some_and(|(x, y)| self.on_quit(x, y));
        let quit = self.quit_link();
        let quiet = theme.secondary(base.bg, leaving);
        let style = Style {
            fg: theme.text.fg,
            bg: ground(quiet),
            ..base
        };
        let cells = crate::measures::button_cells(QUIT, measure.as_ref());
        grid.button(quit.x, quit.y, &cells, QUIT, quiet, style);
        if let Some(cell) = self.quit_button() {
            let on_it = self.hover.is_some_and(|(x, y)| cell.contains(x, y));
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
}

/// What a workspace's chip shows before its name: the marks a tab shows for what its agents
/// are doing, a spinner while they work, a dot in the colour of what needs the person and a
/// green one for a finished turn, a dimmed one once it stopped, and nothing where a tab would
/// show nothing.
#[derive(Clone, Copy, PartialEq)]
enum Mark {
    Dot(Rgb),
    Spin(Rgb),
    /// A dot standing in a ring of the second colour: any dot on a filled chip.
    Ringed(Rgb, Rgb),
    None,
}

/// How much bigger the ring round a dot on a filled chip is than the dot, across: 2 px either
/// side of a dot of about 11 at the tour's size.
const RING: f32 = 1.37;

impl Workspaces {
    /// A tile's mark at `x`: the spinner and dots at the size a spinner beside the status bar's
    /// words is, a little before where a word would start, so as much air stands between
    /// them and the name as between the chip's edge and them.
    fn draw_mark(&self, grid: &mut Grid, (x, y): (f32, u16), mark: Mark, base: Style) {
        let size = crate::measures::WAIT_SPIN;
        let at = x - MARK_LEAD;
        // How wide a mark `size` rows tall is, in columns.
        let measure = self.measure();
        let across = |size: f32| {
            size * crate::measures::rows_to_columns(measure.as_ref()) / measure.row_air()
        };
        match mark {
            Mark::Spin(color) => grid.spinner(at, y, size, color, base),
            Mark::Dot(color) => grid.dot(at, y, size, color, base),
            Mark::Ringed(color, ring) => {
                let wider = (across(size * RING) - across(size)) / 2.0;
                grid.dot(at - wider, y, size * RING, ring, base);
                grid.dot(at, y, size, color, base);
            }
            Mark::None => {}
        }
    }

    /// The columns a tile's mark takes before its name, with the air after it: none where
    /// it has none, so its name has the chip's padding before it as at its other end.
    fn mark_room(&self, mark: Mark) -> f32 {
        if mark == Mark::None {
            0.0
        } else {
            self.room(MARK) + self.room(GAP)
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

    /// A tile has the marks a tab has and only those: a green dot for a finished turn, where
    /// it had a check, and none where a tab has none, idle, empty or stopped, where it had a
    /// dark dot, a ring and a square. The first screen shows the same marks, a spinner where
    /// it had a still dot, and Open and Quit as the buttons every screen has, not words.
    #[test]
    fn a_tile_and_the_first_screen_have_the_tabs_marks_and_the_shared_buttons() {
        use cellix_render::{FontMeasure, Paint, UNIT};
        let (_dir, first, second) = projects();
        let mut spaces = spaces();
        spaces.open(first);
        spaces.open(second);
        spaces.handle(Event::Resize(140, 30));
        spaces
            .app_mut()
            .set_measure(Rc::new(FontMeasure::new(17, 36)));
        let theme = *spaces.theme();
        let foot = i32::from(spaces.size().1 - 1) * UNIT - UNIT / 2;
        let marks = |spaces: &Workspaces| -> Vec<(Rgb, bool)> {
            let grid = screen(spaces);
            let found = grid.paint.iter().filter_map(|p| match p {
                Paint::Spinner {
                    bounds,
                    color,
                    step,
                } if bounds.y > foot => Some((*color, step.is_some())),
                // The check and the square tiles had; a tile's × and the status bar's icons
                // aren't marks.
                Paint::Icon {
                    bounds,
                    color,
                    icon: Icon::Check | Icon::Stop,
                } if bounds.y > foot => Some((*color, false)),
                _ => None,
            });
            found.collect()
        };
        // Nothing running: no mark on either tile, nor a letter standing for one.
        assert!(marks(&spaces).is_empty(), "nothing to say");
        let row = bar(&spaces);
        for letter in ['○', '✓', '■'] {
            assert!(!row.contains(letter), "{row}");
        }
        // Behind, a finished turn: the green dot a tab has, still. Its agent has a tab there:
        // one outside Cellix isn't counted.
        let behind = 1 - spaces.active;
        spaces.list[behind].app.tab_for("Claude Code", "s1");
        spaces.list[behind]
            .app
            .serve(event(AgentState::Working, "Working"));
        spaces.list[behind]
            .app
            .serve(event(AgentState::Idle, "Finished"));
        assert_eq!(marks(&spaces), [(theme.git_added, false)]);

        // The first screen: the working one's spinner, and Open filled and Quit quiet.
        spaces.list[behind]
            .app
            .serve(event(AgentState::Working, "Working"));
        spaces.choosing = true;
        spaces.fit();
        let grid = screen(&spaces);
        let turning = grid.paint.iter().any(
            |p| matches!(p, Paint::Spinner { color, step: Some(_), .. } if *color == theme.accent),
        );
        assert!(turning, "a spinner for the one at work");
        let y = spaces.open_all_row();
        let on_row = |fill: Rgb| {
            grid.paint.iter().any(|p| {
                matches!(p, Paint::Panel { bounds, fill: Some(f), .. }
                    if *f == fill && bounds.y / UNIT == i32::from(y))
            })
        };
        assert!(on_row(theme.button), "Open, filled");
        let base = theme.text.bg;
        assert!(on_row(theme.secondary(base, false)), "Quit, quiet");
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
            name: String::new(),
            prompt: String::new(),
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
        spaces.handle(Event::Resize(170, 30));
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

    /// The status bar's words come before the workspaces': wherever a language server's wait
    /// and the file's details wouldn't all fit beside the tiles, the tiles take a row of their
    /// own, where New Workspace has its words, and wherever they share the status bar's, none
    /// of its words is left out. Narrower once agents outside Cellix weren't counted, the
    /// tiles had the row by the 72 columns kept for a message, with Resume and the bell not
    /// counted, and "Spaces: 4" went while rust-analyzer fetched (the review of `next` before
    /// 0.11.1, finding 6). The rows are the same with the wait and without it.
    #[test]
    fn the_status_bars_words_come_before_the_workspaces_row() {
        use cellix_render::{CellMeasure, FontMeasure, Paint, UNIT};
        let (_dir, first, second) = projects();
        fs::write(first.join("queue.rs"), "fn main() {\n    let x = 1;\n}\n").unwrap();
        let mut spaces = spaces();
        spaces.open(first.clone());
        spaces.open(second);
        spaces.active = 0;
        spaces.app_mut().open(&first.join("queue.rs")).unwrap();
        let painted: Rc<dyn Measure> = Rc::new(FontMeasure::new(17, 37));
        let cells: Rc<dyn Measure> = Rc::new(CellMeasure);
        for measure in [painted, cells] {
            for workspace in &mut spaces.list {
                workspace.app.set_measure(measure.clone());
            }
            let (mut shared, mut own) = (0, 0);
            for width in (100..=240).step_by(4) {
                spaces.handle(Event::Resize(width, 30));
                spaces.app_mut().source_settled_for_test();
                let quiet = spaces.one_bar();
                spaces.app_mut().fetching_for_test();
                let one = spaces.one_bar();
                assert_eq!(one, quiet, "{width}: the rows change with the wait");
                let grid = screen(&spaces);
                // What's drawn on the foot's rows, painted or in cells.
                let foot = if one { 29 } else { 28 };
                let words: Vec<String> = if measure.in_cells() {
                    vec![grid.row_text(foot), grid.row_text(29)]
                } else {
                    (grid.paint.iter())
                        .filter_map(|paint| match paint {
                            // Words on the foot are lifted or lowered within their rows.
                            Paint::Text { y, text, .. }
                                if *y >= i32::from(foot) * UNIT - UNIT / 2 =>
                            {
                                Some(text.clone())
                            }
                            _ => None,
                        })
                        .collect()
                };
                let said = |what: &str| words.iter().any(|w| w.contains(what));
                assert!(said("rust-analyzer: fetching"), "{width}: {words:?}");
                assert!(said("Spaces: 4"), "{width}: one bar {one}: {words:?}");
                if one {
                    shared += 1;
                } else {
                    own += 1;
                    assert!(said("New Workspace"), "{width}: {words:?}");
                }
            }
            assert!(shared > 0 && own > 0, "both are seen: {shared} {own}");
        }
    }

    /// Where the bar has a row of its own under the status bar, the grid says its foot is two
    /// rows, so a backend with pixels left under them stands both evenly in what's there.
    /// Sharing the status bar's row it's one, and so is the first screen's.
    #[test]
    fn a_bar_with_a_row_of_its_own_makes_a_foot_of_two_rows() {
        let (_dir, first, second) = projects();
        let mut spaces = spaces();
        spaces.open(first);
        spaces.open(second);
        spaces.handle(Event::Resize(170, 30));
        assert!(spaces.one_bar());
        assert_eq!(screen(&spaces).foot, 1, "one bar is one row");
        spaces.handle(Event::Resize(90, 30));
        assert!(!spaces.one_bar());
        let grid = screen(&spaces);
        assert_eq!(grid.foot, 2, "the status bar's row and the workspaces'");
        assert!(
            grid.row_text(29).contains("New Workspace"),
            "{}",
            grid.row_text(29)
        );
        spaces.choosing = true;
        assert_eq!(
            screen(&spaces).foot,
            1,
            "the first screen has one row there"
        );
    }

    /// On rows with room, the two rows of a foot stand as a pair: the status bar's words a
    /// little lower in their row and the workspaces a little higher in theirs. The rows are
    /// evenly apart, but less of the status bar's row is drawn on than of a chip's, and with
    /// both in the middle of their rows there was more between the two than round them. The
    /// tip over New Workspace stands at the top of its row, with its shadow inside it.
    #[test]
    fn the_two_rows_of_a_foot_stand_as_a_pair() {
        use cellix_render::{FontMeasure, Paint, UNIT, units};
        let (_dir, first, second) = projects();
        let mut spaces = spaces();
        spaces.open(first);
        spaces.open(second);
        spaces.handle(Event::Resize(90, 30));
        spaces.app_mut().set_branch("main");
        let tall: Rc<dyn Measure> = Rc::new(FontMeasure::new(17, 50).with_type_row(36));
        for workspace in &mut spaces.list {
            workspace.app.set_measure(tall.clone());
        }
        spaces.handle(Event::Tick);
        assert!(!spaces.one_bar());
        let (status, bar) = (28 * UNIT, 29 * UNIT);
        let near = |y: i32, row: i32| (y - row).abs() < UNIT / 2;
        let places = |spaces: &Workspaces| {
            let grid = screen(spaces);
            let accent = spaces.theme().accent;
            let chip = grid.paint.iter().find_map(|p| match p {
                Paint::Panel {
                    bounds,
                    fill: Some(fill),
                    ..
                } if *fill == accent && near(bounds.y, bar) => Some(bounds.y - bar),
                _ => None,
            });
            let word = grid.paint.iter().find_map(|p| match p {
                Paint::Text { text, y, .. } if text == "main" && near(*y, status) => {
                    Some(*y - status)
                }
                _ => None,
            });
            (chip.expect("a chip"), word.expect("the branch"))
        };
        let (chip, word) = places(&spaces);
        assert_eq!(chip, units(CHIP_INSET) - units(PAIR.1), "the chips, higher");
        assert_eq!(word, units(PAIR.0), "the status bar's words, lower");
        // The tip over the button, on the status bar's row: at its top, and kept to it.
        let (start, _) = spaces.new_button();
        spaces.handle(Event::Mouse(MouseEvent {
            kind: MouseKind::Move,
            x: start as u16 + 1,
            y: 29,
            fine_x: start + 1.5,
            fine_y: 29.5,
            mods: Mods::default(),
        }));
        let grid = screen(&spaces);
        let shadowed = |p: &Paint| matches!(p, Paint::Panel { shadow: true, .. });
        let tip = grid.paint.iter().find_map(|p| match p {
            Paint::Panel {
                shadow: true,
                bounds,
                ..
            } => Some(bounds.y - status),
            _ => None,
        });
        // Drawn exactly where it's placed (`Grid::exact_popup`), from the row's top.
        assert_eq!(tip, Some(0), "at the top of the status bar's row");
        assert!(grid.paint_on_row(28).any(shadowed));
        assert!(
            !grid.paint_on_row(27).any(shadowed),
            "not on the sheet's row"
        );
        assert!(!grid.paint_on_row(29).any(shadowed), "nor on the bar's");
        // The tip isn't moved with the pair: the chips under it are where they were.
        assert_eq!(places(&spaces).0, units(CHIP_INSET) - units(PAIR.1));
        // Sharing the status bar's row there's one row and nothing to pair.
        spaces.handle(Event::Resize(140, 30));
        assert!(spaces.one_bar());
        let grid = screen(&spaces);
        let accent = spaces.theme().accent;
        let chip = grid.paint.iter().find_map(|p| match p {
            Paint::Panel {
                bounds,
                fill: Some(fill),
                ..
            } if *fill == accent && near(bounds.y, bar) => Some(bounds.y - bar),
            _ => None,
        });
        assert_eq!(chip, Some(units(CHIP_INSET)));
    }

    /// New Workspace is a quiet button, as every action that isn't the main one is, and a step
    /// further off the bar under the pointer. It took a picked row's wash, and a menu's hover
    /// under the pointer, which on the frame were 1.07 to 1 apart.
    #[test]
    fn new_workspace_is_a_quiet_button_that_shows_the_pointer() {
        use cellix_render::Paint;
        let (_dir, first, second) = projects();
        let mut spaces = spaces();
        spaces.open(first);
        spaces.open(second);
        spaces.handle(Event::Resize(90, 30));
        let (start, _) = spaces.new_button();
        let chip = |spaces: &Workspaces| {
            let at = cellix_render::units(start);
            let grid = screen(spaces);
            let fills = grid.paint_on_row(29).filter_map(|p| match p {
                Paint::Panel {
                    bounds,
                    fill: Some(fill),
                    ..
                } if bounds.x == at => Some(*fill),
                _ => None,
            });
            fills.last().expect("New Workspace's chip")
        };
        let theme = *spaces.theme();
        let bar = theme.workspaces.bg;
        assert_eq!(chip(&spaces), theme.secondary(bar, false), "at rest");
        spaces.handle(Event::Mouse(MouseEvent {
            kind: MouseKind::Move,
            x: start as u16 + 1,
            y: 29,
            fine_x: start + 1.5,
            fine_y: 29.5,
            mods: Mods::default(),
        }));
        assert_eq!(chip(&spaces), theme.secondary(bar, true), "pointed at");
    }

    /// A worktree made for an agent opens as a workspace of its own, in front, with the
    /// agent picked started in its side pane.
    #[cfg(unix)]
    #[test]
    fn a_folder_opens_as_a_workspace_with_an_agent_started_in_it() {
        use std::os::unix::fs::PermissionsExt;
        let (dir, first, second) = projects();
        let fake = dir.path().join("fake-agent");
        fs::write(&fake, "#!/bin/sh\nsleep 9\n").unwrap();
        fs::set_permissions(&fake, fs::Permissions::from_mode(0o755)).unwrap();
        let program = fake.display().to_string();
        let start: Starter = Box::new(move |root, size| {
            let mut app = App::new(root, size);
            app.stand_in_for_agents(&program);
            Workspace { app, server: None }
        });
        let mut spaces = Workspaces::new((100, 30), "dev-box".to_string(), start);
        spaces.open(first.clone());
        let request = WorkspaceRequest::OpenAndStart(second.clone(), "codex");
        spaces.app_mut().workspace_request = Some(request);
        spaces.settle();
        assert_eq!(spaces.list.len(), 2);
        assert_eq!(spaces.app().workspace.root(), second, "in front");
        assert!(spaces.app().has_session(), "with the agent started");
        assert!(
            !spaces.list[0].app.has_session(),
            "and nothing in the first"
        );
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
        spaces.handle(Event::Resize(170, 30));
        assert_eq!(spaces.app().size(), (170, 30));
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
        let mut spaces = Workspaces::new((160, 30), "ip-10-0-129-183".to_string(), start);
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
        spaces.handle(Event::Resize(170, 30));
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
        spaces.list[1].app.tab_for("Claude Code", "s1");
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

    /// A tile shows what its agents are doing with the marks their tabs have: the spinner
    /// while one works, the needs-you colour's dot while one asks. On the tile in front, the
    /// accent, the spinner is in the chip's ink and the dot keeps its colour, ringed in the ink.
    #[test]
    fn a_tiles_marks_are_its_tabs() {
        use cellix_render::{FontMeasure, Paint, UNIT};
        let (_dir, first, second) = projects();
        let mut spaces = spaces();
        spaces.open(first);
        spaces.open(second);
        spaces.handle(Event::Resize(140, 30));
        spaces
            .app_mut()
            .set_measure(Rc::new(FontMeasure::new(17, 36)));
        let foot = i32::from(spaces.size().1 - 1) * UNIT - UNIT / 2;
        // The marks on the bar, as (colour, turning, left edge, size) in the order drawn.
        let marks = |spaces: &Workspaces| -> Vec<(Rgb, bool, i32, i32)> {
            let grid = screen(spaces);
            let found = grid.paint.iter().filter_map(|p| match p {
                Paint::Spinner {
                    bounds,
                    color,
                    step,
                } if bounds.y > foot => Some((*color, step.is_some(), bounds.x, bounds.h)),
                _ => None,
            });
            found.collect()
        };
        let theme = *spaces.theme();
        let (front, behind) = (spaces.active, 1 - spaces.active);
        for index in [front, behind] {
            spaces.list[index].app.tab_for("Claude Code", "s1");
        }
        // Behind, working: the accent's spinner; asking: the needs-you dot.
        spaces.list[behind]
            .app
            .serve(event(AgentState::Working, "Working"));
        let shown = marks(&spaces);
        assert!(
            shown.iter().any(|m| m.0 == theme.accent && m.1),
            "{shown:?}"
        );
        spaces.list[behind]
            .app
            .serve(event(AgentState::NeedsYou, "Allow sqlx migrate run?"));
        let shown = marks(&spaces);
        assert!(
            shown.iter().any(|m| m.0 == theme.attention && !m.1),
            "{shown:?}"
        );
        // In front, on the accent: the spinner in the ink, the dot ringed in it.
        spaces.list[front]
            .app
            .serve(event(AgentState::Working, "Working"));
        let ink = theme.button_text;
        assert!(
            marks(&spaces).iter().any(|m| m.0 == ink && m.1),
            "{:?}",
            marks(&spaces)
        );
        spaces.list[front]
            .app
            .serve(event(AgentState::NeedsYou, "Allow cargo publish?"));
        // Its dot keeps its colour in a ring of the ink about 2 px wide: plain, it stood on the
        // accent at 1.23:1 (0.11.0's release review). Both tiles' dots are one size.
        let shown = marks(&spaces);
        let dots: Vec<_> = shown
            .iter()
            .filter(|m| m.0 == theme.attention && !m.1)
            .collect();
        // Both tiles ask, and the status bar's words say so after the same dot.
        assert!(dots.len() >= 2, "both tiles ask: {shown:?}");
        assert!(dots.iter().all(|d| d.3 == dots[0].3), "one size: {shown:?}");
        let ringed = |colour: Rgb, shown: &[(Rgb, bool, i32, i32)]| {
            // The ring's middle is the dot's, and it stands about 2 px out from it: a dot's
            // ink is half its square, which is as tall as its bounds, at 36 px a row.
            let middle = |m: &(Rgb, bool, i32, i32)| m.2 * 2 + m.3 * 36 / 17;
            let rings = shown.iter().filter(|r| r.0 == ink && !r.1);
            let pair = rings.into_iter().find_map(|ring| {
                let dot = shown.iter().find(|m| {
                    m.0 == colour && !m.1 && m.3 < ring.3 && (middle(ring) - middle(m)).abs() <= 2
                });
                dot.map(|dot| (ring, dot))
            });
            let (ring, dot) =
                pair.unwrap_or_else(|| panic!("{colour:?} ringed in the ink: {shown:?}"));
            let px = |h: i32| h as f32 / UNIT as f32 * 36.0 / 2.0;
            let width = (px(ring.3) - px(dot.3)) / 2.0;
            assert!((1.5..=2.5).contains(&width), "a ring {width} px wide");
        };
        ringed(theme.attention, &shown);
        // So is every other dot on the chip in front: a finished turn's keeps its green.
        spaces.list[front]
            .app
            .serve(event(AgentState::Working, "Working"));
        spaces.list[front]
            .app
            .serve(event(AgentState::Idle, "Finished"));
        let shown = marks(&spaces);
        ringed(theme.git_added, &shown);
    }

    #[test]
    fn what_needs_you_anywhere_reaches_the_terminal_only_while_cellix_is_behind() {
        let (_dir, first, second) = projects();
        let mut spaces = spaces();
        spaces.open(first);
        spaces.open(second);
        // In front, the person sees it: the inbox has it, the terminal isn't told.
        spaces.list[1].app.tab_for("Claude Code", "s1");
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
        spaces.handle(Event::Resize(170, 36));
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

        // An agent asks in the workspace behind, from its tab there, then tests fail in this
        // one.
        spaces.list[1].app.tab_for("Claude Code", "s1");
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
        // Both tiles say they need you now, and they have a row of their own under the status
        // bar's, which keeps the bell.
        assert!(!spaces.one_bar());
        let bell = grid.row_text(status - 1);
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

        // A click goes to its workspace, and brings its agent's tab forward.
        click(&mut spaces, MouseKind::Left, (x, asking));
        assert_eq!(spaces.active, 1);
        assert_eq!(spaces.app().front_tab_for_test(), Some(0));
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

    /// What comes in the same read as the quit, after it, goes nowhere: the release of the
    /// click on Stop and Quit, a key, the tick, a resize. It went to the app in front, which
    /// had gone with the last workspace, and Cellix panicked as it quit.
    #[test]
    fn what_comes_after_the_last_workspace_quits_goes_nowhere() {
        let (_dir, first, _) = projects();
        let mut spaces = spaces();
        spaces.open(first.clone());
        let ctrl = Mods {
            ctrl: true,
            ..Mods::default()
        };
        key(&mut spaces, Key::Char('q'), ctrl);
        assert!(
            spaces.quit && spaces.list.is_empty(),
            "nothing to ask: it quits"
        );
        click(&mut spaces, MouseKind::Up, (10, 10));
        key(&mut spaces, Key::Char('x'), Mods::default());
        spaces.handle(Event::Tick);
        spaces.handle(Event::Resize(80, 24));
        spaces.poll();
        assert!(spaces.quit && spaces.list.is_empty());
        let _ = fs::remove_dir_all(crate::hold::folder_of(&first));
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
            member: None,
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
        // Its tile has a mark as every other tile with something to say has: the dot, dimmed
        // as an ended tab's mark is. It had none (0.11.0's final review).
        let disabled = spaces.theme().disabled;
        let y = (0..grid.height()).find(|y| grid.row_text(*y).contains("infra stopped"));
        let y = y.expect("the bar");
        let dimmed = (0..grid.width()).any(|x| {
            let cell = grid.cell(x, y);
            cell.style.fg == disabled && cell.symbol == "●"
        });
        assert!(dimmed, "a dimmed dot: {row}");
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
