use std::io;
use std::path::{Path, PathBuf};

use cellix_core::{Buffer, Selection, Workspace};
use cellix_input::{Command, Event, Key, KeyEvent, MouseEvent, MouseKind, simple_keymap};
use cellix_render::{Grid, Rect, Style};

use crate::editor::Editor;
use crate::explorer::Explorer;
use crate::theme::Theme;

const SIDEBAR_WIDTH: u16 = 28;
const WHEEL_LINES: isize = 3;

/// Which part of the screen a press started in, so a drag stays with it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Drag {
    None,
    Text,
}

/// All application state. It has no terminal: tests send events and read the grid.
pub struct App {
    pub workspace: Workspace,
    explorer: Explorer,
    editors: Vec<Editor>,
    active: usize,
    show_sidebar: bool,
    size: (u16, u16),
    theme: Theme,
    drag: Drag,
    message: String,
    /// Set when the user asks to leave.
    pub quit: bool,
}

impl App {
    pub fn new(root: PathBuf, size: (u16, u16)) -> Self {
        let workspace = Workspace::new(root);
        let mut explorer = Explorer::default();
        explorer.refresh(&workspace);
        Self {
            workspace,
            explorer,
            editors: Vec::new(),
            active: 0,
            show_sidebar: true,
            size,
            theme: Theme::default(),
            drag: Drag::None,
            message: String::new(),
            quit: false,
        }
    }

    pub fn open(&mut self, path: &Path) -> io::Result<()> {
        if let Some(i) = self
            .editors
            .iter()
            .position(|e| e.buffer.path() == Some(path))
        {
            self.active = i;
            return Ok(());
        }
        self.editors.push(Editor::new(Buffer::open(path)?));
        self.active = self.editors.len() - 1;
        Ok(())
    }

    pub fn active_text(&self) -> Option<String> {
        self.editors.get(self.active).map(|e| e.buffer.text())
    }

    pub fn selected_text(&self) -> Option<String> {
        self.editors.get(self.active)?.selected_text()
    }

    fn sidebar_area(&self) -> Option<Rect> {
        self.show_sidebar.then(|| {
            Rect::new(
                0,
                0,
                SIDEBAR_WIDTH.min(self.size.0),
                self.size.1.saturating_sub(1),
            )
        })
    }

    fn main_area(&self) -> Rect {
        let x = self.sidebar_area().map_or(0, |a| a.right());
        Rect::new(x, 0, self.size.0 - x, self.size.1.saturating_sub(1))
    }

    fn tabs_area(&self) -> Rect {
        let main = self.main_area();
        Rect::new(main.x, main.y, main.w, 1)
    }

    fn editor_area(&self) -> Rect {
        let main = self.main_area();
        Rect::new(main.x, main.y + 1, main.w, main.h.saturating_sub(1))
    }

    pub fn handle(&mut self, event: Event) {
        match event {
            Event::Key(key) => self.key(key),
            Event::Mouse(mouse) => self.mouse(mouse),
            Event::Paste(text) => {
                if let Some(editor) = self.editors.get_mut(self.active) {
                    editor.buffer.replace(&mut editor.selections, &text);
                }
                self.reveal();
            }
            Event::Resize(w, h) => self.size = (w, h),
            Event::Tick => {}
        }
    }

    fn reveal(&mut self) {
        let area = self.editor_area();
        if let Some(editor) = self.editors.get_mut(self.active) {
            editor.reveal(area);
        }
    }

    fn key(&mut self, key: KeyEvent) {
        if let Some(command) = simple_keymap(key) {
            self.run(command);
        } else if let Key::Char(c) = key.key
            && !key.mods.ctrl
            && !key.mods.alt
            && let Some(editor) = self.editors.get_mut(self.active)
        {
            editor
                .buffer
                .type_text(&mut editor.selections, c.encode_utf8(&mut [0; 4]));
            self.reveal();
        }
    }

    pub fn run(&mut self, command: Command) {
        let area = self.editor_area();
        match command {
            Command::Quit => self.quit = true,
            Command::ToggleSidebar => self.show_sidebar = !self.show_sidebar,
            Command::Save => self.save(),
            Command::CloseTab => self.close_tab(),
            command => {
                if let Some(editor) = self.editors.get_mut(self.active) {
                    editor.run(command, area);
                }
            }
        }
    }

    fn save(&mut self) {
        let Some(editor) = self.editors.get_mut(self.active) else {
            return;
        };
        self.message = match editor.buffer.save() {
            Ok(()) => format!("Saved {}", editor.title()),
            Err(e) => format!("Could not save: {e}"),
        };
    }

    fn close_tab(&mut self) {
        if self.active < self.editors.len() {
            self.editors.remove(self.active);
            self.active = self.active.min(self.editors.len().saturating_sub(1));
        }
    }

    fn mouse(&mut self, mouse: MouseEvent) {
        let (x, y) = (mouse.x, mouse.y);
        let in_sidebar = self.sidebar_area().is_some_and(|a| a.contains(x, y));
        match mouse.kind {
            MouseKind::ScrollUp | MouseKind::ScrollDown => {
                let delta = if mouse.kind == MouseKind::ScrollUp {
                    -WHEEL_LINES
                } else {
                    WHEEL_LINES
                };
                if in_sidebar {
                    self.explorer.scroll_by(delta);
                } else if let Some(editor) = self.editors.get_mut(self.active) {
                    editor.scroll_by(delta);
                }
            }
            MouseKind::Left if in_sidebar => self.click_explorer(y),
            MouseKind::Left if self.tabs_area().contains(x, y) => self.click_tab(x),
            MouseKind::Left if self.editor_area().contains(x, y) => {
                let area = self.editor_area();
                if let Some(editor) = self.editors.get_mut(self.active) {
                    let at = editor.position_at(area, x, y);
                    if mouse.mods.shift {
                        let anchor = editor.selections.primary().anchor;
                        editor.selections.set_single(Selection {
                            anchor,
                            head: at,
                            goal: None,
                        });
                    } else if mouse.mods.alt {
                        editor.selections.add(Selection::cursor(at));
                    } else {
                        editor.selections.set_single(Selection::cursor(at));
                    }
                    self.drag = Drag::Text;
                }
            }
            MouseKind::Drag if self.drag == Drag::Text => {
                let area = self.editor_area();
                if let Some(editor) = self.editors.get_mut(self.active) {
                    let at = editor.position_at(area, x, y);
                    let anchor = editor.selections.primary().anchor;
                    editor.selections.set_single(Selection {
                        anchor,
                        head: at,
                        goal: None,
                    });
                    editor.reveal(area);
                }
            }
            MouseKind::Up => self.drag = Drag::None,
            _ => {}
        }
    }

    fn click_explorer(&mut self, y: u16) {
        let Some(area) = self.sidebar_area() else {
            return;
        };
        let Some(index) = self.explorer.entry_at(area, y) else {
            return;
        };
        self.explorer.selected = Some(index);
        let entry = self.explorer.entries[index].clone();
        if entry.is_dir {
            self.workspace.toggle(&entry.path);
            self.explorer.refresh(&self.workspace);
        } else if let Err(e) = self.open(&entry.path) {
            self.message = format!("Could not open {}: {e}", entry.name);
        }
    }

    fn tab_label(editor: &Editor) -> String {
        let dirty = if editor.buffer.is_dirty() { "● " } else { "" };
        format!(" {dirty}{} ", editor.title())
    }

    fn click_tab(&mut self, x: u16) {
        let mut at = self.tabs_area().x;
        for (i, editor) in self.editors.iter().enumerate() {
            let width = Self::tab_label(editor).chars().count() as u16;
            if x >= at && x < at + width {
                self.active = i;
                return;
            }
            at += width + 1;
        }
    }

    pub fn draw(&self, grid: &mut Grid) {
        grid.cursor = None;
        let theme = &self.theme;
        grid.fill(grid.area(), theme.text);
        if let Some(area) = self.sidebar_area() {
            let name = self
                .workspace
                .root()
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            self.explorer.draw(grid, area, &name, theme);
        }
        self.draw_tabs(grid);
        if let Some(editor) = self.editors.get(self.active) {
            editor.draw(grid, self.editor_area(), theme, true);
        }
        self.draw_status(grid);
    }

    fn draw_tabs(&self, grid: &mut Grid) {
        let area = self.tabs_area();
        grid.fill(area, self.theme.tab);
        let mut x = area.x;
        for (i, editor) in self.editors.iter().enumerate() {
            let style = if i == self.active {
                self.theme.tab_active
            } else {
                self.theme.tab
            };
            x = grid.put_str(x, area.y, &Self::tab_label(editor), style, area.right()) + 1;
        }
    }

    fn draw_status(&self, grid: &mut Grid) {
        let y = self.size.1.saturating_sub(1);
        let width = self.size.0;
        grid.fill(Rect::new(0, y, width, 1), self.theme.bar);
        let left = if self.message.is_empty() {
            "Ctrl+P open  Ctrl+S save  Ctrl+Q quit".to_string()
        } else {
            self.message.clone()
        };
        grid.put_str(1, y, &left, self.theme.bar, width);
        if let Some(editor) = self.editors.get(self.active) {
            let head = editor.selections.primary().head;
            let rope = editor.buffer.rope();
            let line = rope.char_to_line(head);
            let right = format!(
                "Ln {}, Col {}",
                line + 1,
                head - rope.line_to_char(line) + 1
            );
            let x = width.saturating_sub(right.chars().count() as u16 + 1);
            grid.put_str(
                x,
                y,
                &right,
                Style {
                    bold: true,
                    ..self.theme.bar
                },
                width,
            );
        }
    }

    pub fn size(&self) -> (u16, u16) {
        self.size
    }

    pub fn background(&self) -> Style {
        self.theme.text
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cellix_input::{Mods, MouseEvent};
    use std::fs;
    use tempfile::TempDir;

    fn project() -> (TempDir, App) {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("alpha.txt"), "first line\nsecond line\n").unwrap();
        fs::write(dir.path().join("beta.rs"), "fn main() {}\n").unwrap();
        fs::create_dir(dir.path().join("src")).unwrap();
        fs::write(dir.path().join("src/lib.rs"), "pub fn x() {}\n").unwrap();
        let app = App::new(dir.path().to_path_buf(), (100, 30));
        (dir, app)
    }

    fn screen(app: &App) -> Grid {
        let mut grid = Grid::new(app.size().0, app.size().1, app.background());
        app.draw(&mut grid);
        grid
    }

    fn mouse(kind: MouseKind, x: u16, y: u16) -> Event {
        Event::Mouse(MouseEvent {
            kind,
            x,
            y,
            mods: Mods::default(),
        })
    }

    fn typed(app: &mut App, text: &str) {
        for c in text.chars() {
            app.handle(Event::Key(KeyEvent::plain(Key::Char(c))));
        }
    }

    /// Row of the first line containing `text`.
    fn find(grid: &Grid, text: &str) -> (u16, u16) {
        for y in 0..grid.height() {
            if let Some(i) = grid.row_text(y).find(text) {
                return (grid.row_text(y)[..i].chars().count() as u16, y);
            }
        }
        panic!("{text:?} not on screen:\n{}", grid.text());
    }

    #[test]
    fn click_in_explorer_opens_file() {
        let (_dir, mut app) = project();
        let (x, y) = find(&screen(&app), "beta.rs");
        app.handle(mouse(MouseKind::Left, x, y));
        assert!(screen(&app).text().contains("fn main() {}"));
    }

    #[test]
    fn click_on_folder_expands_it() {
        let (_dir, mut app) = project();
        let (x, y) = find(&screen(&app), "src");
        app.handle(mouse(MouseKind::Left, x, y));
        assert!(screen(&app).text().contains("lib.rs"));
        app.handle(mouse(MouseKind::Left, x, y));
        assert!(!screen(&app).text().contains("lib.rs"));
    }

    #[test]
    fn drag_selects_text() {
        let (dir, mut app) = project();
        app.open(&dir.path().join("alpha.txt")).unwrap();
        let (x, y) = find(&screen(&app), "first line");
        app.handle(mouse(MouseKind::Left, x, y));
        app.handle(mouse(MouseKind::Drag, x + 5, y));
        app.handle(mouse(MouseKind::Up, x + 5, y));
        assert_eq!(app.selected_text().as_deref(), Some("first"));
        let grid = screen(&app);
        assert_eq!(grid.cell(x, y).style.bg, Theme::default().selection);
    }

    #[test]
    fn wheel_scrolls_without_moving_the_cursor() {
        let (dir, mut app) = project();
        let long: String = (0..200).map(|i| format!("line {i}\n")).collect();
        fs::write(dir.path().join("long.txt"), long).unwrap();
        app.open(&dir.path().join("long.txt")).unwrap();
        app.handle(mouse(MouseKind::ScrollDown, 60, 10));
        let text = screen(&app).text();
        assert!(text.contains("line 3"));
        assert!(!text.contains("line 2\n"));
    }

    #[test]
    fn type_and_save_writes_file() {
        let (dir, mut app) = project();
        let path = dir.path().join("alpha.txt");
        app.open(&path).unwrap();
        typed(&mut app, "hi ");
        assert!(screen(&app).text().contains("●"));
        app.handle(Event::Key(KeyEvent::ctrl('s')));
        assert_eq!(
            fs::read_to_string(path).unwrap(),
            "hi first line\nsecond line\n"
        );
        assert!(!screen(&app).text().contains("●"));
    }

    #[test]
    fn click_tab_switches_buffer() {
        let (dir, mut app) = project();
        app.open(&dir.path().join("alpha.txt")).unwrap();
        app.open(&dir.path().join("beta.rs")).unwrap();
        let (x, y) = find(&screen(&app), " alpha.txt ");
        app.handle(mouse(MouseKind::Left, x + 2, y));
        assert!(screen(&app).text().contains("first line"));
    }

    #[test]
    fn newline_keeps_indent() {
        let (dir, mut app) = project();
        fs::write(dir.path().join("i.txt"), "    a").unwrap();
        app.open(&dir.path().join("i.txt")).unwrap();
        app.run(Command::Move(cellix_input::Direction::LineEnd, false));
        app.run(Command::Newline);
        typed(&mut app, "b");
        assert_eq!(app.active_text().unwrap(), "    a\n    b");
    }
}
