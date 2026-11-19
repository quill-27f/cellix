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

/// The tour's terminal: columns of 17 px, rows of 50, words sized for a line of 36.
const CELL: f32 = 17.0;

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

/// Columns in units, as pixels.
fn px(units: i32) -> f32 {
    units as f32 * CELL / UNIT as f32
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

/// Reject and Accept by a suggestion on a page are two buttons over it, edged and filled,
/// with no shadow and nothing between them: their shadows met in the 16 px between them, a
/// band of grey (#b0b0b0 on white), and stood out 12 px either side with a hard top edge.
/// In both themes.
#[test]
fn reject_and_accept_by_a_suggestion_have_no_shadow_between_them() {
    for light in [false, true] {
        let (_dir, mut app) = page("# Notes\n\nKeep {++added++} and end.\n");
        if light {
            app.theme = Theme::named(Some("light"));
        }
        let (x, y) = on_page(&app, &screen(&app), "added");
        click(&mut app, x + 2, y);
        let grid = painted(&mut app);
        let words_at = |name: &str| {
            grid.paint.iter().find_map(|p| match p {
                Paint::Text { x, y, text, .. } if text == name => Some((*x, *y)),
                _ => None,
            })
        };
        let reject = words_at("Reject").expect("Reject");
        let accept = words_at("Accept").expect("Accept");
        assert_eq!(reject.1, accept.1, "side by side");
        // The panel each one's words stand on.
        let under = |(x, y): (i32, i32)| {
            grid.paint
                .iter()
                .find_map(|p| match p {
                    Paint::Panel {
                        bounds,
                        fill: Some(fill),
                        border,
                        shadow,
                        ..
                    } if bounds.x <= x
                        && x < bounds.x + bounds.w
                        && bounds.y <= y + UNIT / 2
                        && y + UNIT / 2 < bounds.y + bounds.h
                        && bounds.h < UNIT =>
                    {
                        Some((*bounds, *fill, *border, *shadow))
                    }
                    _ => None,
                })
                .expect("a chip under its words")
        };
        let (left, quiet, edge, left_shadow) = under(reject);
        let (right, filled, _, right_shadow) = under(accept);
        assert_eq!(filled, app.theme.button, "Accept is the filled one");
        assert_eq!(quiet, app.theme.popup);
        assert_eq!(edge, Some(app.theme.popup_border), "Reject is edged");
        assert!(!left_shadow && !right_shadow, "no shadow under either");
        // Nothing else painted on their row, between them or round them.
        let (top, foot) = (left.y.min(right.y), left.y.max(right.y) + left.h);
        let round = (left.x - UNIT, right.x + right.w + UNIT);
        let others = grid.paint.iter().filter(|p| match p {
            Paint::Panel { bounds, .. } => {
                *bounds != left
                    && *bounds != right
                    && bounds.x < round.1
                    && bounds.x + bounds.w > round.0
                    && bounds.y < foot
                    && bounds.y + bounds.h > top
                    && bounds.h < 2 * UNIT
            }
            _ => false,
        });
        assert_eq!(others.count(), 0, "nothing between or round them");
        let gap = px(right.x - (left.x + left.w));
        assert!(gap >= 7.0, "{gap} px between them");
    }
}

/// A selection's Comment and Ask Agent are edged with no shadow, as Reject and Accept are,
/// the page's ground between them, and stand 8 px or more clear of the selection: their
/// shadows met in the 8 px between them, a band of #dadada on white with a hard edge 7 px
/// under their tops, stood out 10 px left of Comment and ended 1 px over the selection.
#[test]
fn a_selections_buttons_have_no_shadow_between_them() {
    for light in [false, true] {
        let (_dir, mut app) = page("# Notes\n\nThe cache goes first, then the rest of it.\n");
        if light {
            app.theme = Theme::named(Some("light"));
        }
        let grid = screen(&app);
        let (x, y) = on_page(&app, &grid, "cache");
        let (end, _) = on_page(&app, &grid, "first");
        app.handle(mouse(MouseKind::Left, x, y));
        app.handle(mouse(MouseKind::Drag, end + 5, y));
        app.handle(mouse(MouseKind::Up, end + 5, y));
        let grid = painted(&mut app);
        let (comment, ask) = app.selection_labels();
        let chip = |name: &str| {
            let (x, y) = grid
                .paint
                .iter()
                .find_map(|p| match p {
                    Paint::Text { x, y, text, .. } if *text == name => Some((*x, *y)),
                    _ => None,
                })
                .unwrap_or_else(|| panic!("{name} painted"));
            grid.paint
                .iter()
                .find_map(|p| match p {
                    Paint::Panel {
                        bounds,
                        border,
                        shadow,
                        fill: Some(_),
                        ..
                    } if bounds.x <= x
                        && x < bounds.x + bounds.w
                        && bounds.y <= y + UNIT / 2
                        && y + UNIT / 2 < bounds.y + bounds.h
                        && bounds.h < UNIT =>
                    {
                        Some((*bounds, *border, *shadow))
                    }
                    _ => None,
                })
                .unwrap_or_else(|| panic!("a chip under {name}"))
        };
        let (left, left_edge, left_shadow) = chip(&comment);
        let (right, right_edge, right_shadow) = chip(&ask);
        assert!(!left_shadow && !right_shadow, "light {light}: no shadow");
        assert_eq!(left_edge, Some(app.theme.popup_border), "edged");
        assert_eq!(right_edge, Some(app.theme.popup_border), "edged");
        // Nothing painted between or round them but the page.
        let (top, foot) = (left.y.min(right.y), left.y.max(right.y) + left.h);
        let round = (left.x - UNIT, right.x + right.w + UNIT);
        let others = grid.paint.iter().filter(|p| match p {
            Paint::Panel { bounds, shadow, .. } => {
                *bounds != left
                    && *bounds != right
                    && bounds.x < round.1
                    && bounds.x + bounds.w > round.0
                    && bounds.y < foot
                    && bounds.y + bounds.h > top
                    && (*shadow || bounds.h < UNIT)
            }
            _ => false,
        });
        assert_eq!(
            others.count(),
            0,
            "light {light}: nothing between or round them"
        );
    }
}

/// The picked emoji offered for a name stands its ground on the column the box over it keeps,
/// as the kinds' pills and the buttons under it do: it started 9 px left of the box.
#[test]
fn the_picked_emoji_for_a_name_starts_on_the_boxs_column() {
    let (_dir, mut app) = page("The cache goes first.\n");
    let grid = screen(&app);
    let (x, y) = on_page(&app, &grid, "cache");
    click(&mut app, x + 1, y);
    click(&mut app, x + 1, y);
    app.run(Command::AddComment);
    typed(&mut app, "Fast :roc");
    let grid = painted(&mut app);
    let found = |fill: Rgb| {
        grid.paint.iter().find_map(|p| match p {
            Paint::Panel {
                bounds,
                fill: Some(f),
                ..
            } if *f == fill => Some(*bounds),
            _ => None,
        })
    };
    let field = found(app.theme.input).expect("the box");
    let picked = found(app.theme.pick).expect("the picked emoji's ground");
    near(
        px(picked.x),
        px(field.x),
        "the picked emoji's ground on the box's column",
    );
}

/// A card's Edit and Delete, the least of its actions, are in the secondary words' colour in
/// both themes: light, they were #53566f, all but the body text's #4c4f69, where dark's are
/// quiet beside its words.
#[test]
fn a_cards_edit_and_delete_are_quiet_in_both_themes() {
    for light in [false, true] {
        let (_dir, mut app) = page("The cache goes first.\n");
        if light {
            app.theme = Theme::named(Some("light"));
        }
        comment(&mut app, "cache", "Why first?");
        app.lit_thread = None;
        let grid = painted(&mut app);
        let theme = &app.theme;
        for name in ["Edit", "Delete"] {
            let colour = painted_in(&grid, name);
            if light {
                assert_eq!(colour, theme.secondary_words(), "{name}: Latte's subtext0");
            }
            let card = theme.popup;
            let read = crate::theme::contrast(colour, card);
            assert!(read >= 4.5, "light {light}: {name} {read} to 1 on the card");
            let apart = crate::theme::contrast(colour, theme.text.fg);
            assert!(
                apart >= 1.4,
                "light {light}: {name} {apart} to 1 from the words"
            );
        }
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

/// A kind's pill as painted: its kind, its kind's colour and its count's, the pill's fill and
/// edge, and whether a × stands in it.
type Pill = (&'static str, Rgb, Rgb, Option<Rgb>, Option<Rgb>, bool);

/// The words of each kind's pill in the strip over the list, as painted.
fn strip(grid: &Grid) -> Vec<Pill> {
    crate::threads::KINDS
        .iter()
        .map(|kind| {
            let (x, y, named) = grid
                .paint
                .iter()
                .find_map(|p| match p {
                    Paint::Text {
                        x, y, text, color, ..
                    } if text == kind => Some((*x, *y, *color)),
                    _ => None,
                })
                .unwrap_or_else(|| panic!("no pill for {kind}"));
            // The count after it on its row, and the pill under both.
            let counted = grid
                .paint
                .iter()
                .find_map(|p| match p {
                    Paint::Text {
                        x: at,
                        y: row,
                        text,
                        color,
                        ..
                    } if *row == y && *at > x && text.parse::<usize>().is_ok() => Some(*color),
                    _ => None,
                })
                .expect("its count");
            let (bounds, fill, edge) = grid
                .paint
                .iter()
                .find_map(|p| match p {
                    Paint::Panel {
                        bounds,
                        fill,
                        border,
                        ..
                    } if bounds.x < x
                        && x < bounds.x + bounds.w
                        && bounds.y >= y
                        && bounds.y < y + UNIT =>
                    {
                        Some((*bounds, *fill, *border))
                    }
                    _ => None,
                })
                .expect("its pill");
            let cleared = grid.paint.iter().any(|p| {
                matches!(p, Paint::Icon { bounds: b, icon: cellix_render::Icon::Close, .. }
                    if b.x > x && b.x < bounds.x + bounds.w && b.y >= y && b.y < y + UNIT)
            });
            (*kind, named, counted, fill, edge, cleared)
        })
        .collect()
}

/// The kinds' pills are outlines: none is filled until it's picked, where those with comments
/// were filled grey and read as switched on. A kind with comments says its name in the words'
/// colour and one with none quietly, so what isn't covered still stands out; every count is
/// quiet. A pressed one is the accent's with a × in it, and the head says what's listed,
/// "Evidence · 1 of 3", where it said "3 of yours" over the one card. In both themes.
#[test]
fn the_kinds_pills_are_outlines_and_the_head_says_the_kind_listed() {
    for light in [false, true] {
        let (_dir, mut app) = page("The cache goes first.\n\nThe tests run last.\n\nShip it.\n");
        if light {
            app.theme = Theme::named(Some("light"));
        }
        comment(&mut app, "cache", "Why first?");
        comment(&mut app, "tests", "Which ones?");
        comment(&mut app, "Ship", "When?");
        app.threads[0].kind = "Clarity".into();
        app.threads[1].kind = "Evidence".into();
        show_comments(&mut app);
        let theme = app.theme;
        let grid = painted(&mut app);
        for (kind, named, counted, fill, edge, cleared) in strip(&grid) {
            assert_eq!(fill, None, "{kind} isn't filled");
            assert_eq!(edge, Some(theme.popup_border), "{kind} is outlined");
            assert_eq!(counted, theme.gutter.fg, "{kind}'s count is quiet");
            assert!(!cleared, "{kind} has no ×");
            let covered = matches!(kind, "Clarity" | "Evidence");
            let want = if covered {
                theme.text.fg
            } else {
                theme.gutter.fg
            };
            assert_eq!(named, want, "{kind}");
        }
        assert!(grid.text().contains("3 of yours"), "{}", grid.text());

        press(&mut app, "Evidence 1");
        let grid = painted(&mut app);
        let text = grid.text();
        assert!(
            text.contains("Evidence · 1 of 3"),
            "the head says it: {text}"
        );
        assert!(!text.contains("3 of yours"), "{text}");
        for (kind, _, _, fill, edge, cleared) in strip(&grid) {
            let picked = kind == "Evidence";
            assert_eq!(edge == Some(theme.accent), picked, "{kind}");
            assert_eq!(fill.is_some(), picked, "{kind}");
            assert_eq!(cleared, picked, "{kind}'s ×");
        }
        // On cells the × is a letter, and a press on the pill lists them all again.
        let text = screen(&app).text();
        assert!(text.contains("Evidence 1 \u{d7}"), "{text}");
        press(&mut app, "Evidence 1");
        let text = screen(&app).text();
        assert!(
            text.contains("3 of yours") && text.contains("Why first?"),
            "{text}"
        );
    }
}

/// The words in the Comments pane, its rows joined, so what wraps reads as one run.
fn pane_words(app: &App, grid: &Grid) -> String {
    let pane = app.dock_panel().expect("the Comments pane");
    let rows: Vec<String> = (pane.y..pane.bottom())
        .map(|y| grid.row_text(y).chars().skip(usize::from(pane.x)).collect())
        .collect();
    rows.join(" ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// With no comment open and suggestions in the text, the empty list names them and how to go
/// to each: "3 suggestions" stood over "No comments yet." with nothing to say how the two
/// went together.
#[test]
fn an_empty_list_names_the_suggestions_in_the_text() {
    let (_dir, mut app) = page("# Notes\n\nKeep {++added++} and {--gone--} and {~~old~>new~~}.\n");
    show_comments(&mut app);
    let words = pane_words(&app, &screen(&app));
    assert!(words.contains("3 suggestions"), "{words}");
    assert!(
        words.contains("No comments yet. 3 suggestions in the text: Next goes to each."),
        "{words}"
    );
    assert!(words.contains("then press Comment."), "{words}");
}

/// A list that runs past the docked pane has a thumb at its right, in the air beside the
/// cards, as the file has; and a control that would stand past the pane's foot isn't painted
/// or pressed until a scroll brings it up: "Show 1 resolved" stood cut in half at the docked
/// pane's foot, with nothing to say the list went on.
#[test]
fn a_long_list_has_a_thumb_and_nothing_cut_at_the_panes_foot() {
    let words = [
        "alpha", "bravo", "charlie", "delta", "echo", "foxtrot", "golf",
    ];
    let text: String = words.iter().map(|w| format!("The {w} line.\n\n")).collect();
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("note.md"), text).unwrap();
    // About the size Claude Code docks Cellix at, the pane under the file.
    let mut app = App::new(dir.path().to_path_buf(), (64, 40));
    app.open(&dir.path().join("note.md")).unwrap();
    for (line, word) in words.iter().enumerate() {
        app.add_comment("note.md".to_string(), line * 2, format!("About {word}?"));
    }
    let last = app.threads.last().unwrap().id;
    app.thread_hit(crate::threads::Hit::Resolve(last));
    show_comments(&mut app);
    let pane = app.dock_panel().expect("the pane");
    let grid = painted(&mut app);
    let thumb = grid.paint.iter().find_map(|p| match p {
        Paint::Panel {
            bounds,
            fill: Some(fill),
            ..
        } if *fill == app.theme.scrollbar && bounds.y >= i32::from(pane.y) * UNIT => Some(*bounds),
        _ => None,
    });
    let thumb = thumb.expect("a thumb, as the list runs past the pane");
    let card = cards(&app, &grid)[0].0;
    let (thumb_left, card_right) = (px(thumb.x), px(card.x + card.w));
    assert!(
        thumb_left - card_right >= 8.0,
        "clear of the cards: {thumb_left} {card_right}"
    );
    assert!(px(thumb.x + thumb.w) <= f32::from(pane.right()) * CELL - 8.0);
    // Where the button for the resolved one is in the list, as the pixels lay it out: the
    // first scroll that paints it, and the row it's on then.
    airy(&mut app);
    let shown = |grid: &Grid| {
        grid.paint.iter().find_map(|p| match p {
            Paint::Text { y, text, .. } if text.starts_with("Show 1 resolved") => Some(*y),
            _ => None,
        })
    };
    let index = (0..60)
        .find_map(|top| {
            app.dock.tops.set(super::Front::Comments, top);
            let y = shown(&screen(&app))?;
            Some(top + (y / UNIT) as usize - usize::from(pane.y))
        })
        .expect("the button for the resolved one");
    // A scroll that puts it on the row over the pane's last: painted whole there.
    let foot = usize::from(pane.h - 1);
    app.dock.tops.set(super::Front::Comments, index - foot + 1);
    let y = shown(&screen(&app)).expect("painted while it's whole");
    assert_eq!(
        y / UNIT,
        i32::from(pane.bottom() - 2),
        "on the row over the last"
    );
    assert!(
        y + UNIT <= i32::from(pane.bottom()) * UNIT,
        "inside the pane"
    );
    // A row further on it's on the last row: not painted cut in half, nor pressed there.
    app.dock.tops.set(super::Front::Comments, index - foot);
    assert_eq!(shown(&screen(&app)), None, "not painted cut in half");
    click(&mut app, pane.x + 4, pane.bottom() - 1);
    assert!(
        !screen(&app).text().contains("Hide 1 resolved"),
        "nor pressed"
    );
    // Up again, a press on it shows the resolved one.
    app.dock.tops.set(super::Front::Comments, index - foot + 1);
    click(&mut app, pane.x + 4, pane.bottom() - 2);
    cells(&mut app);
    assert!(screen(&app).text().contains("Hide 1 resolved"));
}

/// Rows of the tour's terminal, in pixels, and the line its words are set for.
const ROW: f32 = 50.0;
const LINE: f32 = 36.0;

/// Rows in units, as pixels.
fn py(units: i32) -> f32 {
    units as f32 * ROW / UNIT as f32
}

/// Where the capitals of words painted at row `y` start, and their tails end, in pixels: words
/// stand in the middle of the line, a quarter and 86 hundredths of it down.
fn words_at_row(y: i32) -> (f32, f32) {
    let lead = (ROW - LINE) / 2.0;
    (py(y) + lead + 0.25 * LINE, py(y) + lead + 0.86 * LINE)
}

/// The same for words set small, a note's or an answer's: their tails nearer the row's middle
/// by as much as they're smaller. The pictures had the small words' tails 35.5 px down a row
/// of 50, where the interface's are 38.
fn small_at_row(y: i32) -> (f32, f32) {
    let (caps, tails) = words_at_row(y);
    let middle = py(y) + ROW / 2.0;
    (
        caps,
        middle + (tails - middle) * f32::from(crate::threads::SMALL) / 100.0,
    )
}

/// Where a shape shorter than a row stands, in pixels, as the painter stands it: its top and
/// its foot.
fn stood_at(bounds: Bounds) -> (f32, f32) {
    let (top, tall) = cellix_render::stood(bounds.y, bounds.h, ROW, LINE);
    (top, top + tall)
}

/// The words of the painted run that starts with `start`: where it starts, its row, and how
/// wide it's painted, in pixels.
fn run(grid: &Grid, start: &str) -> (f32, i32, f32) {
    let measure = FontMeasure::new(17, 50).with_type_row(36);
    grid.paint
        .iter()
        .find_map(|p| match p {
            Paint::Text {
                x,
                y,
                text,
                font,
                centred,
                ..
            } if text.starts_with(start) => {
                let wide = cellix_render::Measure::width(&measure, text, *font) * CELL;
                let left = if *centred {
                    px(*x) - wide / 2.0
                } else {
                    px(*x)
                };
                Some((left, *y, wide))
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("no words {start:?}"))
}

/// The card painted round the row `y` (in units), in pixels: left, right, top, foot.
fn card_round(app: &App, grid: &Grid, y: i32) -> (f32, f32, f32, f32) {
    let (bounds, ..) = cards(app, grid)
        .into_iter()
        .find(|(b, ..)| b.y <= y && y < b.y + b.h)
        .expect("a card round it");
    (
        px(bounds.x),
        px(bounds.x + bounds.w),
        py(bounds.y),
        py(bounds.y + bounds.h),
    )
}

/// The quiet chips painted on row `y` (in units) in a card: their left, right, top and foot
/// in pixels, as the painter stands them.
fn chips_on(grid: &Grid, y: i32, fill: Rgb) -> Vec<(f32, f32, f32, f32)> {
    grid.paint
        .iter()
        .filter_map(|p| match p {
            Paint::Panel {
                bounds,
                fill: Some(f),
                ..
            } if *f == fill
                && bounds.h < UNIT
                && bounds.y + bounds.h > y
                && bounds.y < y + UNIT =>
            {
                let (top, foot) = stood_at(*bounds);
                Some((px(bounds.x), px(bounds.x + bounds.w), top, foot))
            }
            _ => None,
        })
        .collect()
}

/// Close enough to `want` px: the painter rounds to whole pixels and a cell's fraction.
fn near(got: f32, want: f32, what: &str) {
    assert!((got - want).abs() <= 1.5, "{what}: {got} px, not {want}");
}

/// Every card keeps `CARD_INSET`, 24 px, on every side: its words start 24 px in, the pill at
/// its header's end and Delete end 24 px from its right edge, its first words' capitals are 24
/// px under its top and its buttons' foot 24 px over its own. Text stood 25 px in, the pill
/// ended 33 px and Delete 51 px from the right, and the foot had 28 px. The composer's box
/// stands on the same columns, where it was 8 px from the card's left and 17 from its right.
#[test]
fn every_card_keeps_one_inset_on_every_side() {
    let inset = crate::measures::CARD_INSET;
    let (_dir, mut app) = page("The cache goes first.\n\nThe tests run last.\n");
    comment(&mut app, "cache", "Why first?");
    let id = app.threads[0].id;
    answer(&mut app, id, &cellix_review::threads::addressed("moved it"));
    comment(&mut app, "tests", "Which ones?");
    show_comments(&mut app);
    let grid = painted(&mut app);
    let quiet = app.theme.secondary(app.theme.popup, false);
    // The addressed card: its pill ends 24 px in.
    let (_, header_y, _) = run(&grid, "You");
    let (left, right, top, foot) = card_round(&app, &grid, header_y);
    let (header_x, ..) = run(&grid, "You");
    near(
        header_x - left,
        inset,
        "the header's words from the card's left",
    );
    near(
        words_at_row(header_y).0 - top,
        inset,
        "its capitals under its top",
    );
    let pill = grid.paint.iter().find_map(|p| match p {
        Paint::Panel {
            bounds,
            border: Some(edge),
            fill: None,
            ..
        } if *edge == app.theme.git_added => Some(px(bounds.x + bounds.w)),
        _ => None,
    });
    near(
        right - pill.expect("the pill"),
        inset,
        "the pill from its right",
    );
    // Its buttons: from the card's left column, their foot 24 px over its own.
    let resolve = chips_on(&grid, run(&grid, "Not Done").1, quiet);
    let first = resolve.iter().map(|c| c.0).fold(f32::MAX, f32::min);
    near(first - left, inset, "the first button from the card's left");
    let lowest = resolve.iter().map(|c| c.3).fold(0.0, f32::max);
    near(foot - lowest, inset, "the card's foot under its buttons");
    // The other card: Delete's words end 24 px from its right.
    let (_, which_y, _) = run(&grid, "Which ones?");
    let (_, right, ..) = card_round(&app, &grid, which_y);
    let (delete_x, _, delete_wide) = run(&grid, "Delete");
    near(
        right - (delete_x + delete_wide),
        inset,
        "Delete from the card's right",
    );

    // The box a comment is written in stands on the card's columns.
    app.thread_hit(crate::threads::Hit::Reply(id));
    typed(&mut app, "Thanks");
    let grid = painted(&mut app);
    let field = grid.paint.iter().find_map(|p| match p {
        Paint::Panel {
            bounds,
            fill: Some(fill),
            ..
        } if *fill == app.theme.input => Some(*bounds),
        _ => None,
    });
    let field = field.expect("the box");
    let (left, right, ..) = card_round(&app, &grid, field.y);
    near(px(field.x) - left, inset, "the box from the card's left");
    near(
        right - px(field.x + field.w),
        inset,
        "the box from the card's right",
    );
}

/// In the docked pane the cards and the list's head start where every view in the pane starts
/// its words: the Comments' content stood at x27 and the Agents list's at x45. A short card's
/// foot is as far under its last words as its top is over its first: 19 px over and 44 under.
#[test]
fn a_docked_cards_inset_is_the_panes_and_its_foot_is_its_top() {
    let inset = crate::measures::CARD_INSET;
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("note.md"), "The cache goes first.\n").unwrap();
    // About the size Claude Code docks Cellix at, the pane under the file.
    let mut app = App::new(dir.path().to_path_buf(), (64, 40));
    app.open(&dir.path().join("note.md")).unwrap();
    comment(&mut app, "cache", "Why first?");
    let id = app.threads[0].id;
    answer(&mut app, id, "Agree: it should be.");
    show_comments(&mut app);
    // Short, as a card is in a pane this small until it's lit.
    app.lit_thread = None;
    let grid = painted(&mut app);
    let (head_x, ..) = run(&grid, "1 of yours");
    let (_, header_y, _) = run(&grid, "You");
    let (left, _, top, foot) = card_round(&app, &grid, header_y);
    near(left, head_x, "the card's left and the head's words");
    near(
        words_at_row(header_y).0 - top,
        inset,
        "the first words' capitals under its top",
    );
    // Its last row says how many answers it has, where its buttons stand once pointed at.
    let (_, note_y, _) = run(&grid, "1 reply");
    near(
        foot - small_at_row(note_y).1,
        inset,
        "its foot under those words' tails",
    );
    // The Agents list starts its words there too.
    app.run(Command::ListAgents);
    let grid = painted(&mut app);
    let (start_x, ..) = run(&grid, "START");
    near(start_x, head_x, "the Agents list's words and the Comments'");
}

/// An answer stands on the card's column, under a line across the card, with no box: its box
/// had its words 10 px from its edge where the card's are 24 from its own, and its verdict's
/// chip 8 px under its top.
#[test]
fn an_answer_stands_on_the_cards_column_under_a_line() {
    let (_dir, mut app) = page("The cache goes first.\n");
    comment(&mut app, "cache", "Why first?");
    let id = app.threads[0].id;
    answer(&mut app, id, "Agree: it should be.");
    show_comments(&mut app);
    let grid = painted(&mut app);
    let (text_x, ..) = run(&grid, "Why first?");
    let (said_x, said_y, _) = run(&grid, "It should be.");
    near(said_x, text_x, "the answer's words on the card's column");
    let (chip_x, ..) = run(&grid, "Agrees");
    assert!(
        chip_x - text_x < 12.0,
        "its chip from the column: {}",
        chip_x - text_x
    );
    let block = app.theme.quiet_button(app.theme.popup);
    let boxed = grid.paint.iter().any(|p| {
        matches!(p, Paint::Panel { fill: Some(f), bounds, .. }
            if *f == block && bounds.h >= UNIT && bounds.y <= said_y && said_y < bounds.y + bounds.h)
    });
    assert!(!boxed, "no box round the answer");
    let ruled = grid.paint.iter().any(|p| {
        matches!(p, Paint::Hairline { bounds, .. }
            if bounds.y < said_y && said_y - bounds.y <= 2 * UNIT && (px(bounds.x) - text_x).abs() < 1.5)
    });
    assert!(ruled, "a line over the answer, from the card's column");
}

/// The line over an agent's answer stands 16 px under the comment's tails and 16 px over the
/// answer's capitals, and over a rewrite the answer's words keep 8 px or more from its box:
/// it stood 12 px under the comment and 10 over "Claude suggests a rewrite" (0.11.0's release
/// review, B5). Measured in the picture, as the tour's terminal draws it.
#[test]
fn the_line_over_an_answer_has_16_px_either_side() {
    use cellix_render::{Rasterizer, SoftwareRasterizer};
    let (_dir, mut app) = page("The model improves on the baseline.\n");
    comment(&mut app, "improves", "By how much?");
    let id = app.threads[0].id;
    answer(&mut app, id, "Rewrite: improves by 3.6 points");
    show_comments(&mut app);
    app.lit_thread = None;
    let grid = painted(&mut app);
    let (text_x, said_y, _) = run(&grid, "By how much?");
    let first = (said_y / UNIT) as u16;
    let mut raster = SoftwareRasterizer::new(17, 50)
        .expect("fonts are in the binary")
        .with_type_row(36);
    let mut rows: Vec<Vec<Rgb>> = Vec::new();
    for y in first..first + 4 {
        let image = raster.row(&grid, y);
        for line in 0..image.height {
            let at = |x: usize| (line * image.width + x) * 3;
            let row = (0..image.width)
                .map(|x| Rgb(image.rgb[at(x)], image.rgb[at(x) + 1], image.rgb[at(x) + 2]))
                .collect();
            rows.push(row);
        }
    }
    let apart =
        |a: Rgb, b: Rgb| a.0.abs_diff(b.0) > 24 || a.1.abs_diff(b.1) > 24 || a.2.abs_diff(b.2) > 24;
    let (card, border) = (app.theme.popup, app.theme.popup_border);
    let (from, to) = (text_x as usize + 2, text_x as usize + 200);
    let inked = |l: usize| rows[l][from..to].iter().any(|p| apart(*p, card));
    // The line is the border's colour itself, which is near the card's in the dark theme.
    let near =
        |a: Rgb, b: Rgb| a.0.abs_diff(b.0) <= 6 && a.1.abs_diff(b.1) <= 6 && a.2.abs_diff(b.2) <= 6;
    let ruled = |l: usize| rows[l][from..to].iter().all(|p| near(*p, border));
    // Under the comment's capitals: the card's own edge over it is a line too.
    let said = (py(said_y) - f32::from(first) * ROW) as usize + 10;
    let line = (said..rows.len())
        .find(|l| ruled(*l))
        .expect("the line over the answer");
    let under = (line..rows.len())
        .find(|l| !ruled(*l))
        .expect("the line's foot");
    let tails = (0..line)
        .rev()
        .find(|l| inked(*l))
        .expect("the comment's tails");
    let caps = (under..rows.len())
        .find(|l| inked(*l))
        .expect("the answer's capitals");
    let (over, below) = (line - tails, caps - under);
    assert!(
        (15..=17).contains(&over) && (15..=17).contains(&below),
        "{over} px under the comment's tails, {below} over the answer's capitals"
    );
    // The rewrite's box: the first row that's mostly its fill.
    let fill = app.theme.quiet_button(card);
    let boxed = (caps..rows.len())
        .find(|l| {
            rows[*l][from..to]
                .iter()
                .filter(|p| near(**p, fill))
                .count()
                > (to - from) / 2
        })
        .expect("the rewrite's box");
    let foot = (caps..boxed)
        .rev()
        .find(|l| inked(*l))
        .expect("the answer's tails");
    assert!(
        boxed - foot >= 8,
        "{} px from the answer to its box",
        boxed - foot
    );
}

/// An agent's rewrite stands in a box 16 px round its words, the box on the card's columns and
/// 16 px over its buttons, which are one row: Accept Rewrite filled, the card's one primary, and
/// two quiet ones. The box's words were 10 px from its edge, it stood 10 px over its buttons,
/// and the two rows of buttons were 23 px apart. Accept Rewrite was quiet for a night, grey
/// beside a blue Check My Review, and read as one choice among equals.
#[test]
fn a_rewrite_stands_in_a_box_with_room_round_it() {
    let padding = crate::measures::BOX_PADDING;
    let inset = crate::measures::CARD_INSET;
    let (_dir, mut app) = page("The model improves on the baseline.\n");
    comment(&mut app, "improves", "By how much?");
    let id = app.threads[0].id;
    answer(&mut app, id, "Rewrite: improves by 3.6 points");
    show_comments(&mut app);
    app.lit_thread = None;
    let grid = painted(&mut app);
    let quiet = app.theme.secondary(app.theme.popup, false);
    let (words_x, words_y, _) = run(&grid, "improves by 3.6 points");
    let (left, right, ..) = card_round(&app, &grid, words_y);
    let block = app.theme.quiet_button(app.theme.popup);
    let found = grid.paint.iter().find_map(|p| match p {
        Paint::Panel {
            bounds,
            fill: Some(f),
            ..
        } if *f == block && bounds.y <= words_y && words_y < bounds.y + bounds.h => Some(*bounds),
        _ => None,
    });
    let found = found.expect("the rewrite's box");
    let (box_left, box_right) = (px(found.x), px(found.x + found.w));
    let (box_top, box_foot) = (py(found.y), py(found.y + found.h));
    near(box_left - left, inset, "the box on the card's left column");
    near(right - box_right, inset, "and its right one");
    near(words_x - box_left, padding, "its words from its edge");
    let (caps, tails) = small_at_row(words_y);
    near(caps - box_top, padding, "its words' capitals under its top");
    near(box_foot - tails, padding, "its foot under their tails");
    let (accept_x, accept_y, _) = run(&grid, "Accept Rewrite");
    let row = chips_on(&grid, accept_y, quiet);
    assert_eq!(row.len(), 2, "two quiet buttons: {row:?}");
    let filled = chips_on(&grid, accept_y, app.theme.button);
    assert_eq!(filled.len(), 1, "one filled: {filled:?}");
    assert!(
        filled[0].0 <= accept_x && accept_x < filled[0].1,
        "Accept Rewrite is the filled one"
    );
    let top = row.iter().map(|c| c.2).fold(filled[0].2, f32::min);
    near(top - box_foot, padding, "the buttons under the box");
    // The only filled button on the card.
    let (_, _, card_top, card_foot) = card_round(&app, &grid, words_y);
    let filled = grid.paint.iter().filter(|p| {
        matches!(p, Paint::Panel { fill: Some(f), bounds, .. }
            if *f == app.theme.button && bounds.h < UNIT && bounds.w > UNIT
                && py(bounds.y) >= card_top && py(bounds.y) < card_foot)
    });
    assert_eq!(filled.count(), 1, "one filled button on the card");
}

/// With a rewrite to decide, one button in view is filled: Accept Rewrite. Check My Review at
/// the head of the list is quiet, as the pane's other buttons are, with its agent's icon kept
/// and in its words' colour: both were blue (0.11.0's release check, P4).
#[test]
fn one_button_in_view_is_filled_with_a_rewrite_to_decide() {
    let (_dir, mut app) = page("The model improves on the baseline.\n");
    comment(&mut app, "improves", "By how much?");
    let id = app.threads[0].id;
    answer(&mut app, id, "Rewrite: improves by 3.6 points");
    show_comments(&mut app);
    app.lit_thread = None;
    let grid = painted(&mut app);
    // Buttons' chips: the accent's lines under the tabs in front are thinner than half a row.
    let chip = |b: &Bounds| b.h > UNIT / 2 && b.h < UNIT && b.w > UNIT;
    let filled: Vec<Bounds> = grid
        .paint
        .iter()
        .filter_map(|p| match p {
            Paint::Panel {
                bounds,
                fill: Some(f),
                ..
            } if *f == app.theme.button && chip(bounds) => Some(*bounds),
            _ => None,
        })
        .collect();
    assert_eq!(filled.len(), 1, "one filled button: {filled:?}");
    let (accept_x, accept_y, _) = run(&grid, "Accept Rewrite");
    let on_it = |b: &Bounds, x: f32, y: i32| {
        px(b.x) <= x && x < px(b.x + b.w) && b.y <= y + UNIT / 2 && y + UNIT / 2 < b.y + b.h
    };
    assert!(
        on_it(&filled[0], accept_x, accept_y),
        "Accept Rewrite is the filled one"
    );
    let (check_x, check_y, _) = run(&grid, "Check My Review");
    let quiet = app.theme.secondary(app.theme.sidebar.bg, false);
    let quiet_chip = grid.paint.iter().any(|p| {
        matches!(p, Paint::Panel { bounds, fill: Some(f), .. }
            if *f == quiet && on_it(bounds, check_x, check_y))
    });
    assert!(quiet_chip, "Check My Review's chip is the quiet fill");
    assert_eq!(painted_in(&grid, "Check My Review"), app.theme.text.fg);
    let icon = grid.paint.iter().find_map(|p| match p {
        Paint::Icon {
            icon: cellix_render::Icon::Agents,
            color,
            bounds,
        } if (bounds.y - check_y).abs() < UNIT => Some(*color),
        _ => None,
    });
    assert_eq!(
        icon,
        Some(app.theme.text.fg),
        "its agent's icon, as its words"
    );
}

/// On a narrow card the rewrite's buttons wrap inside the card, and Edit and Delete stand
/// behind a "⋯" at the end of the comment's buttons, which opens them: Not This ran 13 px
/// past the box and Edit and Delete took a third row.
#[test]
fn a_narrow_cards_buttons_stay_inside_it_with_edit_and_delete_behind_more() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("note.md"),
        "The model improves on the baseline.\n",
    )
    .unwrap();
    let mut app = App::new(dir.path().to_path_buf(), (100, 40));
    app.open(&dir.path().join("note.md")).unwrap();
    comment(&mut app, "improves", "By how much?");
    let id = app.threads[0].id;
    answer(&mut app, id, "Rewrite: improves by 3.6 points");
    show_comments(&mut app);
    app.lit_thread = None;
    let grid = painted(&mut app);
    let (_, words_y, _) = run(&grid, "By how much?");
    let (left, right, ..) = card_round(&app, &grid, words_y);
    let inset = crate::measures::CARD_INSET;
    let quiet = app.theme.secondary(app.theme.popup, false);
    for name in [
        "Accept Rewrite",
        "Apply as Suggestion",
        "Not This",
        "Reply",
        "Resolve",
    ] {
        let (x, y, wide) = run(&grid, name);
        // Accept Rewrite is the filled one.
        let fill = if name == "Accept Rewrite" {
            app.theme.button
        } else {
            quiet
        };
        let chip = chips_on(&grid, y, fill)
            .into_iter()
            .find(|c| c.0 <= x && x + wide <= c.1)
            .unwrap_or_else(|| panic!("{name}'s chip"));
        assert!(
            chip.0 >= left + inset - 1.5,
            "{name} starts inside the card"
        );
        assert!(
            chip.1 <= right - inset + 1.5,
            "{name} ends inside the card: {chip:?}"
        );
    }
    let edit = grid
        .paint
        .iter()
        .any(|p| matches!(p, Paint::Text { text, .. } if text == "Edit"));
    assert!(!edit, "Edit and Delete aren't on a row of their own");
    let more = grid.paint.iter().any(|p| {
        matches!(
            p,
            Paint::Icon {
                icon: cellix_render::Icon::More,
                ..
            }
        )
    });
    assert!(more, "a ⋯ stands for them");
    // On cells it's a button that opens them.
    let (x, y) = find(&screen(&app), "\u{22ef}");
    click(&mut app, x, y);
    let text = screen(&app).text();
    assert!(text.contains("Edit") && text.contains("Delete"), "{text}");
    press(&mut app, "Delete");
    assert!(screen(&app).text().contains("Delete this comment?"));
}
