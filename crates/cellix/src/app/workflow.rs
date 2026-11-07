//! Saving as VS Code does it: formatting on save, and saving on its own after a pause or as
//! the editor loses the keys.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use cellix_input::{Key, KeyEvent, Mods};
use cellix_lang::Registry;

use cellix_render::{Bounds, Grid, Rect, Style};

use super::{App, Overlay, WorkspaceRequest, format_with};
use crate::keys::Bindings;
use crate::palette::Palette;
use crate::recent::Recent;

/// How long a save waits for the language server's formatting before saving without it.
const FORMAT_WAIT: Duration = Duration::from_millis(1500);
/// The welcome page's Recent column: how wide, how far from Start, and how many rows.
const RECENT_WIDTH: u16 = 36;
const RECENT_GAP: u16 = 6;
const RECENT_SHOWN: usize = 5;
/// VS Code's `files.autoSaveDelay`.
const AUTO_SAVE_DELAY: u64 = 1000;

/// When files save themselves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AutoSave {
    Off,
    AfterDelay,
    OnFocusChange,
}

impl AutoSave {
    pub fn from_setting(value: Option<&str>) -> Self {
        match value {
            Some("after_delay" | "afterDelay") => Self::AfterDelay,
            Some("on_focus_change" | "onFocusChange" | "on_window_change" | "onWindowChange") => {
                Self::OnFocusChange
            }
            _ => Self::Off,
        }
    }
}

/// What saving is waiting on.
#[derive(Default)]
pub struct State {
    /// A file asked to save, waiting for its server's formatting, and since when.
    format_then_save: Option<(PathBuf, Instant)>,
    /// Each unsaved file's revision and when it last changed, for saving after a pause.
    edited: HashMap<PathBuf, (u64, Instant)>,
    /// Folders and files opened lately.
    pub recent: Recent,
    /// Tasks run, and the problems they printed.
    pub tasks: super::tasks::State,
    /// The keys the person set in place of the defaults.
    pub keys: Bindings,
}

impl App {
    /// A workspace opened: it goes on the recent list, and the person's keys are read.
    pub(super) fn start_workflow(&mut self) {
        self.workflow.recent = Recent::load();
        let root = self.workspace.root().to_path_buf();
        self.workflow.recent.opened_folder(&root);
        self.workflow.keys = Bindings::new(&self.settings.keybindings);
    }

    fn auto_save(&self) -> AutoSave {
        AutoSave::from_setting(self.settings.auto_save.as_deref())
    }

    /// The editor that has the keys, if one does.
    pub(super) fn keyboard_editor(&self) -> Option<PathBuf> {
        if self.dock.focused || self.settings_page.is_some() {
            return None;
        }
        let editor = self.editors.get(self.active())?;
        editor.buffer.path().map(Path::to_path_buf)
    }

    /// After every event: saves a file that just lost the keys, or one left alone long
    /// enough, and gives up waiting on a formatter that never answered.
    pub(super) fn after_event(&mut self, before: Option<PathBuf>) {
        self.watch_tasks();
        if let Some((path, since)) = &self.workflow.format_then_save
            && since.elapsed() >= FORMAT_WAIT
        {
            let path = path.clone();
            self.workflow.format_then_save = None;
            self.save_path(&path);
        }
        match self.auto_save() {
            AutoSave::Off => {}
            AutoSave::OnFocusChange => {
                if let Some(left) = before
                    && self.keyboard_editor().as_ref() != Some(&left)
                {
                    self.save_quietly(&left, true);
                }
            }
            AutoSave::AfterDelay => self.save_after_pause(),
        }
    }

    /// Open Recent: folders, then files, newest first.
    pub(super) fn open_recent(&mut self) {
        let root = self.workspace.root().to_path_buf();
        let home = self.home.clone();
        let items = self.workflow.recent.items(&root, home.as_deref());
        if items.is_empty() {
            self.say("Nothing opened lately: folders and files appear here once opened");
            return;
        }
        self.overlay = Overlay::Palette(Palette::asking(items, "Open a recent folder or file"));
    }

    /// Columns the welcome page's Recent takes beside Start, gap included, or none when
    /// there's nothing recent or no room.
    pub(super) fn welcome_recent_width(&self, room: u16) -> u16 {
        let root = self.workspace.root();
        let wanted = super::WELCOME_WIDTH + RECENT_GAP + RECENT_WIDTH + 4;
        let any = !self.workflow.recent.newest(root, 1).is_empty();
        if any && room >= wanted {
            RECENT_GAP + RECENT_WIDTH
        } else {
            0
        }
    }

    /// The Recent column's rows: each one's place, and what it opens. The last is More….
    fn welcome_recent_rows(&self, start: Rect, first: u16) -> Vec<(Rect, Option<(PathBuf, bool)>)> {
        let main = self.main_area();
        if self.welcome_recent_width(main.w) == 0 {
            return Vec::new();
        }
        let x = start.right() + RECENT_GAP;
        let root = self.workspace.root();
        let mut rows: Vec<(Rect, Option<(PathBuf, bool)>)> = self
            .workflow
            .recent
            .newest(root, RECENT_SHOWN)
            .into_iter()
            .enumerate()
            .map(|(i, entry)| (Rect::new(x, first + i as u16, RECENT_WIDTH, 1), Some(entry)))
            .collect();
        let more = first + rows.len() as u16;
        rows.push((Rect::new(x, more, RECENT_WIDTH, 1), None));
        rows
    }

    /// Recent, beside Start: each folder or file by name, where it is in grey.
    pub(super) fn draw_welcome_recent(&self, grid: &mut Grid, start: Rect, first: u16) {
        let rows = self.welcome_recent_rows(start, first);
        let Some((top, _)) = rows.first() else {
            return;
        };
        let theme = &self.theme;
        let quiet = Style {
            fg: theme.gutter.fg,
            ..theme.text
        };
        let heading = Style {
            bold: true,
            ..quiet
        };
        let (x, right) = (top.x, top.right());
        grid.label_at(x as f32, x, first - 1, "RECENT", heading, right, 80);
        let link = Style {
            fg: theme.accent,
            ..theme.text
        };
        let home = self.home.as_deref();
        for (row, entry) in &rows {
            if self.hover.is_some_and(|h| row.contains(h.0, h.1)) {
                let wash = Bounds::cells(
                    row.x as f32 - 0.6,
                    row.y as f32 + 0.04,
                    row.w as f32 + 1.2,
                    0.92,
                );
                grid.panel(wash, 16, Some(theme.row_hover), None);
            }
            let Some((path, _)) = entry else {
                grid.label_at(x as f32, x, row.y, "More…", quiet, right, 95);
                continue;
            };
            let name = path.file_name().map_or_else(
                || path.display().to_string(),
                |n| n.to_string_lossy().into_owned(),
            );
            grid.label_at(x as f32, x, row.y, &name, link, right, 95);
            // A cell after the name as drawn, shortened from its middle to fit.
            let font = cellix_render::Font {
                size: 95,
                ..Default::default()
            };
            let from = x as f32 + self.measure.width(&name, font) + 1.0;
            if let Some(parent) = path.parent() {
                let folder = crate::recent::written(parent, home);
                let folder =
                    crate::palette::middle_out(&folder, f32::from(right) - from, &*self.measure);
                grid.label_at(from, from as u16, row.y, &folder, quiet, right, 85);
            }
        }
    }

    /// Opens what's under a click on Recent.
    pub(super) fn click_welcome_recent(&mut self, x: u16, y: u16) -> bool {
        let start = self.welcome_box();
        let first = self.welcome_rows().first().map_or(0, |row| row.0);
        let rows = self.welcome_recent_rows(start, first);
        let Some((_, entry)) = rows.into_iter().find(|(r, _)| r.contains(x, y)) else {
            return false;
        };
        match entry {
            Some((path, true)) => {
                let path = dunce::canonicalize(&path).unwrap_or(path);
                self.workspace_request = Some(WorkspaceRequest::Open(path));
            }
            Some((path, false)) => {
                if let Err(e) = self.open(&path) {
                    self.say(format!("Could not open: {e}"));
                }
            }
            None => self.open_recent(),
        }
        true
    }

    /// Lists every command's keys again on the Settings page, as they stand now.
    pub(super) fn refresh_shortcuts(&mut self) {
        let list = crate::keys::shortcuts(&self.settings.keybindings);
        if let Some(page) = &mut self.settings_page {
            page.shortcuts = list;
        }
    }

    /// Gives a command the keys pressed, or with `None` its own back, and saves that.
    pub(super) fn set_keys(&mut self, name: &str, keys: Option<String>) -> String {
        match keys {
            Some(keys) => {
                self.settings
                    .keybindings
                    .insert(name.to_string(), vec![keys]);
            }
            None => {
                self.settings.keybindings.remove(name);
            }
        }
        self.workflow.keys = Bindings::new(&self.settings.keybindings);
        self.refresh_shortcuts();
        let all: serde_json::Map<String, serde_json::Value> = self
            .settings
            .keybindings
            .iter()
            .map(|(name, keys)| (name.clone(), keys.clone().into()))
            .collect();
        super::store("keybindings", all.into())
    }

    /// While a command's new keys are pressed, every key is taken as one of them, even those
    /// that would otherwise quit or close. Enter keeps them, Escape gives up.
    pub(super) fn recording_keys(&mut self, key: KeyEvent) -> bool {
        let Some(page) = &mut self.settings_page else {
            return false;
        };
        let Some((index, pressed)) = page.recording.clone() else {
            return false;
        };
        match key.key {
            Key::Esc => page.recording = None,
            Key::Enter if key.mods == Mods::default() => {
                let Some(keys) = pressed else {
                    return true;
                };
                page.recording = None;
                let name = page.shortcuts[index].name;
                let note = self.set_keys(name, Some(keys.clone()));
                self.say(if note.is_empty() {
                    format!("{name} is on {keys} now")
                } else {
                    note
                });
            }
            // A letter alone would type nothing ever again.
            Key::Char(c) if !key.mods.ctrl && !key.mods.alt && c != ' ' => {
                self.say("Add Ctrl or Alt to a letter, or use a key such as F5");
            }
            _ => {
                let name = crate::keys::name(crate::keys::normal(key));
                page.recording = Some((index, Some(name)));
            }
        }
        true
    }

    /// Typing on the Keyboard section searches it.
    pub(super) fn search_shortcuts(&mut self, key: KeyEvent) -> bool {
        let Some(page) = &mut self.settings_page else {
            return false;
        };
        if page.section != crate::settings_page::Section::Keyboard {
            return false;
        }
        match key.key {
            Key::Char(c) if !key.mods.ctrl && !key.mods.alt => {
                page.keys_query.push(c);
                page.keys_top = 0;
                page.focus = None;
            }
            Key::Backspace if !page.keys_query.is_empty() => {
                page.keys_query.pop();
                page.keys_top = 0;
            }
            _ => return false,
        }
        true
    }

    /// The terminal lost focus: with saving on focus change, every unsaved file is saved.
    pub fn window_left(&mut self) {
        if self.auto_save() != AutoSave::OnFocusChange {
            return;
        }
        let paths: Vec<PathBuf> = self
            .editors
            .iter()
            .filter(|e| e.buffer.is_dirty())
            .filter_map(|e| e.buffer.path().map(Path::to_path_buf))
            .collect();
        for path in paths {
            self.save_quietly(&path, true);
        }
    }

    fn save_after_pause(&mut self) {
        let delay = Duration::from_millis(self.settings.auto_save_delay.unwrap_or(AUTO_SAVE_DELAY));
        let now = Instant::now();
        let mut due = Vec::new();
        let mut seen = HashMap::new();
        for editor in &self.editors {
            let Some(path) = editor.buffer.path() else {
                continue;
            };
            if !editor.buffer.is_dirty() {
                continue;
            }
            let revision = editor.buffer.revision();
            let (was, since) = self
                .workflow
                .edited
                .get(path)
                .copied()
                .unwrap_or((revision, now));
            let since = if was == revision { since } else { now };
            if now.duration_since(since) >= delay {
                due.push(path.to_path_buf());
            } else {
                seen.insert(path.to_path_buf(), (revision, since));
            }
        }
        self.workflow.edited = seen;
        for path in due {
            // A pause while typing: the space just typed stays.
            self.save_quietly(&path, false);
        }
    }

    /// Saves one open file without a word, as auto save does. A file changed on disk is left
    /// for the person to settle.
    fn save_quietly(&mut self, path: &Path, tidy: bool) {
        let Some(editor) = self
            .editors
            .iter_mut()
            .find(|e| e.buffer.path() == Some(path))
        else {
            return;
        };
        if !editor.buffer.is_dirty() || editor.conflict || editor.buffer.changed_on_disk() {
            return;
        }
        let saved = if tidy {
            editor.save()
        } else {
            editor.save_as_is()
        };
        if saved.is_err() {
            return;
        }
        editor.recount();
        self.poll();
        if let Some(language) = Registry::language_name(path)
            && let Some(client) = self.servers.get(language)
        {
            client.saved(path);
        }
    }

    /// Saves the open file at `path` as Ctrl+S would, if it's still open.
    fn save_path(&mut self, path: &Path) {
        let Some(index) = self
            .editors
            .iter()
            .position(|e| e.buffer.path() == Some(path))
        else {
            return;
        };
        if index == self.active() {
            self.write_file();
        } else {
            self.save_quietly(path, true);
        }
    }

    /// With format on save, formats the file in front first. True when the save now waits
    /// for the language server, which saves it once it answers.
    pub(super) fn format_before_saving(&mut self) -> bool {
        if self.settings.format_on_save != Some(true) {
            return false;
        }
        self.poll();
        let Some(editor) = self.editors.get(self.active()) else {
            return false;
        };
        // A page is prose: a code formatter has no business reflowing it.
        if editor.rendered || editor.notebook.is_some() {
            return false;
        }
        let Some(path) = editor.buffer.path().map(Path::to_path_buf) else {
            return false;
        };
        let tab_size = editor.indentation.size;
        let language = Registry::language_name(&path).unwrap_or("Plain Text");
        if let Some(client) = self.servers.get_mut(language) {
            client.format(&path, tab_size);
            self.workflow.format_then_save = Some((path, Instant::now()));
            return true;
        }
        let Some(tool) = self.settings.formatter(language) else {
            return false;
        };
        let tool: Vec<&str> = tool.iter().map(String::as_str).collect();
        let text = editor.buffer.text();
        // A missing formatter or one that fails leaves the file to save as it is.
        if let Ok(formatted) = format_with(&tool, &path.display().to_string(), &text)
            && formatted != text
            && let Some(editor) = self.editor_mut()
        {
            let end = editor.buffer.rope().len_chars();
            let head = editor.selections.primary().head;
            editor
                .buffer
                .replace_range(&mut editor.selections, 0, end, &formatted);
            let last = editor.buffer.rope().len_chars();
            editor
                .selections
                .set_single(cellix_core::Selection::cursor(head.min(last)));
        }
        false
    }

    /// The server's formatting arrived: a save waiting on it goes ahead.
    pub(super) fn formatted(&mut self, path: &Path) -> bool {
        let waiting = self
            .workflow
            .format_then_save
            .as_ref()
            .is_some_and(|(p, _)| p == path);
        if waiting {
            self.workflow.format_then_save = None;
            self.save_path(path);
        }
        waiting
    }

    /// The server couldn't format: a save waiting on it goes ahead without.
    pub(super) fn format_failed(&mut self) {
        if let Some((path, _)) = self.workflow.format_then_save.take() {
            self.save_path(&path);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use cellix_input::{Command, Event, Key, KeyEvent};
    use tempfile::TempDir;

    use super::*;

    fn project(settings: &str) -> (TempDir, App) {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join(".cellix")).unwrap();
        fs::write(dir.path().join(".cellix/settings.json"), settings).unwrap();
        fs::write(dir.path().join("a.txt"), "one\n").unwrap();
        fs::write(dir.path().join("b.txt"), "two\n").unwrap();
        let app = App::new(dir.path().to_path_buf(), (100, 30));
        (dir, app)
    }

    fn typed(app: &mut App, text: &str) {
        for c in text.chars() {
            app.handle(Event::Key(KeyEvent::plain(Key::Char(c))));
        }
    }

    #[test]
    fn auto_save_after_a_pause_saves_without_a_keypress() {
        let (dir, mut app) = project(r#"{"auto_save": "after_delay", "auto_save_delay": 50}"#);
        let a = dir.path().join("a.txt");
        app.open(&a).unwrap();
        typed(&mut app, "new ");
        app.handle(Event::Tick);
        assert_eq!(
            fs::read_to_string(&a).unwrap(),
            "one\n",
            "not before the pause"
        );
        std::thread::sleep(Duration::from_millis(120));
        app.handle(Event::Tick);
        assert_eq!(fs::read_to_string(&a).unwrap(), "new one\n");
        assert!(!app.editors[app.active()].buffer.is_dirty());
    }

    #[test]
    fn auto_save_on_focus_change_saves_the_file_that_lost_the_keys() {
        let (dir, mut app) = project(r#"{"auto_save": "on_focus_change"}"#);
        fs::write(
            dir.path().join(".editorconfig"),
            "[*]\ntrim_trailing_whitespace = true\n",
        )
        .unwrap();
        let (a, b) = (dir.path().join("a.txt"), dir.path().join("b.txt"));
        app.open(&a).unwrap();
        app.open(&b).unwrap();
        app.handle(Event::Key(KeyEvent::plain(Key::End)));
        typed(&mut app, "  ");
        assert_eq!(
            fs::read_to_string(&b).unwrap(),
            "two\n",
            "not while it has the keys"
        );
        // Going to the other tab saves it, tidied as a save by hand would be.
        let previous = KeyEvent {
            key: Key::PageUp,
            mods: Mods {
                ctrl: true,
                ..Mods::default()
            },
        };
        app.handle(Event::Key(previous));
        assert_eq!(fs::read_to_string(&b).unwrap(), "two\n");
        assert!(!app.editors.iter().any(|e| e.buffer.is_dirty()));
        // Leaving the window saves too.
        typed(&mut app, "x");
        app.window_left();
        assert_eq!(fs::read_to_string(&a).unwrap(), "xone\n");
    }

    #[test]
    fn format_on_save_runs_the_formatter_before_writing() {
        // No server, so the formatter runs: a stand-in that upper-cases.
        let (dir, mut app) = project(
            r#"{"format_on_save": true,
                "language_servers": {"Rust": ["cellix-no-such-server"]},
                "formatters": {"Rust": ["tr", "a-z", "A-Z"]}}"#,
        );
        let file = dir.path().join("x.rs");
        fs::write(&file, "fn x() {}\n").unwrap();
        app.open(&file).unwrap();
        typed(&mut app, "a");
        app.run(Command::Save);
        assert_eq!(fs::read_to_string(&file).unwrap(), "AFN X() {}\n");
        assert!(!app.editors[app.active()].buffer.is_dirty());
    }

    fn screen(app: &App) -> cellix_render::Grid {
        let mut grid = cellix_render::Grid::new(app.size().0, app.size().1, app.background());
        app.draw(&mut grid);
        grid
    }

    #[test]
    fn open_recent_lists_folders_then_files_and_opens_the_pick() {
        let (dir, mut app) = project("{}");
        let other = tempfile::tempdir().unwrap();
        // As another Cellix would have noted it.
        app.workflow.recent.opened_folder(other.path());
        app.open(&dir.path().join("a.txt")).unwrap();
        app.open(&dir.path().join("b.txt")).unwrap();
        app.run(Command::OpenRecent);
        // The palette's rows, below the tabs that also name the files.
        let grid = screen(&app);
        let list = Palette::area(grid.area());
        let row = |name: &str, other: &str| {
            (list.y + 2..list.bottom())
                .find(|y| {
                    let text = grid.row_text(*y);
                    text.contains(name) && !text.contains(other)
                })
                .unwrap_or_else(|| panic!("{name}: {}", grid.text()))
        };
        let folder_name = other
            .path()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let (folder, b, a) = (
            row(&folder_name, "a.txt"),
            row("b.txt", "a.txt"),
            row("a.txt", "b.txt"),
        );
        assert!(
            folder < b && b < a,
            "folders, then files newest first: {}",
            grid.text()
        );
        app.handle(Event::Key(KeyEvent::plain(Key::Enter)));
        assert!(matches!(
            app.workspace_request,
            Some(super::super::WorkspaceRequest::Open(_))
        ));
    }

    #[test]
    fn the_welcome_page_lists_recent_folders_and_files_beside_start() {
        let dir = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let note = other.path().join("notes.md");
        fs::write(&note, "# Notes\n").unwrap();
        let mut app = App::new(dir.path().to_path_buf(), (160, 40));
        app.workflow.recent.opened_file(&note);
        app.workflow.recent.opened_folder(other.path());
        let grid = screen(&app);
        let text = grid.text();
        assert!(text.contains("START") && text.contains("RECENT"), "{text}");
        let name = other
            .path()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let row = (0..40)
            .find(|y| grid.row_text(*y).contains(&name))
            .expect("the folder");
        assert!(
            grid.row_text(row).contains("Open a File"),
            "beside Start's first link"
        );
        assert!(grid.row_text(row + 1).contains("notes.md"));
        // Clicking the file opens it; the folder would open as a workspace.
        let x = grid.row_text(row + 1).find("notes.md").unwrap() as u16;
        let click = cellix_input::MouseEvent {
            kind: cellix_input::MouseKind::Left,
            x: x + 1,
            y: row + 1,
            fine_x: f32::from(x + 1),
            fine_y: f32::from(row + 1) + 0.5,
            mods: Default::default(),
        };
        app.handle(Event::Mouse(click));
        assert_eq!(app.active_text().unwrap(), "# Notes\n");
    }

    fn click(app: &mut App, x: u16, y: u16) {
        for kind in [cellix_input::MouseKind::Left, cellix_input::MouseKind::Up] {
            app.handle(Event::Mouse(cellix_input::MouseEvent {
                kind,
                x,
                y,
                fine_x: f32::from(x),
                fine_y: f32::from(y) + 0.5,
                mods: Default::default(),
            }));
        }
    }

    fn find(grid: &cellix_render::Grid, text: &str) -> (u16, u16) {
        for y in 0..grid.area().h {
            let row: Vec<char> = grid.row_text(y).chars().collect();
            let want: Vec<char> = text.chars().collect();
            if let Some(x) = row.windows(want.len()).position(|w| w == want.as_slice()) {
                return (x as u16, y);
            }
        }
        panic!("{text} not on screen:\n{}", grid.text());
    }

    #[test]
    fn a_command_gets_new_keys_on_the_shortcuts_page_and_they_work() {
        let (_dir, mut app) = project("{}");
        app.size = (140, 40);
        app.run(Command::OpenSettings);
        let (x, y) = find(&screen(&app), "Keyboard");
        click(&mut app, x + 1, y);
        assert!(screen(&app).text().contains("COMMAND"));
        typed(&mut app, "rerun");
        let grid = screen(&app);
        let (_, row) = find(&grid, "Rerun Last Task");
        assert!(!grid.text().contains("Save"), "the search narrows the list");
        let (x, _) = find(&grid, "Default");
        let keys_x = grid.row_text(row).find('—').expect("no keys yet") as u16;
        assert!(keys_x < x);
        click(&mut app, keys_x, row);
        assert!(screen(&app).text().contains("Press the keys"));
        let ctrl_alt_r = KeyEvent {
            key: Key::Char('r'),
            mods: Mods {
                ctrl: true,
                alt: true,
                shift: false,
            },
        };
        app.handle(Event::Key(ctrl_alt_r));
        assert!(screen(&app).text().contains("Enter saves"));
        app.handle(Event::Key(KeyEvent::plain(Key::Enter)));
        let text = screen(&app).text();
        assert!(
            text.contains("Ctrl+Alt+R") && text.contains("User"),
            "{text}"
        );
        assert_eq!(app.settings.keybindings["Rerun Last Task"], ["Ctrl+Alt+R"]);
        // A key two commands share is flagged.
        for _ in 0.."rerun".len() {
            app.handle(Event::Key(KeyEvent::plain(Key::Backspace)));
        }
        let _ = app.set_keys("Open Recent…", Some("Ctrl+R".into()));
        typed(&mut app, "ctrl+r");
        assert!(screen(&app).text().contains("2 commands"));
        // Out of Settings, the new keys run the command.
        app.handle(Event::Key(KeyEvent::plain(Key::Esc)));
        app.handle(Event::Key(ctrl_alt_r));
        assert!(screen(&app).text().contains("No tasks here"));
        app.handle(Event::Key(KeyEvent::ctrl('r')));
        assert!(screen(&app).text().contains("Nothing opened lately"));
    }
}
