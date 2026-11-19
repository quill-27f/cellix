//! The comment cards and what's round them, as the final review of 0.11.0 measured them in
//! the tour's pictures (17 px columns, rows of 50 with words sized for 36): one colour for
//! one meaning, the same words for the same thing, and one inset for every card. Driven
//! through the App's input, and read from the Grid and the paint it draws.

use std::fs;
use std::rc::Rc;
use std::time::Duration;

use cellix_agent::Request;
use cellix_input::{Command, Event, Key, KeyEvent, Mods, MouseEvent, MouseKind};
use cellix_render::{Bounds, CellMeasure, FontMeasure, Grid, Paint, Rgb, UNIT};
use tempfile::TempDir;

use super::App;
use crate::theme::Theme;

fn airy(app: &mut App) {
    app.set_measure(Rc::new(FontMeasure::new(17, 50).with_type_row(36)));
}

/// Every letter a cell again, where words are found and pressed by what the cells say.
fn cells(app: &mut App) {
    app.set_measure(Rc::new(CellMeasure));
}

/// The screen as the tour's terminal paints it, with the pixels' measure for this look only.
fn painted(app: &mut App) -> Grid {
    airy(app);
    let grid = screen(app);
    cells(app);
    grid
}

fn page(text: &str) -> (TempDir, App) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("note.md");
    fs::write(&path, text).unwrap();
    let mut app = App::new(dir.path().to_path_buf(), (150, 40));
    app.open(&path).unwrap();
    (dir, app)
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
        fine_x: x as f32,
        fine_y: y as f32 + 0.5,
        mods: Mods::default(),
    })
}

fn click(app: &mut App, x: u16, y: u16) {
    app.handle(mouse(MouseKind::Left, x, y));
    app.handle(mouse(MouseKind::Up, x, y));
}

fn typed(app: &mut App, text: &str) {
    for c in text.chars() {
        app.handle(Event::Key(KeyEvent::plain(Key::Char(c))));
    }
}

fn enter(app: &mut App) {
    app.handle(Event::Key(KeyEvent::plain(Key::Enter)));
}

/// Where `text` first shows on the screen, as its column and row.
fn find(grid: &Grid, text: &str) -> (u16, u16) {
    for y in 0..grid.height() {
        if let Some(i) = grid.row_text(y).find(text) {
            return (grid.row_text(y)[..i].chars().count() as u16, y);
        }
    }
    panic!("{text:?} not on screen:\n{}", grid.text());
}

/// Presses the first place on screen that says `words`.
fn press(app: &mut App, words: &str) {
    let (x, y) = find(&screen(app), words);
    click(app, x + 1, y);
}

/// Where `text` first shows on the page, left of the side pane where it's open.
fn on_page(app: &App, grid: &Grid, text: &str) -> (u16, u16) {
    let pane = app.dock_area().map_or(grid.width(), |pane| pane.x);
    for y in 0..grid.height() {
        let row = grid.row_text(y);
        if let Some(i) = row.find(text) {
            let x = row[..i].chars().count() as u16;
            if x < pane {
                return (x, y);
            }
        }
    }
    panic!("{text:?} not on the page:\n{}", grid.text());
}

/// Drags over the page from where `from` starts to the end of `to`, and comments on it.
fn comment_over(app: &mut App, (from, to): (&str, &str), text: &str) {
    let grid = screen(app);
    let (x, y) = on_page(app, &grid, from);
    let (end, end_y) = on_page(app, &grid, to);
    let end = end + to.chars().count() as u16;
    app.handle(mouse(MouseKind::Left, x, y));
    app.handle(mouse(MouseKind::Drag, end, end_y));
    app.handle(mouse(MouseKind::Up, end, end_y));
    app.run(Command::AddComment);
    typed(app, text);
    enter(app);
}

/// Comments on a word, picked with a double click as a person picks one.
fn comment(app: &mut App, word: &str, text: &str) {
    let grid = screen(app);
    let (x, y) = on_page(app, &grid, word);
    click(app, x + 1, y);
    click(app, x + 1, y);
    app.run(Command::AddComment);
    typed(app, text);
    enter(app);
}

/// An agent changes the file on disk, and Cellix sees it on its next tick.
fn agent_writes(app: &mut App, dir: &TempDir, text: &str) {
    std::thread::sleep(Duration::from_millis(20));
    fs::write(dir.path().join("note.md"), text).unwrap();
    app.handle(Event::Tick);
}

/// An agent suggests a comment on `quote`, of `kind`.
fn suggest(app: &mut App, (line, quote): (usize, &str), kind: &str, text: &str) -> u32 {
    let (made, number) = std::sync::mpsc::channel();
    app.serve(Request::Suggest {
        file: "note.md".into(),
        line,
        kind: kind.into(),
        quote: quote.into(),
        text: text.into(),
        author: "Claude".into(),
        made,
    });
    number.recv().unwrap()
}

/// Opens the Comments pane, or keeps it open: the command shuts it when it's in front.
fn show_comments(app: &mut App) {
    app.dock.open = true;
    app.dock.show_comments();
}

/// The comment cards painted in the side pane: panels with a ground, rounded as a card is,
/// and more than two rows tall. With whether each one's edge is dashed.
fn cards(app: &App, grid: &Grid) -> Vec<(Bounds, Rgb, Option<Rgb>, bool)> {
    let pane = app.dock_area().expect("the side pane");
    grid.paint
        .iter()
        .filter_map(|p| match p {
            Paint::Panel {
                bounds,
                fill: Some(fill),
                border,
                dashed,
                ..
            } if bounds.h > 2 * UNIT && bounds.x >= i32::from(pane.x.saturating_sub(4)) * UNIT => {
                Some((*bounds, *fill, *border, *dashed))
            }
            _ => None,
        })
        .collect()
}

/// The colour the words of the painted run `text` are in.
fn painted_in(grid: &Grid, text: &str) -> Rgb {
    grid.paint
        .iter()
        .find_map(|p| match p {
            Paint::Text {
                text: words, color, ..
            } if words == text => Some(*color),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no words {text:?}"))
}

/// A suggestion's card stands on the ground every card has, with its dashed edge in a quiet
/// grey; the accent's tint and edge are the lit card's alone. A suggestion was a shade of the
/// accent with an accent edge, and beside a lit card the two were told apart by the dashes
/// only (#2a2f38 beside #2d3440 in the tour's 50-flow-next). In both themes.
#[test]
fn only_the_lit_card_has_the_accent_and_a_suggestion_has_the_cards_ground() {
    for light in [false, true] {
        let (_dir, mut app) = page("The cache goes first.\n\nThe tests run last.\n");
        if light {
            app.theme = Theme::named(Some("light"));
        }
        comment(&mut app, "cache", "Why first?");
        suggest(&mut app, (3, "run last"), "Evidence", "Name the tests.");
        show_comments(&mut app);
        // Next Comment lights each in turn: the reviewer's own, not the suggestion.
        let mine = app.threads[0].id;
        for _ in 0..3 {
            if app.lit_thread != Some(mine) {
                app.run(Command::NextComment);
            }
        }
        assert_eq!(app.lit_thread, Some(mine), "the comment's card is lit");
        let grid = painted(&mut app);
        let theme = app.theme;
        let found = cards(&app, &grid);
        let offered: Vec<_> = found.iter().filter(|card| card.3).collect();
        let lit: Vec<_> = found
            .iter()
            .filter(|card| card.2 == Some(theme.accent))
            .collect();
        let panels: Vec<_> = grid
            .paint
            .iter()
            .filter(|p| matches!(p, Paint::Panel { .. }))
            .collect();
        assert_eq!(
            (offered.len(), lit.len()),
            (1, 1),
            "{found:?} {:?} {panels:?}\n{}",
            app.dock_area(),
            grid.text()
        );
        let (_, fill, edge, _) = offered[0];
        assert_eq!(*fill, theme.popup, "a suggestion is on a card's ground");
        let edge = edge.expect("a dashed edge");
        let blue = i16::from(edge.2) - i16::from(edge.0);
        // Grey, as the theme's greys are: the accent's half way to the card was 99 bluer.
        assert!(
            blue.abs() < 30,
            "its edge is grey, not the accent's: {edge:?}"
        );
        assert!(lit[0].1 != theme.popup, "the lit card is tinted");
        assert!(!lit[0].3, "and its edge whole");
    }
}

/// Words a comment was made on that went are in the removed colour, as a diff's removed lines
/// are: "Words gone", and what went in the words shown as they changed. Both were in the
/// colour of what needs you (#f0705f), a colour that means something else. In both themes.
#[test]
fn words_that_went_are_in_the_removed_colour() {
    for light in [false, true] {
        let (dir, mut app) = page("The cache goes first.\n\nThe tests stay fast on every run.\n");
        if light {
            app.theme = Theme::named(Some("light"));
        }
        let removed = app.theme.error;
        assert!(removed != app.theme.attention);
        comment_over(&mut app, ("tests", "every"), "Even on CI?");
        agent_writes(
            &mut app,
            &dir,
            "The cache goes first.\n\nThe tests stay quick on every run.\n",
        );
        let (x, y) = on_page(&app, &screen(&app), "cache");
        click(&mut app, x + 1, y);
        press(&mut app, crate::threads::CHANGED_SINCE);
        let grid = painted(&mut app);
        assert_eq!(painted_in(&grid, "fast"), removed, "what went");
        assert_eq!(painted_in(&grid, "quick"), app.theme.git_added, "what came");
        let struck = grid
            .paint
            .iter()
            .any(|p| matches!(p, Paint::Hairline { color, .. } if *color == removed));
        assert!(struck, "struck through in it");

        agent_writes(
            &mut app,
            &dir,
            "The cache goes first.\n\nNothing about that now.\n",
        );
        let (x, y) = on_page(&app, &screen(&app), "cache");
        click(&mut app, x + 1, y);
        let grid = painted(&mut app);
        let gone = crate::threads::WORDS_GONE;
        assert_eq!(painted_in(&grid, gone), removed, "{}", grid.text());
        let outlined = grid.paint.iter().any(|p| {
            matches!(p, Paint::Panel { border: Some(edge), fill: None, .. } if *edge == removed)
        });
        assert!(outlined, "its pill outlined in it");
    }
}

/// An agent answers a comment with `text`, as `cellix reply` sends it.
fn answer(app: &mut App, id: u32, text: &str) {
    let (found, _) = std::sync::mpsc::channel();
    app.serve(Request::Reply {
        id,
        author: "Claude".into(),
        text: text.into(),
        found,
    });
}

/// A button says what the palette calls the same thing, in its Title Case: a card's Ask Agent,
/// under a hint that names Ask Agent, and the pane's Check My Review, the palette's "Comments:
/// Check My Review". The card said "Ask agent" and the pane "Check my review".
#[test]
fn a_cards_buttons_say_what_the_palette_calls_them() {
    let (_dir, mut app) = page("The cache goes first.\n");
    comment(&mut app, "cache", "Why first?");
    let grid = screen(&app);
    let text = grid.text();
    assert!(text.contains("prompt. Ask Agent"), "the hint: {text}");
    let buttons = (0..grid.height())
        .map(|y| grid.row_text(y))
        .find(|row| row.contains("Resolve"))
        .expect("the card's buttons");
    assert!(buttons.contains(" Ask Agent "), "{buttons}");
    assert!(text.contains("Check My Review"), "{text}");
    assert!(!text.contains("Ask agent") && !text.contains("Check my review"));
    // The palette's command is the button's words.
    app.run(Command::CommandPalette);
    typed(&mut app, "check my");
    let text = screen(&app).text();
    assert!(text.contains("Comments: Check My Review"), "{text}");
}

/// While the pill at a card's top says "Addressed by Claude", the answer under it says who
/// and what with no chip of its own: the card said "Addressed" twice. Its button is Not Done,
/// in the Title Case its neighbours have.
#[test]
fn an_addressed_card_says_so_once() {
    let (_dir, mut app) = page("The cache goes first.\n");
    comment(&mut app, "cache", "Move it later.");
    let id = app.threads[0].id;
    let done = cellix_review::threads::addressed("moved it after the tests");
    answer(&mut app, id, &done);
    show_comments(&mut app);
    let text = screen(&app).text();
    assert_eq!(text.matches("Addressed").count(), 1, "{text}");
    assert!(text.contains("Addressed by Claude"), "{text}");
    assert!(text.contains("Moved it after the tests"), "{text}");
    assert!(text.contains("Not Done"), "{text}");
    // Settled, the pill goes, and the answer's chip is what says it.
    app.thread_hit(crate::threads::Hit::Resolve(id));
    press(&mut app, "Show 1 resolved");
    let text = screen(&app).text();
    assert_eq!(text.matches("Addressed").count(), 1, "{text}");
    assert!(!text.contains("Addressed by"), "{text}");
}

/// The card the pointer brings up over commented words says an agent's verdict as the pane
/// does, its word and then who, and counts only the answers it doesn't show: "1 reply" stood
/// under the one reply it showed.
#[test]
fn the_card_over_the_page_says_a_verdict_as_the_pane_does() {
    let (_dir, mut app) = page("The cache goes first, then the queue.\n");
    comment(&mut app, "cache", "Why first?");
    let id = app.threads[0].id;
    answer(&mut app, id, "Agree (Clarity): the order needs a reason.");
    app.run(Command::CloseDock);
    assert!(!app.dock.open);
    let (x, y) = on_page(&app, &screen(&app), "cache");
    app.handle(mouse(MouseKind::Move, x + 1, y));
    let text = screen(&app).text();
    assert!(text.contains("Agrees Claude: The order"), "{text}");
    assert!(!text.contains("Claude agrees"), "{text}");
    assert!(
        !text.contains("1 reply"),
        "the one reply is in view: {text}"
    );
    assert!(text.contains(super::glance::OPEN), "{text}");
    // In the pane, the same word on its chip, then who.
    show_comments(&mut app);
    let text = screen(&app).text();
    let said = text.lines().find(|row| row.contains("Agrees"));
    let said = said.unwrap_or_else(|| panic!("no verdict:\n{text}"));
    assert!(said.find("Claude") > said.find("Agrees"), "{said}");
}
