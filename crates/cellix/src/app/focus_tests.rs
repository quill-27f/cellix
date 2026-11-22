//! Where the person's keys are, as the side pane's programs are told it and its cursor shows it.
//! Driven through the app with stand-ins that write down every byte they're sent, and read off
//! the drawn grid.

use std::fs;
use std::path::Path;
use std::rc::Rc;
use std::time::{Duration, Instant};

use cellix_input::{Command, Event, Key, KeyEvent, Mods, MouseEvent, MouseKind};
use cellix_render::{FontMeasure, Grid};
use tempfile::TempDir;

use super::App;

/// How long a test waits on a stand-in: the shared machine can be loaded.
const PATIENCE: Duration = Duration::from_secs(60);

fn project() -> (TempDir, App) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("alpha.txt");
    fs::write(&path, "first line\nsecond line\n").unwrap();
    let mut app = App::new(dir.path().to_path_buf(), (150, 40));
    app.open(&path).unwrap();
    (dir, app)
}

fn screen(app: &App) -> Grid {
    let mut grid = Grid::new(app.size().0, app.size().1, app.background());
    app.draw(&mut grid);
    grid
}

fn click(app: &mut App, (x, y): (u16, u16)) {
    for kind in [MouseKind::Left, MouseKind::Up] {
        app.handle(Event::Mouse(MouseEvent {
            kind,
            x,
            y,
            fine_x: f32::from(x) + 0.5,
            fine_y: f32::from(y) + 0.5,
            mods: Mods::default(),
        }));
    }
}

/// A press in the file's text.
fn click_file(app: &mut App) {
    let area = app.focused_area();
    click(app, (area.x + 6, area.y + 1));
    assert!(!app.dock.focused, "the keyboard is the file's");
}

/// A press in the terminal in front of the side pane.
fn click_pane(app: &mut App) {
    let screen = app.dock_screen().expect("the pane is open");
    click(app, (screen.x + 4, screen.y + 3));
    assert!(app.dock.focused, "the keyboard is the pane's");
}

/// A press on the side pane's tab that says `name`.
fn click_tab(app: &mut App, name: &str) {
    let y = app.dock_area().expect("the pane is open").y;
    let row = screen(app).row_text(y);
    let at = row
        .find(name)
        .unwrap_or_else(|| panic!("no {name} in {row:?}"));
    click(app, (row[..at].chars().count() as u16 + 1, y));
}

/// A stand-in in a tab of its own that says `ready` and writes down every byte it's sent, raw,
/// in `log`; `asks` has it ask to hear of focus first, as Claude Code does as it starts.
fn stand_in(app: &mut App, name: &str, log: &Path, asks: bool) {
    let ask = if asks { "\\033[?1004h" } else { "" };
    let script = format!(
        "stty raw -echo; printf '{ask}{name} ready'; cat > '{}'",
        log.display()
    );
    app.start_stand_in(name, &script);
}

/// Polls until `name` has printed that it's ready.
fn ready(app: &mut App, name: &str) {
    let deadline = Instant::now() + PATIENCE;
    let index = app
        .dock
        .sessions
        .iter()
        .position(|s| s.name == name)
        .unwrap();
    while !app.dock.sessions[index]
        .lines()
        .join("\n")
        .contains("ready")
    {
        assert!(Instant::now() < deadline, "{name} never started");
        app.poll();
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Polls until the stand-in has been sent exactly `told`, and fails saying what it was sent if
/// that never comes: a report sent twice, or one too many, never matches.
fn heard(app: &mut App, log: &Path, told: &str) {
    let deadline = Instant::now() + PATIENCE;
    loop {
        app.poll();
        let got = fs::read_to_string(log).unwrap_or_default();
        if got == told {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "sent {got:?} where {told:?} was wanted"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

const IN: &str = "\x1b[I";
const OUT: &str = "\x1b[O";

/// A program in the side pane that asked to hear of focus (DECSET 1004) is told as the
/// person's keys come and go: `CSI O` at once if it asked without them, `CSI I` when the pane
/// is clicked, `CSI O` on a press in the file, another tab in front, the palette over all, the
/// window going behind another and the pane closing, and `CSI I` as each comes back. Nothing is
/// said twice, and a program that never asked is sent nothing.
#[cfg(unix)]
#[test]
fn a_program_that_asked_to_hear_of_focus_is_told_as_the_keys_come_and_go() {
    let (dir, mut app) = project();
    let (asked, plain) = (dir.path().join("asked.log"), dir.path().join("plain.log"));
    stand_in(&mut app, "Plain", &plain, false);
    ready(&mut app, "Plain");
    // It starts with the keys, and the person presses the file before it has asked.
    stand_in(&mut app, "Claude Code", &asked, true);
    click_file(&mut app);
    ready(&mut app, "Claude Code");
    let mut told = String::from(OUT);
    heard(&mut app, &asked, &told);

    // Into the pane and out again; a tick, another press in the pane and a key change nothing
    // it's told, but the key reaches it.
    click_pane(&mut app);
    told.push_str(IN);
    heard(&mut app, &asked, &told);
    app.handle(Event::Tick);
    click_pane(&mut app);
    app.handle(Event::Key(KeyEvent::plain(Key::Char('x'))));
    told.push('x');
    heard(&mut app, &asked, &told);
    click_file(&mut app);
    told.push_str(OUT);
    heard(&mut app, &asked, &told);

    // Another tab in front has the keys, and back.
    click_pane(&mut app);
    click_tab(&mut app, "Plain");
    assert!(app.dock.focused);
    told.push_str(&format!("{IN}{OUT}"));
    heard(&mut app, &asked, &told);
    click_tab(&mut app, "Claude Code");
    told.push_str(IN);
    heard(&mut app, &asked, &told);

    // The window behind another, and in front again.
    app.set_window_focused(false);
    told.push_str(OUT);
    heard(&mut app, &asked, &told);
    app.set_window_focused(true);
    told.push_str(IN);
    heard(&mut app, &asked, &told);

    // The palette over everything has the keys till it's put away.
    app.run(Command::CommandPalette);
    app.handle(Event::Tick);
    told.push_str(OUT);
    heard(&mut app, &asked, &told);
    app.handle(Event::Key(KeyEvent::plain(Key::Esc)));
    told.push_str(IN);
    heard(&mut app, &asked, &told);

    // The pane put away.
    app.run(Command::CloseDock);
    app.handle(Event::Tick);
    told.push_str(OUT);
    heard(&mut app, &asked, &told);

    // The one that never asked was sent nothing, though it had the keys for a while.
    assert_eq!(fs::read_to_string(&plain).unwrap_or_default(), "");
}

/// A program joined again from its host, which an earlier Cellix may have told it hadn't the
/// keys, is told where they are at once, even with them; one started here, which takes it that
/// it has them, isn't told so again.
#[cfg(unix)]
#[test]
fn a_program_joined_again_is_told_where_the_keys_are_at_once() {
    let (dir, mut app) = project();
    let (joined, fresh) = (dir.path().join("joined.log"), dir.path().join("fresh.log"));
    stand_in(&mut app, "Codex", &fresh, true);
    ready(&mut app, "Codex");
    heard(&mut app, &fresh, "");
    stand_in(&mut app, "Claude Code", &joined, true);
    let index = app.dock.active.unwrap();
    app.dock.sessions[index].rejoined = true;
    ready(&mut app, "Claude Code");
    heard(&mut app, &joined, IN);
    heard(&mut app, &fresh, OUT);
}

/// A helper looked at in peek has the keys while it's open, as it has the pastes, though the
/// pane had the keyboard: it's told so, the tab under its card is told it hasn't them, and keys
/// go to the helper. Closed, the tab has them again.
#[cfg(unix)]
#[test]
fn a_helper_in_peek_has_the_keys_while_it_is_open() {
    let (dir, mut app) = project();
    let (lead_log, helper_log) = (dir.path().join("lead.log"), dir.path().join("helper.log"));
    stand_in(&mut app, "Claude Code", &lead_log, true);
    ready(&mut app, "Claude Code");
    let lead = app.dock.active.unwrap();
    stand_in(&mut app, "Codex", &helper_log, true);
    let helper = app.dock.active.unwrap();
    // A helper the lead started, with no tab of its own; the lead's tab in front with the
    // keyboard, as after a press in it.
    app.dock.sessions[helper].member = Some(crate::hold::Member {
        name: "reviewer".into(),
        kind: "codex".into(),
        parent: None,
        outlive: false,
    });
    app.dock.sessions[helper].tabbed = false;
    app.dock.active = Some(lead);
    app.dock.focused = true;
    ready(&mut app, "Codex");
    heard(&mut app, &helper_log, OUT);

    app.open_peek("reviewer");
    app.handle(Event::Tick);
    heard(&mut app, &lead_log, OUT);
    heard(&mut app, &helper_log, &format!("{OUT}{IN}"));
    app.handle(Event::Key(KeyEvent::plain(Key::Char('k'))));
    heard(&mut app, &helper_log, &format!("{OUT}{IN}k"));
    assert_eq!(
        fs::read_to_string(&lead_log).unwrap(),
        OUT,
        "not the tab under it"
    );

    app.close_peek();
    app.handle(Event::Tick);
    heard(&mut app, &lead_log, &format!("{OUT}{IN}"));
    heard(&mut app, &helper_log, &format!("{OUT}{IN}k{OUT}"));
}

/// A terminal pane's cursor, the block a shell shows, follows the keys as its program is told:
/// shown with them, and gone on a press in the file and while the window is behind another.
/// The file's caret goes the other way, so only one place says it's where typing goes, and with
/// the keys in the pane and the window behind another, none does.
#[cfg(unix)]
#[test]
fn a_terminal_panes_cursor_shows_only_while_it_has_the_keys() {
    let (dir, mut app) = project();
    app.set_measure(Rc::new(FontMeasure::new(17, 50).with_type_row(36)));
    stand_in(&mut app, "Terminal", &dir.path().join("typed.log"), false);
    ready(&mut app, "Terminal");
    let index = app.dock.active.unwrap();
    let theme = *app.theme();
    let area = app.dock_screen().unwrap();
    let (x, y) = app.dock.sessions[index]
        .cursor_in(area, true)
        .expect("a cursor after its words");
    let shows = |app: &App| screen(app).cell(x, y).style.bg == theme.text.fg;
    let caret = |app: &App| {
        let grid = screen(app);
        grid.cursor.is_some() || grid.caret.is_some()
    };
    assert!(app.has_keys(Some(index)) && shows(&app), "with the keys");
    assert!(!caret(&app), "and the file has no caret");
    click_file(&mut app);
    assert!(!shows(&app), "gone with the keys to the file");
    assert!(caret(&app), "whose caret shows");
    click_pane(&mut app);
    assert!(shows(&app) && !caret(&app), "back with them");
    app.set_window_focused(false);
    assert!(!shows(&app), "gone while the window is behind another");
    assert!(!caret(&app), "and the file's caret doesn't stand in for it");
    app.set_window_focused(true);
    assert!(shows(&app));
    // The file's own caret stays behind another window, as it did, when the keys are its.
    click_file(&mut app);
    app.set_window_focused(false);
    assert!(caret(&app) && !shows(&app));
}

/// Where the keyboard starts: a workspace in front for the first time, with an agent's tab in
/// front of its open pane, has it there, unless a file named at the command line or a box over
/// everything has it. Settled once, it's left where the person puts it.
#[cfg(unix)]
#[test]
fn the_keyboard_starts_in_the_agent_in_front_unless_a_file_was_named() {
    let start = |shell: bool| {
        let (dir, mut app) = project();
        let name = if shell { "Terminal" } else { "Claude Code" };
        stand_in(&mut app, name, &dir.path().join("typed.log"), false);
        // As a workspace comes back from the last run: the pane open on the tab that was in
        // front, the keyboard in the file.
        app.dock.focused = false;
        (dir, app)
    };
    let (_dir, mut app) = start(false);
    app.place_keys();
    assert!(app.dock.focused, "in the agent's pane");
    // Once settled, never again: the person's press in the file stands.
    click_file(&mut app);
    app.place_keys();
    assert!(!app.dock.focused);

    // A file named at the command line keeps it.
    let (_dir, mut app) = start(false);
    app.keep_keys_in_file();
    app.place_keys();
    assert!(!app.dock.focused, "the named file has it");
    // So does the palette, over everything.
    let (_dir, mut app) = start(false);
    app.run(Command::CommandPalette);
    app.place_keys();
    assert!(!app.dock.focused, "the palette has it");
    // And a shell isn't an agent waiting at its prompt.
    let (_dir, mut app) = start(true);
    app.place_keys();
    assert!(!app.dock.focused, "a terminal's tab leaves it in the file");
}
