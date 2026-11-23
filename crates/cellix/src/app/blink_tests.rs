//! The caret's blink, driven through the app on a clock the test sets and read off the drawn
//! grid: half a second shown and half away, back at a key or a click, still behind another
//! window and with Settings' switch off, and a terminal pane's cursor blinking unless its
//! program asked for a steady one. On cells the terminal is told to blink its own.

use std::fs;
use std::path::Path;
use std::rc::Rc;
use std::time::{Duration, Instant};

use cellix_input::{Command, Event, Key, KeyEvent, Mods, MouseEvent, MouseKind};
use cellix_render::{CellMeasure, CursorShape, FontMeasure, Grid};
use tempfile::TempDir;

use super::App;
use super::blink::BLINK;
use crate::dock::Front;
use crate::settings_page::Control;

fn screen(app: &App) -> Grid {
    let mut grid = Grid::new(app.size().0, app.size().1, app.background());
    app.draw(&mut grid);
    grid
}

/// The tour's terminal: columns of 17 px, rows of 50 with words set for 36.
fn pixels(app: &mut App) {
    app.set_measure(Rc::new(FontMeasure::new(17, 50).with_type_row(36)));
}

fn file() -> (TempDir, App) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("alpha.txt");
    fs::write(&path, "first line\nsecond line\n").unwrap();
    let mut app = App::new(dir.path().to_path_buf(), (120, 36));
    pixels(&mut app);
    app.open(&path).unwrap();
    (dir, app)
}

fn key(app: &mut App, key: Key) {
    app.handle(Event::Key(KeyEvent {
        key,
        mods: Mods::default(),
    }));
}

fn click(app: &mut App, x: u16, y: u16) {
    for kind in [MouseKind::Left, MouseKind::Up] {
        app.handle(Event::Mouse(MouseEvent {
            kind,
            x,
            y,
            fine_x: x as f32,
            fine_y: y as f32 + 0.5,
            mods: Mods::default(),
        }));
    }
}

/// The clock `ms` milliseconds after `start`, as the app's blink reads it.
fn at(app: &mut App, start: Instant, ms: u64) {
    app.blink.now = Some(start + Duration::from_millis(ms));
}

/// The caret in a file is drawn, then not after half a second, then is again after another;
/// a key brings it back at once and starts the blink over, and so does a click. It's VS
/// Code's half second each way, and the next half is when the frame shown asks to be shown
/// again.
#[test]
fn the_caret_blinks_and_a_key_or_a_click_brings_it_back() {
    let (_dir, mut app) = file();
    let start = Instant::now();
    at(&mut app, start, 0);
    let grid = screen(&app);
    let place = (grid.cursor, grid.caret);
    assert!(place.0.is_some(), "a caret in the file");
    assert!(grid.cursor_blinks && grid.blinks());
    assert_eq!(app.next_blink(&grid), Some(start + BLINK));
    at(&mut app, start, 499);
    assert_eq!(screen(&app).cursor, place.0, "shown for half a second");
    at(&mut app, start, 600);
    let grid = screen(&app);
    assert_eq!((grid.cursor, grid.caret), (None, None), "then away");
    assert!(!grid.caret_shown());
    assert_eq!(app.next_blink(&grid), Some(start + BLINK * 2));
    at(&mut app, start, 1100);
    assert_eq!(
        (screen(&app).cursor, screen(&app).caret),
        place,
        "then back"
    );
    // A key that doesn't move it shows it, and the blink starts over from the key.
    at(&mut app, start, 1600);
    assert_eq!(screen(&app).cursor, None);
    key(&mut app, Key::Esc);
    assert_eq!(screen(&app).cursor, place.0, "shown at the key");
    at(&mut app, start, 2000);
    assert_eq!(
        screen(&app).cursor,
        place.0,
        "shown half a second from the key"
    );
    at(&mut app, start, 2150);
    assert_eq!(screen(&app).cursor, None, "and away after it");
    // A click on the caret's own line shows it too.
    let (x, y) = place.0.unwrap();
    click(&mut app, x, y);
    assert!(screen(&app).cursor.is_some(), "shown at the click");
    // A caret moved by a key is shown where it went, from then.
    at(&mut app, start, 2700);
    key(&mut app, Key::Down);
    let moved = screen(&app).cursor.expect("shown where it went");
    assert_eq!(moved.1, y + 1);
    at(&mut app, start, 3100);
    assert_eq!(screen(&app).cursor, Some(moved));
}

/// Behind another window the caret stays shown and asks for no frame; back in front it's
/// shown and blinks again from then. Inside Claude Code's pane it never blinks.
#[test]
fn the_caret_stays_shown_behind_another_window() {
    let (_dir, mut app) = file();
    let start = Instant::now();
    at(&mut app, start, 0);
    let shown = screen(&app).cursor;
    app.set_window_focused(false);
    at(&mut app, start, 700);
    let grid = screen(&app);
    assert_eq!(grid.cursor, shown, "still there");
    assert!(!grid.cursor_blinks);
    assert_eq!(app.next_blink(&grid), None, "nothing wakes for it");
    at(&mut app, start, 1300);
    app.set_window_focused(true);
    let grid = screen(&app);
    assert_eq!(grid.cursor, shown, "back in front, shown");
    assert_eq!(
        app.next_blink(&grid),
        Some(start + Duration::from_millis(1300) + BLINK)
    );
    at(&mut app, start, 1900);
    assert_eq!(screen(&app).cursor, None, "and blinking");
    app.in_pane = true;
    let grid = screen(&app);
    assert_eq!(grid.cursor, shown, "in Claude Code's pane it stays");
    assert_eq!(app.next_blink(&grid), None);
}

/// Cursor blinking is a switch on Settings' Editor page, on unless turned off. Off, the caret
/// never goes and nothing wakes for it; it's kept for every Cellix on the machine, and a
/// running one follows it as its window comes back to the front.
#[test]
fn cursor_blinking_is_a_switch_in_settings() {
    let (dir, mut app) = file();
    app.run(Command::OpenSettings);
    let editor = {
        let page = app.settings_page.as_ref().unwrap();
        let spots = page.controls(app.sheet_area());
        let editor = spots
            .iter()
            .find(|(_, c)| *c == Control::Section(crate::settings_page::Section::Editor));
        editor.unwrap().0
    };
    click(&mut app, editor.x + 1, editor.y);
    let text = screen(&app).text();
    assert!(text.contains("Cursor blinking"), "{text}");
    assert!(
        text.contains("The cursor blinks in files, pages, text boxes and terminals"),
        "{text}"
    );
    let switch = {
        let page = app.settings_page.as_ref().unwrap();
        let spots = page.controls(app.sheet_area());
        spots
            .iter()
            .find(|(_, c)| *c == Control::CursorBlink)
            .unwrap()
            .0
    };
    let row = screen(&app).row_text(switch.y);
    assert!(row.contains("[on]"), "on unless turned off: {row}");
    click(&mut app, switch.x + 1, switch.y);
    assert_eq!(app.settings.cursor_blink, Some(false));
    assert!(screen(&app).row_text(switch.y).contains("[off]"));
    // Back in the file, the caret stays, however long.
    app.run(Command::CloseTab);
    let start = Instant::now();
    for ms in [0, 600, 1200, 1700, 5300] {
        at(&mut app, start, ms);
        let grid = screen(&app);
        assert!(grid.cursor.is_some(), "{ms} ms: the caret stays");
        assert!(!grid.cursor_blinks);
        assert_eq!(app.next_blink(&grid), None, "{ms} ms: nothing wakes for it");
    }
    // The filter finds it by its name.
    app.run(Command::OpenSettings);
    app.settings_page.as_mut().unwrap().filter = "blink".into();
    // (Its page's name, after it, is set over the end of its cells.)
    let found = screen(&app).text();
    assert!(
        found.contains("1 setting") && found.contains("Cursor blink"),
        "{found}"
    );
    // Another Cellix turned it on again, in the machine's file: this one follows as its
    // window comes back.
    let home = tempfile::tempdir().unwrap();
    let file = home.path().join(".config/cellix/settings.json");
    fs::create_dir_all(file.parent().unwrap()).unwrap();
    fs::write(&file, r#"{"cursor_blink": true}"#).unwrap();
    app.settings_file = Some(file);
    app.window_returned();
    assert_eq!(app.settings.cursor_blink, Some(true));
    drop(dir);
}

/// On cells the terminal blinks its own cursor: the caret shows at any time, and the grid
/// asks the terminal for a blinking bar, so nothing in Cellix wakes for it.
#[test]
fn on_cells_the_terminal_blinks_its_own_cursor() {
    let (_dir, mut app) = file();
    app.set_measure(Rc::new(CellMeasure));
    let start = Instant::now();
    for ms in [0, 600, 1200, 1700] {
        at(&mut app, start, ms);
        let grid = screen(&app);
        assert!(grid.cursor.is_some(), "{ms} ms");
        assert_eq!(
            (grid.cursor_shape, grid.cursor_blinks),
            (CursorShape::Bar, true)
        );
        assert_eq!(
            app.next_blink(&grid),
            None,
            "{ms} ms: no wake of Cellix's own"
        );
    }
    app.set_window_focused(false);
    assert!(!screen(&app).cursor_blinks, "behind another window, steady");
}

/// A stand-in for an agent that prints `text` and waits.
fn printing(dir: &Path, text: &str) -> String {
    let fake = dir.join("printing-agent");
    let script = format!(
        "#!/bin/sh\nprintf '\\033]0;✳ pricing page\\007'\nprintf '{text}'\nexec sleep 30\n"
    );
    fs::write(&fake, script).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&fake, fs::Permissions::from_mode(0o755)).unwrap();
    }
    fake.display().to_string()
}

/// Where a terminal pane's cursor is on screen.
fn cursor_of(app: &App, index: usize) -> (u16, u16) {
    let area = app.dock_screen().expect("the pane is open");
    let cursor = app.dock.sessions[index].cursor_in(area, true);
    cursor.expect("its cursor shows")
}

/// A terminal in the side pane, in front with the keys, whose program printed `text`, ending
/// in "ready": its place among the sessions.
fn terminal(dir: &Path, app: &mut App, text: &str) -> usize {
    app.agent_program = Some(printing(dir, text));
    app.open_agent("claude");
    let index = app.dock.sessions.len() - 1;
    app.dock.active = Some(index);
    app.dock.front = Front::Session;
    app.dock.focused = true;
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let area = app.dock_screen().expect("the pane is open");
        if screen(app).row_text(area.y).contains("ready") {
            return index;
        }
        assert!(Instant::now() < deadline, "the stand-in never printed");
        app.poll();
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// A terminal pane's cursor is its cell inverted, and it blinks with the caret: half a second
/// away it's the cell's own look. A program that asked for a steady cursor (DECSCUSR 2) keeps
/// one, and nothing wakes for it. On cells the terminal's own cursor stands there, in the
/// program's shape, blinking or steady as it asked.
#[test]
#[cfg(unix)]
fn a_terminal_panes_cursor_blinks_unless_its_program_asks_for_a_steady_one() {
    for (asked, blinks) in [("", true), ("\\033[2 q", false)] {
        let dir = tempfile::tempdir().unwrap();
        let mut app = App::new(dir.path().to_path_buf(), (150, 44));
        pixels(&mut app);
        let start = Instant::now();
        at(&mut app, start, 0);
        let index = terminal(dir.path(), &mut app, &format!("{asked}ready"));
        let (x, y) = cursor_of(&app, index);
        let theme = *app.theme();
        let grid = screen(&app);
        assert_eq!(grid.cell(x, y).style.bg, theme.text.fg, "the cursor shows");
        assert_eq!(grid.cursor, None, "no caret but the terminal's");
        assert_eq!(grid.blinks(), blinks, "{asked:?}");
        at(&mut app, start, 600);
        let grid = screen(&app);
        let bg = grid.cell(x, y).style.bg;
        if blinks {
            assert_ne!(bg, theme.text.fg, "away, the cell's own look");
            assert_eq!(app.next_blink(&grid), Some(start + BLINK * 2));
        } else {
            assert_eq!(bg, theme.text.fg, "a steady cursor stays");
            assert_eq!(app.next_blink(&grid), None, "and nothing wakes for it");
        }
        // On cells, the terminal's own cursor, a block, blinking as the program asked.
        app.set_measure(Rc::new(CellMeasure));
        let _ = screen(&app);
        let (x, y) = cursor_of(&app, index);
        let grid = screen(&app);
        assert_eq!(grid.cursor, Some((x, y)));
        assert_eq!(grid.cursor_shape, CursorShape::Block);
        assert_eq!(grid.cursor_blinks, blinks, "{asked:?}");
        assert_ne!(
            grid.cell(x, y).style.bg,
            theme.text.fg,
            "not inverted as well"
        );
        assert_eq!(app.next_blink(&grid), None, "the terminal blinks it");
    }
}
