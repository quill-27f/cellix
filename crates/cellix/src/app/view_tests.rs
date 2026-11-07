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

    // Down at the end, a scope with only its `}` left in view isn't pinned; the one outside
    // it, still with a line to show, is.
    app.editors[0].top = 42;
    let grid = screen(&app);
    assert!(grid.row_text(top).contains("impl A {"), "{}", grid.text());
    assert!(!grid.text().contains("fn run"), "{}", grid.text());
    assert!(
        grid.row_text(top + 1).contains("step(39)"),
        "{}",
        grid.text()
    );
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
}
