//! Suggestions on a page: Accept and Reject by the one at the caret or under the pointer, the
//! row in the Comments pane that counts them and settles them all, and an agent's rewrite taken
//! in one step. Driven through the app and read off the drawn grid.

use std::fs;

use cellix_input::{Event, Key, KeyEvent, Mods, MouseEvent, MouseKind};
use cellix_render::Grid;
use tempfile::TempDir;

use super::{App, Command};

fn page(name: &str, text: &str) -> (TempDir, App) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(name);
    fs::write(&path, text).unwrap();
    let mut app = App::new(dir.path().to_path_buf(), (150, 40));
    app.open(&path).unwrap();
    (dir, app)
}

/// A paper loading the package its suggestions print with, around `body`.
fn paper(body: &str) -> String {
    format!(
        "\\documentclass{{article}}\n\\usepackage{{changes}}\n\\begin{{document}}\n\n{body}\n\n\\end{{document}}\n"
    )
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
        fine_x: x as f32 + 0.5,
        fine_y: y as f32 + 0.5,
        mods: Mods::default(),
    })
}

/// A press and its release, as a click is.
fn click(app: &mut App, (x, y): (u16, u16)) {
    app.handle(mouse(MouseKind::Left, x, y));
    app.handle(mouse(MouseKind::Up, x, y));
}

fn point(app: &mut App, (x, y): (u16, u16)) {
    app.handle(mouse(MouseKind::Move, x, y));
}

/// Where `text` is first drawn, under the tabs.
fn find(grid: &Grid, text: &str) -> Option<(u16, u16)> {
    (1..grid.height()).find_map(|y| {
        let row = grid.row_text(y);
        let at = row.find(text)?;
        Some((row[..at].chars().count() as u16, y))
    })
}

fn spot(app: &App, text: &str) -> (u16, u16) {
    let grid = screen(app);
    find(&grid, text).unwrap_or_else(|| panic!("{text:?} isn't drawn:\n{}", grid.text()))
}

/// Accept and Reject by a suggestion, as Review has them: where each stands, if they do.
fn chips(app: &App) -> Option<((u16, u16), (u16, u16))> {
    let grid = screen(app);
    let (x, y) = find(&grid, " Reject  Accept ")?;
    Some(((x + 2, y), (x + 10, y)))
}

fn said(app: &App) -> String {
    app.message.borrow().clone()
}

fn undo(app: &mut App) {
    app.handle(Event::Key(KeyEvent {
        key: Key::Char('z'),
        mods: Mods {
            ctrl: true,
            ..Mods::default()
        },
    }));
}

#[test]
fn a_suggestion_at_the_caret_is_accepted_or_rejected_by_its_chips() {
    let (_dir, mut app) = page(
        "note.md",
        "# Notes\n\nKeep {++added++} and {--gone--} end.\n",
    );
    assert!(chips(&app).is_none(), "the caret is in no suggestion");
    let (x, y) = spot(&app, "added");
    click(&mut app, (x + 2, y));
    let (_, accept) = chips(&app).expect("Accept and Reject by it");
    assert!(
        accept.1 < y,
        "over the row it's on, as a selection's buttons are"
    );
    click(&mut app, accept);
    assert_eq!(
        app.active_text().unwrap(),
        "# Notes\n\nKeep added and {--gone--} end.\n"
    );
    assert_eq!(said(&app), "Accepted. 1 suggestion left");
    assert!(
        chips(&app).is_none(),
        "the caret is after it, in plain words now"
    );
    let (x, y) = spot(&app, "gone");
    click(&mut app, (x + 1, y));
    let (reject, _) = chips(&app).expect("by the other one");
    click(&mut app, reject);
    assert_eq!(
        app.active_text().unwrap(),
        "# Notes\n\nKeep added and gone end.\n"
    );
    assert_eq!(said(&app), "Rejected. No suggestions left");
    undo(&mut app);
    assert_eq!(
        app.active_text().unwrap(),
        "# Notes\n\nKeep added and {--gone--} end.\n",
        "one edit to undo"
    );
}

#[test]
fn the_chips_stand_by_the_suggestion_under_the_pointer_and_stay_on_the_way_to_them() {
    let (_dir, mut app) = page(
        "note.md",
        "# Notes\n\nThe {~~tests~>experiments~~} show it, and {++clearly++} so.\n",
    );
    assert!(chips(&app).is_none());
    let (x, y) = spot(&app, "clearly");
    point(&mut app, (x + 3, y));
    let (_, accept) = chips(&app).expect("chips by what's pointed at");
    // Up from the words to the chips, by way of the row's other words.
    point(&mut app, (accept.0, y));
    point(&mut app, accept);
    let (_, still) = chips(&app).expect("still there on the chips");
    assert_eq!(still, accept);
    click(&mut app, accept);
    assert_eq!(
        app.active_text().unwrap(),
        "# Notes\n\nThe {~~tests~>experiments~~} show it, and clearly so.\n",
        "the one pointed at, not another"
    );
    // Off the page, nothing is pointed at, and the caret's in no suggestion.
    point(&mut app, (2, 2));
    assert!(chips(&app).is_none());
    let (x, y) = spot(&app, "experiments");
    point(&mut app, (x + 2, y));
    let (reject, _) = chips(&app).expect("by a replacement too");
    click(&mut app, reject);
    assert_eq!(
        app.active_text().unwrap(),
        "# Notes\n\nThe tests show it, and clearly so.\n"
    );
}

#[test]
fn a_selection_keeps_its_own_buttons_over_a_suggestion() {
    let (_dir, mut app) = page("note.md", "# Notes\n\nKeep {++added words++} here.\n");
    let (x, y) = spot(&app, "added");
    for _ in 0..2 {
        click(&mut app, (x + 1, y));
    }
    let text = screen(&app).text();
    assert!(text.contains("Comment"), "{text}");
    assert!(chips(&app).is_none(), "{text}");
}

#[test]
fn a_paper_s_suggestions_have_the_same_chips() {
    let source = paper(
        "This sentence \\added{gains words}, \\deleted{loses these}, and \
         \\replaced{a new word}{an old word}.",
    );
    let (_dir, mut app) = page("paper.tex", &source);
    let (x, y) = spot(&app, "gains");
    click(&mut app, (x + 2, y));
    let (_, accept) = chips(&app).expect("by \\added");
    click(&mut app, accept);
    let text = app.active_text().unwrap();
    assert!(
        text.contains("This sentence gains words, \\deleted"),
        "{text}"
    );
    let (x, y) = spot(&app, "new word");
    click(&mut app, (x + 2, y));
    let (reject, _) = chips(&app).expect("by \\replaced");
    click(&mut app, reject);
    let text = app.active_text().unwrap();
    assert!(text.contains(", and an old word."), "{text}");
    assert_eq!(said(&app), "Rejected. 1 suggestion left");
}

#[test]
fn the_right_click_menu_and_the_commands_still_settle_one() {
    let (_dir, mut app) = page("note.md", "# Notes\n\nKeep {++added++} end.\n");
    let (x, y) = spot(&app, "added");
    click(&mut app, (x + 2, y));
    app.run(Command::AcceptChange);
    assert_eq!(app.active_text().unwrap(), "# Notes\n\nKeep added end.\n");
}
