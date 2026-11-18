//! A right click on a link opens a menu at the pointer with Open Link and Copy Link, wherever
//! Cellix shows a link the person can click: a comment's words and its replies, a page's text
//! (Markdown, LaTeX, code), and a terminal's output. Copy Link puts the link's address on the
//! clipboard, the target and not the words, and hands it to the terminal loop that sends it by
//! OSC 52; Open Link opens it as a click does. A link to a file has Open File and Copy Path.
//! Driven through the app with the mouse, and read off the grid.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use cellix_input::{Command, Event, Mods, MouseEvent, MouseKind};
use cellix_render::{Grid, Paint};
use tempfile::TempDir;

use super::{App, Overlay};

/// How long a test waits for work on another thread, here a stand-in browser or a terminal.
const BACKGROUND: Duration = Duration::from_secs(120);

/// A project, and a stand-in for the system's browser that writes down what it was asked to
/// open, so nothing real opens.
fn project() -> (TempDir, App, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("alpha.txt"), "first line\nsecond line\n").unwrap();
    let mut app = App::new(dir.path().to_path_buf(), (130, 30));
    let log = dir.path().join("opened.txt");
    let script = format!("printf %s \"$1\" > '{}'", log.display());
    app.settings.browser = None;
    app.system_browser = ["sh", "-c", &script, "browser"].map(String::from).to_vec();
    (dir, app, log)
}

fn screen(app: &App) -> Grid {
    let mut grid = Grid::new(app.size().0, app.size().1, app.background());
    app.draw(&mut grid);
    grid
}

fn press(app: &mut App, kind: MouseKind, (x, y): (u16, u16), mods: Mods) {
    app.handle(Event::Mouse(MouseEvent {
        kind,
        x,
        y,
        fine_x: f32::from(x) + 0.5,
        fine_y: f32::from(y) + 0.5,
        mods,
    }));
}

fn right(app: &mut App, at: (u16, u16)) {
    press(app, MouseKind::Right, at, Mods::default());
}

fn click(app: &mut App, at: (u16, u16)) {
    press(app, MouseKind::Left, at, Mods::default());
    press(app, MouseKind::Up, at, Mods::default());
}

/// Where `text` starts on the screen, on the first row that has it.
fn find(grid: &Grid, text: &str) -> (u16, u16) {
    place(grid, text).unwrap_or_else(|| panic!("{text:?} not on screen:\n{}", grid.text()))
}

fn place(grid: &Grid, text: &str) -> Option<(u16, u16)> {
    (0..grid.height()).find_map(|y| {
        let row = grid.row_text(y);
        let i = row.find(text)?;
        Some((row[..i].chars().count() as u16, y))
    })
}

/// The rows of the menu that's open, in order, as the screen shows them.
fn menu_rows(app: &App, labels: &[&str]) -> Vec<Option<u16>> {
    assert!(
        matches!(app.overlay, Overlay::Menu(_)),
        "no menu:\n{}",
        screen(app).text()
    );
    let grid = screen(app);
    labels
        .iter()
        .map(|label| place(&grid, label).map(|(_, y)| y))
        .collect()
}

/// Picks a row of the menu that's open.
fn choose(app: &mut App, label: &str) {
    let (x, y) = find(&screen(app), label);
    click(app, (x + 1, y));
    assert!(matches!(app.overlay, Overlay::None), "the menu stayed open");
}

/// Whether two paths name one file, however the temporary folder's name is written.
fn same(a: &Path, b: &Path) -> bool {
    a.is_absolute() && fs::canonicalize(a).ok() == fs::canonicalize(b).ok()
}

/// Whether the file in front is `path`.
fn in_front(app: &App, path: &Path) -> bool {
    let shown = app.editors[app.active()].buffer.path();
    shown.is_some_and(|shown| same(shown, path))
}

/// What the stand-in browser was asked to open, once it has been.
fn opened(log: &Path) -> String {
    let deadline = Instant::now() + BACKGROUND;
    while !log.exists() {
        assert!(Instant::now() < deadline, "the browser never ran");
        std::thread::sleep(Duration::from_millis(20));
    }
    std::thread::sleep(Duration::from_millis(50));
    let said = fs::read_to_string(log).unwrap();
    fs::remove_file(log).unwrap();
    said
}

/// Copy Link put `address` on Cellix's clipboard and handed it to the terminal loop, which
/// sends it by OSC 52, and said so.
fn copied(app: &mut App, address: &str) {
    assert_eq!(app.clipboard, address);
    assert_eq!(app.outgoing_clipboard.take().as_deref(), Some(address));
    assert_eq!(*app.message.borrow(), "Copied the link");
}

/// A link in a comment and one in its answer: a right click on either has Open Link and Copy
/// Link at the pointer, Copy Link puts that exact address on the clipboard, and Open Link
/// opens it as a click does. A right click on the words beside them opens nothing.
#[test]
fn a_comments_link_has_open_and_copy_on_a_right_click() {
    let (dir, mut app, log) = project();
    app.show_sidebar = false;
    app.open(&dir.path().join("alpha.txt")).unwrap();
    let text = "See https://example.com/spec/a, then the rest.";
    let fields = format!(r#""id":4,"file":"alpha.txt","line":1,"quote":"first","text":"{text}""#);
    let reply = r#"{"author":"Claude","text":"Agree: https://example.com/answer/b too."}"#;
    let thread = format!(r#"{{{fields},"replies":[{reply}]}}"#);
    app.threads = vec![serde_json::from_str(&thread).unwrap()];
    app.run(Command::ShowComments);

    let (x, y) = find(&screen(&app), "https://example.com/spec/a");
    right(&mut app, (x + 4, y));
    let rows = menu_rows(&app, &["Open Link", "Copy Link"]);
    // Dropped from under the pointer's row, so the link stays in view, in a menu's colour as
    // every menu is, over a card too.
    assert_eq!(rows, [Some(y + 1), Some(y + 2)], "{}", screen(&app).text());
    let grid = screen(&app);
    // The last raised panel painted, over the card's.
    let menu = grid.paint.iter().rev().find_map(|p| match p {
        Paint::Panel {
            fill: Some(fill),
            shadow: true,
            ..
        } => Some(*fill),
        _ => None,
    });
    assert_eq!(menu, Some(app.theme.popup), "a menu's colour");
    assert!(
        screen(&app)
            .row_text(y)
            .contains("https://example.com/spec/a")
    );
    choose(&mut app, "Copy Link");
    copied(&mut app, "https://example.com/spec/a");

    // The answer's link, and Open Link opens it.
    let (x, y) = find(&screen(&app), "https://example.com/answer/b");
    right(&mut app, (x + 4, y));
    choose(&mut app, "Copy Link");
    copied(&mut app, "https://example.com/answer/b");
    right(&mut app, (x + 4, y));
    choose(&mut app, "Open Link");
    assert_eq!(opened(&log), "https://example.com/answer/b");

    // Words that aren't a link have no menu.
    let (x, y) = find(&screen(&app), "See https");
    right(&mut app, (x + 1, y));
    assert!(
        matches!(app.overlay, Overlay::None),
        "{}",
        screen(&app).text()
    );
}

/// On a page a right click already has a menu. On a link the link's two rows stand at its
/// top, over the page's own, and Copy Link copies where the link goes, not its words. A link
/// to a file beside the page has Open File and Copy Path; words that aren't a link keep the
/// page's menu as it was.
#[test]
fn a_pages_link_puts_open_and_copy_at_the_top_of_its_menu() {
    let (dir, mut app, log) = project();
    let page =
        "See [the docs](https://example.com/docs/page) first.\n\nAnd [notes](notes.md) too.\n";
    fs::write(dir.path().join("page.md"), page).unwrap();
    fs::write(dir.path().join("notes.md"), "# Notes\n").unwrap();
    app.show_sidebar = false;
    app.open(&dir.path().join("page.md")).unwrap();
    assert!(
        !screen(&app).text().contains("](https"),
        "a page, not its source"
    );

    let (x, y) = find(&screen(&app), "the docs");
    right(&mut app, (x + 2, y));
    let rows = menu_rows(&app, &["Open Link", "Copy Link", "Show Source", "Paste"]);
    let [Some(open), Some(copy), Some(source), Some(paste)] = rows[..] else {
        panic!("{rows:?}: {}", screen(&app).text());
    };
    assert!(open < copy && copy < source && source < paste, "{rows:?}");
    assert_eq!(open, y + 1, "it drops from under the pointer's row");
    choose(&mut app, "Copy Link");
    copied(&mut app, "https://example.com/docs/page");
    right(&mut app, (x + 2, y));
    choose(&mut app, "Open Link");
    assert_eq!(opened(&log), "https://example.com/docs/page");

    // A link to a file beside it.
    let (x, y) = find(&screen(&app), "notes");
    right(&mut app, (x + 1, y));
    let rows = menu_rows(&app, &["Open File", "Copy Path", "Open Link"]);
    assert!(
        rows[0].is_some() && rows[1].is_some() && rows[2].is_none(),
        "{rows:?}"
    );
    choose(&mut app, "Copy Path");
    let notes = dir.path().join("notes.md");
    assert!(same(Path::new(&app.clipboard), &notes), "{}", app.clipboard);
    right(&mut app, (x + 1, y));
    choose(&mut app, "Open File");
    assert!(in_front(&app, &notes));

    // Words that aren't a link: the page's menu, as it was.
    app.open(&dir.path().join("page.md")).unwrap();
    let (x, y) = find(&screen(&app), "first.");
    right(&mut app, (x + 1, y));
    let rows = menu_rows(&app, &["Open Link", "Copy Link", "Show Source"]);
    assert_eq!(rows[..2], [None, None], "{}", screen(&app).text());
    assert!(rows[2].is_some());
}

/// A paper's `\href` and a URL in code's comment are links too: their menus start with the
/// link's two rows, and Copy Link copies the address.
#[test]
fn a_papers_link_and_one_in_code_have_open_and_copy_too() {
    let (dir, mut app, log) = project();
    let paper = "\\documentclass{article}\n\\begin{document}\nRead \\href{https://example.com/paper}{the paper} first.\n\\end{document}\n";
    fs::write(dir.path().join("paper.tex"), paper).unwrap();
    app.show_sidebar = false;
    app.open(&dir.path().join("paper.tex")).unwrap();
    let (x, y) = find(&screen(&app), "the paper");
    right(&mut app, (x + 5, y));
    let rows = menu_rows(&app, &["Open Link", "Copy Link", "Build PDF"]);
    let [Some(open), Some(copy), Some(build)] = rows[..] else {
        panic!("{rows:?}: {}", screen(&app).text());
    };
    assert!(open < copy && copy < build, "{rows:?}");
    choose(&mut app, "Copy Link");
    copied(&mut app, "https://example.com/paper");

    let code = "// The rules: \"https://example.com/spec#rules\".\nfn main() {}\n";
    fs::write(dir.path().join("main.rs"), code).unwrap();
    app.open(&dir.path().join("main.rs")).unwrap();
    let (x, y) = find(&screen(&app), "example.com/spec");
    right(&mut app, (x + 2, y));
    let rows = menu_rows(&app, &["Open Link", "Copy Link", "Go to Definition"]);
    let [Some(open), Some(copy), Some(definition)] = rows[..] else {
        panic!("{rows:?}: {}", screen(&app).text());
    };
    assert!(open < copy && copy < definition, "{rows:?}");
    choose(&mut app, "Copy Link");
    copied(&mut app, "https://example.com/spec#rules");
    right(&mut app, (x + 2, y));
    choose(&mut app, "Open Link");
    assert_eq!(opened(&log), "https://example.com/spec#rules");
}

/// Starts `script` in a terminal in the side pane, and waits until the screen shows `want`.
fn terminal(app: &mut App, script: &str, want: &str) {
    let argv = ["sh".to_string(), "-c".to_string(), script.to_string()];
    let root = app.workspace.root().to_path_buf();
    app.start_session("Stand-in", &argv, &[], &root);
    let deadline = Instant::now() + BACKGROUND;
    loop {
        app.poll();
        app.fit_dock();
        if screen(app).text().contains(want) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "no {want}: {}",
            screen(app).text()
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// A URL a terminal prints has the menu on a right click, and is copied whole, without the
/// full stop after it, even where it ran on past the pane's edge onto the rows under it.
/// Ctrl+click opens it. A `file:line` it prints has Open File and Copy Path; other words, no
/// menu.
#[test]
#[cfg(unix)]
fn a_terminals_link_has_open_and_copy_on_a_right_click() {
    let (dir, mut app, log) = project();
    app.show_sidebar = false;
    let long = format!("https://example.com/long/{}/end", "a".repeat(90));
    let script = format!(
        "echo 'see https://example.com/term/a.'; echo '{long}'; echo 'at alpha.txt:2 here'; \
         echo ready; sleep 60"
    );
    terminal(&mut app, &script, "ready");
    let pane = app.dock_screen().unwrap();
    assert!(long.len() > usize::from(pane.w), "it wraps: {}", pane.w);

    let (x, y) = find(&screen(&app), "https://example.com/term/a");
    right(&mut app, (x + 3, y));
    let rows = menu_rows(&app, &["Open Link", "Copy Link"]);
    assert_eq!(rows, [Some(y + 1), Some(y + 2)], "{}", screen(&app).text());
    let row = screen(&app).row_text(y);
    assert!(
        row.contains("https://example.com/term/a"),
        "the link in view: {row}"
    );
    choose(&mut app, "Copy Link");
    copied(&mut app, "https://example.com/term/a");
    right(&mut app, (x + 3, y));
    choose(&mut app, "Open Link");
    assert_eq!(opened(&log), "https://example.com/term/a");
    let ctrl = Mods {
        ctrl: true,
        ..Mods::default()
    };
    press(&mut app, MouseKind::Left, (x + 3, y), ctrl);
    press(&mut app, MouseKind::Up, (x + 3, y), ctrl);
    assert_eq!(
        opened(&log),
        "https://example.com/term/a",
        "Ctrl+click opens it"
    );

    // The long one, from its last row.
    let (x, y) = find(&screen(&app), "/end");
    right(&mut app, (x + 1, y));
    choose(&mut app, "Copy Link");
    copied(&mut app, &long);

    // Other words have none.
    let (x, y) = find(&screen(&app), "here");
    right(&mut app, (x + 1, y));
    assert!(
        matches!(app.overlay, Overlay::None),
        "{}",
        screen(&app).text()
    );

    // A file the output names, at its line.
    let (x, y) = find(&screen(&app), "alpha.txt:2");
    right(&mut app, (x + 1, y));
    let rows = menu_rows(&app, &["Open File", "Copy Path", "Open Link"]);
    assert!(
        rows[0].is_some() && rows[1].is_some() && rows[2].is_none(),
        "{rows:?}"
    );
    choose(&mut app, "Copy Path");
    let alpha = dir.path().join("alpha.txt");
    assert!(same(Path::new(&app.clipboard), &alpha), "{}", app.clipboard);
    right(&mut app, (x + 1, y));
    choose(&mut app, "Open File");
    assert!(in_front(&app, &alpha));
    assert_eq!(
        app.editors[app.active()].selections.primary().head,
        11,
        "line 2"
    );
}
