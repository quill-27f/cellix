//! The code view: bracket pairs, word wrap, folding and sticky scroll, driven through the app
//! and read off the drawn grid.

use std::fs;
use std::rc::Rc;

use cellix_input::{Event, Key, KeyEvent, Mods};
use cellix_render::{Grid, Paint, Rgb};
use tempfile::TempDir;

use super::App;

fn code(text: &str) -> (TempDir, App) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("code.rs");
    fs::write(&path, text).unwrap();
    let mut app = App::new(dir.path().to_path_buf(), (100, 30));
    app.open(&path).unwrap();
    (dir, app)
}

fn screen(app: &App) -> Grid {
    let mut grid = Grid::new(app.size().0, app.size().1, app.background());
    app.draw(&mut grid);
    grid
}

fn press(app: &mut App, key: Key, mods: Mods) {
    app.handle(Event::Key(KeyEvent { key, mods }));
}

fn plain(app: &mut App, key: Key) {
    press(app, key, Mods::default());
}

fn mouse(kind: cellix_input::MouseKind, x: u16, y: u16) -> Event {
    Event::Mouse(cellix_input::MouseEvent {
        kind,
        x,
        y,
        fine_x: x as f32 + 0.5,
        fine_y: y as f32 + 0.5,
        mods: Mods::default(),
    })
}

/// Where `text` starts on screen, below the tabs.
fn find(grid: &Grid, text: &str) -> (u16, u16) {
    (2..grid.height())
        .find_map(|y| {
            let row = grid.row_text(y);
            let at = row.find(text)?;
            Some((row[..at].chars().count() as u16, y))
        })
        .unwrap_or_else(|| panic!("{text:?} isn't drawn:\n{}", grid.text()))
}

fn fg(grid: &Grid, (x, y): (u16, u16)) -> Rgb {
    grid.cell(x, y).style.fg
}

fn bg(grid: &Grid, (x, y): (u16, u16)) -> Rgb {
    grid.cell(x, y).style.bg
}

#[test]
fn brackets_take_a_colour_by_depth_and_the_carets_pair_is_marked() {
    let (_dir, mut app) = code("fn f(a: [u8; 2]) {\n    g((1), \"(\");\n}\n");
    let colours = app.theme.brackets();
    let grid = screen(&app);
    let (x, y) = find(&grid, "fn f(a: [u8; 2]) {");
    assert_eq!(fg(&grid, (x + 4, y)), colours[0], "( at the top");
    assert_eq!(fg(&grid, (x + 8, y)), colours[1], "[ inside it");
    assert_eq!(fg(&grid, (x + 14, y)), colours[1], "] closes it");
    assert_eq!(fg(&grid, (x + 15, y)), colours[0], ") closes the first");
    assert_eq!(fg(&grid, (x + 17, y)), colours[0], "{{ after it");
    let (x, y) = find(&grid, "g((1)");
    assert_eq!(fg(&grid, (x + 1, y)), colours[1], "inside the braces");
    assert_eq!(fg(&grid, (x + 2, y)), colours[2]);
    let string = find(&grid, "\"(\"");
    assert_ne!(
        fg(&grid, (string.0 + 1, string.1)),
        colours[2],
        "a bracket in a string isn't one"
    );
    assert_ne!(bg(&grid, (x + 2, y)), app.theme.bracket_match());

    // The caret after `g(` touches the pair around `(1)`, from either side.
    plain(&mut app, Key::Down);
    for _ in 0..6 {
        plain(&mut app, Key::Right);
    }
    let grid = screen(&app);
    assert_eq!(
        bg(&grid, (x + 2, y)),
        app.theme.bracket_match(),
        "{}",
        grid.text()
    );
    assert_eq!(
        bg(&grid, (x + 4, y)),
        app.theme.bracket_match(),
        "its partner"
    );
    assert_ne!(bg(&grid, (x + 1, y)), app.theme.bracket_match());
    let rings = grid
        .paint
        .iter()
        .filter(
            |p| matches!(p, Paint::Panel { border: Some(b), .. } if *b == app.theme.bracket_ring()),
        )
        .count();
    assert_eq!(rings, 0, "cells have no ring");

    // Off in Settings, brackets are text again.
    app.settings.bracket_pairs = Some(false);
    app.editors[0].bracket_pairs = false;
    let grid = screen(&app);
    assert_eq!(fg(&grid, (x + 2, y)), app.theme.text.fg);
}

#[test]
fn a_stray_bracket_shows_in_the_error_colour_and_pixels_ring_the_pair() {
    let (_dir, mut app) = code("let a = (1));\n");
    app.set_measure(Rc::new(cellix_render::FontMeasure::new(10, 20)));
    let grid = screen(&app);
    let (x, y) = find(&grid, "(1))");
    assert_eq!(fg(&grid, (x + 3, y)), app.theme.error);
    for _ in 0..8 {
        plain(&mut app, Key::Right);
    }
    let grid = screen(&app);
    let rings = grid
        .paint
        .iter()
        .filter(
            |p| matches!(p, Paint::Wash { border: Some(b), .. } if *b == app.theme.bracket_ring()),
        )
        .count();
    assert_eq!(rings, 2, "both brackets of the pair");
    assert_ne!(
        bg(&grid, (x, y)),
        app.theme.bracket_match(),
        "one rounded shape, with no square cell under it"
    );
}

fn alt() -> Mods {
    Mods {
        alt: true,
        ..Mods::default()
    }
}

fn ctrl_shift() -> Mods {
    Mods {
        ctrl: true,
        shift: true,
        ..Mods::default()
    }
}

/// The line the caret is on, counted from 0.
fn caret_line(app: &App) -> usize {
    let editor = &app.editors[app.active()];
    editor
        .buffer
        .rope()
        .char_to_line(editor.selections.primary().head)
}

fn cursor(app: &App) -> (u16, u16) {
    screen(app).cursor.expect("a caret on screen")
}

#[test]
fn alt_z_wraps_long_lines_and_the_caret_moves_by_row() {
    let long = format!("let s = \"{}\";", "alpha beta gamma delta ".repeat(10));
    let (_dir, mut app) = code(&format!("{long}\nnext();\n"));
    let grid = screen(&app);
    let (x, y) = find(&grid, "let s");
    assert!(
        grid.row_text(y + 1).contains("next();"),
        "one row before wrapping"
    );

    press(&mut app, Key::Char('z'), alt());
    let grid = screen(&app);
    assert_eq!(find(&grid, "let s"), (x, y), "the first row stays put");
    let second = grid.row_text(y + 1);
    assert!(
        second.contains("alpha"),
        "the line goes on below: {second:?}"
    );
    let gutter: String = second.chars().take(x as usize).collect();
    assert!(
        !gutter.chars().any(|c| c.is_ascii_digit()),
        "a later row has no number: {gutter:?}"
    );
    let (_, next) = find(&grid, "next();");
    assert!(next > y + 2, "the long line takes several rows");
    let last = (0..grid.width())
        .rev()
        .find(|c| grid.cell(*c, y).symbol != " ")
        .unwrap();
    assert!(last < grid.width() - 2, "nothing runs off the pane");

    // Down goes to the next row of the same line; End to that row's end, then the line's.
    plain(&mut app, Key::Down);
    assert_eq!(caret_line(&app), 0);
    assert_eq!(cursor(&app).1, y + 1);
    plain(&mut app, Key::End);
    assert_eq!(caret_line(&app), 0);
    assert_eq!(cursor(&app).1, y + 1, "the row's end first");
    plain(&mut app, Key::End);
    assert_eq!(cursor(&app).1, next - 1, "then the line's");
    plain(&mut app, Key::Down);
    assert_eq!(caret_line(&app), 1);

    // A click on a later row puts the caret there.
    let grid = screen(&app);
    let (ax, ay) = (x + 3, y + 1);
    app.handle(mouse(cellix_input::MouseKind::Left, ax, ay));
    app.handle(mouse(cellix_input::MouseKind::Up, ax, ay));
    assert_eq!(caret_line(&app), 0);
    assert_eq!(cursor(&app), (ax, ay), "{}", grid.text());

    // Unwrapped, the view would scroll sideways to the caret, so it goes to the end first.
    press(
        &mut app,
        Key::End,
        Mods {
            ctrl: true,
            ..Mods::default()
        },
    );
    press(&mut app, Key::Char('z'), alt());
    let grid = screen(&app);
    assert_eq!(find(&grid, "next();").1, y + 1, "Alt+Z again unwraps");
}

#[test]
fn word_wrap_leaves_pages_alone() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("n.md");
    fs::write(&path, "# Title\n\nsome text\n").unwrap();
    let mut app = App::new(dir.path().to_path_buf(), (100, 30));
    app.open(&path).unwrap();
    press(&mut app, Key::Char('z'), alt());
    assert!(!app.editors[0].wrap);
}

const NESTED: &str = "struct A;\n\nimpl A {\n    fn one() {\n        first();\n        second();\n    }\n\n    fn two() {}\n}\n\nfn after() {}\n";

#[test]
fn ctrl_shift_brackets_fold_and_unfold_and_a_jump_inside_opens_the_fold() {
    let (_dir, mut app) = code(NESTED);
    // To `fn one() {`.
    for _ in 0..3 {
        plain(&mut app, Key::Down);
    }
    press(&mut app, Key::Char('['), ctrl_shift());
    let grid = screen(&app);
    let (x, y) = find(&grid, "fn one() {");
    let row = grid.row_text(y);
    assert!(row.contains("fn one() { ⋯ }"), "folded to one row: {row:?}");
    assert!(!grid.text().contains("first();"), "{}", grid.text());
    assert!(
        grid.row_text(y + 1).trim_end().ends_with('8'),
        "line 8 comes next"
    );
    assert!(
        grid.row_text(y).contains('›'),
        "a folded line keeps its chevron: {row:?}"
    );
    let brace = x + "fn one() { ⋯ ".chars().count() as u16;
    assert_eq!(grid.cell(brace, y).symbol, "}");
    assert_eq!(
        fg(&grid, (brace, y)),
        app.theme.brackets()[1],
        "the brace it took in keeps its pair's colour"
    );

    // Down steps over the fold rather than into it.
    plain(&mut app, Key::Down);
    assert_eq!(caret_line(&app), 7);

    press(&mut app, Key::Char(']'), ctrl_shift());
    let grid = screen(&app);
    assert!(
        !grid.text().contains("first();"),
        "Unfold at line 8 opens nothing"
    );
    plain(&mut app, Key::Up);
    press(&mut app, Key::Char(']'), ctrl_shift());
    assert!(screen(&app).text().contains("first();"));

    // Fold again, then go to a line inside it: the fold opens.
    press(&mut app, Key::Char('['), ctrl_shift());
    assert!(!screen(&app).text().contains("first();"));
    let area = app.focused_area();
    app.editors[0].go_to_line(5, area);
    let grid = screen(&app);
    assert!(grid.text().contains("first();"), "{}", grid.text());
    assert_eq!(caret_line(&app), 4);
    let _ = x;
}

#[test]
fn fold_all_and_unfold_all_and_folds_follow_their_line_through_edits() {
    let (_dir, mut app) = code(NESTED);
    app.run(cellix_input::Command::FoldAll);
    let grid = screen(&app);
    assert!(grid.text().contains("impl A { ⋯ }"), "{}", grid.text());
    assert!(!grid.text().contains("fn one"));
    app.run(cellix_input::Command::UnfoldAll);
    assert!(screen(&app).text().contains("first();"));

    // Folded, then two lines typed above it: the fold stays on its header.
    for _ in 0..3 {
        plain(&mut app, Key::Down);
    }
    press(&mut app, Key::Char('['), ctrl_shift());
    for _ in 0..3 {
        plain(&mut app, Key::Up);
    }
    plain(&mut app, Key::Enter);
    plain(&mut app, Key::Enter);
    let grid = screen(&app);
    assert!(grid.text().contains("fn one() { ⋯ }"), "{}", grid.text());
    assert!(!grid.text().contains("first();"));
}

#[test]
fn chevrons_show_on_hovering_the_gutter_and_a_click_folds() {
    let (_dir, mut app) = code(NESTED);
    let grid = screen(&app);
    let (x, y) = find(&grid, "fn one() {");
    assert!(!grid.row_text(y).contains('⌄'), "none at rest");
    // Over the gutter, every region that can fold shows its chevron, in the two cells
    // before the space that starts the text.
    let gutter = x - 4 - 3;
    app.handle(mouse(cellix_input::MouseKind::Move, gutter, y));
    let grid = screen(&app);
    assert!(grid.row_text(y).contains('⌄'), "{}", grid.row_text(y));
    let (_, impl_y) = find(&grid, "impl A {");
    assert!(grid.row_text(impl_y).contains('⌄'));
    assert_eq!(app.pointer(), "pointer");
    app.handle(mouse(cellix_input::MouseKind::Left, gutter, y));
    app.handle(mouse(cellix_input::MouseKind::Up, gutter, y));
    let grid = screen(&app);
    assert!(!grid.text().contains("first();"), "{}", grid.text());
    // The chip opens it again.
    let row = grid.row_text(y);
    let chip = row
        .find('⋯')
        .map(|at| row[..at].chars().count() as u16)
        .unwrap();
    app.handle(mouse(cellix_input::MouseKind::Left, chip, y));
    app.handle(mouse(cellix_input::MouseKind::Up, chip, y));
    assert!(screen(&app).text().contains("first();"));
}

#[test]
fn a_language_servers_regions_replace_the_indents() {
    let (dir, mut app) = code("call(\n    a,\n    b,\n);\nrest\n");
    // By indent `call(` folds its arguments; the server says only lines 1 to 2.
    let path = dir.path().join("code.rs");
    app.lsp_event(cellix_lang::lsp::LspEvent::Folds(path, vec![(1, 2)]));
    plain(&mut app, Key::Down);
    press(&mut app, Key::Char('['), ctrl_shift());
    let grid = screen(&app);
    assert!(grid.text().contains("call("), "{}", grid.text());
    assert!(grid.text().contains("a, ⋯"), "{}", grid.text());
    assert!(!grid.text().contains("b,"));
}

fn long_scope() -> String {
    let body: String = (0..40).map(|n| format!("        step({n});\n")).collect();
    format!("struct A;\n\nimpl A {{\n    fn run(&self) {{\n{body}    }}\n}}\n")
}

#[test]
fn sticky_scroll_pins_the_scopes_headers_and_a_click_jumps_there() {
    let (_dir, mut app) = code(&long_scope());
    let area = app.focused_area();
    app.editors[0].scroll_by(20);
    let grid = screen(&app);
    let (_, impl_y) = find(&grid, "impl A {");
    let (_, run_y) = find(&grid, "fn run(&self) {");
    assert_eq!(run_y, impl_y + 1, "outermost first");
    let top = area.y;
    assert!(impl_y <= top + 1, "pinned at the top: {}", grid.text());
    assert!(
        grid.row(run_y).iter().any(|c| c.style.underline),
        "a line under them on cells"
    );
    assert!(!grid.text().contains("step(0)"), "{}", grid.text());
    assert!(grid.row_text(run_y + 1).contains("step("));
    // Rows under the pinned lines move with the wheel; the pinned ones don't.
    let region = app.editors[0].scroll_region(area);
    assert_eq!(region.y, run_y + 1);

    let (run_x, _) = find(&grid, "fn run(&self) {");
    app.handle(mouse(cellix_input::MouseKind::Left, run_x, run_y));
    app.handle(mouse(cellix_input::MouseKind::Up, run_x, run_y));
    assert_eq!(caret_line(&app), 3, "the caret goes to the header");
    let grid = screen(&app);
    assert_eq!(
        find(&grid, "fn run(&self) {").1,
        run_y,
        "under the header outside it"
    );
    assert_eq!(find(&grid, "impl A {").1, impl_y);
    assert_eq!(
        find(&grid, "step(0)").1,
        run_y + 1,
        "its scope shows from the start"
    );

    // Down at the end, a scope whose last line would be first under the headers keeps its
    // header over that line, with its `}` after it: step(39) stood under `impl` with no `fn
    // run` over it (changed for 112, 19 November). One row on, with only its `}` left in
    // view, it isn't pinned; the one outside it, still with a line to show, is.
    app.editors[0].top = 42;
    let grid = screen(&app);
    assert!(grid.row_text(top).contains("impl A {"), "{}", grid.text());
    assert!(grid.row_text(top + 1).contains("fn run"), "{}", grid.text());
    let under = grid.row_text(top + 2);
    assert!(under.contains('}') && !under.contains("step("), "{under}");
    app.editors[0].top = 43;
    let grid = screen(&app);
    assert!(grid.row_text(top).contains("impl A {"), "{}", grid.text());
    assert!(!grid.text().contains("fn run"), "{}", grid.text());
    app.editors[0].top = 44;
    let grid = screen(&app);
    assert!(!grid.text().contains("impl A {"), "{}", grid.text());

    // Off in Settings, nothing is pinned.
    app.settings.sticky_scroll = Some(false);
    app.editors[0].sticky_lines = 0;
    app.editors[0].scroll_by(20);
    let grid = screen(&app);
    assert!(!grid.text().contains("impl A {"), "{}", grid.text());
}

/// The tour's `src/scheduler.rs`, whose `match` (lines 23 to 31) is inside a `while` (18),
/// inside `run_due` (16), inside `impl` (11).
const SCHEDULER: &str = r#"//! Runs jobs in order, each when its time comes.
use std::collections::BinaryHeap;
use std::time::{Duration, Instant};

/// Holds jobs until they're due; a job that fails is tried again after a pause that doubles each time, up to a limit, and is then reported with the error it last gave.
pub struct Scheduler {
    queue: BinaryHeap<Job>,
    retries: u32,
}

impl Scheduler {
    pub fn new(retries: u32) -> Self {
        Self { queue: BinaryHeap::new(), retries }
    }

    pub fn run_due(&mut self, now: Instant) -> Vec<Result<(), String>> {
        let mut done = Vec::new();
        while let Some(job) = self.queue.peek() {
            if job.due > now {
                break;
            }
            let mut job = self.queue.pop().expect("peeked");
            match (job.work)() {
                Ok(()) => done.push(Ok(())),
                Err(error) if job.tries < self.retries => {
                    job.tries += 1;
                    job.due = now + Duration::from_millis(100 * 2u64.pow(job.tries));
                    self.queue.push(job);
                }
                Err(error) => done.push(Err(format!("{} failed after {} tries: {error}", job.name, job.tries))),
            }
        }
        done
    }

    pub fn len(&self) -> usize {
        self.queue.len()
    }
}

struct Job {
    name: String,
    due: Instant,
    tries: u32,
    work: Box<dyn FnMut() -> Result<(), String>>,
}
"#;

/// At the offset 112-view-sticky landed on, line 27 at the top, the headers were `impl`,
/// `run_due` and `while`, and the first line under them was line 30, the `match`'s last arm,
/// with the `match` nowhere: left out because its last line would be under its own header,
/// which left that line first under the others. A scope's header stays while any line of it
/// is first under the headers, and then what's first is looked at again: the `}` closing the
/// `match`, which says nothing of a scope, so nothing more is pinned.
#[test]
fn a_scopes_header_stays_while_a_line_of_it_is_first_under_the_headers() {
    let (_dir, mut app) = code(SCHEDULER);
    let top = app.focused_area().y;
    app.editors[0].top = 26;
    let grid = screen(&app);
    let headers = [
        "impl Scheduler {",
        "pub fn run_due",
        "while let Some(job)",
        "match (job.work)() {",
    ];
    for (row, header) in headers.iter().enumerate() {
        let text = grid.row_text(top + row as u16);
        assert!(
            text.contains(header),
            "{header} on row {row}:\n{}",
            grid.text()
        );
    }
    // The `match`'s `}` under them, with the arm its header stands over out of view.
    let under = grid.row_text(top + 4);
    assert!(under.contains('}') && !under.contains("Err("), "{under}");
    assert!(!grid.text().contains("failed after"), "{}", grid.text());
    // A row on, the `match` has only its `}` left, and its header goes with it.
    app.editors[0].top = 27;
    let grid = screen(&app);
    assert!(!grid.text().contains("match (job.work)"), "{}", grid.text());
    assert!(grid.row_text(top + 2).contains("while let Some(job)"));
}

/// Go to Line, and whatever else reveals a line, shows it under the headers pinned over the
/// top, as VS Code does: reached from below, line 27 stood on the top row, under `impl`,
/// `run_due`, `while` and `match`, out of sight (the tour's 112 and 115).
#[test]
fn going_to_a_line_in_nested_scopes_shows_it_under_the_pinned_headers() {
    let (_dir, mut app) = code(SCHEDULER);
    // As short as the tour's window, where line 46 in view leaves line 27 above it.
    app.handle(Event::Resize(120, 24));
    let go = |app: &mut App, line: &str| {
        app.run(cellix_input::Command::GoToLine);
        for c in line.chars() {
            plain(app, Key::Char(c));
        }
        plain(app, Key::Enter);
    };
    // To the end first, so the line is come to from below, as the tour does.
    go(&mut app, "46");
    go(&mut app, "27");
    assert_eq!(caret_line(&app), 26);
    let area = app.focused_area();
    let grid = screen(&app);
    let text = app.editors[0].text_area(area);
    let first = app.editors[0].scroll_region(area).y;
    let (_, y) = find(&grid, "job.due = now");
    assert!(
        y >= first,
        "on row {y}, under the pinned rows that end at {first}"
    );
    // The headers of the scopes it's in stand over it, the innermost, the arm it's in, last,
    // and it's the first line under them.
    assert!(first > text.y, "headers are pinned: {}", grid.text());
    let pinned = grid.row_text(first - 1);
    assert!(
        pinned.contains("Err(error) if job.tries"),
        "{}",
        grid.text()
    );
    assert_eq!(y, first, "first under them: {}", grid.text());
    // A line already on screen but behind the headers is brought out from under them too:
    // line 28 with line 26 at the top, where four headers stand over lines 26 to 29.
    app.editors[0].top = 25;
    let first = app.editors[0].scroll_region(area).y;
    assert!(first >= text.y + 4, "line 28 is behind them");
    go(&mut app, "28");
    let first = app.editors[0].scroll_region(area).y;
    let (_, y) = find(&screen(&app), "self.queue.push(job);");
    assert!(
        y >= first,
        "on row {y}, under the pinned rows that end at {first}"
    );
}

/// A language server for the reveals below, never a real one: answers the handshake, finds a
/// file's `fn` lines as its symbols and the workspace's, and sends every definition to the
/// line that declares `pause`.
const REVEAL_SERVER: &str = r#"
import json, re, sys
texts = {}
def symbols(uri, text):
    out = []
    for n, line in enumerate(text.split("\n")):
        hit = re.search(r"fn (\w+)", line)
        if hit:
            out.append({"name": hit.group(1), "kind": 12, "containerName": "",
                "location": {"uri": uri, "range": {
                    "start": {"line": n, "character": hit.start(1)},
                    "end": {"line": n, "character": hit.end(1)}}}})
    return out
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
        texts[m["params"]["textDocument"]["uri"]] = m["params"]["textDocument"]["text"]
    elif method == "textDocument/didChange":
        texts[m["params"]["textDocument"]["uri"]] = m["params"]["contentChanges"][-1]["text"]
    elif method == "textDocument/documentSymbol":
        uri = m["params"]["textDocument"]["uri"]
        send({"jsonrpc": "2.0", "id": i, "result": symbols(uri, texts.get(uri, ""))})
    elif method == "workspace/symbol":
        q = m["params"]["query"].lower()
        found = [s for u, t in texts.items() for s in symbols(u, t) if q in s["name"].lower()]
        send({"jsonrpc": "2.0", "id": i, "result": found})
    elif method == "textDocument/definition":
        uri = m["params"]["textDocument"]["uri"]
        lines = texts.get(uri, "").split("\n")
        n = next(k for k, line in enumerate(lines) if "fn pause" in line)
        at = lines[n].index("pause")
        send({"jsonrpc": "2.0", "id": i, "result": {"uri": uri,
            "range": {"start": {"line": n, "character": at}, "end": {"line": n, "character": at + 5}}}})
    elif i is not None:
        send({"jsonrpc": "2.0", "id": i, "result": None})
"#;

/// The scheduler with a helper declared in the arm that retries, five scopes in: `impl`,
/// `run_due`, `while`, `match` and the arm. Line 26 declares `pause`, and line 27 is
/// `job.due = now`, still inside them all.
fn nested_scheduler() -> String {
    let nested = SCHEDULER.replace(
        "                    job.tries += 1;\n",
        "                    fn pause(tries: u32) -> u64 { 100 * 2u64.pow(tries) }\n",
    );
    assert_ne!(nested, SCHEDULER);
    nested
}

/// The nested scheduler open in a window as short as the tour's, with the stand-in language
/// server where `served`.
fn scheduler(served: bool) -> (TempDir, App) {
    let dir = tempfile::tempdir().unwrap();
    if served {
        fs::write(dir.path().join("server.py"), REVEAL_SERVER).unwrap();
        fs::create_dir(dir.path().join(".cellix")).unwrap();
        let server = dir.path().join("server.py");
        let settings = serde_json::json!({"language_servers": {"Rust": ["python3", "-I", server]}});
        fs::write(
            dir.path().join(".cellix/settings.json"),
            settings.to_string(),
        )
        .unwrap();
    }
    let path = dir.path().join("code.rs");
    fs::write(&path, nested_scheduler()).unwrap();
    let mut app = App::new(dir.path().to_path_buf(), (120, 24));
    app.open(&path).unwrap();
    (dir, app)
}

/// Polls what runs in the background until `done`, for 30 seconds at most.
fn wait(app: &mut App, done: impl Fn(&App) -> bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
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

fn go_to_line(app: &mut App, line: &str) {
    app.run(cellix_input::Command::GoToLine);
    for c in line.chars() {
        plain(app, Key::Char(c));
    }
    plain(app, Key::Enter);
}

/// Where a jump starts from: its line come to from below, with the end of the file in view, as
/// the tour does; or on screen but behind the headers pinned over it, line 26 at the top,
/// where they stand over line 27.
#[derive(Clone, Copy, Debug)]
enum From {
    Below,
    Behind,
}

fn start(app: &mut App, from: From) {
    match from {
        From::Below => go_to_line(app, "46"),
        From::Behind => {
            let index = app.active();
            app.editors[index].top = 25;
            let area = app.focused_area();
            let text = app.editors[index].text_area(area);
            let first = app.editors[index].scroll_region(area).y;
            assert!(first >= text.y + 2, "line 27 is behind the headers");
        }
    }
}

/// Where `words` start in `area` of the screen.
fn find_in(grid: &Grid, area: cellix_render::Rect, words: &str) -> Option<(u16, u16)> {
    (area.y..area.bottom()).find_map(|y| {
        let row: String = (area.x..area.right())
            .map(|x| grid.cell(x, y).symbol.to_string())
            .collect();
        let at = row.find(words)?;
        Some((area.x + row[..at].chars().count() as u16, y))
    })
}

/// The caret is on `line`, counted from one, and the line holding `words` is drawn under the
/// headers pinned over the top of the file, not behind them, and when `first`, as the first line
/// under them, as Go to Line leaves a line come to from below.
fn under_the_headers(app: &App, (words, line): (&str, usize), first: bool, what: &str) {
    assert_eq!(caret_line(app) + 1, line, "{what}: the caret's line");
    let area = app.focused_area();
    let pinned = app.editors[app.active()].scroll_region(area).y;
    let grid = screen(app);
    let (_, y) = find_in(&grid, area, words)
        .unwrap_or_else(|| panic!("{what}: {words:?} isn't in view:\n{}", grid.text()));
    assert!(
        y >= pinned,
        "{what}: on row {y}, under the pinned rows that end at {pinned}:\n{}",
        grid.text()
    );
    if first {
        assert_eq!(
            y,
            pinned,
            "{what}: on row {y}, first under the pinned rows that end at {pinned}:\n{}",
            grid.text()
        );
    }
}

/// A result of a search of the project, picked in the sidebar, shows its line under the
/// headers pinned over it, as Go to Line does (`going_to_a_line_in_nested_scopes_...`).
#[test]
fn a_search_result_in_nested_scopes_shows_under_the_pinned_headers() {
    for from in [From::Below, From::Behind] {
        let (_dir, mut app) = scheduler(false);
        start(&mut app, from);
        app.run(cellix_input::Command::FindInProject);
        for c in "due = now".chars() {
            plain(&mut app, Key::Char(c));
        }
        wait(&mut app, |a| {
            screen(a).text().contains("1 result in 1 file")
        });
        let sidebar = app.sidebar_area().expect("the sidebar");
        // The result's row, not the query in the box over it.
        let (x, y) = find_in(&screen(&app), sidebar, "job.due = now").expect("the result");
        app.handle(mouse(cellix_input::MouseKind::Left, x, y));
        app.handle(mouse(cellix_input::MouseKind::Up, x, y));
        let what = format!("a search's result, from {from:?}");
        under_the_headers(
            &app,
            ("job.due = now", 27),
            matches!(from, From::Below),
            &what,
        );
    }
}

/// A problem picked in the Problems list does the same.
#[test]
fn a_problem_in_nested_scopes_shows_under_the_pinned_headers() {
    for from in [From::Below, From::Behind] {
        let (dir, mut app) = scheduler(false);
        app.diagnostics.insert(
            dir.path().join("code.rs"),
            vec![cellix_lang::lsp::Diagnostic {
                line: 26,
                column: 20,
                message: "The pause can overflow".to_string(),
                severity: 1,
            }],
        );
        let (at, _) = app.status_problems().expect("the count of problems");
        let foot = app.size().1 - 1;
        app.handle(mouse(cellix_input::MouseKind::Left, at, foot));
        app.handle(mouse(cellix_input::MouseKind::Up, at, foot));
        // With the list open beside it, so the file is as wide as it will be.
        start(&mut app, from);
        let grid = screen(&app);
        let (x, y) = find(&grid, "The pause can overflow");
        app.handle(mouse(cellix_input::MouseKind::Left, x, y));
        app.handle(mouse(cellix_input::MouseKind::Up, x, y));
        let what = format!("a problem, from {from:?}");
        under_the_headers(
            &app,
            ("job.due = now", 27),
            matches!(from, From::Below),
            &what,
        );
    }
}

/// Go to Symbol, in the file and in the workspace, shows the symbol's line under the headers
/// too: in the middle of the file where it was out of view, as VS Code does, and brought out
/// from under the headers where it was behind them.
#[test]
fn a_symbol_in_nested_scopes_shows_under_the_pinned_headers() {
    for from in [From::Below, From::Behind] {
        let (_dir, mut app) = scheduler(true);
        start(&mut app, from);
        app.run(cellix_input::Command::GoToSymbol);
        // The stand-in server answers: what it found is listed.
        wait(&mut app, |a| screen(a).text().contains("functions (4)"));
        for c in "pause".chars() {
            plain(&mut app, Key::Char(c));
        }
        plain(&mut app, Key::Enter);
        let what = format!("Go to Symbol in the file, from {from:?}");
        under_the_headers(&app, ("fn pause", 26), false, &what);

        let (_dir, mut app) = scheduler(true);
        // The server knows the file once it's been asked about it.
        app.run(cellix_input::Command::GoToSymbol);
        wait(&mut app, |a| screen(a).text().contains("functions (4)"));
        plain(&mut app, Key::Esc);
        start(&mut app, from);
        app.handle(Event::Key(KeyEvent::ctrl('t')));
        for c in "pause".chars() {
            plain(&mut app, Key::Char(c));
        }
        wait(&mut app, |a| screen(a).text().contains("1 symbol"));
        plain(&mut app, Key::Enter);
        let what = format!("Go to Symbol in the workspace, from {from:?}");
        under_the_headers(&app, ("fn pause", 26), false, &what);
    }
}

/// Go to Definition, answered by the server, shows the definition's line under the headers.
#[test]
fn a_definition_in_nested_scopes_shows_under_the_pinned_headers() {
    for from in [From::Below, From::Behind] {
        let (_dir, mut app) = scheduler(true);
        start(&mut app, from);
        app.run(cellix_input::Command::GoToDefinition);
        wait(&mut app, |a| caret_line(a) == 25);
        // Come to from below, the arm's own first line, 25, stands over it unpinned, the
        // headers over that, as VS Code shows a line whose scope starts just over it.
        let what = format!("Go to Definition, from {from:?}");
        under_the_headers(&app, ("fn pause", 26), false, &what);
    }
}

#[test]
fn a_pinned_header_shows_its_chevron_and_folds_from_it() {
    let (_dir, mut app) = code(&long_scope());
    app.editors[0].scroll_by(20);
    let grid = screen(&app);
    let (x, run_y) = find(&grid, "fn run(&self) {");
    let gutter = x - 4 - 3;
    app.handle(mouse(cellix_input::MouseKind::Move, gutter, run_y));
    let grid = screen(&app);
    assert!(
        grid.row_text(run_y).contains('⌄'),
        "{}",
        grid.row_text(run_y)
    );
    app.handle(mouse(cellix_input::MouseKind::Left, gutter, run_y));
    app.handle(mouse(cellix_input::MouseKind::Up, gutter, run_y));
    let grid = screen(&app);
    assert!(
        grid.text().contains("fn run(&self) { ⋯ }"),
        "the pinned scope folds, not the row under it: {}",
        grid.text()
    );
}

#[test]
fn sticky_lines_draw_a_shadow_on_pixels() {
    let (_dir, mut app) = code(&long_scope());
    app.set_measure(Rc::new(cellix_render::FontMeasure::new(10, 20)));
    app.editors[0].scroll_by(20);
    let grid = screen(&app);
    let shadows = grid
        .paint
        .iter()
        .filter(|p| matches!(p, Paint::Wash { color, .. } if *color == Rgb(0, 0, 0)))
        .count();
    assert_eq!(shadows, 3, "three bands fading down");
    // On graphite's dark sheet a shadow hardly shows, so a hairline, as between any two
    // regions, stands where the pinned lines end, at the shadow's top.
    let first = grid.paint.iter().find_map(|p| match p {
        Paint::Wash { bounds, color, .. } if *color == Rgb(0, 0, 0) => Some(bounds.y),
        _ => None,
    });
    assert!(first.is_some());
    let border = app.theme.border;
    let line = grid.paint.iter().any(
        |p| matches!(p, Paint::Hairline { bounds, color } if *color == border && Some(bounds.y) == first),
    );
    assert!(line, "a hairline at the shadow's top");
    // Light, the shadow shows and stands alone.
    app.match_terminal_background(Some((0xEF, 0xF1, 0xF5)));
    let grid = screen(&app);
    let border = app.theme.border;
    let first = grid.paint.iter().find_map(|p| match p {
        Paint::Wash { bounds, color, .. } if *color == Rgb(0, 0, 0) => Some(bounds.y),
        _ => None,
    });
    assert!(first.is_some(), "the shadow");
    let line = grid
        .paint
        .iter()
        .any(|p| matches!(p, Paint::Hairline { bounds, color } if *color == border && Some(bounds.y) == first));
    assert!(!line, "no line under them in the light theme");
}
