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
    assert!(
        status(&app)
            .0
            .contains(&format!("{STEP} Exporting paper.pdf"))
    );
    app.build = None;
    app.export_to = None;
    app.run(cellix_input::Command::OpenSettings);
    let (_send, installing) = std::sync::mpsc::channel();
    let page = app.settings_page.as_mut().unwrap();
    page.installing = Some(installing);
    page.installing_what = Some(Control::InstallServer(0));
    assert!(status(&app).0.contains(&format!("{STEP} Installing")));
}

/// A paper being built is named once at the foot, by the spinner: "Building main.tex". A
/// message saying the same stood beside it, so the bar said it twice.
#[test]
#[cfg(unix)]
fn a_build_is_named_once_while_it_runs() {
    use std::os::unix::fs::PermissionsExt;
    let (dir, mut app) = project();
    let source = "\\documentclass{article}\n\\begin{document}\nHi\n\\end{document}\n";
    fs::write(dir.path().join("main.tex"), source).unwrap();
    // A stand-in engine, which takes its time and makes nothing.
    let engine = dir.path().join("engine.sh");
    fs::write(&engine, "#!/bin/sh\nsleep 3\n").unwrap();
    fs::set_permissions(&engine, fs::Permissions::from_mode(0o755)).unwrap();
    app.settings.latex_engine = Some(vec![engine.display().to_string()]);
    app.open(&dir.path().join("main.tex")).unwrap();
    app.run(cellix_input::Command::BuildPdf);
    assert!(app.build.is_some(), "it runs");
    let (row, grid) = status(&app);
    assert!(row.contains(&format!("{STEP} Building main.tex")), "{row}");
    assert_eq!(row.matches("main.tex").count(), 1, "said once: {row}");
    assert!(grid.spinning);
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

/// One set of marks: the status bar's "An agent needs you" has the dot an asking tab has, at
/// the size the bar's spinner is, where a dot half that stood before its words; and the
/// Agent Context picker has each agent's own mark, where a blue dot stood before every agent
/// alike (0.11.0's final review, T5).
#[test]
fn the_bar_and_the_picker_have_the_marks_tabs_have() {
    use cellix_agent::context::Agent;
    use cellix_render::{FontMeasure, Paint, UNIT};
    let (dir, mut app) = project();
    app.set_measure(std::rc::Rc::new(FontMeasure::new(17, 36)));
    // An agent with a tab here: one outside Cellix isn't counted.
    app.tab_for("Claude Code", "s1");
    app.serve(cellix_agent::Request::Agent(cellix_agent::AgentEvent {
        session: "s1".into(),
        agent: "Claude Code".into(),
        state: cellix_agent::AgentState::NeedsYou,
        detail: "Allow Bash?".into(),
        folder: "/w".into(),
        pid: None,
        transcript: String::new(),
        name: String::new(),
    }));
    let (row, grid) = status(&app);
    assert!(row.contains("An agent needs you"), "{row}");
    let y = i32::from(grid.height() - 1) * UNIT;
    let attention = app.theme().attention;
    let dots: Vec<_> = (grid.paint.iter())
        .filter_map(|p| match p {
            Paint::Spinner {
                bounds,
                color,
                step: None,
            } if bounds.y >= y && *color == attention => Some(bounds.h),
            _ => None,
        })
        .collect();
    let size = (crate::measures::WAIT_SPIN * UNIT as f32).round() as i32;
    assert_eq!(dots, [size], "the dot, at the spinner's size");
    let small = grid.paint.iter().any(|p| {
        matches!(p, Paint::Panel { bounds, fill: Some(f), .. } if bounds.y >= y && *f == attention)
    });
    assert!(!small, "no smaller dot of its own");
    // The picker names each agent with its own mark.
    app.run(cellix_input::Command::ShowAgentContext);
    let root = dir.path().to_path_buf();
    for agent in Agent::ALL {
        app.context.show(agent, &root);
        let grid = screen(&app);
        // The picker is the row under the view's heading.
        let heading = (0..grid.height()).find(|y| grid.row_text(*y).contains("AGENT CONTEXT"));
        let at = heading.unwrap_or_else(|| panic!("{}: {}", agent.name(), grid.text())) + 1;
        let mark = crate::dock::program_icon(agent.name(), false, false);
        let marked = grid.paint_on_row(at).any(|p| {
            matches!(p, Paint::Icon { icon, color, .. }
                if *icon == mark && *color == crate::dock::icon_colour(mark, app.theme()))
        });
        assert!(marked, "{}'s own mark: {}", agent.name(), grid.row_text(at));
    }
}

/// What Cellix waits on and the file's details share the bar by what they take as they're
/// drawn. Counted a column a letter, where painted letters are a good deal narrower,
/// "rust-analyzer: fetching" took "Spaces: 4" off the bar with 600 px of it free (0.11.0's
/// release check, P9 and R6). On cells, where a letter is a column, what doesn't fit still
/// goes from the far end, and nothing is drawn over the wait's words.
#[test]
fn a_wait_leaves_the_files_details_the_room_they_have() {
    use cellix_render::{FontMeasure, Paint, UNIT};
    let (dir, mut app) = project();
    fs::write(
        dir.path().join("queue.rs"),
        "fn main() {\n    let x = 1;\n}\n",
    )
    .unwrap();
    app.open(&dir.path().join("queue.rs")).unwrap();
    // A window where, a column a letter, the details and the wait don't fit side by side,
    // and painted they do with room to spare.
    app.handle(cellix_input::Event::Resize(84, 30));
    // As long as rust-analyzer's words when it fetches.
    app.source.busy = Some("rust-analyzer: fetching…");
    app.set_measure(std::rc::Rc::new(FontMeasure::new(17, 36)));
    // What's painted, which the screen's text, a cell a letter, can't show side by side.
    let (_, grid) = status(&app);
    // Painted, every detail is there, and the wait's words end well before the details start.
    let unit = UNIT as f32;
    let y = i32::from(grid.height() - 1) * UNIT;
    let on_bar = |p: &&Paint| matches!(p, Paint::Text { y: at, .. } if *at >= y);
    let texts: Vec<_> = (grid.paint.iter().filter(on_bar))
        .filter_map(|p| match p {
            Paint::Text {
                x,
                text,
                font,
                align_right,
                ..
            } => Some((*x as f32 / unit, text.clone(), *font, *align_right)),
            _ => None,
        })
        .collect();
    let wait = (texts.iter())
        .find(|(_, text, _, _)| text == "rust-analyzer: fetching")
        .map(|(x, text, font, _)| x + app.measure().width(text, *font))
        .unwrap_or_else(|| panic!("the wait isn't painted whole: {texts:?}"));
    let details = (texts.iter())
        .find(|(_, text, _, right)| *right && text.contains("Spaces: 4"))
        .map(|(x, text, font, _)| x - app.measure().width(text, *font))
        .unwrap_or_else(|| panic!("the details aren't painted: {texts:?}"));
    assert!(details - wait >= 2.0, "{wait} to {details}");
    // On cells the details that fit keep their place and the rest go from the far end.
    app.set_measure(std::rc::Rc::new(cellix_render::CellMeasure));
    let (row, _) = status(&app);
    assert!(row.contains("rust-analyzer: fetching"), "{row}");
    assert!(row.contains("Ln 1, Col 1"), "{row}");
}
