//! What Cellix itself waits on, shown with the spinner where VS Code shows progress: a status
//! bar item with words, the Search view's heading, and the button that started it.

use std::fs;
use std::time::{Duration, Instant};

use cellix_render::Grid;
use tempfile::TempDir;

use super::App;
use crate::settings_page::Control;

fn project() -> (TempDir, App) {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("alpha.txt"), "first line\n").unwrap();
    let mut app = App::new(dir.path().to_path_buf(), (120, 30));
    app.turn_at = Some(4);
    (dir, app)
}

fn screen(app: &App) -> Grid {
    let mut grid = Grid::new(app.size().0, app.size().1, app.background());
    app.draw(&mut grid);
    grid
}

const STEP: &str = cellix_render::spinner::FRAMES[4];

/// The status bar's item: a spinner and what it's about, cut off from the rest of the bar so
/// a step sends only it.
fn status(app: &App) -> (String, Grid) {
    let grid = screen(app);
    let y = grid.height() - 1;
    (grid.row_text(y), grid)
}

#[test]
fn a_push_turns_in_the_status_bar_and_on_its_button() {
    let (_dir, mut app) = project();
    let (row, grid) = status(&app);
    assert!(
        !row.contains(STEP) && !grid.spinning,
        "nothing waited on: {row}"
    );
    assert_eq!(grid.row_bands, None);
    app.source.busy = Some("Pushing…");
    let (row, grid) = status(&app);
    assert!(row.contains(&format!("{STEP} Pushing")), "{row}");
    assert!(grid.spinning);
    let y = grid.height() - 1;
    let (row_y, bands) = grid.row_bands.clone().expect("a band of its own");
    assert_eq!(row_y, y);
    let spin = (0..grid.width())
        .find(|x| grid.cell(*x, y).symbol == STEP)
        .unwrap();
    assert!(
        bands[0] <= spin && spin < bands[1],
        "{bands:?} round {spin}"
    );
    // In Source Control, on the button that started it.
    app.run(cellix_input::Command::ShowSourceControl);
    let grid = screen(&app);
    let found =
        (0..grid.height() - 1).any(|y| grid.row_text(y).contains(&format!("{STEP} Pushing")));
    assert!(found, "{}", grid.text());
    app.source.busy = None;
    assert!(!status(&app).1.spinning, "done: still again");
}

#[test]
fn a_build_and_an_install_say_so_while_they_run() {
    let (_dir, mut app) = project();
    let (_send, building) = std::sync::mpsc::channel();
    app.build = Some(building);
    assert_eq!(app.waiting().as_deref(), Some("Building PDF"));
    app.export_to = Some("paper.pdf".into());
    assert!(status(&app).0.contains(&format!("{STEP} Exporting PDF")));
    app.build = None;
    app.export_to = None;
    app.run(cellix_input::Command::OpenSettings);
    let (_send, installing) = std::sync::mpsc::channel();
    let page = app.settings_page.as_mut().unwrap();
    page.installing = Some(installing);
    page.installing_what = Some(Control::InstallServer(0));
    assert!(status(&app).0.contains(&format!("{STEP} Installing")));
}

/// A task turns in its tab while it runs, as VS Code's terminal tab does, and says so in the
/// status bar; ended, both are still.
#[test]
#[cfg(unix)]
fn a_running_task_turns_in_its_tab() {
    let (dir, mut app) = project();
    fs::write(dir.path().join("Makefile"), "build:\n\t@sleep 20\n").unwrap();
    app.run(cellix_input::Command::RunTask);
    app.handle(cellix_input::Event::Key(cellix_input::KeyEvent::plain(
        cellix_input::Key::Enter,
    )));
    let index = (app.dock.sessions.iter())
        .position(|s| s.name == "make build")
        .expect("its tab");
    let grid = screen(&app);
    let tabs = grid.row_text(app.dock_area().unwrap().y);
    assert!(tabs.contains(STEP), "{tabs}");
    assert!(
        status(&app)
            .0
            .contains(&format!("{STEP} Running make build"))
    );
    app.dock.sessions[index].stop();
    let deadline = Instant::now() + Duration::from_secs(30);
    while !app.dock.sessions.get(index).is_none_or(|s| s.exited) {
        assert!(Instant::now() < deadline, "it never ended");
        app.poll();
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(!screen(&app).spinning, "ended: nothing turns");
}

/// A language server says what it's doing as it starts and loads: here a stand-in that
/// answers, says it's indexing, and stays at that.
#[test]
#[cfg(unix)]
fn a_language_server_indexing_turns_in_the_status_bar() {
    let (dir, _) = project();
    let server = dir.path().join("indexer.py");
    fs::write(&server, INDEXER).unwrap();
    fs::create_dir(dir.path().join(".cellix")).unwrap();
    let settings = serde_json::json!({"language_servers": {"Rust": ["python3", "-I", server]}});
    fs::write(
        dir.path().join(".cellix/settings.json"),
        settings.to_string(),
    )
    .unwrap();
    let mut app = App::new(dir.path().to_path_buf(), (120, 30));
    app.turn_at = Some(4);
    fs::write(dir.path().join("main.rs"), "fn main() {}\n").unwrap();
    app.open(&dir.path().join("main.rs")).unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        app.poll();
        let (row, grid) = status(&app);
        if row.contains(&format!("{STEP} indexer: indexing")) {
            assert!(grid.spinning);
            break;
        }
        assert!(Instant::now() < deadline, "{row}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Answers `initialize`, then says it has begun indexing and never says it's done.
const INDEXER: &str = r#"
import json, sys
def read():
    length = 0
    while True:
        line = sys.stdin.buffer.readline()
        if not line:
            sys.exit(0)
        if line.strip() == b"":
            break
        if line.lower().startswith(b"content-length:"):
            length = int(line.split(b":")[1])
    return json.loads(sys.stdin.buffer.read(length))
def send(message):
    body = json.dumps(message).encode()
    sys.stdout.buffer.write(b"Content-Length: %d\r\n\r\n" % len(body) + body)
    sys.stdout.buffer.flush()
while True:
    m = read()
    i = m.get("id")
    if m.get("method") == "initialize":
        send({"jsonrpc": "2.0", "id": i, "result": {"capabilities": {}}})
        send({"jsonrpc": "2.0", "method": "$/progress",
              "params": {"token": "index", "value": {"kind": "begin", "title": "Indexing"}}})
    elif i is not None and "method" in m:
        send({"jsonrpc": "2.0", "id": i, "result": None})
"#;
