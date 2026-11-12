use std::borrow::Cow;

use std::path::PathBuf;

use cellix_doc::table::Edit as TableEdit;
use cellix_input::{Command, Key};
use cellix_render::{Bounds, Font, Grid, Measure, Paint, Rect, Style, units};

use crate::palette::{keys_of, popup, raised};
use crate::theme::Theme;
use crate::toolbar::Tool;

/// Space from the menu's edge to its text: two columns, as text stands inside whatever
/// holds it.
const INSET: f32 = 2.0;
/// Space between a label and its keys.
const GAP: f32 = 3.0;
const KEYS_SIZE: u8 = 85;
const MIN_WIDTH: u16 = 16;
/// Air above the first row and below the last, in rows: 8 px of a 32 px row. Painted
/// menus only; the cell tier has no fractions of a row.
const PADDING: f32 = 0.25;
/// What a rule between groups takes, in rows, so a hover band never meets it.
const RULE_ROOM: f32 = 0.5;
/// The hover band inside its row: 26 px of 32 tall, and a column in from each side, as a
/// wash is from a panel's edge.
const BAND_HEIGHT: f32 = 26.0 / 32.0;
const BAND_INSET: f32 = 1.0;

/// What a row of a menu does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    Run(Command),
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
    /// Brings a tab of the side pane to the front, from the list of every tab.
    PaneTab(crate::dock::PaneTab),
    /// Lists what can be started in the side pane, as its + does.
    PaneNew,
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
    /// The row the keyboard is on.
    pub selected: Option<usize>,
    /// Each row's submenu, placed beside it, and the one open.
    subs: Vec<Option<Menu>>,
    open: Option<usize>,
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
        // 8 px above the first row whatever the count; rounding to whole rows only adds air
        // at the bottom.
        let pad = if fine { PADDING } else { 0.0 };
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
        let mut menu = Self {
            area: Rect::new(x, y, width, height),
            items,
            spaced,
            fine,
            pad,
            selected: None,
            subs: Vec::new(),
            open: None,
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
            _ => {}
        }
        Outcome::Stay
    }

    fn width(items: &[Item], measure: &dyn Measure) -> u16 {
        let plain = Font::default();
        let small = Font {
            size: KEYS_SIZE,
            ..Font::default()
        };
        let row = |item: &Item| {
            let mark = (!item.sub.is_empty()).then_some("›");
            let keys = item
                .keys
                .or(mark)
                .map_or(0.0, |k| measure.width(k, small) + GAP);
            measure.width(&item.label, plain) + keys
        };
        let widest = items.iter().map(row).fold(0.0, f32::max);
        ((widest + INSET * 2.0).ceil() as u16 + 1).max(MIN_WIDTH)
    }

    /// Where an item's row starts, in rows, counting the room of the rules above it.
    fn top_of(&self, index: usize) -> f32 {
        let rules = self.items[..=index].iter().filter(|i| i.rule).count();
        if self.fine {
            f32::from(self.area.y) + self.pad + index as f32 + rules as f32 * RULE_ROOM
        } else {
            let blank = if self.spaced { rules } else { 0 };
            f32::from(self.area.y) + (index + blank) as f32
        }
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
        let area = self.area;
        if self.fine {
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
                    RULE_ROOM / 2.0
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
            let style = if usable { base } else { greyed };
            let x = area.x as f32 + INSET;
            words(
                grid,
                (x, top),
                &item.label,
                style,
                (area.right(), 100, false),
            );
            if let Some(keys) = item.keys {
                // As far from the menu's far edge as its words are from the near one.
                let right = area.right().saturating_sub(INSET as u16);
                let shown = if usable { keys_style } else { greyed };
                words(
                    grid,
                    (right as f32, top),
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
        if let Some(child) = self.child() {
            child.draw(grid, hover, theme);
        }
    }
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

    /// Painted, a menu is whole rows: 8 px of air above and at least that below, half a row
    /// for each rule, rounded up, so its edges sit on row lines.
    #[test]
    fn with_paint_a_menu_is_whole_rows_and_a_rule_has_half_a_row() {
        let measure = cellix_render::FontMeasure::new(17, 37);
        let menu = Menu::new((1, 1), items(), &measure, SCREEN);
        // Three rows, half a row for the rule and a quarter above and below.
        assert_eq!(menu.area.h, 4);
        assert_eq!(menu.top_of(0), 1.25);
        assert_eq!(menu.top_of(1), 2.25);
        assert_eq!(menu.top_of(2), 3.75, "past half a row for the rule");
        // Each row is hit across all of it, by the fraction of a row the pointer's on.
        assert_eq!(menu.row_at(2, 1.3), Some(0));
        assert_eq!(menu.row_at(2, 3.2), Some(1));
        assert_eq!(menu.row_at(2, 3.5), None, "the rule's own room");
        assert_eq!(menu.row_at(2, 3.8), Some(2));
        assert_eq!(menu.row_at(2, 1.1), None, "the padding");
        // Its frame is its rows: nothing painted reaches past them.
        let mut grid = Grid::new(SCREEN.w, SCREEN.h, Style::default());
        menu.draw(&mut grid, Some((2, 3.8)), &Theme::default());
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
