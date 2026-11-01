use std::path::PathBuf;

use cellix_input::Command;
use cellix_render::{Bounds, Font, Grid, Measure, Rect, Style};

use crate::palette::{keys_of, raised};
use crate::theme::Theme;

/// Space from the menu's edge to its text.
const INSET: f32 = 1.0;
/// Space between a label and its keys.
const GAP: f32 = 3.0;
const KEYS_SIZE: u8 = 85;
const MIN_WIDTH: u16 = 16;

/// What a row of a menu does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    Run(Command),
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
}

/// One row. `None` for an action shows what can't be done here, rather than hiding it, so
/// the menu keeps its shape from file to file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub label: &'static str,
    pub keys: Option<&'static str>,
    pub action: Option<Action>,
    /// A line above this row, starting a new group.
    pub rule: bool,
}

impl Item {
    pub fn new(label: &'static str, action: Action) -> Self {
        let keys = match action {
            Action::Run(command) => keys_of(command),
            _ => None,
        };
        Self {
            label,
            keys,
            action: Some(action),
            rule: false,
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
    /// The row the keyboard is on.
    pub selected: Option<usize>,
}

impl Menu {
    /// Opens at `(x, y)`, or as near as fits on screen.
    pub fn new((x, y): (u16, u16), items: Vec<Item>, measure: &dyn Measure, screen: Rect) -> Self {
        let width = Self::width(&items, measure).min(screen.w);
        let height = (items.len() as u16).min(screen.h.saturating_sub(2));
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
        Self {
            area: Rect::new(x, y, width, height),
            items,
            selected: None,
        }
    }

    fn width(items: &[Item], measure: &dyn Measure) -> u16 {
        let plain = Font::default();
        let small = Font {
            size: KEYS_SIZE,
            ..Font::default()
        };
        let row = |item: &Item| {
            let keys = item.keys.map_or(0.0, |k| measure.width(k, small) + GAP);
            measure.width(item.label, plain) + keys
        };
        let widest = items.iter().map(row).fold(0.0, f32::max);
        ((widest + INSET * 2.0).ceil() as u16 + 1).max(MIN_WIDTH)
    }

    pub fn row_at(&self, x: u16, y: u16) -> Option<usize> {
        self.area
            .contains(x, y)
            .then(|| (y - self.area.y) as usize)
            .filter(|row| *row < self.items.len())
    }

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

    pub fn draw(&self, grid: &mut Grid, hover: Option<(u16, u16)>, theme: &Theme) {
        let area = self.area;
        raised(grid, area, theme);
        let base = Style {
            fg: theme.text.fg,
            bg: theme.popup,
            ..theme.bar
        };
        let quiet = Style {
            fg: theme.status.fg,
            ..base
        };
        for (row, item) in self.items.iter().take(area.h as usize).enumerate() {
            let y = area.y + row as u16;
            let line = Rect::new(area.x, y, area.w, 1);
            if item.rule {
                let across = area.w as f32 - INSET * 2.0;
                let rule = Bounds::cells(area.x as f32 + INSET, y as f32, across, 0.0);
                grid.hairline(rule, theme.popup_border);
            }
            let usable = item.action.is_some();
            let hovered = hover.is_some_and(|h| line.contains(h.0, h.1));
            if usable && (self.selected == Some(row) || hovered) {
                grid.panel(Bounds::inset(line, 0.35, 0.08), 16, Some(theme.pick), None);
            }
            let style = if usable { base } else { quiet };
            let x = area.x as f32 + INSET;
            grid.label_at(x, area.x + 1, y, item.label, style, area.right(), 100);
            if let Some(keys) = item.keys {
                let right = area.right().saturating_sub(1);
                grid.label_right(right, y, keys, quiet, KEYS_SIZE);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cellix_render::CellMeasure;

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
    fn a_greyed_row_does_nothing() {
        let menu = Menu::new((1, 1), items(), &CellMeasure, SCREEN);
        assert_eq!(menu.action(1), None);
        assert_eq!(menu.action(0), Some(&Action::Run(Command::Cut)));
        assert_eq!(menu.row_at(2, 2), Some(1));
        assert_eq!(menu.row_at(2, 9), None);
    }
}
