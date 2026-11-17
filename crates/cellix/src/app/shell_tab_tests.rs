//! A shell's tab in the side pane: named after the program running in it, as VS Code names a
//! terminal's tab, turning while it runs, and back to its own name at the prompt. A command
//! that ends within about a second leaves the tab as it was. Driven through the app, with a
//! real interactive shell, and read off the grid.

use std::time::{Duration, Instant};

use cellix_render::Grid;
use tempfile::TempDir;

use super::App;

fn project() -> (TempDir, App) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("alpha.txt"), "first line\n").unwrap();
    let app = App::new(dir.path().to_path_buf(), (130, 30));
    (dir, app)
}

fn screen(app: &App) -> Grid {
    let mut grid = Grid::new(app.size().0, app.size().1, app.background());
    app.draw(&mut grid);
    grid
}

/// The side pane's row of tabs, as text, and the grid it's on.
fn tabs(app: &App) -> (Grid, u16, String) {
    let grid = screen(app);
    let y = app.dock_area().unwrap().y;
    let row = grid.row_text(y);
    (grid, y, row)
}

/// Polls until `done` holds of the app, or fails saying what the tabs said.
fn until(app: &mut App, what: &str, done: impl Fn(&App) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !done(app) {
        assert!(Instant::now() < deadline, "never {what}: {}", tabs(app).2);
        app.poll();
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn printed(app: &App, words: &str) -> bool {
    app.dock.sessions[0]
        .lines()
        .iter()
        .any(|line| line.contains(words))
}

#[test]
#[cfg(unix)]
fn a_shells_tab_names_what_runs_in_it_and_turns_while_it_does() {
    let (_dir, mut app) = project();
    app.turn_at = Some(3);
    let root = app.workspace.root().to_path_buf();
    // Interactive, a shell gives each command a process group of its own, as a person's does.
    let shell = ["sh".to_string(), "-i".to_string()];
    let env = ["PS1=ready$ ".to_string(), "ENV=".to_string()];
    app.start_session(crate::agents::SHELL.0, &shell, &env, &root);
    until(&mut app, "a prompt", |app| printed(app, "ready$"));
    let (grid, _, row) = tabs(&app);
    assert!(row.contains("Terminal"), "{row}");
    assert!(!grid.spinning);

    // A command over within a second leaves the tab as it was the whole time.
    app.dock.sessions[0].send_line("sleep 0.3; echo qu''ick");
    let deadline = Instant::now() + Duration::from_millis(1600);
    while Instant::now() < deadline {
        app.poll();
        let (grid, _, row) = tabs(&app);
        assert!(
            !row.contains("sleep") && !grid.spinning,
            "it flashed: {row}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(printed(&app, "quick"), "it ran");

    // One that runs on: its tab is named after it, with a shell's mark before the name and
    // the spinner after it, in the accent, before the ×.
    let sent = Instant::now();
    app.dock.sessions[0].send_line("sleep 5; echo sl''ow");
    until(&mut app, "named after sleep", |app| {
        tabs(app).2.contains("sleep")
    });
    assert!(
        sent.elapsed() >= Duration::from_millis(900),
        "not before about a second"
    );
    let (grid, y, row) = tabs(&app);
    assert!(
        row.contains("> sleep"),
        "the shell's mark, one letter on cells: {row}"
    );
    assert!(!row.contains("Terminal"), "{row}");
    let column = |symbol: &str| (0..grid.width()).find(|x| grid.cell(*x, y).symbol == symbol);
    let spin = column("⠸").expect("the fourth braille frame");
    let close = column("×").expect("its ×");
    let name = row[..row.find("sleep").unwrap()].chars().count() as u16;
    assert!(name < spin && spin < close, "{row}");
    assert_eq!(grid.cell(spin, y).style.fg, app.theme().accent);
    assert!(grid.spinning, "and it asks for its next step");

    // Back at the prompt, the shell's own name, and still.
    until(&mut app, "back to its name", |app| {
        printed(app, "slow") && tabs(app).2.contains("Terminal")
    });
    let (grid, _, row) = tabs(&app);
    assert!(!row.contains("sleep") && !grid.spinning, "{row}");
}
