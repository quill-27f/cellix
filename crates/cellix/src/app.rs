use std::cell::RefCell;
use std::io;
use std::path::{Path, PathBuf};

use cellix_core::{Buffer, Selection, Workspace};
use cellix_input::{Command, Event, Key, KeyEvent, MouseEvent, MouseKind, simple_keymap};
use cellix_lang::Registry;
use cellix_render::{Grid, Rect, Style};
use cellix_review::{Comment, append_comments, blame_line, changed_files};

use cellix_agent::{Request, SharedComments};

use crate::editor::Editor;
use crate::explorer::Explorer;
use crate::palette::{COMMANDS, Item, Palette};
use crate::review::{Hit, Review};
use crate::theme::Theme;

const SIDEBAR_WIDTH: u16 = 28;
const WHEEL_LINES: isize = 3;
const MENU_ITEMS: [&str; 4] = ["New File", "New Folder", "Rename", "Delete"];
const MENU_WIDTH: u16 = 16;
/// Ticks between explorer refreshes, so files an agent creates appear on their own.
const EXPLORER_REFRESH_TICKS: u32 = 4;

/// Which part of the screen a press started in, so a drag stays with it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Drag {
    None,
    Text,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum PromptAction {
    NewFile(PathBuf),
    NewFolder(PathBuf),
    Rename(PathBuf),
    Delete(PathBuf),
    Comment(String, usize),
    Quit,
    CloseTab,
}

struct Prompt {
    label: String,
    input: String,
    action: PromptAction,
}

struct Menu {
    x: u16,
    y: u16,
    target: PathBuf,
}

/// At most one of these takes input ahead of everything else.
enum Overlay {
    None,
    Palette(Palette),
    Prompt(Prompt),
    Menu(Menu),
}

/// All application state. It has no terminal: tests send events and read the grid.
pub struct App {
    pub workspace: Workspace,
    explorer: Explorer,
    editors: Vec<Editor>,
    /// The editor each side-by-side pane shows.
    panes: Vec<usize>,
    focus: usize,
    show_sidebar: bool,
    overlay: Overlay,
    size: (u16, u16),
    theme: Theme,
    registry: Registry,
    drag: Drag,
    message: RefCell<String>,
    clipboard: String,
    /// Text for the terminal's clipboard, sent with OSC 52 by the terminal loop.
    pub outgoing_clipboard: Option<String>,
    ticks: u32,
    review: Option<Review>,
    /// Shared with the socket thread, so agents can read comments at any time.
    pub comments: SharedComments,
    /// With `--wait`: comments on this file are appended to it on exit, so an agent that
    /// only reads the file back still gets them.
    pub wait_file: Option<PathBuf>,
    /// Set when the user asks to leave.
    pub quit: bool,
}

impl App {
    pub fn new(root: PathBuf, size: (u16, u16)) -> Self {
        let workspace = Workspace::new(root);
        let mut explorer = Explorer::default();
        explorer.refresh(&workspace);
        let mut app = Self {
            workspace,
            explorer,
            editors: Vec::new(),
            panes: vec![0],
            focus: 0,
            show_sidebar: true,
            overlay: Overlay::None,
            size,
            theme: Theme::default(),
            registry: Registry::default(),
            drag: Drag::None,
            message: RefCell::new(String::new()),
            clipboard: String::new(),
            outgoing_clipboard: None,
            ticks: 0,
            review: None,
            comments: SharedComments::default(),
            wait_file: None,
            quit: false,
        };
        app.refresh_tree();
        app
    }

    fn say(&self, text: impl Into<String>) {
        *self.message.borrow_mut() = text.into();
    }

    fn active(&self) -> usize {
        self.panes[self.focus]
    }

    fn set_active(&mut self, index: usize) {
        self.panes[self.focus] = index;
    }

    fn editor_mut(&mut self) -> Option<&mut Editor> {
        let index = self.active();
        self.editors.get_mut(index)
    }

    pub fn open(&mut self, path: &Path) -> io::Result<()> {
        if let Some(i) = self
            .editors
            .iter()
            .position(|e| e.buffer.path() == Some(path))
        {
            self.set_active(i);
            return Ok(());
        }
        self.editors.push(Editor::new(Buffer::open(path)?));
        self.set_active(self.editors.len() - 1);
        Ok(())
    }

    pub fn active_text(&self) -> Option<String> {
        self.editors.get(self.active()).map(|e| e.buffer.text())
    }

    pub fn selected_text(&self) -> Option<String> {
        self.editors.get(self.active())?.selected_text()
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

    fn pane_area(&self, pane: usize) -> Rect {
        let main = self.main_area();
        let count = self.panes.len() as u16;
        let width = main.w / count.max(1);
        let x = main.x + width * pane as u16;
        let w = if pane + 1 == self.panes.len() {
            main.right() - x
        } else {
            width.saturating_sub(1)
        };
        Rect::new(x, main.y + 1, w, main.h.saturating_sub(1))
    }

    fn focused_area(&self) -> Rect {
        self.pane_area(self.focus)
    }

    pub fn handle(&mut self, event: Event) {
        self.message.borrow_mut().clear();
        match event {
            Event::Key(key) => self.key(key),
            Event::Mouse(mouse) => self.mouse(mouse),
            Event::Paste(text) => self.paste(&text),
            Event::Resize(w, h) => self.size = (w, h),
            Event::Tick => self.tick(),
        }
    }

    fn paste(&mut self, text: &str) {
        match &mut self.overlay {
            Overlay::Palette(palette) => {
                palette.query.push_str(text);
                palette.filter();
            }
            Overlay::Prompt(prompt) => prompt.input.push_str(text),
            _ => {
                if let Some(editor) = self.editor_mut() {
                    editor.buffer.replace(&mut editor.selections, text);
                }
                self.reveal();
            }
        }
    }

    fn tick(&mut self) {
        let mut note = None;
        for editor in &mut self.editors {
            if !editor.buffer.changed_on_disk() {
                continue;
            }
            if editor.buffer.is_dirty() {
                note = Some(format!(
                    "{} changed on disk, keeping your edits",
                    editor.title()
                ));
                editor.buffer.acknowledge_disk();
            } else {
                note = Some(match editor.buffer.reload(&mut editor.selections) {
                    Ok(()) => format!("Reloaded {}", editor.title()),
                    Err(e) => format!("Could not reload {}: {e}", editor.title()),
                });
            }
        }
        if let Some(note) = note {
            self.say(note);
        }
        self.ticks += 1;
        if self.ticks.is_multiple_of(EXPLORER_REFRESH_TICKS) {
            self.refresh_tree();
            if let Some(review) = &mut self.review {
                review.refresh();
            }
        }
    }

    fn refresh_tree(&mut self) {
        self.explorer.refresh(&self.workspace);
        let root = self.workspace.root().to_path_buf();
        self.explorer.status.clear();
        for change in changed_files(&root) {
            let path = root.join(&change.path);
            for parent in path
                .ancestors()
                .skip(1)
                .take_while(|p| p.starts_with(&root) && *p != root)
            {
                self.explorer
                    .status
                    .entry(parent.to_path_buf())
                    .or_insert('•');
            }
            self.explorer.status.insert(path, change.status);
        }
    }

    fn reveal(&mut self) {
        let area = self.focused_area();
        if let Some(editor) = self.editor_mut() {
            editor.reveal(area);
        }
    }

    fn key(&mut self, key: KeyEvent) {
        if !matches!(self.overlay, Overlay::None) {
            self.overlay_key(key);
            return;
        }
        if let Some(command) = simple_keymap(key) {
            self.run(command);
        } else if let Key::Char(c) = key.key
            && !key.mods.ctrl
            && !key.mods.alt
        {
            if let Some(editor) = self.editor_mut() {
                editor
                    .buffer
                    .type_text(&mut editor.selections, c.encode_utf8(&mut [0; 4]));
            }
            self.reveal();
        }
    }

    fn overlay_key(&mut self, key: KeyEvent) {
        let overlay = std::mem::replace(&mut self.overlay, Overlay::None);
        match overlay {
            Overlay::Palette(mut palette) => match key.key {
                Key::Esc => {}
                Key::Enter => self.pick(palette.current().cloned()),
                Key::Up => {
                    palette.move_by(-1);
                    self.overlay = Overlay::Palette(palette);
                }
                Key::Down => {
                    palette.move_by(1);
                    self.overlay = Overlay::Palette(palette);
                }
                Key::Backspace => {
                    palette.query.pop();
                    palette.filter();
                    self.overlay = Overlay::Palette(palette);
                }
                Key::Char(c) if !key.mods.ctrl && !key.mods.alt => {
                    palette.query.push(c);
                    palette.filter();
                    self.overlay = Overlay::Palette(palette);
                }
                _ => self.overlay = Overlay::Palette(palette),
            },
            Overlay::Prompt(mut prompt) => match key.key {
                Key::Esc => {}
                Key::Enter => self.finish_prompt(prompt),
                Key::Backspace => {
                    prompt.input.pop();
                    self.overlay = Overlay::Prompt(prompt);
                }
                Key::Char(c) if !key.mods.ctrl && !key.mods.alt => {
                    prompt.input.push(c);
                    self.overlay = Overlay::Prompt(prompt);
                }
                _ => self.overlay = Overlay::Prompt(prompt),
            },
            Overlay::Menu(_) | Overlay::None => {}
        }
    }

    fn pick(&mut self, item: Option<Item>) {
        match item {
            Some(Item::File(path, _)) => {
                if let Err(e) = self.open(&path) {
                    self.say(format!("Could not open: {e}"));
                }
            }
            Some(Item::Command(_, command)) => self.run(command),
            None => {}
        }
    }

    fn finish_prompt(&mut self, prompt: Prompt) {
        let name = prompt.input.trim();
        let result = match &prompt.action {
            PromptAction::Comment(file, line) => {
                if !name.is_empty() {
                    self.add_comment(file.clone(), *line, name.to_string());
                }
                Ok(())
            }
            PromptAction::Quit | PromptAction::CloseTab => {
                if name.eq_ignore_ascii_case("y") {
                    self.save_dirty(matches!(prompt.action, PromptAction::CloseTab));
                }
                if !name.is_empty() {
                    match prompt.action {
                        PromptAction::Quit => self.quit = true,
                        _ => self.close_tab(),
                    }
                }
                Ok(())
            }
            PromptAction::Delete(path) if name.eq_ignore_ascii_case("y") => {
                self.workspace.delete(path)
            }
            PromptAction::Delete(_) => Ok(()),
            _ if name.is_empty() => Ok(()),
            PromptAction::NewFile(dir) => self.workspace.create_file(&dir.join(name)),
            PromptAction::NewFolder(dir) => self.workspace.create_dir(&dir.join(name)),
            PromptAction::Rename(path) => match path.parent() {
                Some(parent) => self.workspace.rename(path, &parent.join(name)),
                None => Ok(()),
            },
        };
        if let Err(e) = result {
            self.say(format!("Could not {}: {e}", prompt.label.to_lowercase()));
        }
        if let PromptAction::NewFile(dir) | PromptAction::NewFolder(dir) = &prompt.action {
            if dir != self.workspace.root() {
                self.workspace.expand(dir);
            }
            if matches!(prompt.action, PromptAction::NewFile(_)) && !name.is_empty() {
                let path = dir.join(name);
                if let Err(e) = self.open(&path) {
                    self.say(format!("Could not open: {e}"));
                }
            }
        }
        self.explorer.refresh(&self.workspace);
    }

    pub fn run(&mut self, command: Command) {
        let area = self.focused_area();
        match command {
            Command::Quit if self.editors.iter().any(|e| e.buffer.is_dirty()) => {
                self.ask("Save changes first? y/n", PromptAction::Quit);
            }
            Command::Quit => self.quit = true,
            Command::CloseTab
                if self
                    .editors
                    .get(self.active())
                    .is_some_and(|e| e.buffer.is_dirty()) =>
            {
                self.ask("Save changes first? y/n", PromptAction::CloseTab);
            }
            Command::OpenDiff => {
                self.review = match self.review {
                    Some(_) => None,
                    None => Some(Review::new(self.workspace.root().to_path_buf())),
                };
            }
            Command::AddComment => self.prompt_comment_at_cursor(),
            Command::BlameLine => self.blame(),
            Command::ToggleSidebar => self.show_sidebar = !self.show_sidebar,
            Command::Save => self.save(),
            Command::CloseTab => self.close_tab(),
            Command::NextTab => self.cycle_tab(1),
            Command::PreviousTab => self.cycle_tab(-1),
            Command::SplitPane => {
                self.panes.push(self.active());
                self.focus = self.panes.len() - 1;
            }
            Command::FocusNextPane => self.focus = (self.focus + 1) % self.panes.len(),
            Command::QuickOpen => {
                let root = self.workspace.root().to_path_buf();
                let mut files: Vec<PathBuf> = self.workspace.files();
                files.sort();
                let items = files
                    .into_iter()
                    .map(|path| {
                        let label = path
                            .strip_prefix(&root)
                            .unwrap_or(&path)
                            .to_string_lossy()
                            .into_owned();
                        Item::File(path, label)
                    })
                    .collect();
                self.overlay = Overlay::Palette(Palette::new(items));
            }
            Command::CommandPalette => {
                let items = COMMANDS
                    .iter()
                    .map(|(label, command)| Item::Command(label, *command))
                    .collect();
                self.overlay = Overlay::Palette(Palette::new(items));
            }
            Command::Copy => self.copy(),
            Command::Cut => {
                self.copy();
                if let Some(editor) = self.editor_mut()
                    && editor.selected_text().is_some()
                {
                    editor.buffer.replace(&mut editor.selections, "");
                }
            }
            Command::Paste => {
                let text = self.clipboard.clone();
                self.paste(&text);
            }
            Command::Cancel if self.review.is_some() => self.review = None,
            Command::Cancel => {
                if let Some(editor) = self.editor_mut() {
                    let head = editor.selections.primary().head;
                    editor.selections.set_single(Selection::cursor(head));
                }
            }
            command => {
                if let Some(editor) = self.editor_mut() {
                    editor.run(command, area);
                }
            }
        }
    }

    fn ask(&mut self, label: &str, action: PromptAction) {
        self.overlay = Overlay::Prompt(Prompt {
            label: label.to_string(),
            input: String::new(),
            action,
        });
    }

    fn save_dirty(&mut self, only_active: bool) {
        let active = self.active();
        for (i, editor) in self.editors.iter_mut().enumerate() {
            if editor.buffer.is_dirty() && (!only_active || i == active) {
                let _ = editor.buffer.save();
            }
        }
    }

    fn relative(&self, path: &Path) -> String {
        path.strip_prefix(self.workspace.root())
            .unwrap_or(path)
            .to_string_lossy()
            .into_owned()
    }

    fn add_comment(&mut self, file: String, line: usize, text: String) {
        if let Ok(mut comments) = self.comments.lock() {
            comments.push(Comment { file, line, text });
        }
    }

    fn comment_lines(&self, file: &str) -> Vec<usize> {
        self.comments.lock().map_or_else(
            |_| Vec::new(),
            |c| {
                c.iter()
                    .filter(|c| c.file == file)
                    .map(|c| c.line)
                    .collect()
            },
        )
    }

    fn cursor_line(&self) -> Option<(PathBuf, usize)> {
        let editor = self.editors.get(self.active())?;
        let path = editor.buffer.path()?.to_path_buf();
        let rope = editor.buffer.rope();
        Some((
            path,
            rope.char_to_line(editor.selections.primary().head) + 1,
        ))
    }

    fn prompt_comment_at_cursor(&mut self) {
        if let Some((path, line)) = self.cursor_line() {
            let file = self.relative(&path);
            self.ask(
                &format!("Comment on line {line}"),
                PromptAction::Comment(file, line),
            );
        }
    }

    fn blame(&mut self) {
        if let Some((path, line)) = self.cursor_line() {
            let file = self.relative(&path);
            let root = self.workspace.root().to_path_buf();
            let note = blame_line(&root, Path::new(&file), line);
            self.say(note.unwrap_or_else(|| "No blame for this line".to_string()));
        }
    }

    /// Runs requests from the socket thread.
    pub fn serve(&mut self, request: Request) {
        match request {
            Request::Open(path) => {
                if let Err(e) = self.open(&path) {
                    self.say(format!("Could not open {}: {e}", path.display()));
                }
            }
            Request::CommentsTaken => self.say("An agent took the review comments"),
        }
    }

    /// Writes comments into the `--wait` file. Called once, as the app exits.
    pub fn finish(&self) -> io::Result<()> {
        let Some(path) = &self.wait_file else {
            return Ok(());
        };
        let file = self.relative(path);
        let comments: Vec<Comment> = self.comments.lock().map_or_else(
            |_| Vec::new(),
            |c| c.iter().filter(|c| c.file == file).cloned().collect(),
        );
        if comments.is_empty() {
            return Ok(());
        }
        let text = std::fs::read_to_string(path)?;
        std::fs::write(path, append_comments(&text, &comments))
    }

    fn click_review(&mut self, x: u16, y: u16) {
        let area = self.main_area();
        let Some(review) = &mut self.review else {
            return;
        };
        match review.hit(area, x, y) {
            Some(Hit::File(index)) => review.select(index),
            Some(Hit::Accept(hunk)) => review.accept(hunk),
            Some(Hit::Reject(hunk)) => self.reject(hunk),
            Some(Hit::Line(file, line)) => {
                self.ask(
                    &format!("Comment on line {line}"),
                    PromptAction::Comment(file, line),
                );
            }
            None => {}
        }
    }

    /// Puts one hunk back on disk. Refuses if the file has unsaved edits, which the
    /// rejection would silently overwrite.
    fn reject(&mut self, hunk: usize) {
        let Some(review) = &self.review else {
            return;
        };
        let (Some(path), Some(text)) = (review.absolute_path(), review.rejected_text(hunk)) else {
            return;
        };
        let dirty = self
            .editors
            .iter()
            .any(|e| e.buffer.path() == Some(path.as_path()) && e.buffer.is_dirty());
        if dirty {
            self.say("Save or undo your edits to this file first");
            return;
        }
        let note = match std::fs::write(&path, text) {
            Ok(()) => "Rejected the hunk".to_string(),
            Err(e) => format!("Could not write the file: {e}"),
        };
        self.say(note);
        if let Some(review) = &mut self.review {
            review.refresh();
        }
    }

    fn copy(&mut self) {
        if let Some(text) = self.selected_text() {
            self.clipboard.clone_from(&text);
            self.outgoing_clipboard = Some(text);
        }
    }

    fn save(&mut self) {
        let Some(editor) = self.editor_mut() else {
            return;
        };
        let note = match editor.buffer.save() {
            Ok(()) => format!("Saved {}", editor.title()),
            Err(e) => format!("Could not save: {e}"),
        };
        self.say(note);
    }

    fn cycle_tab(&mut self, delta: isize) {
        let count = self.editors.len() as isize;
        if count > 0 {
            self.set_active((self.active() as isize + delta).rem_euclid(count) as usize);
        }
    }

    fn close_tab(&mut self) {
        let index = self.active();
        if index >= self.editors.len() {
            return;
        }
        self.editors.remove(index);
        let last = self.editors.len().saturating_sub(1);
        for pane in &mut self.panes {
            if *pane > index {
                *pane -= 1;
            }
            *pane = (*pane).min(last);
        }
        if self.panes.len() > 1 {
            self.panes.remove(self.focus);
            self.focus = self.focus.min(self.panes.len() - 1);
        }
    }

    fn pane_at(&self, x: u16, y: u16) -> Option<usize> {
        (0..self.panes.len()).find(|i| self.pane_area(*i).contains(x, y))
    }

    fn mouse(&mut self, mouse: MouseEvent) {
        if !matches!(self.overlay, Overlay::None) {
            self.overlay_mouse(mouse);
            return;
        }
        let (x, y) = (mouse.x, mouse.y);
        let in_sidebar = self.sidebar_area().is_some_and(|a| a.contains(x, y));
        if self.review.is_some() && !in_sidebar && self.main_area().contains(x, y) {
            match mouse.kind {
                MouseKind::Left => self.click_review(x, y),
                MouseKind::ScrollUp | MouseKind::ScrollDown => {
                    let delta = if mouse.kind == MouseKind::ScrollUp {
                        -WHEEL_LINES
                    } else {
                        WHEEL_LINES
                    };
                    if let Some(review) = &mut self.review {
                        review.scroll_by(delta);
                    }
                }
                _ => {}
            }
            return;
        }
        match mouse.kind {
            MouseKind::ScrollUp | MouseKind::ScrollDown => {
                let delta = if mouse.kind == MouseKind::ScrollUp {
                    -WHEEL_LINES
                } else {
                    WHEEL_LINES
                };
                if in_sidebar {
                    self.explorer.scroll_by(delta);
                } else if let Some(editor) = self.editor_mut() {
                    editor.scroll_by(delta);
                }
            }
            MouseKind::Right if in_sidebar => self.open_menu(x, y),
            MouseKind::Right if self.pane_at(x, y).is_some() => {
                if let Some(pane) = self.pane_at(x, y) {
                    self.focus = pane;
                }
                let area = self.focused_area();
                if let Some(editor) = self.editor_mut() {
                    let at = editor.position_at(area, x, y);
                    editor.selections.set_single(Selection::cursor(at));
                }
                self.prompt_comment_at_cursor();
            }
            MouseKind::Left if in_sidebar => self.click_explorer(y),
            MouseKind::Left if self.tabs_area().contains(x, y) => self.click_tab(x),
            MouseKind::Left => {
                let Some(pane) = self.pane_at(x, y) else {
                    return;
                };
                self.focus = pane;
                let area = self.focused_area();
                if let Some(editor) = self.editor_mut() {
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
                let area = self.focused_area();
                if let Some(editor) = self.editor_mut() {
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

    fn overlay_mouse(&mut self, mouse: MouseEvent) {
        if mouse.kind != MouseKind::Left {
            return;
        }
        let screen = Rect::new(0, 0, self.size.0, self.size.1);
        let overlay = std::mem::replace(&mut self.overlay, Overlay::None);
        match overlay {
            Overlay::Palette(mut palette) => {
                if let Some(row) = palette.row_at(screen, mouse.y) {
                    palette.selected = row;
                    self.pick(palette.current().cloned());
                }
            }
            Overlay::Menu(menu) => {
                let row = mouse.y.checked_sub(menu.y).map(usize::from);
                let inside = mouse.x >= menu.x && mouse.x < menu.x + MENU_WIDTH;
                if let Some(row) = row.filter(|r| inside && *r < MENU_ITEMS.len()) {
                    self.menu_choice(row, &menu.target);
                }
            }
            other => self.overlay = other,
        }
    }

    fn open_menu(&mut self, x: u16, y: u16) {
        let target = self.sidebar_area().and_then(|area| {
            let index = self.explorer.entry_at(area, y)?;
            self.explorer.selected = Some(index);
            Some(self.explorer.entries[index].path.clone())
        });
        let target = target.unwrap_or_else(|| self.workspace.root().to_path_buf());
        self.overlay = Overlay::Menu(Menu { x, y, target });
    }

    fn menu_choice(&mut self, row: usize, target: &Path) {
        let dir = if target.is_dir() {
            target.to_path_buf()
        } else {
            target
                .parent()
                .unwrap_or(self.workspace.root())
                .to_path_buf()
        };
        let name = target
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let (label, input, action) = match row {
            0 => (
                "New file".to_string(),
                String::new(),
                PromptAction::NewFile(dir),
            ),
            1 => (
                "New folder".to_string(),
                String::new(),
                PromptAction::NewFolder(dir),
            ),
            2 => (
                "Rename".to_string(),
                name,
                PromptAction::Rename(target.to_path_buf()),
            ),
            _ => (
                format!("Delete {name}? y/n"),
                String::new(),
                PromptAction::Delete(target.to_path_buf()),
            ),
        };
        self.overlay = Overlay::Prompt(Prompt {
            label,
            input,
            action,
        });
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
            self.say(format!("Could not open {}: {e}", entry.name));
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
                self.set_active(i);
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
        if let Some(review) = &self.review {
            let comments = self.comments.lock().map(|c| c.clone()).unwrap_or_default();
            let commented =
                |file: &str, line: usize| comments.iter().any(|c| c.file == file && c.line == line);
            review.draw(grid, self.main_area(), theme, commented);
            self.draw_status(grid);
            self.draw_overlay(grid);
            return;
        }
        self.draw_tabs(grid);
        for (pane, index) in self.panes.iter().enumerate() {
            if let Some(editor) = self.editors.get(*index) {
                let area = self.pane_area(pane);
                let file = editor.buffer.path().map(|p| self.relative(p));
                let lines = file.map_or_else(Vec::new, |f| self.comment_lines(&f));
                editor.draw(
                    grid,
                    area,
                    theme,
                    pane == self.focus,
                    &self.registry,
                    &lines,
                );
                if pane + 1 < self.panes.len() {
                    let line = Rect::new(area.right(), area.y, 1, area.h);
                    grid.fill(line, theme.tab);
                }
            }
        }
        self.draw_status(grid);
        self.draw_overlay(grid);
    }

    fn draw_overlay(&self, grid: &mut Grid) {
        let theme = &self.theme;
        match &self.overlay {
            Overlay::Palette(palette) => palette.draw(grid, theme),
            Overlay::Menu(menu) => self.draw_menu(grid, menu),
            Overlay::Prompt(prompt) => self.draw_prompt(grid, prompt),
            Overlay::None => {}
        }
    }

    fn draw_menu(&self, grid: &mut Grid, menu: &Menu) {
        for (i, item) in MENU_ITEMS.iter().enumerate() {
            let y = menu.y + i as u16;
            grid.fill(Rect::new(menu.x, y, MENU_WIDTH, 1), self.theme.bar);
            let text = format!(" {item}");
            grid.put_str(menu.x, y, &text, self.theme.bar, menu.x + MENU_WIDTH);
        }
    }

    fn draw_prompt(&self, grid: &mut Grid, prompt: &Prompt) {
        let y = self.size.1.saturating_sub(1);
        grid.fill(Rect::new(0, y, self.size.0, 1), self.theme.bar);
        let text = format!(" {}: {}", prompt.label, prompt.input);
        let end = grid.put_str(0, y, &text, self.theme.bar, self.size.0);
        grid.cursor = Some((end.min(self.size.0.saturating_sub(1)), y));
    }

    fn draw_tabs(&self, grid: &mut Grid) {
        let area = self.tabs_area();
        grid.fill(area, self.theme.tab);
        let mut x = area.x;
        for (i, editor) in self.editors.iter().enumerate() {
            let style = if i == self.active() {
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
        let message = self.message.borrow();
        let left = if message.is_empty() {
            "Ctrl+P open  Ctrl+S save  Ctrl+G review  Ctrl+Q quit"
        } else {
            message.as_str()
        };
        grid.put_str(1, y, left, self.theme.bar, width);
        if let Some(editor) = self.editors.get(self.active()) {
            let head = editor.selections.primary().head;
            let rope = editor.buffer.rope();
            let line = rope.char_to_line(head);
            let right = format!(
                "Ln {}, Col {}",
                line + 1,
                head - rope.line_to_char(line) + 1
            );
            let x = width.saturating_sub(right.chars().count() as u16 + 1);
            let style = Style {
                bold: true,
                ..self.theme.bar
            };
            grid.put_str(x, y, &right, style, width);
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

    fn enter(app: &mut App) {
        app.handle(Event::Key(KeyEvent::plain(Key::Enter)));
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

    /// Right-clicks a row in the explorer and picks a context menu entry.
    fn menu(app: &mut App, name: &str, item: &str) {
        let (x, y) = find(&screen(app), name);
        app.handle(mouse(MouseKind::Right, x, y));
        let (x, y) = find(&screen(app), item);
        app.handle(mouse(MouseKind::Left, x + 1, y));
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
    fn explorer_create_rename_delete() {
        let (dir, mut app) = project();
        menu(&mut app, "alpha.txt", "New File");
        typed(&mut app, "made.txt");
        enter(&mut app);
        assert!(dir.path().join("made.txt").exists());
        assert!(screen(&app).text().contains("made.txt"));

        menu(&mut app, "beta.rs", "Rename");
        for _ in 0.."beta.rs".len() {
            app.handle(Event::Key(KeyEvent::plain(Key::Backspace)));
        }
        typed(&mut app, "gamma.rs");
        enter(&mut app);
        assert!(dir.path().join("gamma.rs").exists());
        assert!(!dir.path().join("beta.rs").exists());

        menu(&mut app, "gamma.rs", "Delete");
        typed(&mut app, "n");
        enter(&mut app);
        assert!(dir.path().join("gamma.rs").exists(), "n must not delete");
        menu(&mut app, "gamma.rs", "Delete");
        typed(&mut app, "y");
        enter(&mut app);
        assert!(!dir.path().join("gamma.rs").exists());
        assert!(!screen(&app).text().contains("gamma.rs"));
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

    #[test]
    fn bracketed_paste_is_one_undo() {
        let (dir, mut app) = project();
        app.open(&dir.path().join("alpha.txt")).unwrap();
        app.handle(Event::Paste("many\nlines\nof text".into()));
        assert!(
            app.active_text()
                .unwrap()
                .starts_with("many\nlines\nof text")
        );
        app.run(Command::Undo);
        assert_eq!(app.active_text().unwrap(), "first line\nsecond line\n");
    }

    #[test]
    fn copy_and_paste_round_trip_with_osc52() {
        let (dir, mut app) = project();
        app.open(&dir.path().join("alpha.txt")).unwrap();
        app.run(Command::SelectAll);
        app.run(Command::Copy);
        assert_eq!(
            app.outgoing_clipboard.take().as_deref(),
            Some("first line\nsecond line\n")
        );
        app.run(Command::Cut);
        assert_eq!(app.active_text().unwrap(), "");
        app.run(Command::Paste);
        assert_eq!(app.active_text().unwrap(), "first line\nsecond line\n");
    }

    #[test]
    fn split_shows_two_panes() {
        let (dir, mut app) = project();
        app.open(&dir.path().join("alpha.txt")).unwrap();
        app.run(Command::SplitPane);
        app.open(&dir.path().join("beta.rs")).unwrap();
        let text = screen(&app).text();
        assert!(text.contains("first line"), "{text}");
        assert!(text.contains("fn main()"), "{text}");
        let (x, y) = find(&screen(&app), "first line");
        app.handle(mouse(MouseKind::Left, x, y));
        typed(&mut app, "!");
        assert!(screen(&app).text().contains("!first line"));
        assert!(!screen(&app).text().contains("!fn main"));
    }

    #[test]
    fn closing_a_tab_keeps_panes_valid() {
        let (dir, mut app) = project();
        app.open(&dir.path().join("alpha.txt")).unwrap();
        app.run(Command::SplitPane);
        app.open(&dir.path().join("beta.rs")).unwrap();
        app.run(Command::CloseTab);
        assert!(screen(&app).text().contains("first line"));
        app.run(Command::CloseTab);
        app.run(Command::CloseTab);
        screen(&app);
    }

    #[test]
    fn quick_open_fuzzy_matches() {
        let (_dir, mut app) = project();
        app.run(Command::QuickOpen);
        typed(&mut app, "lbr");
        assert!(screen(&app).text().contains("src/lib.rs"));
        assert!(!screen(&app).row_text(3).contains("alpha"));
        enter(&mut app);
        assert!(screen(&app).text().contains("pub fn x()"));
    }

    #[test]
    fn command_palette_runs_commands_by_mouse() {
        let (_dir, mut app) = project();
        app.run(Command::CommandPalette);
        typed(&mut app, "sidebar");
        let (x, y) = find(&screen(&app), "Toggle Sidebar");
        app.handle(mouse(MouseKind::Left, x, y));
        assert!(!screen(&app).text().contains("alpha.txt"));
    }

    #[test]
    fn rust_keywords_are_highlighted() {
        let (dir, mut app) = project();
        app.open(&dir.path().join("beta.rs")).unwrap();
        let grid = screen(&app);
        let (x, y) = find(&grid, "fn main");
        assert_eq!(grid.cell(x, y).style.fg, Theme::default().keyword);
        assert_eq!(grid.cell(x + 3, y).style.fg, Theme::default().function);
    }

    #[test]
    fn external_change_reloads_clean_buffer() {
        let (dir, mut app) = project();
        let path = dir.path().join("alpha.txt");
        app.open(&path).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        fs::write(&path, "written by an agent\n").unwrap();
        app.handle(Event::Tick);
        assert!(screen(&app).text().contains("written by an agent"));
        assert!(screen(&app).text().contains("Reloaded alpha.txt"));
        app.run(Command::Undo);
        assert!(screen(&app).text().contains("first line"));
    }

    #[test]
    fn external_change_keeps_unsaved_edits() {
        let (dir, mut app) = project();
        let path = dir.path().join("alpha.txt");
        app.open(&path).unwrap();
        typed(&mut app, "mine ");
        std::thread::sleep(std::time::Duration::from_millis(20));
        fs::write(&path, "theirs\n").unwrap();
        app.handle(Event::Tick);
        let text = screen(&app).text();
        assert!(text.contains("mine first line"), "{text}");
        assert!(text.contains("keeping your edits"), "{text}");
    }

    #[test]
    fn new_files_appear_in_the_explorer_by_themselves() {
        let (dir, mut app) = project();
        fs::write(dir.path().join("from-agent.txt"), "x").unwrap();
        for _ in 0..EXPLORER_REFRESH_TICKS {
            app.handle(Event::Tick);
        }
        assert!(screen(&app).text().contains("from-agent.txt"));
    }

    fn git(dir: &Path, args: &[&str]) {
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .unwrap()
            .status;
        assert!(status.success(), "git {args:?}");
    }

    /// A committed project with one agent edit to alpha.txt.
    fn reviewed() -> (TempDir, App) {
        let (dir, _) = project();
        git(dir.path(), &["init", "-q"]);
        git(dir.path(), &["config", "user.email", "t@example.com"]);
        git(dir.path(), &["config", "user.name", "Tester"]);
        git(dir.path(), &["add", "."]);
        git(dir.path(), &["commit", "-q", "-m", "base"]);
        fs::write(dir.path().join("alpha.txt"), "first line\nCHANGED\n").unwrap();
        let app = App::new(dir.path().to_path_buf(), (120, 30));
        (dir, app)
    }

    #[test]
    fn diff_view_shows_both_versions_side_by_side() {
        let (_dir, mut app) = reviewed();
        app.run(Command::OpenDiff);
        let grid = screen(&app);
        let (_, y) = find(&grid, "second line");
        assert!(grid.row_text(y).contains("CHANGED"), "{}", grid.text());
        assert_eq!(find(&grid, "first line").1 + 1, y);
    }

    #[test]
    fn reject_hunk_restores_file() {
        let (dir, mut app) = reviewed();
        app.run(Command::OpenDiff);
        let (x, y) = find(&screen(&app), "[Reject]");
        app.handle(mouse(MouseKind::Left, x + 1, y));
        assert_eq!(
            fs::read_to_string(dir.path().join("alpha.txt")).unwrap(),
            "first line\nsecond line\n"
        );
        assert!(screen(&app).text().contains("No changes"));
    }

    #[test]
    fn accept_marks_a_hunk_and_keeps_the_file() {
        let (dir, mut app) = reviewed();
        app.run(Command::OpenDiff);
        let (x, y) = find(&screen(&app), "[Accept]");
        app.handle(mouse(MouseKind::Left, x + 1, y));
        assert!(screen(&app).text().contains("accepted"));
        assert!(
            fs::read_to_string(dir.path().join("alpha.txt"))
                .unwrap()
                .contains("CHANGED")
        );
    }

    #[test]
    fn reject_refuses_to_overwrite_unsaved_edits() {
        let (dir, mut app) = reviewed();
        app.open(&dir.path().join("alpha.txt")).unwrap();
        typed(&mut app, "mine");
        app.run(Command::OpenDiff);
        let (x, y) = find(&screen(&app), "[Reject]");
        app.handle(mouse(MouseKind::Left, x + 1, y));
        assert!(
            fs::read_to_string(dir.path().join("alpha.txt"))
                .unwrap()
                .contains("CHANGED")
        );
        assert!(screen(&app).text().contains("Save or undo"));
    }

    #[test]
    fn comment_is_attached_to_line() {
        let (dir, mut app) = reviewed();
        app.run(Command::OpenDiff);
        let (x, y) = find(&screen(&app), "CHANGED");
        app.handle(mouse(MouseKind::Left, x, y));
        typed(&mut app, "why this");
        enter(&mut app);
        let comment = app.comments.lock().unwrap()[0].clone();
        assert_eq!(
            comment,
            Comment {
                file: "alpha.txt".into(),
                line: 2,
                text: "why this".into()
            }
        );
        app.run(Command::OpenDiff);
        app.open(&dir.path().join("alpha.txt")).unwrap();
        assert!(screen(&app).text().contains("◆"));
    }

    #[test]
    fn plan_review_returns_comments() {
        let (dir, mut app) = project();
        let plan = dir.path().join("plan.md");
        fs::write(&plan, "# Plan\nstep one\n").unwrap();
        app.wait_file = Some(plan.clone());
        app.open(&plan).unwrap();
        app.handle(Event::Mouse(MouseEvent {
            kind: MouseKind::Right,
            x: find(&screen(&app), "step one").0,
            y: find(&screen(&app), "step one").1,
            mods: Mods::default(),
        }));
        typed(&mut app, "split this step");
        enter(&mut app);
        app.finish().unwrap();
        assert_eq!(
            fs::read_to_string(plan).unwrap(),
            "# Plan\nstep one\n\n## Review comments\n\n- Line 2: split this step\n"
        );
    }

    #[test]
    fn explorer_shows_git_status() {
        let (dir, mut app) = reviewed();
        let (_, y) = find(&screen(&app), "alpha.txt");
        assert!(
            screen(&app).row_text(y)[..28].ends_with("M "),
            "{}",
            screen(&app).row_text(y)
        );
        fs::write(dir.path().join("beta.rs"), "changed\n").unwrap();
        for _ in 0..EXPLORER_REFRESH_TICKS {
            app.handle(Event::Tick);
        }
        let (_, y) = find(&screen(&app), "beta.rs");
        assert!(screen(&app).row_text(y).contains("M"));
    }

    #[test]
    fn blame_reports_the_commit() {
        let (dir, mut app) = reviewed();
        app.open(&dir.path().join("beta.rs")).unwrap();
        app.run(Command::BlameLine);
        assert!(screen(&app).text().contains("Tester: base"));
    }

    #[test]
    fn quit_with_unsaved_changes_asks_first() {
        let (dir, mut app) = project();
        app.open(&dir.path().join("alpha.txt")).unwrap();
        typed(&mut app, "x");
        app.run(Command::Quit);
        assert!(!app.quit);
        assert!(screen(&app).text().contains("Save changes first"));
        app.handle(Event::Key(KeyEvent::plain(Key::Esc)));
        assert!(!app.quit, "Esc must cancel");
        app.run(Command::Quit);
        typed(&mut app, "y");
        enter(&mut app);
        assert!(app.quit);
        assert!(
            fs::read_to_string(dir.path().join("alpha.txt"))
                .unwrap()
                .starts_with("xfirst")
        );
    }

    #[test]
    fn quit_can_discard_on_request() {
        let (dir, mut app) = project();
        app.open(&dir.path().join("alpha.txt")).unwrap();
        typed(&mut app, "x");
        app.run(Command::Quit);
        typed(&mut app, "n");
        enter(&mut app);
        assert!(app.quit);
        assert!(
            fs::read_to_string(dir.path().join("alpha.txt"))
                .unwrap()
                .starts_with("first")
        );
    }

    #[test]
    fn closing_a_dirty_tab_asks_first() {
        let (dir, mut app) = project();
        app.open(&dir.path().join("alpha.txt")).unwrap();
        typed(&mut app, "x");
        app.run(Command::CloseTab);
        assert!(screen(&app).text().contains("xfirst line"));
        typed(&mut app, "n");
        enter(&mut app);
        assert!(!screen(&app).text().contains("xfirst line"));
    }

    #[test]
    fn small_windows_do_not_panic() {
        let (dir, mut app) = project();
        app.open(&dir.path().join("alpha.txt")).unwrap();
        for size in [(1, 1), (5, 2), (28, 3), (29, 2), (40, 1)] {
            app.handle(Event::Resize(size.0, size.1));
            screen(&app);
        }
    }
}
