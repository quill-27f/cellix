use std::borrow::Cow;

use std::path::PathBuf;

use cellix_doc::table::Edit as TableEdit;
use cellix_input::{Command, Key};
use cellix_render::{Bounds, Font, Grid, Icon, Measure, Paint, Rect, Rgb, Style, units};

use crate::palette::{keys_of, popup, raised};
use crate::theme::Theme;
use crate::toolbar::Tool;

/// Space from the menu's edge to its text: two columns, as text stands inside whatever
/// holds it.
const INSET: f32 = 2.0;
/// Space between a label and its keys.
const GAP: f32 = 3.0;
const KEYS_SIZE: u8 = 85;
/// How far the keys are dropped in their row, in rows, to share the words' baseline.
const KEYS_DROP: f32 = 0.035;
const MIN_WIDTH: u16 = 16;
/// Air above the first row and below the last, in rows, the same in every menu: half a row,
/// 25 px of an airy row. Painted menus only; the cell tier has no fractions of a row. It was
/// a quarter, with what rounding to whole rows left over shared above and below, so a menu
/// had 26 px from its edge to its words or 39 by how many rules it had.
const PADDING: f32 = 0.5;
/// What a rule between groups takes at least, in rows, so a hover band never meets it. Where
/// the rules leave half a row over, they share it: the edges keep their air.
const RULE_ROOM: f32 = 0.5;
/// The hover band inside its row: 26 px of 32 tall, and a column in from each side, as a
/// wash is from a panel's edge.
const BAND_HEIGHT: f32 = crate::measures::BAND_HEIGHT;
const BAND_INSET: f32 = 1.0;
/// Columns a row's icon takes before its words, where any row of the menu has one: the
/// icon's two and some air, about what the tree leaves between a file's icon and its name.
const ICON_COLUMNS: f32 = 2.4;
/// Space between a row's words and the quieter words after them, and their size.
const DETAIL_GAP: f32 = 1.5;
const DETAIL_SIZE: u8 = 90;
/// A heading's size: a section's, small bold capitals.
const HEADING_SIZE: u8 = 80;

/// What a row of a menu does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    Run(Command),
    /// Lists the Comments pane in another order.
    SortComments(crate::threads::Sort),
    /// One of the language server's fixes, by its place in the list it sent.
    ServerFix(usize),
    NewFile(PathBuf),
    NewFolder(PathBuf),
    Rename(PathBuf),
    Delete(PathBuf),
    CopyPath(PathBuf),
    CopyRelativePath(PathBuf),
    Reveal(PathBuf),
    CloseTab(usize),
    CloseOtherTabs(usize),
    CloseAllTabs,
    Tool(Tool),
    Table(TableEdit),
    /// Sizes the image at the caret, in percent of the page.
    ImageWidth(u32),
    ReplaceImage,
    RemoveImage,
    ChooseImage,
    PasteImage,
    TypeImagePath,
    /// Adds a picture to the comment being written: one of the project's, or the
    /// clipboard's.
    AttachPicture,
    AttachClipboard,
    /// Adds a new diagram from a template.
    Diagram(usize),
    /// Adds a drawing to a paper from a template.
    PaperDiagram(usize),
    /// Puts a suggested word in place of a range of the document.
    Fix(usize, usize, String),
    /// Stops counting a word as wrong.
    Learn(String),
    /// Undoes one line of a change in Review.
    RejectLine(cellix_review::LineUndo),
    AddCodeCell,
    /// Something on the Task page.
    Task(crate::app::TaskAct),
    /// Runs a program in the side pane, to set something up.
    InPane(String, Vec<String>),
    /// Opens the rows under this one, beside it.
    Submenu,
    /// Brings a file's tab to the front, from the list of those out of view.
    ShowTab(usize),
    /// Opens the transcript of the agent in a tab of the side pane, by its place there.
    Transcript(usize),
    /// Closes a tab of the side pane.
    CloseSession(usize),
    /// Asks for a name for a tab of the side pane.
    RenameSession(usize),
    /// Brings a tab of the side pane to the front, from the list of every tab.
    PaneTab(crate::dock::PaneTab),
    /// Lists what can be started in the side pane, as its + does.
    PaneNew,
    /// Starts a program in the side pane: an agent, or a terminal (`agents::SHELL.1`).
    Start(&'static str),
    /// Starts an agent with a role's prompt as its first.
    StartRole(&'static str, String),
    /// Starts this agent reviewing the changes.
    ReviewWith(&'static str),
    /// Reviews what changed in a worktree, as a click on its row does.
    ReviewWorktree(PathBuf),
    /// Asks to take a worktree away.
    RemoveWorktree(PathBuf),
}

/// What a click or a key on a menu came to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    Chose(Action),
    /// It stays open: a greyed row, or a submenu opening.
    Stay,
    /// Outside every menu, which closes them.
    Outside,
}

/// One row. `None` for an action shows what can't be done here, rather than hiding it, so
/// the menu keeps its shape from file to file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub label: Cow<'static, str>,
    pub keys: Option<&'static str>,
    pub action: Option<Action>,
    /// A line above this row, starting a new group.
    pub rule: bool,
    /// The rows a submenu holds.
    pub sub: Vec<Item>,
    /// An icon before the words, in its own colour.
    pub icon: Option<(Icon, Rgb)>,
    /// Quieter words after the row's own, saying more of it.
    pub detail: Option<Cow<'static, str>>,
    /// A key that picks the row while the menu is open, shown at its end.
    pub accel: Option<char>,
    /// A heading over the rows after it, in small capitals: nothing to pick.
    pub heading: bool,
    /// What its end says is a note, quiet but readable whether the row can be picked or
    /// not, rather than keys that go grey with it.
    pub noted: bool,
}

impl Item {
    pub fn new(label: impl Into<Cow<'static, str>>, action: Action) -> Self {
        let keys = match action {
            Action::Run(command) => keys_of(command),
            _ => None,
        };
        Self {
            label: label.into(),
            keys,
            action: Some(action),
            rule: false,
            sub: Vec::new(),
            icon: None,
            detail: None,
            accel: None,
            heading: false,
            noted: false,
        }
    }

    /// A heading over the rows after it.
    pub fn heading(label: &'static str) -> Self {
        Self {
            heading: true,
            action: None,
            ..Self::new(label, Action::Submenu)
        }
    }

    /// The icon before its words.
    pub fn icon(self, icon: Icon, color: Rgb) -> Self {
        Self {
            icon: Some((icon, color)),
            ..self
        }
    }

    /// Quieter words after its own.
    pub fn detail(self, detail: impl Into<Cow<'static, str>>) -> Self {
        let detail = detail.into();
        Self {
            detail: (!detail.is_empty()).then_some(detail),
            ..self
        }
    }

    /// The key that picks it while the menu's open, and what its end says of it.
    pub fn key(self, (key, shown): (char, &'static str)) -> Self {
        Self {
            accel: Some(key),
            keys: Some(shown),
            ..self
        }
    }

    /// What its end says, where keys would be, as an agent that isn't installed.
    pub fn note(self, note: &'static str) -> Self {
        Self {
            keys: Some(note),
            noted: true,
            ..self
        }
    }

    /// A row that opens `items` beside it, as Format › does.
    pub fn submenu(label: &'static str, mut items: Vec<Item>) -> Self {
        // Nothing above a submenu's first row to set it apart from.
        if let Some(first) = items.first_mut() {
            first.rule = false;
        }
        let usable = items.iter().any(|item| item.action.is_some());
        Self {
            sub: items,
            ..Self::new(label, Action::Submenu).when(usable)
        }
    }

    pub fn run(label: &'static str, command: Command) -> Self {
        Self::new(label, Action::Run(command))
    }

    /// Starts a new group.
    pub fn after_rule(self) -> Self {
        Self { rule: true, ..self }
    }

    /// Greys the row out unless `available`.
    pub fn when(self, available: bool) -> Self {
        Self {
            action: self.action.filter(|_| available),
            ..self
        }
    }
}

/// A menu at the pointer, for the mouse or the keyboard.
pub struct Menu {
    area: Rect,
    items: Vec<Item>,
    /// Whether each rule has a blank row of its own, which needs the room.
    spaced: bool,
    /// Painted: rows sit between the terminal's, padded and parted by half rows, and the
    /// menu is whole rows around them.
    fine: bool,
    /// Rows from the menu's top to its first item's.
    pad: f32,
    /// Rows each rule takes: `RULE_ROOM`, and a share of what whole rows leave over.
    rule_room: f32,
    /// How far down its first row its panel starts, where it opens under a line (`hang`).
    drop: f32,
    /// Columns from the text edge to a row's words: an icon's, where any row has one.
    lead: f32,
    /// How wide each row's words are, for what comes after them.
    widths: Vec<f32>,
    /// The row the keyboard is on.
    pub selected: Option<usize>,
    /// Each row's submenu, placed beside it, and the one open.
    subs: Vec<Option<Menu>>,
    open: Option<usize>,
    /// Where the button that opened it is, which stands lit while it's open.
    pub from: Option<(u16, u16)>,
    /// It opens over a card, a menu's colour, so it stands a step above it
    /// (`Theme::over_card`).
    pub over_card: bool,
}

impl Menu {
    /// Opens at `(x, y)`, or as near as fits on screen.
    pub fn new((x, y): (u16, u16), items: Vec<Item>, measure: &dyn Measure, screen: Rect) -> Self {
        let width = Self::width(&items, measure).min(screen.w);
        let room = screen.h.saturating_sub(2);
        let rules = items.iter().filter(|i| i.rule).count() as u16;
        // The cell tier has no hairline, so a blank row parts its groups instead.
        let fine = !measure.in_cells();
        let spaced = !fine && items.len() as u16 + rules <= room;
        // Painted, the rows and rules sit inside whole rows of their own, so the menu starts
        // and ends on a row line.
        let rows = items.len() as f32 + f32::from(rules) * RULE_ROOM;
        let height = if fine {
            ((rows + PADDING * 2.0).ceil() as u16).min(room)
        } else {
            (items.len() as u16 + if spaced { rules } else { 0 }).min(room)
        };
        // The same air over the first row and under the last in every menu. What rounding to
        // whole rows leaves over, half a row with an odd number of rules, goes to the rules.
        // A menu with no rules has none over, its rows and padding being whole.
        let spare = (f32::from(height) - rows - PADDING * 2.0).max(0.0);
        let (pad, rule_room) = match (fine, rules) {
            (false, _) => (0.0, RULE_ROOM),
            (true, 0) => (PADDING + spare / 2.0, RULE_ROOM),
            (true, rules) => (PADDING, RULE_ROOM + spare / f32::from(rules)),
        };
        let x = x.min(screen.right().saturating_sub(width));
        let bottom = screen.bottom().saturating_sub(1);
        // A menu that won't fit below the pointer opens upward from it.
        let y = if y + height > bottom {
            y.saturating_sub(height)
                .max(1)
                .min(bottom.saturating_sub(height))
        } else {
            y.max(1)
        };
        let lead = Self::lead(&items);
        let widths = items
            .iter()
            .map(|item| measure.width(&item.label, Font::default()))
            .collect();
        let mut menu = Self {
            area: Rect::new(x, y, width, height),
            items,
            spaced,
            fine,
            pad,
            rule_room,
            drop: 0.0,
            lead,
            widths,
            selected: None,
            subs: Vec::new(),
            open: None,
            from: None,
            over_card: false,
        };
        menu.subs = (0..menu.items.len())
            .map(|row| menu.place_submenu(row, measure, screen))
            .collect();
        menu
    }

    /// A row's submenu, beside it: to the right where it fits, else to the left, else over
    /// this menu's right edge, as near the row as the screen allows.
    fn place_submenu(&self, row: usize, measure: &dyn Measure, screen: Rect) -> Option<Menu> {
        let items = self.items[row].sub.clone();
        if items.is_empty() {
            return None;
        }
        let width = Self::width(&items, measure).min(screen.w);
        let x = if self.area.right() + width <= screen.right() {
            self.area.right()
        } else if self.area.x >= screen.x + width {
            self.area.x - width
        } else {
            screen.right().saturating_sub(width)
        };
        // Its first row level with the row that opened it, as the pointer leaves that row:
        // its top as far above that as its own first row is below its top.
        let top = (self.top_of(row) - self.pad).floor() as u16;
        Some(Menu::new((x, top), items, measure, screen))
    }

    /// The submenu showing, if any.
    fn child(&self) -> Option<&Menu> {
        self.subs.get(self.open?)?.as_ref()
    }

    fn child_mut(&mut self) -> Option<&mut Menu> {
        self.subs.get_mut(self.open?)?.as_mut()
    }

    /// Rows it has, not counting any submenu's.
    #[cfg(test)]
    pub fn rows(&self) -> usize {
        self.items.len()
    }

    /// Where it stands.
    #[cfg(test)]
    pub fn area(&self) -> Rect {
        self.area
    }

    /// The label of the row the keyboard is on in this menu.
    #[cfg(test)]
    pub fn selected_label(&self) -> Option<&str> {
        Some(&self.items[self.selected?].label)
    }

    /// The label the keyboard is on, in the submenu if it's there.
    #[cfg(test)]
    pub fn chosen_label(&self) -> Option<&str> {
        let inside = self.child().filter(|c| c.selected.is_some());
        inside.map_or_else(|| self.selected_label(), Menu::chosen_label)
    }

    /// Whether a point is on this menu or the submenu showing.
    pub fn contains(&self, x: u16, y: u16) -> bool {
        self.area.contains(x, y) || self.child().is_some_and(|c| c.contains(x, y))
    }

    /// The pointer moved: over a row with a submenu it opens, over another row it closes.
    pub fn hover(&mut self, x: u16, y: f32) {
        if let Some(child) = self.child_mut().filter(|c| c.contains(x, y as u16)) {
            child.hover(x, y);
            return;
        }
        if let Some(row) = self.row_at(x, y) {
            let opens = self.subs[row].is_some() && self.items[row].action.is_some();
            if opens && self.open != Some(row) {
                self.open_submenu(row);
            } else if !opens {
                self.open = None;
            }
        }
    }

    fn open_submenu(&mut self, row: usize) {
        self.open = Some(row);
        if let Some(child) = self.child_mut() {
            child.selected = None;
            child.open = None;
        }
    }

    /// A click at a point, `y` with its fraction of a row.
    pub fn click(&mut self, x: u16, y: f32) -> Outcome {
        if let Some(child) = self.child_mut() {
            match child.click(x, y) {
                Outcome::Outside => {}
                done => return done,
            }
        }
        let Some(row) = self.row_at(x, y) else {
            return Outcome::Outside;
        };
        match self.items[row].action.clone() {
            Some(Action::Submenu) => {
                self.open_submenu(row);
                Outcome::Stay
            }
            Some(action) => Outcome::Chose(action),
            None => Outcome::Stay,
        }
    }

    /// A key while the menu is up: arrows move, Right and Enter open a submenu, Left and
    /// Escape close the one open. `Outside` means close the menu.
    pub fn key(&mut self, key: Key) -> Outcome {
        let inside = self.child().is_some_and(|c| c.selected.is_some());
        if inside {
            if key == Key::Left
                || (key == Key::Esc && self.child().is_some_and(|c| c.open.is_none()))
            {
                self.open = None;
                return Outcome::Stay;
            }
            if let Some(child) = self.child_mut() {
                return child.key(key);
            }
        }
        match key {
            Key::Up | Key::BackTab => self.step(-1),
            Key::Down | Key::Tab => self.step(1),
            Key::Right | Key::Enter => {
                let Some(row) = self.selected else {
                    return Outcome::Stay;
                };
                match self.items[row].action.clone() {
                    Some(Action::Submenu) => {
                        self.open_submenu(row);
                        if let Some(child) = self.child_mut() {
                            child.step(1);
                        }
                    }
                    Some(action) if key == Key::Enter => return Outcome::Chose(action),
                    _ => {}
                }
            }
            Key::Esc => return Outcome::Outside,
            // A row's own key picks it.
            Key::Char(typed) => {
                let picked = self
                    .items
                    .iter()
                    .find(|item| item.accel.is_some_and(|k| k.eq_ignore_ascii_case(&typed)));
                if let Some(action) = picked.and_then(|item| item.action.clone()) {
                    return Outcome::Chose(action);
                }
            }
            _ => {}
        }
        Outcome::Stay
    }

    /// Columns before a row's words: an icon's, where any row of `items` has one.
    fn lead(items: &[Item]) -> f32 {
        if items.iter().any(|item| item.icon.is_some()) {
            ICON_COLUMNS
        } else {
            0.0
        }
    }

    fn width(items: &[Item], measure: &dyn Measure) -> u16 {
        let plain = Font::default();
        let small = Font {
            size: KEYS_SIZE,
            ..Font::default()
        };
        let quiet = Font {
            size: DETAIL_SIZE,
            ..Font::default()
        };
        let lead = Self::lead(items);
        let row = |item: &Item| {
            let mark = (!item.sub.is_empty()).then_some("›");
            let keys = item
                .keys
                .or(mark)
                .map_or(0.0, |k| measure.width(k, small) + GAP);
            let detail = item
                .detail
                .as_deref()
                .map_or(0.0, |d| DETAIL_GAP + measure.width(d, quiet));
            lead + measure.width(&item.label, plain) + detail + keys
        };
        let widest = items.iter().map(row).fold(0.0, f32::max);
        ((widest + INSET * 2.0).ceil() as u16 + 1).max(MIN_WIDTH)
    }

    /// Where an item's row starts, in rows, counting the room of the rules above it.
    fn top_of(&self, index: usize) -> f32 {
        let rules = self.items[..=index].iter().filter(|i| i.rule).count();
        if self.fine {
            f32::from(self.area.y) + self.pad + index as f32 + rules as f32 * self.rule_room
        } else {
            let blank = if self.spaced { rules } else { 0 };
            f32::from(self.area.y) + (index + blank) as f32
        }
    }

    /// Opened on the row under a line, as a tab's menu is under the tab row: its panel starts
    /// `LINE_AIR` down its first row, rather than on the line, and its rows keep their air
    /// under that. It takes another row where it has to and there's room below.
    pub fn hang(&mut self, measure: &dyn Measure, screen: Rect) {
        let Some((_, row)) = measure.cell_pixels().filter(|_| self.fine) else {
            return;
        };
        // The air and a pixel for the line, in whole sixty-fourths of a row, rounded up, as
        // paint places it.
        let unit = cellix_render::UNIT as f32;
        let air = crate::measures::LINE_AIR + 1.0;
        let drop = (air / row.max(1) as f32 * unit).ceil() / unit;
        let rules = self.items.iter().filter(|i| i.rule).count();
        let rows = self.items.len() as f32 + rules as f32 * RULE_ROOM;
        let height = (rows + PADDING * 2.0 + drop).ceil() as u16;
        if self.area.y + height > screen.bottom().saturating_sub(1) {
            return;
        }
        // What the drop leaves over of its whole rows goes over its first row and under its
        // last alike, so its rules keep the room they have in every menu.
        let spare = (f32::from(height) - rows - PADDING * 2.0 - drop).max(0.0);
        self.area.h = height;
        self.drop = drop;
        self.pad = drop + PADDING + spare / 2.0;
        self.rule_room = RULE_ROOM;
        // Its rows moved down, and their submenus with them.
        self.subs = (0..self.items.len())
            .map(|row| self.place_submenu(row, measure, screen))
            .collect();
    }

    /// The row its top is on.
    pub fn top(&self) -> u16 {
        self.area.y
    }

    /// The item under the pointer, `y` with its fraction of a row.
    pub fn row_at(&self, x: u16, y: f32) -> Option<usize> {
        let area = self.area;
        let inside = x >= area.x
            && x < area.right()
            && y >= f32::from(area.y)
            && y < f32::from(area.bottom());
        if !inside {
            return None;
        }
        (0..self.items.len()).find(|i| {
            let top = self.top_of(*i);
            y >= top && y < top + 1.0
        })
    }

    #[cfg(test)]
    pub fn action(&self, row: usize) -> Option<&Action> {
        self.items.get(row)?.action.as_ref()
    }

    /// Moves to the next row that can be chosen, wrapping around.
    pub fn step(&mut self, delta: isize) {
        let count = self.items.len() as isize;
        let usable = self.items.iter().filter(|i| i.action.is_some()).count();
        if usable == 0 {
            return;
        }
        // Moving along this menu leaves the submenu its row opened.
        if self.child().is_some_and(|c| c.selected.is_none()) {
            self.open = None;
        }
        let start = match self.selected {
            Some(row) => row as isize,
            None if delta > 0 => -1,
            None => count,
        };
        let mut row = start;
        loop {
            row = (row + delta).rem_euclid(count);
            if self.items[row as usize].action.is_some() {
                self.selected = Some(row as usize);
                return;
            }
        }
    }

    /// `hover` is where the pointer is, its row with the fraction.
    pub fn draw(&self, grid: &mut Grid, hover: Option<(u16, f32)>, theme: &Theme) {
        let lifted;
        let theme = if self.over_card {
            lifted = theme.over_card();
            &lifted
        } else {
            theme
        };
        let area = self.area;
        // Hung under a line, its first row's cells keep their ground, which shows over its
        // panel's top: the words its first row holds would have lit them in its colour.
        let behind: Vec<Rgb> = (area.x..area.right())
            .filter(|_| self.drop > 0.0)
            .map(|x| grid.cell(x, area.y).style.bg)
            .collect();
        if self.fine && self.drop > 0.0 {
            hung(grid, area, self.drop, theme);
        } else if self.fine {
            // On its rows, covering each it touches whole: nothing reaches past its edges.
            popup(grid, area, theme);
        } else {
            raised(grid, area, theme);
        }
        let base = Style {
            fg: theme.text.fg,
            bg: theme.popup,
            ..theme.bar
        };
        let keys_style = Style {
            fg: theme.popup_dim,
            ..base
        };
        let greyed = Style {
            fg: theme.disabled,
            ..base
        };
        for (row, item) in self.items.iter().enumerate() {
            let top = self.top_of(row);
            if top + 1.0 > f32::from(area.bottom()) + 0.01 {
                break;
            }
            if item.rule {
                let across = area.w as f32 - INSET * 2.0;
                // In the middle of its own room, painted or a blank row in cells.
                let above = if self.fine {
                    self.rule_room / 2.0
                } else if self.spaced {
                    0.5
                } else {
                    0.0
                };
                let rule = Bounds::cells(area.x as f32 + INSET, top - above, across, 0.0);
                grid.hairline(rule, theme.popup_border);
            }
            let usable = item.action.is_some();
            let hovered = hover.is_some_and(|(x, y)| self.row_at(x, y) == Some(row));
            // The row whose submenu is open stays lit while the pointer is in the submenu.
            let opened = self.open == Some(row);
            if usable && (self.selected == Some(row) || hovered || opened) {
                let fill = if self.selected == Some(row) || opened {
                    theme.pick
                } else {
                    theme.popup_hover
                };
                let band = Bounds::cells(
                    area.x as f32 + BAND_INSET,
                    top + (1.0 - BAND_HEIGHT) / 2.0,
                    area.w as f32 - BAND_INSET * 2.0,
                    BAND_HEIGHT,
                );
                grid.panel(band, 18, Some(fill), None);
            }
            // A row that can't be picked and says why, as an agent that isn't installed, has its
            // name as quiet as its note and no quieter: dimmed past it, the name read as less
            // than "not installed" after it.
            let unavailable = Style {
                fg: theme.unavailable(),
                ..base
            };
            let style = match (usable, item.noted) {
                (true, _) => base,
                (false, true) => unavailable,
                (false, false) => greyed,
            };
            let x = area.x as f32 + INSET;
            if item.heading {
                // A section's heading: small, bold, quiet capitals, from the text edge.
                let heading = Style {
                    bold: true,
                    fg: theme.popup_dim,
                    ..base
                };
                let shown = item.label.to_uppercase();
                words(
                    grid,
                    (x, top),
                    &shown,
                    heading,
                    (area.right(), HEADING_SIZE, false),
                );
                continue;
            }
            if let Some((icon, color)) = item.icon {
                let color = if usable { color } else { theme.disabled };
                mark(grid, (x, top), icon, color, style);
            }
            let x = x + self.lead;
            words(
                grid,
                (x, top),
                &item.label,
                style,
                (area.right(), 100, false),
            );
            if let Some(detail) = item.detail.as_deref() {
                let after = x + self.widths[row] + DETAIL_GAP;
                let quiet = Style {
                    fg: if usable {
                        theme.popup_dim
                    } else {
                        theme.disabled
                    },
                    ..base
                };
                // Smaller than the words, so dropped a pixel to stand on their line.
                let end = area.right().saturating_sub(INSET as u16);
                words(
                    grid,
                    (after, top + KEYS_DROP),
                    detail,
                    quiet,
                    (end, DETAIL_SIZE, false),
                );
            }
            if let Some(keys) = item.keys {
                // As far from the menu's far edge as its words are from the near one.
                let right = area.right().saturating_sub(INSET as u16);
                let shown = match (usable, item.noted) {
                    (true, _) => keys_style,
                    (false, true) => unavailable,
                    (false, false) => greyed,
                };
                // Smaller than the words, so dropped a pixel to stand on their line.
                words(
                    grid,
                    (right as f32, top + KEYS_DROP),
                    keys,
                    shown,
                    (right, KEYS_SIZE, true),
                );
            }
            if !item.sub.is_empty() {
                // As far from the menu's far edge as its words are from the near one.
                let right = area.right().saturating_sub(INSET as u16);
                let shown = if usable { keys_style } else { greyed };
                words(grid, (right as f32, top), "›", shown, (right, 100, true));
            }
        }
        for (x, bg) in (area.x..).zip(behind) {
            grid.set_style(Rect::new(x, area.y, 1, 1), |style| Style { bg, ..style });
        }
        if let Some(child) = self.child() {
            child.draw(grid, hover, theme);
        }
    }
}

/// A menu's panel that starts `drop` down its first row, under a line: the rows under that
/// are its own, and the first keeps what's behind it over the panel's top.
fn hung(grid: &mut Grid, area: Rect, drop: f32, theme: &Theme) {
    let base = Style {
        fg: theme.text.fg,
        bg: theme.popup,
        ..theme.bar
    };
    for y in area.y + 1..area.bottom() {
        // The last row's corner cells keep what's behind them, under the round corners.
        let (x, w) = if y + 1 == area.bottom() {
            (area.x + 1, area.w.saturating_sub(2))
        } else {
            (area.x, area.w)
        };
        grid.fill(Rect::new(x, y, w, 1), base);
    }
    let bounds = Bounds::cells(
        f32::from(area.x),
        f32::from(area.y) + drop,
        f32::from(area.w),
        f32::from(area.h) - drop,
    );
    grid.popup(bounds, 28, theme.popup, theme.popup_border);
}

/// An icon at a row's start, as `words` places words: painted where the row is, and in cells
/// a character for it on the nearest row.
fn mark(grid: &mut Grid, (x, top): (f32, f32), icon: Icon, color: Rgb, style: Style) {
    let side = cellix_render::GLYPH;
    let row = (top + 0.499).floor() as u16;
    let shown = Style {
        fg: color,
        painted: true,
        ..style
    };
    grid.put_str(
        x as u16,
        row,
        crate::toolbar::fallback(icon),
        shown,
        x as u16 + 2,
    );
    if row >= grid.area().bottom() {
        return;
    }
    grid.paint.push(Paint::Icon {
        bounds: Bounds::cells(x, top + (1.0 - side) / 2.0, side * 2.0, side),
        icon,
        color,
    });
}

/// Words on a row that may sit between the terminal's: painted where they belong, and in
/// cells on the nearest row. `right` is where they stop, or end when `align_right`.
fn words(
    grid: &mut Grid,
    (x, top): (f32, f32),
    text: &str,
    style: Style,
    (right, size, align_right): (u16, u8, bool),
) {
    // The row holding most of it; a row starting halfway takes the upper one, whose middle
    // is still on it, so a click on the cells lands on the row they show.
    let row = (top + 0.499).floor() as u16;
    let cells = text.chars().count() as u16;
    let column = if align_right {
        right.saturating_sub(cells)
    } else {
        x as u16
    };
    let shown = Style {
        painted: true,
        ..style
    };
    grid.put_str(column, row, text, shown, right);
    if row >= grid.area().bottom() {
        return;
    }
    let width = grid.area().w;
    grid.paint.push(Paint::Text {
        x: units(x),
        y: units(top),
        max_width: units(if align_right {
            x
        } else {
            f32::from(right.min(width)) - x
        }),
        text: text.to_string(),
        color: style.fg,
        font: Font {
            bold: style.bold,
            size,
            ..Font::default()
        },
        align_right,
        centred: false,
        underline: false,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use cellix_render::CellMeasure;

    /// The middle of a row, where a terminal without pixels puts the pointer.
    fn on(row: u16) -> f32 {
        f32::from(row) + 0.5
    }

    const SCREEN: Rect = Rect {
        x: 0,
        y: 0,
        w: 80,
        h: 24,
    };

    fn items() -> Vec<Item> {
        vec![
            Item::run("Cut", Command::Cut),
            Item::run("Copy", Command::Copy).when(false),
            Item::run("Paste", Command::Paste).after_rule(),
        ]
    }

    #[test]
    fn it_stays_on_screen() {
        let menu = Menu::new((79, 23), items(), &CellMeasure, SCREEN);
        assert!(menu.area.right() <= SCREEN.right());
        assert!(
            menu.area.bottom() < SCREEN.bottom(),
            "clear of the status bar"
        );
        assert_eq!(menu.area.bottom(), 23, "and it opens up from the pointer");
    }

    #[test]
    fn the_keyboard_skips_what_cant_be_done() {
        let mut menu = Menu::new((1, 1), items(), &CellMeasure, SCREEN);
        menu.step(1);
        assert_eq!(menu.selected, Some(0));
        menu.step(1);
        assert_eq!(menu.selected, Some(2), "Copy is greyed out");
        menu.step(1);
        assert_eq!(menu.selected, Some(0), "and it wraps");
        menu.step(-1);
        assert_eq!(menu.selected, Some(2));
    }

    #[test]
    fn a_short_window_gives_up_the_blank_rows_first() {
        let short = Rect::new(0, 0, 80, 5);
        let menu = Menu::new((1, 1), items(), &CellMeasure, short);
        assert_eq!(menu.area.h, 3);
        assert_eq!(menu.row_at(2, on(menu.area.y + 2)), Some(2));
    }

    /// Painted, a menu is whole rows with half a row of air over its first row and under its
    /// last, whatever it holds: a rule has half a row at least, and where the rules leave half
    /// a row over, they share it, so its edges sit on row lines. The air at the edges was a
    /// quarter of a row or half by how many rules a menu had.
    #[test]
    fn with_paint_a_menu_is_whole_rows_and_a_rule_has_half_a_row() {
        let measure = cellix_render::FontMeasure::new(17, 37);
        let menu = Menu::new((1, 1), items(), &measure, SCREEN);
        // Three rows, a rule, and half a row above and below: the rule has a whole row.
        assert_eq!(menu.area.h, 5);
        assert_eq!(menu.top_of(0), 1.5);
        assert_eq!(menu.top_of(1), 2.5);
        assert_eq!(menu.top_of(2), 4.5, "past the rule's row");
        assert_eq!(
            f32::from(menu.area.bottom()) - (menu.top_of(2) + 1.0),
            0.5,
            "as much under the last"
        );
        // Each row is hit across all of it, by the fraction of a row the pointer's on.
        assert_eq!(menu.row_at(2, 1.6), Some(0));
        assert_eq!(menu.row_at(2, 3.4), Some(1));
        assert_eq!(menu.row_at(2, 4.0), None, "the rule's own room");
        assert_eq!(menu.row_at(2, 4.8), Some(2));
        assert_eq!(menu.row_at(2, 1.3), None, "the padding");
        // Any number of rules, or none: the same air at the edges.
        for rules in 0..4 {
            let list: Vec<Item> = (0..5)
                .map(|i| {
                    let item = Item::run("Cut", Command::Cut);
                    if i > 0 && i <= rules {
                        item.after_rule()
                    } else {
                        item
                    }
                })
                .collect();
            let menu = Menu::new((1, 1), list, &measure, SCREEN);
            let top = menu.top_of(0) - f32::from(menu.area.y);
            let under = f32::from(menu.area.bottom()) - (menu.top_of(4) + 1.0);
            let half = |air: f32| (air - 0.5).abs() < 0.001;
            assert!(
                half(top) && half(under),
                "{rules} rules: {top} over, {under} under"
            );
        }
        // Its frame is its rows: nothing painted reaches past them.
        let mut grid = Grid::new(SCREEN.w, SCREEN.h, Style::default());
        menu.draw(&mut grid, Some((2, 4.8)), &Theme::default());
        let (top, bottom) = (
            i32::from(menu.area.y) * 64,
            i32::from(menu.area.bottom()) * 64,
        );
        for paint in &grid.paint {
            if let Paint::Panel { bounds, .. } = paint {
                assert!(
                    bounds.y >= top && bounds.y + bounds.h <= bottom,
                    "{bounds:?}"
                );
            }
        }
        // The hover band stands clear of the rule above it.
        let band = grid.paint.iter().find_map(|p| match p {
            Paint::Panel {
                bounds,
                shadow: false,
                ..
            } => Some(*bounds),
            _ => None,
        });
        let band = band.expect("Paste is lit");
        let rule = grid.paint.iter().find_map(|p| match p {
            Paint::Hairline { bounds, .. } => Some(bounds.y),
            _ => None,
        });
        assert!(band.y > rule.expect("a rule"), "{band:?}");
    }

    /// Whatever the padding, the cells an item is shown in are on it: a click in their
    /// middle chooses that item.
    #[test]
    fn a_painted_rows_cells_are_on_it() {
        let measure = cellix_render::FontMeasure::new(17, 37);
        for count in 1..6 {
            let list: Vec<Item> = (0..count)
                .map(|i| {
                    let item = Item::run("Cut", Command::Cut);
                    if i == 2 { item.after_rule() } else { item }
                })
                .collect();
            let menu = Menu::new((1, 1), list, &measure, SCREEN);
            let mut grid = Grid::new(SCREEN.w, SCREEN.h, Style::default());
            menu.draw(&mut grid, None, &Theme::default());
            let rows: Vec<u16> = (0..SCREEN.h)
                .filter(|y| grid.row_text(*y).contains("Cut"))
                .collect();
            assert_eq!(rows.len(), count, "{rows:?}");
            for (index, row) in rows.into_iter().enumerate() {
                assert_eq!(
                    menu.row_at(2, on(row)),
                    Some(index),
                    "{count} items, row {row}"
                );
            }
        }
    }

    /// A row's own key picks it while the menu is open, wherever the keyboard is; a heading
    /// and a dimmed row are passed over. A row's icon stands at the text edge and its words
    /// after it, quieter words after those, and a note at the end.
    #[test]
    fn a_rows_own_key_picks_it_and_a_heading_is_passed_over() {
        let color = Rgb(1, 2, 3);
        let items = vec![
            Item::new("Terminal", Action::PaneNew)
                .icon(Icon::Terminal, color)
                .key(('t', "T")),
            Item::new("Codex", Action::Start("codex"))
                .key(('1', "1"))
                .after_rule(),
            Item::new("Claude Code", Action::Start("claude"))
                .note("not installed")
                .when(false),
            Item::heading("Roles").after_rule(),
            Item::new("Tester", Action::Start("codex")).detail("runs the tests"),
        ];
        let measure = cellix_render::FontMeasure::new(17, 37);
        let mut menu = Menu::new((1, 1), items, &measure, SCREEN);
        assert_eq!(
            menu.key(Key::Char('1')),
            Outcome::Chose(Action::Start("codex"))
        );
        assert_eq!(menu.key(Key::Char('T')), Outcome::Chose(Action::PaneNew));
        assert_eq!(menu.key(Key::Char('x')), Outcome::Stay);
        let picked: Vec<usize> = (0..4)
            .map(|_| {
                menu.step(1);
                menu.selected.unwrap()
            })
            .collect();
        assert_eq!(picked, [0, 1, 4, 0], "past the dimmed row and the heading");
        let mut grid = Grid::new(SCREEN.w, SCREEN.h, Style::default());
        menu.draw(&mut grid, None, &Theme::default());
        let texts: Vec<(String, i32)> = grid
            .paint
            .iter()
            .filter_map(|p| match p {
                Paint::Text { text, x, .. } => Some((text.clone(), *x)),
                _ => None,
            })
            .collect();
        let x_of = |words: &str| texts.iter().find(|(t, _)| t == words).map(|(_, x)| *x);
        let edge = units(f32::from(menu.area.x) + INSET);
        let icon = grid.paint.iter().find_map(|p| match p {
            Paint::Icon {
                bounds, color: c, ..
            } if *c == color => Some(bounds.x),
            _ => None,
        });
        assert_eq!(icon, Some(edge), "the icon at the text edge");
        assert_eq!(
            x_of("Terminal"),
            Some(edge + units(ICON_COLUMNS)),
            "its words after it"
        );
        assert_eq!(x_of("Codex"), x_of("Terminal"), "every row's words line up");
        assert_eq!(
            x_of("ROLES"),
            Some(edge),
            "a heading in capitals, from the edge"
        );
        assert!(x_of("runs the tests") > x_of("Tester"), "{texts:?}");
        assert!(x_of("not installed").is_some(), "{texts:?}");
    }

    /// An agent that isn't installed is a row that can't be picked and says why: its name is
    /// as strong as "not installed" after it, in both themes. It was dimmer than its note, which
    /// read as the row's main words.
    #[test]
    fn a_rows_name_is_no_quieter_than_its_note() {
        let measure = cellix_render::FontMeasure::new(17, 37);
        for theme in [Theme::default(), Theme::named(Some("light"))] {
            let items = vec![
                Item::new("Codex", Action::Start("codex")),
                Item::new("Claude Code", Action::Start("claude"))
                    .note("not installed")
                    .when(false),
            ];
            let menu = Menu::new((1, 1), items, &measure, SCREEN);
            let mut grid = Grid::new(SCREEN.w, SCREEN.h, Style::default());
            menu.draw(&mut grid, None, &theme);
            let colour = |words: &str| {
                grid.paint.iter().find_map(|p| match p {
                    Paint::Text { text, color, .. } if text == words => Some(*color),
                    _ => None,
                })
            };
            let (name, note) = (colour("Claude Code"), colour("not installed"));
            assert_eq!(name, Some(theme.unavailable()), "the name, readable");
            let contrast = crate::theme::contrast;
            let ratio = contrast(theme.unavailable(), theme.popup);
            assert!(ratio >= 4.5, "{ratio:.2} to 1");
            let ratio = contrast(theme.unavailable(), theme.text.fg);
            assert!(
                ratio >= 1.4,
                "and quieter than one that can be picked: {ratio:.2}"
            );
            assert_eq!(note, name, "as strong as its note");
            assert_eq!(
                colour("Codex"),
                Some(theme.text.fg),
                "one that can be picked"
            );
        }
    }

    #[test]
    fn a_greyed_row_does_nothing() {
        let menu = Menu::new((1, 1), items(), &CellMeasure, SCREEN);
        assert_eq!(menu.action(1), None);
        assert_eq!(menu.action(0), Some(&Action::Run(Command::Cut)));
        assert_eq!(menu.row_at(2, on(2)), Some(1));
        assert_eq!(
            menu.row_at(2, on(3)),
            None,
            "the rule has this row to itself"
        );
        assert_eq!(menu.row_at(2, on(4)), Some(2));
        assert_eq!(menu.row_at(2, on(9)), None);
    }
}
