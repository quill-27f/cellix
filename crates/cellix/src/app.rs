use std::cell::RefCell;
use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};

use cellix_core::{Buffer, Selection, Workspace};
use cellix_input::{
    Command, Event, Key, KeyEvent, MouseEvent, MouseKind, Vim, VimAction, VimMode, simple_keymap,
};
use cellix_lang::Registry;
use cellix_lang::lsp::{Client, Diagnostic, LspEvent, TextEdit};
use cellix_render::{Grid, Icon, Rect, Style};
use cellix_review::{Comment, append_comments, blame_line, changed_files, current_branch};

use cellix_agent::{Request, SharedComments};

use crate::editor::{Editor, Marks};
use crate::explorer::Explorer;
use crate::palette::{COMMANDS, Item, Palette};
use crate::review::{Hit, Review};
use crate::settings::{Settings, default_server};
use crate::theme::Theme;

const ACTIVITY_WIDTH: u16 = 5;
/// Activity bar buttons, with what each one runs.
const ACTIVITY: [(Icon, Command); 4] = [
    (Icon::Files, Command::ToggleSidebar),
    (Icon::Search, Command::FindInProject),
    (Icon::Git, Command::OpenDiff),
    (Icon::Settings, Command::CommandPalette),
];
/// Cells each activity icon spans.
const ACTIVITY_ICON: (u8, u8) = (3, 2);
/// Rows from one activity icon to the next.
const ACTIVITY_STEP: u16 = 4;
const SIDEBAR_WIDTH: u16 = 28;
const SIDEBAR_MIN_WIDTH: u16 = 20;
/// Below this width the window is a side pane, as beside an agent: chrome gives way to
/// the code, and the sidebar becomes a drawer that opens over it.
const COMPACT_WIDTH: u16 = 70;
/// Columns for the menu button at the start of the tab bar, in the compact layout.
const MENU_BUTTON: u16 = 3;
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
    Scrollbar,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum PromptAction {
    NewFile(PathBuf),
    NewFolder(PathBuf),
    Rename(PathBuf),
    Delete(PathBuf),
    Comment(String, usize),
    Search,
    ReplaceFind,
    ReplaceWith(String),
    RenameSymbol,
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
    pub settings: Settings,
    /// Set in Vim mode. Other key handling stays the same underneath it.
    vim: Option<Vim>,
    /// Running servers, by language.
    servers: HashMap<&'static str, Client>,
    diagnostics: HashMap<PathBuf, Vec<Diagnostic>>,
    /// The file and revision a rename was asked against. Its edits are only valid for
    /// that text, so one runs at a time and a stale reply is dropped.
    pending_rename: Option<(PathBuf, u64)>,
    branch: Option<String>,
    /// In the compact layout, whether the sidebar is open over the code.
    drawer: bool,
    /// Set when the user asks to leave.
    pub quit: bool,
}

impl App {
    pub fn new(root: PathBuf, size: (u16, u16)) -> Self {
        let settings = Settings::load(&root);
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
            theme: Theme::named(settings.theme.as_deref()),
            registry: Registry::default(),
            drag: Drag::None,
            message: RefCell::new(String::new()),
            clipboard: String::new(),
            outgoing_clipboard: None,
            ticks: 0,
            review: None,
            comments: SharedComments::default(),
            wait_file: None,
            vim: (settings.keymap.as_deref() == Some("vim")).then(Vim::default),
            settings,
            servers: HashMap::new(),
            diagnostics: HashMap::new(),
            pending_rename: None,
            branch: None,
            drawer: false,
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
        self.attach_server(path);
        Ok(())
    }

    /// Starts this language's server if there is one. A missing server only costs the
    /// language features, so it isn't an error.
    fn attach_server(&mut self, path: &Path) {
        let Some(language) = Registry::language_name(path) else {
            return;
        };
        if !self.servers.contains_key(language) {
            let command = self
                .settings
                .language_servers
                .get(language)
                .cloned()
                .or_else(|| default_server(language));
            let root = self.workspace.root().to_path_buf();
            match command.map(|c| Client::start(&c, &root)) {
                Some(Ok(client)) => {
                    self.servers.insert(language, client);
                }
                _ => return,
            }
        }
        let index = self.active();
        let (Some(editor), Some(client)) =
            (self.editors.get_mut(index), self.servers.get_mut(language))
        else {
            return;
        };
        client.open(path, language, &editor.buffer.text());
        editor.synced = editor.buffer.revision();
    }

    /// Sends edits to servers and handles their replies. Returns true if anything changed.
    pub fn poll(&mut self) -> bool {
        for editor in &mut self.editors {
            if editor.synced == editor.buffer.revision() {
                continue;
            }
            let Some(path) = editor.buffer.path() else {
                continue;
            };
            if let Some(client) =
                Registry::language_name(path).and_then(|language| self.servers.get_mut(language))
            {
                client.change(path, &editor.buffer.text());
            }
            editor.synced = editor.buffer.revision();
        }
        let mut events = Vec::new();
        for client in self.servers.values() {
            events.extend(client.events.try_iter());
        }
        let changed = !events.is_empty();
        for event in events {
            self.lsp_event(event);
        }
        changed
    }

    fn lsp_event(&mut self, event: LspEvent) {
        match event {
            LspEvent::Diagnostics(path, items) => {
                self.diagnostics.insert(path, items);
            }
            LspEvent::Definition(path, line, column) => {
                if self.open(&path).is_ok() {
                    let area = self.focused_area();
                    if let Some(editor) = self.editor_mut() {
                        let at = editor.buffer.position(line, column);
                        editor.selections.set_single(Selection::cursor(at));
                        editor.reveal(area);
                    }
                }
            }
            LspEvent::Completion(labels) => {
                // Servers send everything in scope and expect the client to filter.
                let prefix = self
                    .editors
                    .get(self.active())
                    .map_or_else(String::new, |e| {
                        let head = e.selections.primary().head;
                        let (start, _) = e.buffer.word_at(head);
                        e.buffer
                            .rope()
                            .slice(start..head)
                            .to_string()
                            .to_lowercase()
                    });
                let labels: Vec<String> = labels
                    .into_iter()
                    .filter(|l| l.to_lowercase().starts_with(&prefix))
                    .collect();
                if !labels.is_empty() && matches!(self.overlay, Overlay::None) {
                    let items = labels.into_iter().map(Item::Completion).collect();
                    self.overlay = Overlay::Palette(Palette::new(items));
                }
            }
            LspEvent::Failed(message) => {
                self.pending_rename = None;
                self.say(message);
            }
            LspEvent::Rename(files) => {
                let Some((path, revision)) = self.pending_rename.take() else {
                    return;
                };
                let current = self
                    .editors
                    .iter()
                    .find(|e| e.buffer.path() == Some(path.as_path()))
                    .map(|e| e.buffer.revision());
                if current != Some(revision) {
                    self.say("The file changed, so the rename was dropped");
                    return;
                }
                let mut count = 0;
                for (path, edits) in files {
                    count += edits.len();
                    if let Err(e) = self.apply_edits(&path, edits) {
                        self.say(format!("Could not rename in {}: {e}", path.display()));
                    }
                }
                self.say(format!("Renamed {count} occurrences"));
            }
        }
    }

    /// Applies edits last to first, so earlier positions stay valid. Open files change in
    /// their buffers and stay unsaved, and closed files change on disk.
    fn apply_edits(&mut self, path: &Path, mut edits: Vec<TextEdit>) -> io::Result<()> {
        edits.sort_by_key(|e| std::cmp::Reverse(e.start));
        let open = self
            .editors
            .iter_mut()
            .find(|e| e.buffer.path() == Some(path));
        let mut scratch = None;
        let editor = match open {
            Some(editor) => editor,
            None => scratch.insert(Editor::new(Buffer::open(path)?)),
        };
        for edit in edits {
            let start = editor.buffer.position(edit.start.0, edit.start.1);
            let end = editor.buffer.position(edit.end.0, edit.end.1);
            editor
                .buffer
                .replace_range(&mut editor.selections, start, end, &edit.text);
        }
        if let Some(mut closed) = scratch {
            closed.buffer.save()?;
        }
        Ok(())
    }

    fn lsp_target(&mut self) -> Option<(&'static str, PathBuf, usize, usize)> {
        let editor = self.editors.get(self.active())?;
        let path = editor.buffer.path()?.to_path_buf();
        let language = Registry::language_name(&path)?;
        let rope = editor.buffer.rope();
        let head = editor.selections.primary().head;
        let line = rope.char_to_line(head);
        Some((language, path, line, head - rope.line_to_char(line)))
    }

    fn lsp_request(&mut self, command: Command) {
        self.poll();
        let Some((language, path, line, column)) = self.lsp_target() else {
            return;
        };
        let Some(client) = self.servers.get_mut(language) else {
            self.say(format!("No language server for {language}"));
            return;
        };
        match command {
            Command::GoToDefinition => client.definition(&path, line, column),
            _ => client.completion(&path, line, column),
        }
    }

    /// Replaces the word before the cursor with a completion.
    fn insert_completion(&mut self, text: &str) {
        if let Some(editor) = self.editor_mut() {
            let head = editor.selections.primary().head;
            let (start, _) = editor.buffer.word_at(head);
            editor.selections.set_single(Selection {
                anchor: start,
                head,
                goal: None,
            });
            editor.buffer.replace(&mut editor.selections, text);
        }
        self.reveal();
    }

    pub fn active_text(&self) -> Option<String> {
        self.editors.get(self.active()).map(|e| e.buffer.text())
    }

    pub fn selected_text(&self) -> Option<String> {
        self.editors.get(self.active())?.selected_text()
    }

    fn compact(&self) -> bool {
        self.size.0 < COMPACT_WIDTH
    }

    fn activity_area(&self) -> Rect {
        let width = if self.compact() { 0 } else { ACTIVITY_WIDTH };
        Rect::new(0, 0, width, self.size.1.saturating_sub(1))
    }

    fn sidebar_area(&self) -> Option<Rect> {
        let activity = self.activity_area();
        if self.compact() {
            return self
                .drawer
                .then_some(Rect::new(0, 0, self.size.0, activity.h));
        }
        // Review uses the explorer's width: side-by-side code needs it more.
        (self.show_sidebar && self.review.is_none()).then(|| {
            let width = (self.size.0 / 4).clamp(SIDEBAR_MIN_WIDTH, SIDEBAR_WIDTH);
            Rect::new(activity.right(), 0, width, activity.h)
        })
    }

    fn main_area(&self) -> Rect {
        let x = match self.sidebar_area() {
            Some(area) if !self.compact() => area.right(),
            _ => self.activity_area().right(),
        };
        Rect::new(
            x,
            0,
            self.size.0.saturating_sub(x),
            self.size.1.saturating_sub(1),
        )
    }

    fn tabs_area(&self) -> Rect {
        let main = self.main_area();
        let inset = if self.compact() { MENU_BUTTON } else { 0 };
        Rect::new(main.x + inset, main.y, main.w.saturating_sub(inset), 1)
    }

    /// Rows above the code: tabs, and breadcrumbs when there's room for them.
    fn chrome_rows(&self) -> u16 {
        if self.compact() || self.size.1 < 16 {
            1
        } else {
            2
        }
    }

    /// In the compact layout only the focused pane shows, at full width.
    fn pane_area(&self, pane: usize) -> Rect {
        let main = self.main_area();
        let rows = self.chrome_rows();
        let (y, h) = (main.y + rows, main.h.saturating_sub(rows));
        if self.compact() {
            let w = if pane == self.focus { main.w } else { 0 };
            return Rect::new(main.x, y, w, h);
        }
        let count = self.panes.len() as u16;
        let width = main.w / count.max(1);
        let x = main.x + width * pane as u16;
        let w = if pane + 1 == self.panes.len() {
            main.right() - x
        } else {
            width.saturating_sub(1)
        };
        Rect::new(x, y, w, h)
    }

    fn focused_area(&self) -> Rect {
        self.pane_area(self.focus)
    }

    pub fn handle(&mut self, event: Event) {
        // A tick isn't the user moving on, so it leaves messages up.
        if event != Event::Tick {
            self.message.borrow_mut().clear();
        }
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
        let mut reloaded = Vec::new();
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
                    Ok(()) => {
                        reloaded.extend(editor.buffer.path().map(Path::to_path_buf));
                        format!("Reloaded {}", editor.title())
                    }
                    Err(e) => format!("Could not reload {}: {e}", editor.title()),
                });
            }
        }
        if let Some(note) = note {
            self.say(note);
        }
        // A file an agent wrote is a saved file: servers only run their full check on save.
        if !reloaded.is_empty() {
            self.poll();
        }
        for path in reloaded {
            if let Some(client) =
                Registry::language_name(&path).and_then(|language| self.servers.get(language))
            {
                client.saved(&path);
            }
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
        self.branch = current_branch(&root);
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
        if let Some(vim) = &mut self.vim {
            match vim.key(key) {
                VimAction::Commands(commands) => {
                    for command in commands {
                        self.run(command);
                    }
                    return;
                }
                VimAction::Pending => return,
                VimAction::Passthrough => {}
            }
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
            Some(Item::Match(path, line, _)) => {
                if let Err(e) = self.open(&path) {
                    self.say(format!("Could not open: {e}"));
                }
                let area = self.focused_area();
                if let Some(editor) = self.editor_mut() {
                    let at = editor.buffer.position(line, 0);
                    editor.selections.set_single(Selection::cursor(at));
                    editor.reveal(area);
                }
            }
            Some(Item::Completion(text)) => self.insert_completion(&text),
            None => {}
        }
    }

    fn finish_prompt(&mut self, prompt: Prompt) {
        let name = prompt.input.trim();
        let result = match &prompt.action {
            PromptAction::Search => {
                let root = self.workspace.root().to_path_buf();
                let items: Vec<Item> = self
                    .workspace
                    .search(name)
                    .into_iter()
                    .map(|m| {
                        let file = m.path.strip_prefix(&root).unwrap_or(&m.path);
                        let label = format!("{}:{}  {}", file.display(), m.line + 1, m.text);
                        Item::Match(m.path.clone(), m.line, label)
                    })
                    .collect();
                if items.is_empty() {
                    self.say(format!("No results for {name}"));
                } else {
                    self.overlay = Overlay::Palette(Palette::new(items));
                }
                Ok(())
            }
            PromptAction::ReplaceFind => {
                if !name.is_empty() {
                    let label = format!("Replace {name} with");
                    self.ask(&label, PromptAction::ReplaceWith(name.to_string()));
                }
                Ok(())
            }
            PromptAction::ReplaceWith(query) => {
                let with = prompt.input.as_str();
                let dirty: Vec<PathBuf> = self
                    .editors
                    .iter()
                    .filter(|e| e.buffer.is_dirty())
                    .filter_map(|e| e.buffer.path().map(Path::to_path_buf))
                    .collect();
                let changed = self.workspace.replace_all(query, with, &dirty);
                if let Ok(count) = changed {
                    let skipped = if dirty.is_empty() {
                        String::new()
                    } else {
                        format!(", skipped {} with unsaved edits", dirty.len())
                    };
                    self.say(format!("Replaced in {count} files{skipped}"));
                }
                changed.map(drop)
            }
            PromptAction::RenameSymbol if !name.is_empty() => {
                self.poll();
                if self.pending_rename.is_some() {
                    self.say("A rename is already running");
                } else if let Some((language, path, line, column)) = self.lsp_target()
                    && let Some(client) = self.servers.get_mut(language)
                {
                    client.rename(&path, line, column, name);
                    let index = self.active();
                    let revision = self.editors.get(index).map(|e| e.buffer.revision());
                    self.pending_rename = revision.map(|r| (path, r));
                }
                Ok(())
            }
            PromptAction::RenameSymbol => Ok(()),
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
            Command::FindInProject => self.ask("Search", PromptAction::Search),
            Command::ReplaceInProject => self.ask("Replace", PromptAction::ReplaceFind),
            Command::MarkdownPreview => {
                if let Some(editor) = self.editor_mut() {
                    editor.preview = !editor.preview;
                }
            }
            Command::GoToDefinition | Command::Complete => self.lsp_request(command),
            Command::RenameSymbol => self.ask("Rename to", PromptAction::RenameSymbol),
            Command::BlameLine => self.blame(),
            Command::ToggleSidebar if self.compact() => self.drawer = !self.drawer,
            Command::ToggleSidebar => self.show_sidebar = !self.show_sidebar,
            Command::SplitPane if self.compact() => self.say("Not enough room to split"),
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
            Command::PasteLine => self.paste_line(),
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
            Some(Hit::PreviousFile) => review.step_file(-1),
            Some(Hit::NextFile) => review.step_file(1),
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

    /// Vim's `p`: a copied line goes below this one, and other text after the cursor.
    fn paste_line(&mut self) {
        let text = self.clipboard.clone();
        let area = self.focused_area();
        if text.ends_with('\n') {
            self.run(Command::Move(cellix_input::Direction::Down, false));
            self.run(Command::Move(
                cellix_input::Direction::LineColumnZero,
                false,
            ));
        } else {
            self.run(Command::Move(cellix_input::Direction::Right, false));
        }
        if let Some(editor) = self.editor_mut() {
            editor.buffer.replace(&mut editor.selections, &text);
            editor.reveal(area);
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
        // Sent after any pending edit, so the server checks what was saved.
        self.poll();
        if let Some((language, path, ..)) = self.lsp_target()
            && let Some(client) = self.servers.get(language)
        {
            client.saved(&path);
        }
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
        if self.compact() && self.drawer && !self.sidebar_area().is_some_and(|a| a.contains(x, y)) {
            self.drawer = false;
            return;
        }
        if self.activity_area().contains(x, y) {
            if mouse.kind == MouseKind::Left {
                self.click_activity(y);
            }
            return;
        }
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
            MouseKind::Left if self.compact() && y == 0 && x < MENU_BUTTON => {
                self.run(Command::ToggleSidebar);
            }
            MouseKind::Left if self.tabs_area().contains(x, y) => self.click_tab(x),
            MouseKind::Left => {
                let Some(pane) = self.pane_at(x, y) else {
                    return;
                };
                self.focus = pane;
                let area = self.focused_area();
                if let Some(editor) = self.editor_mut() {
                    if editor.on_scrollbar(area, x) {
                        editor.scroll_to_row(area, y);
                        self.drag = Drag::Scrollbar;
                        return;
                    }
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
            MouseKind::Drag if self.drag == Drag::Scrollbar => {
                let area = self.focused_area();
                if let Some(editor) = self.editor_mut() {
                    editor.scroll_to_row(area, y);
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
        } else {
            self.drawer = false;
        }
    }

    fn click_activity(&mut self, y: u16) {
        let index = y.saturating_sub(1) / ACTIVITY_STEP;
        if let Some((_, command)) = ACTIVITY.get(index as usize) {
            if *command == Command::ToggleSidebar && self.review.is_some() {
                self.review = None;
            } else {
                self.run(*command);
            }
        }
    }

    /// The close button is the last glyph: `×`, or `●` while the file has unsaved edits.
    fn tab_label(editor: &Editor) -> String {
        let close = if editor.buffer.is_dirty() {
            "●"
        } else {
            "×"
        };
        format!("  {}  {close} ", editor.title())
    }

    /// Tabs that fit, as (index, x, width), starting late enough to show the active one.
    /// Drawing and clicking share this, so a click always lands on what's shown.
    fn visible_tabs(&self) -> Vec<(usize, u16, u16)> {
        let area = self.tabs_area();
        let widths: Vec<u16> = self
            .editors
            .iter()
            .map(|e| Self::tab_label(e).chars().count() as u16 + 1)
            .collect();
        let active = self.active().min(widths.len().saturating_sub(1));
        let mut first = 0;
        while first < active && widths[first..=active].iter().sum::<u16>() > area.w {
            first += 1;
        }
        let mut tabs = Vec::new();
        let mut x = area.x;
        for (i, width) in widths.iter().enumerate().skip(first) {
            if x >= area.right() {
                break;
            }
            tabs.push((i, x, *width - 1));
            x += width;
        }
        tabs
    }

    fn click_tab(&mut self, x: u16) {
        for (i, at, width) in self.visible_tabs() {
            if x >= at && x < at + width {
                self.set_active(i);
                // A little wider than the glyph, so it isn't a pixel hunt.
                if x + 3 >= at + width {
                    self.run(Command::CloseTab);
                }
                return;
            }
        }
    }

    pub fn draw(&self, grid: &mut Grid) {
        grid.cursor = None;
        let theme = &self.theme;
        grid.fill(grid.area(), theme.text);
        self.draw_activity(grid);
        let drawer_over_main = self.compact() && self.drawer;
        if drawer_over_main {
            self.draw_tabs(grid);
        }
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
        if drawer_over_main {
            self.draw_status(grid);
            self.draw_overlay(grid);
            return;
        }
        self.draw_tabs(grid);
        if self.editors.is_empty() {
            self.draw_welcome(grid);
        }
        self.draw_breadcrumbs(grid);
        for (pane, index) in self.panes.iter().enumerate() {
            if let Some(editor) = self.editors.get(*index) {
                let area = self.pane_area(pane);
                if area.w == 0 {
                    continue;
                }
                let file = editor.buffer.path().map(|p| self.relative(p));
                let marks = Marks {
                    comments: file.map_or_else(Vec::new, |f| self.comment_lines(&f)),
                    errors: editor
                        .buffer
                        .path()
                        .and_then(|p| self.diagnostics.get(p))
                        .map_or_else(Vec::new, |d| d.iter().map(|d| d.line + 1).collect()),
                };
                editor.draw(
                    grid,
                    area,
                    theme,
                    pane == self.focus,
                    &self.registry,
                    &marks,
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

    fn draw_activity(&self, grid: &mut Grid) {
        let area = self.activity_area();
        if area.w == 0 {
            return;
        }
        grid.fill(area, self.theme.activity);
        for (i, (icon, command)) in ACTIVITY.iter().enumerate() {
            let y = 1 + i as u16 * ACTIVITY_STEP;
            if y + 1 >= area.bottom() {
                break;
            }
            let on = match command {
                Command::ToggleSidebar => self.show_sidebar && self.review.is_none(),
                Command::OpenDiff => self.review.is_some(),
                _ => false,
            };
            let style = Style {
                fg: if on {
                    self.theme.activity_active
                } else {
                    self.theme.activity.fg
                },
                ..self.theme.activity
            };
            if on {
                let bar = Style {
                    fg: self.theme.accent,
                    ..self.theme.activity
                };
                for row in 0..u16::from(ACTIVITY_ICON.1) {
                    grid.put_str(area.x, y + row, "▌", bar, area.right());
                }
            }
            grid.put_icon(area.x + 1, y, *icon, ACTIVITY_ICON, style);
        }
    }

    fn draw_tabs(&self, grid: &mut Grid) {
        let main = self.main_area();
        grid.fill(Rect::new(main.x, main.y, main.w, 1), self.theme.tab);
        if self.compact() {
            grid.put_str(main.x, main.y, " ☰ ", self.theme.tab, main.right());
        }
        let area = self.tabs_area();
        for (i, x, _) in self.visible_tabs() {
            let style = if i == self.active() {
                self.theme.tab_active
            } else {
                self.theme.tab
            };
            grid.put_str(
                x,
                area.y,
                &Self::tab_label(&self.editors[i]),
                style,
                area.right(),
            );
        }
    }

    fn draw_breadcrumbs(&self, grid: &mut Grid) {
        if self.chrome_rows() < 2 {
            return;
        }
        let Some(path) = self
            .editors
            .get(self.active())
            .and_then(|e| e.buffer.path())
        else {
            return;
        };
        let main = self.main_area();
        let text = format!("  {}", self.relative(path).replace('/', " › "));
        grid.fill(Rect::new(main.x, 1, main.w, 1), self.theme.breadcrumb);
        grid.put_str(main.x, 1, &text, self.theme.breadcrumb, main.right());
    }

    fn draw_welcome(&self, grid: &mut Grid) {
        let main = self.main_area();
        let rows = [
            "Cellix",
            "",
            "Ctrl+P          Open a file",
            "Ctrl+Shift+P    Commands",
            "Ctrl+G          Review changes",
            "Ctrl+Shift+F    Search the project",
        ];
        let y = main.y + main.h / 3;
        let x = main.x + main.w.saturating_sub(30) / 2;
        for (i, row) in rows.iter().enumerate() {
            let style = Style {
                bold: i == 0,
                fg: if i == 0 {
                    self.theme.text.fg
                } else {
                    self.theme.gutter.fg
                },
                ..self.theme.text
            };
            grid.put_str(x, y + i as u16, row, style, main.right());
        }
    }

    fn draw_status(&self, grid: &mut Grid) {
        let y = self.size.1.saturating_sub(1);
        let width = self.size.0;
        let bar = self.theme.status;
        grid.fill(Rect::new(0, y, width, 1), bar);
        let message = self.message.borrow();
        let at_cursor = self.cursor_line().and_then(|(path, line)| {
            let items = self.diagnostics.get(&path)?;
            items
                .iter()
                .find(|d| d.line + 1 == line)
                .map(|d| d.message.clone())
        });
        let hint = if self.compact() {
            "Ctrl+P open"
        } else {
            "Ctrl+P open  Ctrl+S save  Ctrl+G review  Ctrl+Q quit"
        };
        let left = if !message.is_empty() {
            message.as_str()
        } else if let Some(problem) = &at_cursor {
            problem.as_str()
        } else {
            hint
        };
        // The cursor always shows, and the rest stays only while it leaves `left` room.
        let mut right = Vec::new();
        if let Some(editor) = self.editors.get(self.active()) {
            let head = editor.selections.primary().head;
            let rope = editor.buffer.rope();
            let line = rope.char_to_line(head);
            let column = head - rope.line_to_char(line) + 1;
            let language = editor
                .buffer
                .path()
                .and_then(Registry::language_name)
                .unwrap_or("Plain Text");
            let mode = match self.vim.as_ref().map(|v| v.mode) {
                Some(VimMode::Normal) => "NORMAL",
                Some(VimMode::Insert) => "INSERT",
                Some(VimMode::Visual) => "VISUAL",
                None => "",
            };
            right = vec![
                format!("{mode}   Ln {}, Col {column}", line + 1)
                    .trim_start()
                    .to_string(),
                language.to_string(),
                "UTF-8".to_string(),
                "Spaces: 4".to_string(),
            ];
        }
        let room = width.saturating_sub(left.chars().count().min(24) as u16 + 4) as usize;
        let mut shown = right.clone();
        while shown.len() > 1 && shown.join("   ").chars().count() > room {
            shown.pop();
        }
        // Reads like VS Code, with the language last.
        if shown.len() > 2 {
            shown.rotate_left(2);
            let count = shown.len();
            shown.swap(count - 2, count - 1);
        }
        let right = shown.join("   ");
        let right_x = width.saturating_sub(right.chars().count() as u16 + 1);
        let mut x = 1;
        if !self.compact() {
            if let Some(branch) = &self.branch {
                let piece = (Icon::Git, (1, 1));
                grid.put_icon(x, y, piece.0, piece.1, bar);
                x = grid.put_str(x + 2, y, &format!("{branch}  "), bar, right_x);
            }
            let problems: usize = self.diagnostics.values().map(Vec::len).sum();
            if problems > 0 {
                x = grid.put_str(x, y, &format!("⊗ {problems}  "), bar, right_x);
            }
        }
        grid.put_str(x, y, left, bar, right_x.saturating_sub(1));
        grid.put_str(right_x, y, &right, bar, width);
    }

    pub fn rename_running(&self) -> bool {
        self.pending_rename.is_some()
    }

    pub fn overlay_is_open(&self) -> bool {
        !matches!(self.overlay, Overlay::None)
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
            screen(&app).row_text(y).contains(" M"),
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
    fn messages_outlive_background_ticks() {
        let (dir, mut app) = project();
        app.open(&dir.path().join("alpha.txt")).unwrap();
        app.run(Command::Save);
        app.handle(Event::Tick);
        assert!(screen(&app).text().contains("Saved alpha.txt"));
    }

    fn vim_app(text: &str) -> (TempDir, App) {
        let (dir, _) = project();
        fs::create_dir(dir.path().join(".cellix")).unwrap();
        fs::write(
            dir.path().join(".cellix/settings.json"),
            r#"{"keymap": "vim"}"#,
        )
        .unwrap();
        fs::write(dir.path().join("v.txt"), text).unwrap();
        let mut app = App::new(dir.path().to_path_buf(), (100, 30));
        app.open(&dir.path().join("v.txt")).unwrap();
        (dir, app)
    }

    #[test]
    fn vim_motions_and_operators() {
        let (_dir, mut app) = vim_app("one two three\nsecond\nthird\n");
        assert!(screen(&app).text().contains("NORMAL"));
        typed(&mut app, "wdw");
        assert_eq!(app.active_text().unwrap(), "one three\nsecond\nthird\n");
        typed(&mut app, "jdd");
        assert_eq!(app.active_text().unwrap(), "one three\nthird\n");
        typed(&mut app, "ggyyGp");
        assert!(app.active_text().unwrap().contains("one three\n"));
        typed(&mut app, "u");
        typed(&mut app, "ggx");
        assert_eq!(app.active_text().unwrap(), "ne three\nthird\n");
        typed(&mut app, "u");
        typed(&mut app, "A!");
        assert!(screen(&app).text().contains("INSERT"));
        app.handle(Event::Key(KeyEvent::plain(Key::Esc)));
        assert_eq!(app.active_text().unwrap(), "one three!\nthird\n");
    }

    #[test]
    fn vim_counts_and_change() {
        let (_dir, mut app) = vim_app("a\nb\nc\nd\n");
        typed(&mut app, "2dd");
        assert_eq!(app.active_text().unwrap(), "c\nd\n");
        typed(&mut app, "cwZ");
        assert_eq!(app.active_text().unwrap(), "Z\nd\n");
    }

    #[test]
    fn vim_visual_deletes_the_selection() {
        let (_dir, mut app) = vim_app("hello world\n");
        typed(&mut app, "vlld");
        assert_eq!(app.active_text().unwrap(), "lo world\n");
    }

    #[test]
    fn simple_is_the_default_keymap() {
        let (dir, mut app) = project();
        app.open(&dir.path().join("alpha.txt")).unwrap();
        typed(&mut app, "dd");
        assert!(app.active_text().unwrap().starts_with("ddfirst"));
        assert!(!screen(&app).text().contains("NORMAL"));
    }

    #[test]
    fn project_search_opens_a_result() {
        let (dir, mut app) = project();
        fs::write(dir.path().join("found.txt"), "a\nb\nneedle here\n").unwrap();
        app.run(Command::FindInProject);
        typed(&mut app, "needle");
        enter(&mut app);
        assert!(screen(&app).text().contains("found.txt:3"));
        enter(&mut app);
        assert!(screen(&app).text().contains("Ln 3, Col 1"));
    }

    #[test]
    fn project_replace_changes_all_matches() {
        let (dir, mut app) = project();
        fs::write(dir.path().join("a.txt"), "old old\n").unwrap();
        fs::write(dir.path().join("src/b.txt"), "old\n").unwrap();
        fs::write(dir.path().join("keep.txt"), "old\n").unwrap();
        app.open(&dir.path().join("keep.txt")).unwrap();
        typed(&mut app, "x");
        app.run(Command::ReplaceInProject);
        typed(&mut app, "old");
        enter(&mut app);
        typed(&mut app, "new");
        enter(&mut app);
        let read = |p: &str| fs::read_to_string(dir.path().join(p)).unwrap();
        assert_eq!(read("a.txt"), "new new\n");
        assert_eq!(read("src/b.txt"), "new\n");
        assert_eq!(read("keep.txt"), "old\n", "unsaved files are skipped");
        assert!(screen(&app).text().contains("Replaced in 2 files"));
    }

    #[test]
    fn markdown_opens_with_source_and_preview_side_by_side() {
        let (dir, mut app) = project();
        fs::write(dir.path().join("n.md"), "# Title\n\ntext\n").unwrap();
        app.open(&dir.path().join("n.md")).unwrap();
        let grid = screen(&app);
        let (_, y) = find(&grid, "# Title");
        let row = grid.row_text(y);
        assert_eq!(row.matches("Title").count(), 2, "{row}");
        let x = row[..row.rfind("Title").unwrap()].chars().count() as u16;
        assert!(grid.cell(x, y).style.bold, "the preview is rendered");
        assert!(!grid.cell(find(&grid, "# Title").0, y).style.bold);

        typed(&mut app, "x");
        assert!(screen(&app).text().contains("x# Title"));
        app.run(Command::MarkdownPreview);
        assert_eq!(screen(&app).row_text(y).matches("Title").count(), 1);
    }

    #[test]
    fn typing_in_markdown_updates_the_preview() {
        let (dir, mut app) = project();
        fs::write(dir.path().join("n.md"), "body\n").unwrap();
        app.open(&dir.path().join("n.md")).unwrap();
        typed(&mut app, "# ");
        let grid = screen(&app);
        let (_, y) = find(&grid, "# body");
        assert_eq!(grid.row_text(y).matches("body").count(), 2);
    }

    #[test]
    fn narrow_panes_show_markdown_source_only() {
        let (dir, mut app) = project();
        fs::write(dir.path().join("n.md"), "# Title\n").unwrap();
        app.open(&dir.path().join("n.md")).unwrap();
        app.handle(Event::Resize(50, 20));
        assert_eq!(screen(&app).text().matches("Title").count(), 1);
    }

    #[test]
    fn clicking_x_closes_a_tab() {
        let (dir, mut app) = project();
        app.open(&dir.path().join("alpha.txt")).unwrap();
        app.open(&dir.path().join("beta.rs")).unwrap();
        let (x, y) = find(&screen(&app), "alpha.txt  ×");
        app.handle(mouse(MouseKind::Left, x + 11, y));
        let text = screen(&app).text();
        assert!(!text.contains("alpha.txt  ×"), "{text}");
        assert!(text.contains("fn main"), "{text}");
    }

    #[test]
    fn clicking_a_tab_name_does_not_close_it() {
        let (dir, mut app) = project();
        app.open(&dir.path().join("alpha.txt")).unwrap();
        let (x, y) = find(&screen(&app), "alpha.txt  ×");
        app.handle(mouse(MouseKind::Left, x + 2, y));
        assert!(screen(&app).text().contains("first line"));
    }

    #[test]
    fn dirty_tab_shows_a_dot_and_asks_before_closing() {
        let (dir, mut app) = project();
        app.open(&dir.path().join("alpha.txt")).unwrap();
        typed(&mut app, "x");
        let (x, y) = find(&screen(&app), "●");
        assert!(!screen(&app).row_text(y).contains('×'));
        app.handle(mouse(MouseKind::Left, x, y));
        assert!(screen(&app).text().contains("Save changes first"));
    }

    #[test]
    fn activity_bar_opens_review_by_mouse() {
        let (dir, mut app) = project();
        git(dir.path(), &["init", "-q"]);
        app.handle(mouse(MouseKind::Left, 2, 1 + ACTIVITY_STEP * 2));
        assert!(!screen(&app).text().contains("EXPLORER"));
        app.handle(mouse(MouseKind::Left, 2, 1));
        assert!(screen(&app).text().contains("EXPLORER"));
    }

    #[test]
    fn activity_icons_span_several_cells() {
        let (_dir, app) = project();
        let grid = screen(&app);
        let pieces = (0..ACTIVITY_WIDTH)
            .flat_map(|x| (0..5).map(move |y| (x, y)))
            .filter(|(x, y)| cellix_render::Piece::decode(&grid.cell(*x, *y).symbol).is_some())
            .count();
        assert_eq!(pieces, 6, "a 3 by 2 icon");
    }

    /// A project in a side pane the size of an agent's mod panel.
    fn narrow() -> (TempDir, App) {
        let (dir, mut app) = project();
        app.handle(Event::Resize(45, 30));
        app.open(&dir.path().join("alpha.txt")).unwrap();
        (dir, app)
    }

    #[test]
    fn a_narrow_pane_gives_the_code_nearly_all_its_width() {
        let (_dir, app) = narrow();
        let grid = screen(&app);
        let (x, _) = find(&grid, "first line");
        assert!(x <= 4, "code starts at column {x}");
        assert!(!grid.text().contains("EXPLORER"));
    }

    #[test]
    fn the_drawer_opens_over_the_code_and_closes_on_a_pick() {
        let (_dir, mut app) = narrow();
        app.handle(mouse(MouseKind::Left, 1, 0));
        let grid = screen(&app);
        assert!(grid.text().contains("EXPLORER"));
        assert!(!grid.text().contains("first line"));
        let (x, y) = find(&grid, "beta.rs");
        app.handle(mouse(MouseKind::Left, x, y));
        let text = screen(&app).text();
        assert!(text.contains("fn main"), "{text}");
        assert!(!text.contains("EXPLORER"), "{text}");
    }

    #[test]
    fn the_active_tab_stays_visible_when_tabs_overflow() {
        let (dir, mut app) = project();
        app.handle(Event::Resize(40, 30));
        for name in ["one", "two", "three", "four", "five"] {
            let path = dir.path().join(format!("{name}-long-name.txt"));
            fs::write(&path, name).unwrap();
            app.open(&path).unwrap();
        }
        assert!(screen(&app).row_text(0).contains("five-long-name"));
        let (x, y) = find(&screen(&app), "×");
        app.handle(mouse(MouseKind::Left, x, y));
        assert!(!screen(&app).text().contains("five-long-name"));
    }

    #[test]
    fn narrow_status_keeps_the_cursor_and_drops_the_rest() {
        let (_dir, app) = narrow();
        let grid = screen(&app);
        let status = grid.row_text(29);
        assert!(status.contains("Ln 1, Col 1"), "{status}");
        assert!(status.contains("Ctrl+P open"), "{status}");
        assert!(!status.contains("Spaces"), "{status}");
    }

    #[test]
    fn narrow_panes_refuse_to_split() {
        let (_dir, mut app) = narrow();
        app.run(Command::SplitPane);
        assert!(screen(&app).text().contains("Not enough room"));
    }

    #[test]
    fn narrow_review_stacks_changes_in_one_column() {
        let (_dir, mut app) = reviewed();
        app.handle(Event::Resize(50, 30));
        app.run(Command::OpenDiff);
        let grid = screen(&app);
        assert_eq!(find(&grid, "second line").1 + 1, find(&grid, "CHANGED").1);
        let (x, y) = find(&grid, "[Reject]");
        app.handle(mouse(MouseKind::Left, x + 1, y));
        assert!(screen(&app).text().contains("No changes"));
    }

    #[test]
    fn a_split_stays_usable_after_shrinking() {
        let (dir, mut app) = project();
        app.open(&dir.path().join("alpha.txt")).unwrap();
        app.run(Command::SplitPane);
        app.open(&dir.path().join("beta.rs")).unwrap();
        app.handle(Event::Resize(45, 30));
        let text = screen(&app).text();
        assert!(text.contains("fn main"), "{text}");
        assert!(!text.contains("first line"), "{text}");
    }

    #[test]
    fn status_bar_shows_branch_and_language() {
        let (dir, mut app) = reviewed();
        app.open(&dir.path().join("beta.rs")).unwrap();
        let text = screen(&app).text();
        assert!(text.contains("master") || text.contains("main"), "{text}");
        assert!(text.contains("Rust"), "{text}");
    }

    #[test]
    fn breadcrumbs_show_the_path() {
        let (dir, mut app) = project();
        app.open(&dir.path().join("src/lib.rs")).unwrap();
        assert!(screen(&app).text().contains("src › lib.rs"));
    }

    #[test]
    fn scrollbar_click_jumps_through_the_file() {
        let (dir, mut app) = project();
        let long: String = (0..300).map(|i| format!("line {i}\n")).collect();
        fs::write(dir.path().join("long.txt"), long).unwrap();
        app.open(&dir.path().join("long.txt")).unwrap();
        let area = app.focused_area();
        app.handle(mouse(MouseKind::Left, area.right() - 1, area.bottom() - 1));
        assert!(
            screen(&app).text().contains("line 29"),
            "{}",
            screen(&app).text()
        );
    }

    #[test]
    fn indented_lines_show_guides() {
        let (dir, mut app) = project();
        fs::write(dir.path().join("i.txt"), "        deep\n").unwrap();
        app.open(&dir.path().join("i.txt")).unwrap();
        let grid = screen(&app);
        let (x, y) = find(&grid, "deep");
        assert_eq!(grid.cell(x - 8, y).symbol, "│");
        assert_eq!(grid.cell(x - 4, y).symbol, "│");
        assert_eq!(app.active_text().unwrap(), "        deep\n");
    }

    #[test]
    fn the_cursor_line_is_highlighted() {
        let (dir, mut app) = project();
        app.open(&dir.path().join("alpha.txt")).unwrap();
        let grid = screen(&app);
        let (x, y) = find(&grid, "first line");
        assert_eq!(grid.cell(x, y).style.bg, Theme::default().line);
        let (x, y) = find(&grid, "second line");
        assert_eq!(grid.cell(x, y).style.bg, Theme::default().text.bg);
    }

    #[test]
    fn settings_change_theme() {
        let (dir, _) = project();
        fs::create_dir(dir.path().join(".cellix")).unwrap();
        fs::write(
            dir.path().join(".cellix/settings.json"),
            r#"{"theme": "light"}"#,
        )
        .unwrap();
        let light = App::new(dir.path().to_path_buf(), (100, 30));
        assert_eq!(light.background(), Theme::named(Some("light")).text);
        assert_ne!(light.background(), Theme::default().text);
    }

    /// Scripted language server: answers the handshake, reports one diagnostic and
    /// jumps every definition to line 2.
    const FAKE_SERVER: &str = r#"
import json, sys
def read():
    n = 0
    while True:
        h = sys.stdin.buffer.readline()
        if not h: sys.exit()
        h = h.strip()
        if not h: break
        if h.lower().startswith(b"content-length:"): n = int(h.split(b":")[1])
    return json.loads(sys.stdin.buffer.read(n))
def send(m):
    b = json.dumps(m).encode()
    sys.stdout.buffer.write(b"Content-Length: %d\r\n\r\n" % len(b) + b)
    sys.stdout.buffer.flush()
while True:
    m = read()
    method, i = m.get("method"), m.get("id")
    if method == "initialize":
        send({"jsonrpc": "2.0", "id": i, "result": {"capabilities": {}}})
    elif method == "textDocument/didOpen":
        uri = m["params"]["textDocument"]["uri"]
        send({"jsonrpc": "2.0", "method": "textDocument/publishDiagnostics", "params": {"uri": uri,
            "diagnostics": [{"range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 1}},
            "message": "fake problem here", "severity": 1}]}})
    elif method == "textDocument/definition":
        uri = m["params"]["textDocument"]["uri"]
        send({"jsonrpc": "2.0", "id": i, "result": {"uri": uri,
            "range": {"start": {"line": 1, "character": 3}, "end": {"line": 1, "character": 4}}}})
    elif method == "textDocument/completion":
        send({"jsonrpc": "2.0", "id": i, "result": [{"label": "completed_name"}]})
    elif method == "textDocument/rename":
        uri = m["params"]["textDocument"]["uri"]
        send({"jsonrpc": "2.0", "id": i, "result": {"changes": {uri: [{"newText": m["params"]["newName"],
            "range": {"start": {"line": 0, "character": 3}, "end": {"line": 0, "character": 7}}}]}}})
    elif i is not None:
        send({"jsonrpc": "2.0", "id": i, "result": None})
"#;

    fn with_server() -> (TempDir, App) {
        let (dir, _) = project();
        fs::write(dir.path().join("server.py"), FAKE_SERVER).unwrap();
        fs::create_dir(dir.path().join(".cellix")).unwrap();
        let settings = serde_json::json!({"language_servers": {
            "Rust": ["python3", "-I", dir.path().join("server.py")]
        }});
        fs::write(
            dir.path().join(".cellix/settings.json"),
            settings.to_string(),
        )
        .unwrap();
        let mut app = App::new(dir.path().to_path_buf(), (100, 30));
        fs::write(dir.path().join("beta.rs"), "fn main() {}\nfn other() {}\n").unwrap();
        app.open(&dir.path().join("beta.rs")).unwrap();
        (dir, app)
    }

    fn wait(app: &mut App, done: impl Fn(&App) -> bool) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !done(app) {
            assert!(
                std::time::Instant::now() < deadline,
                "{}",
                screen(app).text()
            );
            app.poll();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    #[test]
    fn lsp_diagnostics_are_shown() {
        let (_dir, mut app) = with_server();
        wait(&mut app, |a| screen(a).text().contains("fake problem here"));
        let grid = screen(&app);
        let (x, y) = find(&grid, "fn main");
        assert_eq!(grid.cell(x - 2, y).style.fg, Theme::default().error);
    }

    #[test]
    fn lsp_goto_definition_jumps() {
        let (_dir, mut app) = with_server();
        app.run(Command::GoToDefinition);
        wait(&mut app, |a| screen(a).text().contains("Ln 2, Col 4"));
    }

    #[test]
    fn lsp_completion_replaces_the_word() {
        let (_dir, mut app) = with_server();
        typed(&mut app, "");
        app.run(Command::Complete);
        wait(&mut app, |a| screen(a).text().contains("completed_name"));
        enter(&mut app);
        assert!(
            app.active_text()
                .unwrap()
                .starts_with("completed_namefn main")
        );
    }

    #[test]
    fn lsp_rename_edits_the_open_buffer() {
        let (_dir, mut app) = with_server();
        app.run(Command::RenameSymbol);
        typed(&mut app, "start");
        enter(&mut app);
        wait(&mut app, |a| {
            a.active_text().unwrap().starts_with("fn start()")
        });
    }

    /// Needs rust-analyzer. Run with `harness/x test real_rust_analyzer -- --ignored`.
    #[test]
    #[ignore = "needs a real language server"]
    fn real_rust_analyzer_finds_a_definition() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("src")).unwrap();
        fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        fs::write(
            dir.path().join("src/main.rs"),
            "fn helper() -> i32 {\n    1\n}\n\nfn main() {\n    helper();\n}\n",
        )
        .unwrap();
        let mut app = App::new(dir.path().to_path_buf(), (100, 30));
        app.open(&dir.path().join("src/main.rs")).unwrap();
        for _ in 0..5 {
            app.run(Command::Move(cellix_input::Direction::Down, false));
        }
        for _ in 0..5 {
            app.run(Command::Move(cellix_input::Direction::Right, false));
        }
        assert!(screen(&app).text().contains("Ln 6, Col 6"));
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
        while !screen(&app).text().contains("Ln 1, Col 4") {
            assert!(
                std::time::Instant::now() < deadline,
                "{}",
                screen(&app).text()
            );
            app.run(Command::GoToDefinition);
            std::thread::sleep(std::time::Duration::from_millis(500));
            app.poll();
        }
    }

    /// Needs rust-analyzer. Covers what the scripted server can't: real indexing,
    /// real edits and real compiler output.
    #[test]
    #[ignore = "needs a real language server"]
    fn real_rust_analyzer_completes_renames_and_reports_errors() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("src")).unwrap();
        fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        fs::write(
            dir.path().join("src/main.rs"),
            "fn helper() -> i32 {\n    1\n}\n\nfn main() {\n    helper();\n}\n",
        )
        .unwrap();
        // Servers report canonical paths, as the real app uses from startup.
        let root = dir.path().canonicalize().unwrap();
        let path = root.join("src/main.rs");
        let mut app = App::new(root, (100, 30));
        app.open(&path).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
        let wait_for =
            |app: &mut App, done: &dyn Fn(&App) -> bool, retry: &mut dyn FnMut(&mut App)| {
                while !done(app) {
                    assert!(
                        std::time::Instant::now() < deadline,
                        "{}",
                        screen(app).text()
                    );
                    retry(app);
                    std::thread::sleep(std::time::Duration::from_millis(300));
                    app.poll();
                }
            };

        // Rename from the call site, retrying until the server has indexed the project.
        for _ in 0..5 {
            app.run(Command::Move(cellix_input::Direction::Down, false));
        }
        for _ in 0..5 {
            app.run(Command::Move(cellix_input::Direction::Right, false));
        }
        wait_for(
            &mut app,
            &|a| a.active_text().unwrap().contains("fn renamed()"),
            &mut |a| {
                if !a.rename_running() {
                    a.run(Command::RenameSymbol);
                    typed(a, "renamed");
                    enter(a);
                }
            },
        );
        let text = app.active_text().unwrap();
        assert!(text.contains("    renamed();"), "{text}");

        // Complete a prefix of the new name on a fresh line in main.
        app.run(Command::Move(cellix_input::Direction::LineEnd, false));
        app.run(Command::Newline);
        typed(&mut app, "rena");
        wait_for(
            &mut app,
            &|a| a.overlay_is_open() && screen(a).text().contains("renamed"),
            &mut |a| {
                if a.overlay_is_open() {
                    a.handle(Event::Key(KeyEvent::plain(Key::Esc)));
                }
                a.run(Command::Complete);
            },
        );
        app.handle(Event::Key(KeyEvent::plain(Key::Esc)));
        app.run(Command::Undo);
    }

    /// Needs rust-analyzer. The error is already on disk when the server starts.
    #[test]
    #[ignore = "needs a real language server"]
    fn real_rust_analyzer_reports_compiler_errors() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("src")).unwrap();
        fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        let root = dir.path().canonicalize().unwrap();
        let path = root.join("src/main.rs");
        fs::write(
            &path,
            "fn main() {\n    let x: i32 = \"text\";\n    println!(\"{x}\");\n}\n",
        )
        .unwrap();
        let mut app = App::new(root, (100, 30));
        app.open(&path).unwrap();
        app.run(Command::Move(cellix_input::Direction::Down, false));
        app.run(Command::Save);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(90);
        while !screen(&app).text().contains("mismatched types") {
            assert!(
                std::time::Instant::now() < deadline,
                "{}",
                screen(&app).text()
            );
            app.handle(Event::Key(KeyEvent::plain(Key::Esc)));
            app.poll();
            std::thread::sleep(std::time::Duration::from_millis(300));
        }
    }

    /// Needs rust-analyzer. The main case: an agent rewrites a file that's open.
    #[test]
    #[ignore = "needs a real language server"]
    fn real_rust_analyzer_reports_an_error_an_agent_wrote() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("src")).unwrap();
        fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        let root = dir.path().canonicalize().unwrap();
        let path = root.join("src/main.rs");
        fs::write(
            &path,
            "fn main() {\n    let x: i32 = 1;\n    println!(\"{x}\");\n}\n",
        )
        .unwrap();
        let mut app = App::new(root, (100, 30));
        app.open(&path).unwrap();
        let settle = std::time::Instant::now();
        while settle.elapsed() < std::time::Duration::from_secs(6) {
            app.poll();
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        fs::write(
            &path,
            "fn main() {\n    let x: i32 = \"text\";\n    println!(\"{x}\");\n}\n",
        )
        .unwrap();
        app.handle(Event::Tick);
        app.run(Command::Move(cellix_input::Direction::Top, false));
        app.run(Command::Move(cellix_input::Direction::Down, false));
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(40);
        while !screen(&app).text().contains("mismatched types") {
            assert!(
                std::time::Instant::now() < deadline,
                "{}",
                screen(&app).text()
            );
            app.handle(Event::Key(KeyEvent::plain(Key::Esc)));
            app.handle(Event::Tick);
            app.poll();
            std::thread::sleep(std::time::Duration::from_millis(300));
        }
    }

    /// Needs rust-analyzer. An error typed into a clean file, as opposed to one already
    /// on disk when the server started.
    #[test]
    #[ignore = "needs a real language server"]
    fn real_rust_analyzer_reports_an_error_typed_after_opening() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("src")).unwrap();
        fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        let root = dir.path().canonicalize().unwrap();
        let path = root.join("src/main.rs");
        fs::write(
            &path,
            "fn main() {\n    let x: i32 = 1;\n    println!(\"{x}\");\n}\n",
        )
        .unwrap();
        let mut app = App::new(root, (100, 30));
        app.open(&path).unwrap();
        // Long enough for the first check to finish, so this tests a later one.
        let settle = std::time::Instant::now();
        while settle.elapsed() < std::time::Duration::from_secs(6) {
            app.poll();
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        app.run(Command::Move(cellix_input::Direction::Down, false));
        app.run(Command::Move(cellix_input::Direction::LineEnd, false));
        app.run(Command::Move(cellix_input::Direction::Left, false));
        app.run(Command::DeleteBackward);
        typed(&mut app, "\"text\"");
        app.run(Command::Save);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        while !screen(&app).text().contains("mismatched types") {
            assert!(
                std::time::Instant::now() < deadline,
                "{}",
                screen(&app).text()
            );
            app.handle(Event::Key(KeyEvent::plain(Key::Esc)));
            app.poll();
            std::thread::sleep(std::time::Duration::from_millis(300));
        }
    }

    #[test]
    fn a_rename_is_dropped_if_the_file_changed_while_waiting() {
        let (_dir, mut app) = with_server();
        app.run(Command::RenameSymbol);
        typed(&mut app, "start");
        enter(&mut app);
        assert!(app.rename_running());
        app.run(Command::RenameSymbol);
        typed(&mut app, "again");
        enter(&mut app);
        assert!(screen(&app).text().contains("already running"));
        typed(&mut app, "x");
        wait(&mut app, |a| !a.rename_running());
        assert!(screen(&app).text().contains("rename was dropped"));
        assert!(app.active_text().unwrap().starts_with("xfn main"));
    }

    #[test]
    fn server_errors_are_shown_in_its_own_words() {
        let (_dir, mut app) = with_server();
        app.lsp_event(LspEvent::Failed("No references found".into()));
        assert!(screen(&app).text().contains("No references found"));
    }

    #[test]
    fn a_missing_server_is_not_an_error() {
        let (dir, _) = project();
        fs::create_dir(dir.path().join(".cellix")).unwrap();
        let settings = r#"{"language_servers": {"Rust": ["cellix-no-such-server"]}}"#;
        fs::write(dir.path().join(".cellix/settings.json"), settings).unwrap();
        let mut app = App::new(dir.path().to_path_buf(), (100, 30));
        app.open(&dir.path().join("beta.rs")).unwrap();
        app.run(Command::GoToDefinition);
        assert!(screen(&app).text().contains("No language server for Rust"));
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
