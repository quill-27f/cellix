//! A comment's card under the pointer, while the Comments pane is shut.

use std::fs;

use cellix_input::{Event, Mods, MouseEvent, MouseKind};
use cellix_render::{Grid, Paint};
use cellix_review::Thread;
use tempfile::TempDir;

use super::glance::OPEN;
use super::{App, Front};

fn screen(app: &App) -> Grid {
    let mut grid = Grid::new(app.size().0, app.size().1, app.background());
    app.draw(&mut grid);
    grid
}

fn find(grid: &Grid, text: &str) -> (u16, u16) {
    for y in 0..grid.height() {
        if let Some(i) = grid.row_text(y).find(text) {
            return (grid.row_text(y)[..i].chars().count() as u16, y);
        }
    }
    panic!("{text:?} not on screen:\n{}", grid.text());
}

fn mouse(kind: MouseKind, x: u16, y: u16) -> Event {
    Event::Mouse(MouseEvent {
        kind,
        x,
        y,
        fine_x: f32::from(x) + 0.5,
        fine_y: f32::from(y) + 0.5,
        mods: Mods::default(),
    })
}

fn rest(app: &mut App, x: u16, y: u16) {
    app.handle(mouse(MouseKind::Move, x, y));
}

/// A page with a comment on `quote`, answered twice, the last time with a verdict.
fn commented(text: &str, line: usize, quote: &str) -> (TempDir, App) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("note.md");
    fs::write(&path, text).unwrap();
    let mut app = App::new(dir.path().to_path_buf(), (150, 40));
    app.open(&path).unwrap();
    let thread = format!(
        r#"{{"id": 1, "file": "note.md", "line": {line}, "quote": "{quote}",
            "text": "Why first? Measure it before deciding.", "kind": "Clarity",
            "replies": [{{"author": "You", "text": "Or say why."}},
                        {{"author": "Claude", "text": "Agree (Clarity): the order needs a reason."}}]}}"#
    );
    app.threads = vec![serde_json::from_str::<Thread>(&thread).unwrap()];
    (dir, app)
}

const NOTE: &str =
    "# Plan\n\nThe cache goes first, then the queue.\n\nNothing else is said here.\n";
/// The words commented on, which end their line.
const QUOTE: &str = "goes first, then the queue.";

#[test]
fn resting_on_commented_words_shows_their_comment_until_the_pointer_leaves() {
    let (_dir, mut app) = commented(NOTE, 3, QUOTE);
    assert!(!app.dock.open, "the pane is shut");
    let grid = screen(&app);
    let (x, y) = find(&grid, QUOTE);
    assert!(!grid.text().contains(OPEN));

    rest(&mut app, x + 3, y);
    let grid = screen(&app);
    let text = grid.text();
    assert!(
        text.contains("You · Clarity"),
        "who made it and its kind:\n{text}"
    );
    assert!(
        text.contains("Why first? Measure it before deciding."),
        "{text}"
    );
    assert!(
        text.contains("Agrees Claude:"),
        "the last answer, with its verdict as the pane says it:\n{text}"
    );
    assert!(text.contains("The order needs a reason."), "{text}");
    assert!(
        text.contains(&format!("1 more reply · {OPEN}")),
        "how many answers aren't shown:\n{text}"
    );
    assert!(!text.contains("Or say why"), "only the last answer");
    // Under the words, leaving them in view.
    assert!(grid.row_text(y).contains(QUOTE), "{text}");
    let card = app.glance_card().expect("a card");
    assert_eq!(card.area.y, y + 1, "on the row under the words");
    assert!(
        card.area.contains(x + 3, y + 1),
        "where a move straight down lands"
    );
    assert!(!app.dock.open, "the pane stays shut");

    // On the card it stays, to be pressed.
    rest(&mut app, x + 3, y + 2);
    assert!(screen(&app).text().contains(OPEN), "on the card");

    // Off it, under it, it goes.
    rest(&mut app, x + 3, card.area.bottom());
    assert!(app.glance_card().is_none());
    assert!(!screen(&app).text().contains(OPEN));

    // Past the end of the words, which end the line, is off them too.
    rest(&mut app, x + 3, y);
    assert!(app.glance_card().is_some());
    let (end, _) = find(&grid, "queue.");
    rest(&mut app, end + 12, y);
    assert!(app.glance_card().is_none(), "past the line's end");
    // And before where they start.
    rest(&mut app, x + 3, y);
    rest(&mut app, x.saturating_sub(5), y);
    assert!(app.glance_card().is_none(), "before the words");

    // A key takes it away.
    rest(&mut app, x + 3, y);
    app.handle(Event::Key(cellix_input::KeyEvent::plain(
        cellix_input::Key::Right,
    )));
    assert!(app.glance_card().is_none());
}

#[test]
fn a_press_on_the_card_opens_the_comments_on_its_thread() {
    let (_dir, mut app) = commented(NOTE, 3, QUOTE);
    let (x, y) = find(&screen(&app), QUOTE);
    rest(&mut app, x + 3, y);
    let (open_x, open_y) = find(&screen(&app), OPEN);
    app.handle(mouse(MouseKind::Left, open_x + 2, open_y));
    app.handle(mouse(MouseKind::Up, open_x + 2, open_y));
    assert!(app.dock.open, "the pane opens");
    assert_eq!(app.dock.front, Front::Comments);
    assert_eq!(app.lit_thread, Some(1), "on its thread");
    assert!(!app.dock.focused, "the keyboard stays in the file");
    let grid = screen(&app);
    let pane = app.dock_area().expect("the pane");
    let card = find(&grid, "Why first?");
    assert!(
        card.0 >= pane.x,
        "its card is in the pane:\n{}",
        grid.text()
    );
    assert!(!grid.text().contains(OPEN), "and the glance is gone");
    let selected = app.editors[app.active()].selected_text();
    assert_eq!(selected.as_deref(), Some(QUOTE), "its words are picked");

    // With the pane open, a press on the words is what lights their card, and resting on
    // them shows none.
    rest(&mut app, x + 3, y);
    assert!(app.glance_card().is_none());
    assert!(!screen(&app).text().contains(OPEN));
}

#[test]
fn the_card_stands_over_words_with_no_room_under_them() {
    let items: String = (1..=40).map(|n| format!("- item number {n}\n")).collect();
    let (_dir, mut app) = commented(&items, 34, "item number 34");
    let grid = screen(&app);
    let (x, y) = find(&grid, "item number 34");
    rest(&mut app, x + 3, y);
    let card = app.glance_card().expect("a card");
    assert!(
        card.area.bottom() <= y,
        "over the words: {:?} and row {y}",
        card.area
    );
    let grid = screen(&app);
    assert!(
        grid.row_text(y).contains("item number 34"),
        "the words stay in view"
    );
    assert!(grid.text().contains(OPEN), "{}", grid.text());
}

/// Where there are pictures the card is a raised panel on whole rows under the words, its
/// words half a row in from its top as a menu's are, and the commented words' row is left
/// clear of it.
#[test]
fn painted_the_card_stands_on_its_own_rows_under_the_words() {
    let (_dir, mut app) = commented(NOTE, 3, QUOTE);
    app.set_measure(std::rc::Rc::new(cellix_render::FontMeasure::new(17, 37)));
    screen(&app);
    let area = app.pane_area(0);
    let editor = &app.editors[app.active()];
    let start = editor.buffer.text().find(QUOTE).expect("the words");
    let (fine_x, y) = editor.cell_of(area, start + 4).expect("on screen");
    app.handle(Event::Mouse(MouseEvent {
        kind: MouseKind::Move,
        x: fine_x as u16,
        y,
        fine_x: fine_x + 0.3,
        fine_y: f32::from(y) + 0.5,
        mods: Mods::default(),
    }));
    let card = app.glance_card().expect("a card");
    assert_eq!(card.area.y, y + 1);
    let grid = screen(&app);
    let unit = cellix_render::UNIT;
    let panel = grid
        .paint
        .iter()
        .find_map(|p| match p {
            Paint::Panel {
                bounds,
                shadow: true,
                fill: Some(fill),
                ..
            } if *fill == app.theme.popup => Some(*bounds),
            _ => None,
        })
        .expect("the card's panel");
    assert!(
        panel.y >= i32::from(y + 1) * unit,
        "clear of the words' row: {panel:?}"
    );
    let first = grid
        .paint
        .iter()
        .find_map(|p| match p {
            Paint::Text { text, y, .. } if text == "You · Clarity" => Some(*y),
            _ => None,
        })
        .expect("its head painted");
    let pad = (crate::menu::PADDING * unit as f32) as i32;
    assert!(
        first - panel.y >= pad - 1,
        "half a row in: {first} under {}",
        panel.y
    );
    assert!(grid.text().contains(OPEN), "its words in the cells too");
}
