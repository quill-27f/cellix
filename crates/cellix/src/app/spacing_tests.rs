//! The spacing the reviews of 0.11.0's tour measured, in the pixels the tour's terminal has
//! (17 px columns, rows of 50 with words sized for 36): a task's box square with air before
//! its words, a status message using the room it has, a program in the pane clear of the
//! sheet's edge, bands and cards as far in from both sides of their sheet, nothing within
//! 8 px of a line, and panels as far from their top as from their sides.

use std::fs;
use std::path::PathBuf;
use std::rc::Rc;

use cellix_input::{Event, Mods, MouseEvent, MouseKind};
use cellix_render::{Bounds, FontMeasure, Grid, Paint, UNIT, stood};
use tempfile::TempDir;

use super::*;
use crate::find::{Find, Looked};

/// The tour's terminal: columns of 17 px, rows of 50, words sized for a line of 36.
const CELL: f32 = 17.0;
const ROW: f32 = 50.0;
const LINE: f32 = 36.0;

fn airy(app: &mut App) {
    app.set_measure(Rc::new(FontMeasure::new(17, 50).with_type_row(36)));
}

fn project(size: (u16, u16)) -> (TempDir, App) {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("src")).unwrap();
    fs::write(dir.path().join("src/lib.rs"), "fn main() {}\n").unwrap();
    fs::write(dir.path().join("alpha.txt"), "first line\n").unwrap();
    let mut app = App::new(dir.path().to_path_buf(), size);
    airy(&mut app);
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
        fine_x: f32::from(x),
        fine_y: f32::from(y) + 0.5,
        mods: Mods::default(),
    })
}

/// Columns in units, as pixels.
fn px(units: i32) -> f32 {
    units as f32 * CELL / UNIT as f32
}

/// A shape's top and height in pixels, as the rasterizer stands it on a row of 50.
fn tall(bounds: Bounds) -> (f32, f32) {
    stood(bounds.y, bounds.h, ROW, LINE)
}

/// The pixel a row starts at.
fn row_top(row: u16) -> f32 {
    f32::from(row) * ROW
}

/// Every panel painted, with its fill and border.
fn panels(grid: &Grid) -> Vec<(Bounds, Option<Rgb>, Option<Rgb>, bool)> {
    grid.paint
        .iter()
        .filter_map(|p| match p {
            Paint::Panel {
                bounds,
                fill,
                border,
                inset,
                ..
            } => Some((*bounds, *fill, *border, *inset)),
            _ => None,
        })
        .collect()
}

/// Where the words of a painted run that starts with `start` begin, in pixels.
fn words_at(grid: &Grid, start: &str) -> f32 {
    let x = grid.paint.iter().find_map(|p| match p {
        Paint::Text { x, text, .. } if text.starts_with(start) => Some(*x),
        _ => None,
    });
    px(x.unwrap_or_else(|| panic!("no words {start:?}")))
}

/// A task's box is square on screen, as tall as the line words are sized for allows, and
/// stands at least 9 px before its words, checked or not. It was 33 by 25 with its words
/// touching it.
#[test]
fn a_tasks_box_is_square_with_air_before_its_words() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("note.md");
    fs::write(
        &path,
        "- plain item\n\n- [ ] Check the spend cap\n- [x] Dry-run by default\n",
    )
    .unwrap();
    let mut app = App::new(dir.path().to_path_buf(), (150, 40));
    airy(&mut app);
    app.open(&path).unwrap();
    let grid = screen(&app);
    let theme = app.theme;
    let boxes: Vec<Bounds> = panels(&grid)
        .into_iter()
        .filter(|(_, fill, border, _)| {
            *border == Some(theme.gutter.fg) && fill.is_none()
                || *fill == Some(theme.accent) && *border == Some(theme.accent)
        })
        .map(|(bounds, ..)| bounds)
        .collect();
    assert_eq!(boxes.len(), 2, "a box each");
    for (bounds, words) in boxes.iter().zip(["Check", "Dry"]) {
        let wide = px(bounds.w);
        let (_, high) = tall(*bounds);
        assert!((wide - high).abs() <= 1.0, "square: {wide} by {high}");
        assert!(high >= 18.0, "about a capital's height or more: {high}");
        let air = words_at(&grid, words) - px(bounds.x + bounds.w);
        assert!(air >= 9.0, "{words}: {air} px before its words");
    }
    // The words of every kind of item still line up.
    let bullet = words_at(&grid, "plain");
    assert!((words_at(&grid, "Check") - bullet).abs() < 1.0);
}

/// A status message has the bar up to the details at its other end, as they're painted, and
/// ends in an ellipsis only where even that's too short. With a file maximized, what says so
/// was cut at "to re" with 375 px free.
#[test]
fn a_status_message_uses_the_room_the_details_leave() {
    let (dir, mut app) = project((150, 40));
    app.open(&dir.path().join("alpha.txt")).unwrap();
    app.message.borrow_mut().clear();
    app.maximized = Some(Maximized::Editor);
    let grid = screen(&app);
    let said = grid.paint.iter().find_map(|p| match p {
        Paint::Text { text, .. } if text.starts_with("Sidebar and panes hidden") => {
            Some(text.clone())
        }
        _ => None,
    });
    let said = said.expect("the bar says what's hidden");
    assert!(said.ends_with("to restore"), "{said}");
    // One that can't fit ends in an ellipsis, short of the details.
    app.maximized = None;
    app.say(format!("Couldn't open {}", "a-very-long-name/".repeat(20)));
    let grid = screen(&app);
    let said = grid.paint.iter().find_map(|p| match p {
        Paint::Text { text, x, .. } if text.starts_with("Couldn't open") => {
            Some((text.clone(), *x))
        }
        _ => None,
    });
    let (said, x) = said.expect("the message");
    assert!(said.ends_with('…'), "{said}");
    let font = cellix_render::Font {
        size: 90,
        ..cellix_render::Font::default()
    };
    let end = px(x) + app.measure.width(&said, font) * CELL;
    let (bell, _) = app.status_bell().expect("the bell");
    assert!(
        end <= f32::from(bell) * CELL,
        "{end} px, the bell at {}",
        bell
    );
}

/// With pictures, a program in the pane has a column fewer than the pane, so its last
/// column doesn't stand on the sheet's edge at the window's side; on cells it has them all.
#[test]
fn a_program_in_the_pane_keeps_a_column_of_air_by_the_windows_edge() {
    let (_dir, mut app) = project((150, 40));
    app.run(Command::ListAgents);
    let screen_area = app.dock_screen().unwrap();
    assert_eq!(app.session_size(), Some((screen_area.w - 1, screen_area.h)));
    app.set_measure(Rc::new(CellMeasure));
    let screen_area = app.dock_screen().unwrap();
    assert_eq!(app.session_size(), Some((screen_area.w, screen_area.h)));
}

/// The sheet beside the file starts half way across the pane's first column and ends at
/// the window's side; its line is inside each edge, two pixels thick as the painter draws it
/// here. Returns the gaps between those lines' far sides and a band.
fn gaps(app: &App, band: Bounds) -> (f32, f32) {
    let pane = app.dock_area().unwrap();
    let line = measures::line_pixels(&*app.measure);
    let left_line = (f32::from(pane.x) + SHEET_EDGE) * CELL + line;
    let right_line = f32::from(pane.right()) * CELL - line;
    (px(band.x) - left_line, right_line - px(band.x + band.w))
}

/// A pointed row's band in the pane is 8 px in from the sheet's line on both sides: the
/// Problems list's stood 15 px from it on the left and 6 on the right, the Agents list's 8
/// and 5. A comment's card is as far from the line on its right as on its left.
#[test]
fn bands_and_cards_stand_as_far_in_from_both_sides_of_the_sheet() {
    let (dir, mut app) = project((150, 40));
    let file = dir.path().join("src/lib.rs");
    let problem = Diagnostic {
        line: 0,
        column: 0,
        message: "cannot find value `count`".to_string(),
        severity: 1,
    };
    app.diagnostics.insert(file, vec![problem]);
    app.toggle_problems();
    let grid = screen(&app);
    let y = (0..grid.height())
        .find(|y| grid.row_text(*y).contains("cannot find"))
        .expect("the problem's row");
    app.handle(mouse(MouseKind::Move, app.dock_area().unwrap().x + 6, y));
    let grid = screen(&app);
    let hover = app.theme.row_hover;
    let band = panels(&grid)
        .into_iter()
        .find(|(_, fill, ..)| *fill == Some(hover))
        .expect("a band under the pointer")
        .0;
    let (left, right) = gaps(&app, band);
    assert!(
        (7.5..=8.5).contains(&left),
        "Problems: {left} px on the left"
    );
    assert!(
        (7.5..=8.5).contains(&right),
        "Problems: {right} px on the right"
    );
    // The Agents list's rows.
    app.run(Command::ListAgents);
    let grid = screen(&app);
    let y = (0..grid.height())
        .find(|y| grid.row_text(*y).contains("Terminal"))
        .expect("the Terminal row");
    app.handle(mouse(MouseKind::Move, app.dock_area().unwrap().x + 6, y));
    let grid = screen(&app);
    let band = panels(&grid)
        .into_iter()
        .find(|(_, fill, ..)| *fill == Some(hover))
        .expect("a band under the pointer")
        .0;
    let (left, right) = gaps(&app, band);
    // Its left starts under the frame's half column, which the sheet's edge covers.
    assert!(left <= 9.5, "Agents: {left} px on the left");
    assert!(
        (7.5..=8.5).contains(&right),
        "Agents: {right} px on the right"
    );
}

/// A comment's card stands as far from the sheet's line on its right as on its left. It was
/// 8 px on the left and 17 on the right beside the file.
#[test]
fn a_comments_card_is_as_far_from_both_sides_of_the_sheet() {
    let (dir, mut app) = project((150, 40));
    app.open(&dir.path().join("alpha.txt")).unwrap();
    app.add_comment("alpha.txt".to_string(), 0, "worth a look".to_string());
    app.run(Command::ShowComments);
    let grid = screen(&app);
    let card = app.theme.popup;
    let found = panels(&grid)
        .into_iter()
        .find(|(_, fill, ..)| *fill == Some(card))
        .expect("a card")
        .0;
    let (left, right) = gaps(&app, found);
    assert!(
        (left - right).abs() <= 1.0,
        "{left} px on the left, {right} on the right"
    );
    assert!(left >= 7.0, "{left}");
    // The list's head stands on the cards' edges: the order's chip starts where a card
    // starts, and Check My Review ends where one ends. They stood 8 px inside on the left
    // and 17 on the right.
    app.add_comment("alpha.txt".to_string(), 0, "and this".to_string());
    let grid = screen(&app);
    let found = panels(&grid)
        .into_iter()
        .find(|(_, fill, ..)| *fill == Some(card))
        .expect("a card")
        .0;
    // Both quiet: the order's chip the first on the left, Check My Review the last on the right.
    let quiet = app.theme.quiet_button(app.theme.sidebar.bg);
    let head: Vec<Bounds> = panels(&grid)
        .into_iter()
        .filter(|(b, fill, ..)| *fill == Some(quiet) && b.y < found.y && b.x + UNIT >= found.x)
        .map(|(b, ..)| b)
        .collect();
    let order = *head.iter().min_by_key(|b| b.x).expect("the order's chip");
    assert!(
        (px(order.x) - px(found.x)).abs() <= 1.0,
        "the order at {} px, the card at {}",
        px(order.x),
        px(found.x)
    );
    let check = *head
        .iter()
        .max_by_key(|b| b.x + b.w)
        .expect("Check My Review");
    let (end, card_end) = (px(check.x + check.w), px(found.x + found.w));
    assert!(
        (end - card_end).abs() <= 1.0,
        "it ends at {end}, the card at {card_end}"
    );
}

/// The top of a floating thing under a tab row is at least 8 px from the tab row's line:
/// a tooltip under the pane's ⤢ (4 px before), the menu a tab drops (on the line before,
/// then 7 px from its far side), the menu the pane's + drops (on the line), Show Source
/// (4 px under it), and the Find bar over the pane (5 px over it). The line is two pixels
/// thick here, as the painter draws it on a line of 36, and the air counts from its foot.
#[test]
fn nothing_under_a_tab_row_stands_within_8_px_of_its_line() {
    let (dir, mut app) = project((150, 40));
    app.open(&dir.path().join("alpha.txt")).unwrap();
    app.run(Command::ListAgents);
    assert_eq!(measures::line_pixels(&*app.measure), 2.0);
    let line = row_top(1) + measures::line_pixels(&*app.measure);
    // The tooltip under the button that maximizes the pane.
    let area = app.title_of(app.dock_area().unwrap());
    let button = app.dock.maximize_button(area);
    let on = (button..button + 3).find(|x| app.on_maximize_button(*x, area.y));
    app.handle(mouse(MouseKind::Move, on.expect("the button"), area.y));
    let grid = screen(&app);
    let tip = grid.paint.iter().find_map(|p| match p {
        Paint::Panel {
            bounds,
            shadow: true,
            ..
        } => Some(*bounds),
        _ => None,
    });
    let (top, _) = tall(tip.expect("a tip"));
    assert!(
        top - line >= 8.0,
        "the tip: {} px under the line",
        top - line
    );
    // The menu a tab drops.
    app.handle(mouse(MouseKind::Move, 0, 20));
    let (_, x, _) = app
        .visible_tabs()
        .into_iter()
        .next()
        .expect("the file's tab");
    app.handle(mouse(MouseKind::Right, x + 1, 0));
    assert!(matches!(app.overlay, Overlay::Menu(_)), "a menu");
    let grid = screen(&app);
    let menu = grid.paint.iter().find_map(|p| match p {
        Paint::Panel {
            bounds,
            shadow: true,
            ..
        } => Some(*bounds),
        _ => None,
    });
    let panel = menu.expect("its panel");
    let (top, _) = tall(panel);
    assert!(
        top - line >= 8.0,
        "the menu: {} px under the line",
        top - line
    );
    // And its first words as far under its top as every menu's are.
    let words = grid.paint.iter().find_map(|p| match p {
        Paint::Text { y, text, .. } if text == "Close" => Some(*y),
        _ => None,
    });
    let over = (words.expect("its first row") - panel.y) as f32 / UNIT as f32;
    assert!(
        (over - crate::menu::PADDING).abs() < 0.02,
        "{over} rows over its first row"
    );
    app.overlay = Overlay::None;
    // The menu the pane's + drops.
    let area = app.title_of(app.dock_area().unwrap());
    let plus = app.dock.plus_x(area);
    app.handle(mouse(MouseKind::Left, plus + 1, area.y));
    assert!(matches!(app.overlay, Overlay::Menu(_)), "the + menu");
    let grid = screen(&app);
    let menu = grid.paint.iter().find_map(|p| match p {
        Paint::Panel {
            bounds,
            shadow: true,
            ..
        } => Some(*bounds),
        _ => None,
    });
    let (top, _) = tall(menu.expect("its panel"));
    assert!(
        top - line >= 8.0,
        "the + menu: {} px under the line",
        top - line
    );
    app.overlay = Overlay::None;
    // Find over a list in the pane.
    app.overlay = Overlay::Find(Find {
        listed: true,
        looked: Looked::Pane,
        ..Find::default()
    });
    let bar = app.find_bar();
    assert_eq!(
        bar.y,
        app.dock_screen().unwrap().y + 1,
        "a row under the tab row"
    );
    let grid = screen(&app);
    let raised = grid.paint.iter().find_map(|p| match p {
        Paint::Panel {
            bounds,
            shadow: true,
            ..
        } if bounds.y >= i32::from(bar.y - 1) * UNIT => Some(*bounds),
        _ => None,
    });
    let (top, _) = tall(raised.expect("the bar's panel"));
    assert!(
        top - line >= 8.0,
        "the Find bar: {} px under the line",
        top - line
    );
}

/// Show Source stands on the row under the tab row, as far under the tab row's line as its
/// end is from the line down the sheet's side, more than 8 px: as tall as the row's control
/// it stood 4 px under the line, and then 7 under it against 16 from the side.
#[test]
fn show_source_keeps_clear_of_the_tab_line() {
    let dir = tempfile::tempdir().unwrap();
    let path: PathBuf = dir.path().join("data.csv");
    fs::write(&path, "a,b\n1,2\n").unwrap();
    let mut app = App::new(dir.path().to_path_buf(), (150, 40));
    airy(&mut app);
    app.open(&path).unwrap();
    let Some((cell, _)) = app.page_switch() else {
        panic!("a page with a switch");
    };
    let grid = screen(&app);
    let chip = panels(&grid)
        .into_iter()
        .find(|(bounds, fill, ..)| {
            fill.is_some()
                && bounds.y < 2 * UNIT
                && bounds.y >= UNIT
                && bounds.x >= i32::from(cell.x) * UNIT
                && bounds.x < i32::from(cell.right()) * UNIT
        })
        .expect("its chip");
    assert!(chip.3, "a button inside its row");
    let (top, _) = cellix_render::inset(chip.0.y, chip.0.h, ROW, LINE);
    let line = measures::line_pixels(&*app.measure);
    let under = top - (row_top(1) + line);
    let side = f32::from(app.main_area().right()) * CELL - line - px(chip.0.x + chip.0.w);
    assert!(under >= 8.0, "{under} px under the line");
    assert!(
        (under - side).abs() <= 2.0,
        "{under} px under the line, {side} from the side"
    );
}

/// The scrollbar beside Show Source starts 8 px under the button's foot, its thumb at the top
/// of the page as well as further down: at the top it stood 2 px under the button and 5 px
/// right of it (0.11.0's release check, P3, 65 and 66).
#[test]
fn the_scrollbar_starts_clear_of_show_source() {
    let dir = tempfile::tempdir().unwrap();
    let path: PathBuf = dir.path().join("data.csv");
    let rows: String = (0..200).map(|n| format!("{n},{}\n", n * 2)).collect();
    fs::write(&path, format!("a,b\n{rows}")).unwrap();
    let mut app = App::new(dir.path().to_path_buf(), (150, 40));
    airy(&mut app);
    // With a minimap there's no scrollbar.
    app.settings.minimap = Some(false);
    app.open(&path).unwrap();
    let Some((cell, _)) = app.page_switch() else {
        panic!("a page with a switch");
    };
    let thumb_top = |app: &App| {
        let grid = screen(app);
        let scrollbar = app.theme.scrollbar;
        let thumb = panels(&grid)
            .into_iter()
            .find(|(_, fill, ..)| *fill == Some(scrollbar))
            .expect("a thumb: the page is longer than the pane");
        let chip = panels(&grid)
            .into_iter()
            .find(|(bounds, fill, _, inset)| {
                fill.is_some()
                    && *inset
                    && bounds.x >= i32::from(cell.x) * UNIT
                    && bounds.x < i32::from(cell.right()) * UNIT
            })
            .expect("Show Source's chip");
        let (top, height) = cellix_render::inset(chip.0.y, chip.0.h, ROW, LINE);
        (
            tall(thumb.0).0,
            top + height,
            px(thumb.0.x),
            px(chip.0.x + chip.0.w),
        )
    };
    let (thumb, foot, x, end) = thumb_top(&app);
    assert!(
        x > end,
        "the scrollbar is right of the button: {x} and {end}"
    );
    assert!(
        thumb - foot >= 8.0 - 0.5,
        "at the top, {} px under the button",
        thumb - foot
    );
    // Scrolled a little, the thumb is still on its track, under the button.
    for _ in 0..2 {
        app.handle(mouse(MouseKind::ScrollDown, 40, 20));
    }
    let (moved, foot, ..) = thumb_top(&app);
    assert!(moved >= thumb, "the thumb went down: {moved} from {thumb}");
    assert!(
        moved - foot >= 8.0 - 0.5,
        "{} px under the button",
        moved - foot
    );
}

/// Find's marks in a list are put in under the letters they mark, which moves along what's
/// painted after them; the status bar's paint, which the bar at the foot moves with its row
/// later, is still the status bar's alone. Its range had slipped a step back, and the pane's
/// foot line went down 20 px with the bar.
#[test]
fn finds_marks_in_a_list_leave_the_status_bars_paint_its_own() {
    let (dir, mut app) = project((150, 40));
    app.show_sidebar = false;
    app.open(&dir.path().join("alpha.txt")).unwrap();
    for n in 1..=3 {
        let text = format!("note {n} about needles");
        app.add_comment("alpha.txt".to_string(), 0, text);
    }
    app.run(Command::ShowComments);
    let panel = app.dock_panel().unwrap();
    app.handle(mouse(MouseKind::Left, panel.x + 3, panel.y));
    app.handle(mouse(MouseKind::Up, panel.x + 3, panel.y));
    app.handle(Event::Key(cellix_input::KeyEvent::ctrl('f')));
    assert!(matches!(&app.overlay, Overlay::Find(find) if find.listed));
    for c in "needles".chars() {
        app.handle(Event::Key(cellix_input::KeyEvent::plain(Key::Char(c))));
    }
    let grid = screen(&app);
    let marks = grid
        .paint
        .iter()
        .filter(|p| matches!(p, Paint::Panel { radius: 8, .. }))
        .count();
    assert!(marks >= 3, "marks behind the letters: {marks}");
    let (from, to) = app.status_paint.get();
    assert!(from < to, "the status bar painted");
    let bar = i32::from(app.size().1 - 1) * UNIT;
    for paint in &grid.paint[from..to] {
        let y = match paint {
            Paint::Text { y, .. } => *y,
            Paint::Panel { bounds, .. }
            | Paint::Hairline { bounds, .. }
            | Paint::Icon { bounds, .. }
            | Paint::Spinner { bounds, .. } => bounds.y + bounds.h / 2,
            _ => bar,
        };
        assert!(
            y >= bar - UNIT / 2,
            "{paint:?} isn't the status bar's, on row {}",
            y / UNIT
        );
    }
}

/// The raised panel painted last: a floating bar's or a panel's, as bounds.
fn last_raised(grid: &Grid) -> Bounds {
    let found = grid.paint.iter().rev().find_map(|p| match p {
        Paint::Panel {
            bounds,
            shadow: true,
            ..
        } => Some(*bounds),
        _ => None,
    });
    found.expect("a raised panel")
}

/// Where the painted words starting with `start` stand: their row's top in pixels.
fn words_top(grid: &Grid, start: &str) -> f32 {
    let y = grid.paint.iter().find_map(|p| match p {
        Paint::Text { y, text, .. } if text.starts_with(start) => Some(*y),
        _ => None,
    });
    y.unwrap_or_else(|| panic!("no words {start:?}")) as f32 * ROW / UNIT as f32
}

/// Boxes hold what they hold in the middle: the New file prompt has as much over its
/// question's capitals as under its box (19 px and 10), the symbols panel as much under its
/// last line as over its buttons (13 and 20), and Find over a list in the pane starts its
/// box where the chevron would be and ends where the list's words do (a 54 px empty slot,
/// and 17 px past them).
#[test]
fn boxes_hold_what_they_hold_in_their_middle() {
    let (dir, mut app) = project((150, 40));
    app.open(&dir.path().join("alpha.txt")).unwrap();
    let (caps, tails) = measures::words_in_row(&*app.measure).unwrap();
    // The New file prompt.
    app.menu_action(MenuAction::NewFile(dir.path().to_path_buf()));
    assert!(matches!(app.overlay, Overlay::Prompt(_)));
    let grid = screen(&app);
    let bar = last_raised(&grid);
    let (top, high) = tall(bar);
    let over = words_top(&grid, "New file") + caps - top;
    let field = panels(&grid)
        .into_iter()
        .filter(|(b, fill, ..)| *fill == Some(app.theme.input) && b.y > bar.y)
        .map(|(b, ..)| b)
        .next()
        .expect("its box");
    let (field_top, field_high) = tall(field);
    let under = top + high - (field_top + field_high);
    assert!(
        (over - under).abs() <= 2.0,
        "{over} px over the question, {under} under the box"
    );
    // Its sides off the file's sheet's line.
    let line = f32::from(app.main_area().x) * CELL;
    let left = px(bar.x);
    assert!(
        (left - line).abs() >= measures::LINE_AIR,
        "{left} by {line}"
    );
    app.overlay = Overlay::None;
    // The symbols panel.
    let panel = crate::panels::MathPanel::new((30, 4), &*app.measure);
    app.overlay = Overlay::Math(panel);
    let grid = screen(&app);
    let raised = last_raised(&grid);
    let (top, high) = tall(raised);
    // Its last line: the symbol's name and source, the lowest words painted on it.
    let last = grid
        .paint
        .iter()
        .filter_map(|p| match p {
            Paint::Text { y, x, .. }
                if *y >= raised.y
                    && *y < raised.y + raised.h
                    && *x >= raised.x
                    && *x < raised.x + raised.w =>
            {
                Some(*y)
            }
            _ => None,
        })
        .max()
        .expect("its words");
    let under = top + high - (last as f32 * ROW / UNIT as f32 + tails);
    let side = 1.2 * CELL;
    assert!(
        (under - side).abs() <= 2.0,
        "{under} px under its last line, {side} round its buttons"
    );
    app.overlay = Overlay::None;
}

/// Find over a list in the pane has no chevron, and its box starts where the chevron would
/// be; the bar ends where the list's words do, two columns from the pane's side.
#[test]
fn find_over_a_list_starts_its_box_at_its_edge() {
    let (dir, mut app) = project((150, 40));
    app.open(&dir.path().join("alpha.txt")).unwrap();
    app.add_comment("alpha.txt".to_string(), 0, "needles".to_string());
    app.run(Command::ShowComments);
    app.overlay = Overlay::Find(Find {
        listed: true,
        looked: Looked::Pane,
        ..Find::default()
    });
    let bar = app.find_bar();
    let screen_area = app.dock_screen().unwrap();
    assert_eq!(bar.right() + 2, screen_area.right(), "two columns in");
    let grid = screen(&app);
    let field = panels(&grid)
        .into_iter()
        .filter(|(b, fill, ..)| *fill == Some(app.theme.input) && b.y >= i32::from(bar.y) * UNIT)
        .map(|(b, ..)| b)
        .next()
        .expect("its box");
    // A column in, and the box's own line inside its cells, where the chevron's room put it
    // three and a half columns in.
    assert!(
        px(field.x) - f32::from(bar.x) * CELL <= CELL + 4.0,
        "the box at {} px, the bar at {}",
        px(field.x),
        f32::from(bar.x) * CELL
    );
}

/// The rows `rows` of what's drawn, as the tour's terminal shows them: each pixel row's
/// colours, from the window's left edge.
fn picture(grid: &Grid, rows: std::ops::Range<u16>) -> Vec<Vec<Rgb>> {
    use cellix_render::{Rasterizer, SoftwareRasterizer};
    let mut rasterizer = SoftwareRasterizer::new(17, 50)
        .expect("fonts are in the binary")
        .with_type_row(36);
    let mut out = Vec::new();
    for y in rows {
        let image = rasterizer.row(grid, y);
        for line in 0..image.height {
            let pixels = (0..image.width)
                .map(|x| {
                    let at = (line * image.width + x) * 3;
                    Rgb(image.rgb[at], image.rgb[at + 1], image.rgb[at + 2])
                })
                .collect();
            out.push(pixels);
        }
    }
    out
}

/// Whether two colours are apart by more than a hair.
fn apart(a: Rgb, b: Rgb) -> bool {
    let d = |p: u8, q: u8| p.abs_diff(q) > 24;
    d(a.0, b.0) || d(a.1, b.1) || d(a.2, b.2)
}

/// A card's air in pixels, measured in its picture: over its title's capitals, under its
/// buttons, and between what's last over its buttons and them. `area` is its cells, `main`
/// its main button's colour.
fn card_air(app: &App, area: Rect, main: Rgb) -> (usize, usize, usize) {
    let grid = screen(app);
    let rows = picture(&grid, area.y..area.bottom());
    let popup = app.theme.popup;
    // Down its right side, past its words: the panel's own colour from its top to its foot.
    let side = usize::from(area.right()) * 17 - 12;
    let inside: Vec<usize> = (0..rows.len())
        .filter(|y| rows[*y][side] == popup)
        .collect();
    let (top, foot) = (inside[0], inside[inside.len() - 1]);
    let (from, to) = (
        usize::from(area.x) * 17 + 24,
        usize::from(area.right()) * 17 - 24,
    );
    let inked = |y: usize| rows[y][from..to].iter().any(|p| apart(*p, popup));
    let caps = (top..foot).find(|y| inked(*y)).expect("its title");
    // Its buttons' rows: the main one's colour or a quiet one's, whole. The blended row at
    // their edge isn't counted as words.
    let quiet = app.theme.quiet_button(popup);
    let button_rows: Vec<usize> = (top..foot)
        .filter(|y| {
            let row = &rows[*y][from..to];
            row.iter().filter(|p| **p == main || **p == quiet).count() >= 20
        })
        .collect();
    let (button, button_foot) = (button_rows[0], button_rows[button_rows.len() - 1]);
    let over = (caps..button - 1)
        .rev()
        .find(|y| inked(*y))
        .expect("its words");
    (caps - top, foot - button_foot, button - over - 1)
}

/// A card has a dialog's air round it: as much over its title's capitals and under its
/// buttons as at its sides, 24 to 32 px, and 24 px between its words, or the box of a
/// command, and its buttons. The install card had 21 over and under and 17 over its
/// buttons, the logout card 11 under its command's box, against 34 px at their sides.
#[test]
fn a_card_has_a_dialogs_air_round_it() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("Main.java"), "class Main {}\n").unwrap();
    let mut app = App::new(dir.path().to_path_buf(), (120, 30));
    airy(&mut app);
    // Its last line says what Install runs. The air is read off the ink, so the line has a
    // letter with a tail, as the line before it had: "Runs exit 0" ends at its baseline, and
    // read 28 px from it.
    app.server_install = Some("sleep 0".to_string());
    app.open(&dir.path().join("Main.java")).unwrap();
    app.servers.clear();
    app.note_missing("Java");
    let button = app.theme.button;
    let (over, under, gap) = card_air(&app, app.offer_area(), button);
    assert!((24..=34).contains(&over), "{over} px over its title");
    assert!(over.abs_diff(under) <= 3, "{over} px over, {under} under");
    assert!((22..=27).contains(&gap), "{gap} px over its buttons");
    // The logout card, whose command's box is last over its buttons.
    app.not_now.push("Java".to_string());
    app.linger = Linger::Offered;
    let (over, under, gap) = card_air(&app, app.linger_area(), button);
    assert!((24..=34).contains(&over), "{over} px over its title");
    assert!(over.abs_diff(under) <= 3, "{over} px over, {under} under");
    assert!(
        (22..=27).contains(&gap),
        "{gap} px from its box to its buttons"
    );
}

/// A card over the side pane stands 16 px from the pane's lines on both sides with its words
/// 24 px in, its buttons ending inside its words' column, and a command's box 12 px outside
/// that column with the command on it, as far in from the card's edge as the command is from
/// its own. The install, logout and stopped-session cards stood 24 px from the pane's left line
/// and 33 from its right with their words 33 px in; the logout card's box had its edge on its
/// words' column, 12 px from its own words, and then 16 px outside it, 8 px inside the card's
/// edge (0.11.0's release check, P5).
#[test]
fn a_card_over_the_pane_stands_16_px_from_its_sides_with_its_words_24_in() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("Main.java"), "class Main {}\n").unwrap();
    let mut app = App::new(dir.path().to_path_buf(), (150, 40));
    airy(&mut app);
    app.server_install = Some("exit 0".to_string());
    app.open(&dir.path().join("Main.java")).unwrap();
    app.run(Command::ListAgents);
    app.servers.clear();
    app.note_missing("Java");
    let buttons = [app.theme.button, app.theme.quiet_button(app.theme.popup)];
    // The card under `title`: its panel, and where its words start, in pixels.
    let card = |app: &App, grid: &Grid, title: &str| {
        let words = words_at(grid, title);
        let found = grid.paint.iter().rev().find_map(|p| match p {
            Paint::Panel {
                bounds,
                shadow: true,
                ..
            } if px(bounds.x) <= words && words < px(bounds.x + bounds.w) && bounds.h > UNIT => {
                Some(*bounds)
            }
            _ => None,
        });
        let found = found.unwrap_or_else(|| panic!("{title}'s card"));
        let (left, right) = gaps(app, found);
        assert!(
            (14.5..=17.5).contains(&left),
            "{title}: {left} px from the pane's left"
        );
        assert!(
            (14.5..=17.5).contains(&right),
            "{title}: {right} px from its right"
        );
        let inside = words - px(found.x);
        assert!(
            (22.5..=26.5).contains(&inside),
            "{title}: its words {inside} px in"
        );
        // Its buttons end at least as far in from its right.
        for (bounds, fill, ..) in panels(grid) {
            let within = bounds.x >= found.x && bounds.x + bounds.w <= found.x + found.w;
            if within && bounds.h < UNIT && fill.is_some_and(|f| buttons.contains(&f)) {
                let end = px(found.x + found.w) - px(bounds.x + bounds.w);
                assert!(
                    end >= inside - 1.5,
                    "{title}: a button ends {end} px from its right"
                );
            }
        }
        (found, words)
    };
    let grid = screen(&app);
    card(&app, &grid, "Install Java");
    // The logout card, whose command stands in a box.
    app.not_now.push("Java".to_string());
    app.linger = Linger::Offered;
    let grid = screen(&app);
    let (_, words) = card(&app, &grid, "Agents here stop");
    let well = panels(&grid)
        .into_iter()
        .find(|(_, fill, ..)| *fill == Some(app.theme.input))
        .expect("the command's box")
        .0;
    let out = words - px(well.x);
    assert!(
        (10.5..=13.5).contains(&out),
        "the box {out} px outside the words"
    );
    let (card, _) = card(&app, &grid, "Agents here stop");
    let (inside, inside_right) = (
        px(well.x) - px(card.x),
        px(card.x + card.w) - px(well.x + well.w),
    );
    assert!(
        (inside - out).abs() <= 2.0 && (inside_right - out).abs() <= 2.0,
        "the box {inside} and {inside_right} px inside the card, {out} outside the words"
    );
    let command = words_at(&grid, "loginctl");
    assert!(
        (command - words).abs() <= 1.5,
        "the command on the words' column"
    );
}

/// A card in a sheet's corner stands as far over the line at its sheet's foot as its side
/// stands from the sheet's side: 16 px over the side pane, where the install and logout cards
/// floated 79 px over the pane's foot (0.11.0's release check, P6), and over the file as far as
/// from the file's side. Its buttons go down with it, and a press on the foot of one, painted
/// under its row, takes it.
#[test]
fn a_corner_card_stands_as_far_over_its_sheets_foot_as_from_its_side() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("Main.java"), "class Main {}\n").unwrap();
    let mut app = App::new(dir.path().to_path_buf(), (150, 40));
    airy(&mut app);
    app.server_install = Some("exit 0".to_string());
    app.open(&dir.path().join("Main.java")).unwrap();
    app.servers.clear();
    app.note_missing("Java");
    let line = measures::line_pixels(&*app.measure);
    // The raised panel holding `title`: how far its foot is over the line at the foot of
    // `sheet`, and its side from the line at the sheet's side.
    let air = |app: &App, title: &str, sheet: Rect| {
        let grid = screen(app);
        let words = words_at(&grid, title);
        let found = grid.paint.iter().rev().find_map(|p| match p {
            Paint::Panel {
                bounds,
                shadow: true,
                ..
            } if px(bounds.x) <= words && words < px(bounds.x + bounds.w) && bounds.h > UNIT => {
                Some(*bounds)
            }
            _ => None,
        });
        let card = found.unwrap_or_else(|| panic!("{title}'s card"));
        let foot = (card.y + card.h) as f32 * ROW / UNIT as f32;
        let under = f32::from(sheet.bottom()) * ROW - line - foot;
        let side = f32::from(sheet.right()) * CELL - line - px(card.x + card.w);
        (under, side)
    };
    let (under, side) = air(&app, "Install Java", app.main_area());
    assert!(
        side > 8.0 && (under - side).abs() <= 1.5,
        "over the file: {under} px over its foot, {side} from its side"
    );
    app.run(Command::ListAgents);
    let pane = app.dock_area().expect("the side pane");
    let (under, side) = air(&app, "Install Java", pane);
    assert!(
        (14.5..=17.5).contains(&under) && (under - side).abs() <= 1.5,
        "over the pane: {under} px over its foot, {side} from its side"
    );
    // Not Now's foot is painted a tenth of a row into the row under its cells.
    let row = app.offer_button_row();
    let (_, cells, x) = app
        .offer_buttons()
        .into_iter()
        .find(|(label, ..)| *label == "Not Now")
        .expect("Not Now");
    let x = x + cells.chars().count() as u16 / 2;
    let press = |kind| {
        Event::Mouse(MouseEvent {
            kind,
            x,
            y: row + 1,
            fine_x: f32::from(x) + 0.5,
            fine_y: f32::from(row) + 1.1,
            mods: Mods::default(),
        })
    };
    app.handle(press(MouseKind::Left));
    app.handle(press(MouseKind::Up));
    assert!(
        app.not_now.iter().any(|l| l == "Java"),
        "the press took Not Now"
    );
    // The logout card, over the pane too.
    app.linger = Linger::Offered;
    let (under, side) = air(&app, "Agents here stop", pane);
    assert!(
        (14.5..=17.5).contains(&under) && (under - side).abs() <= 1.5,
        "the logout card: {under} px over the pane's foot, {side} from its side"
    );
}

/// The table picker's squares stand as far from its top as from its sides: on its first row
/// they were 7 px from its top and 20 from its sides.
#[test]
fn a_panels_grid_is_as_far_from_its_top_as_its_sides() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("note.md");
    fs::write(&path, "# Title\n\nSome words.\n").unwrap();
    let mut app = App::new(dir.path().to_path_buf(), (150, 40));
    airy(&mut app);
    app.open(&path).unwrap();
    let picker = TablePicker::new((40, 3), &*app.measure);
    let area = picker.area();
    app.overlay = Overlay::TablePicker(picker);
    let grid = screen(&app);
    let squares: Vec<Bounds> = panels(&grid)
        .into_iter()
        .filter(|(bounds, _, border, _)| {
            border.is_some() && bounds.y >= i32::from(area.y) * UNIT && bounds.h < UNIT
        })
        .map(|(bounds, ..)| bounds)
        .collect();
    let first = squares.first().expect("the squares");
    let side = px(first.x) - f32::from(area.x) * CELL;
    let (top, _) = tall(*first);
    let above = top - row_top(area.y);
    assert!(
        (above - side).abs() <= 3.0,
        "{above} px over, {side} beside"
    );
    // A press on the first square still picks one by one.
    app.handle(mouse(MouseKind::Move, area.x + 2, area.y));
    let Overlay::TablePicker(picker) = &app.overlay else {
        panic!("still open");
    };
    let fine = (top + 4.0) / ROW;
    assert_eq!(picker.size_at(area.x + 2, fine), Some((1, 1)));
}

/// The table picker's label stands on the squares' left edge, its capitals 24 px under them
/// and its words 22 px over the panel's foot: it stood 15 px right of their edge, 59 px under
/// them and 12 over the foot (0.11.0's release review, B7, 48-table-picker).
#[test]
fn the_table_pickers_label_stands_under_its_squares() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("note.md");
    fs::write(&path, "# Title\n\nSome words.\n").unwrap();
    let mut app = App::new(dir.path().to_path_buf(), (150, 40));
    airy(&mut app);
    app.open(&path).unwrap();
    let picker = TablePicker::new((40, 3), &*app.measure);
    let area = picker.area();
    app.overlay = Overlay::TablePicker(picker);
    let grid = screen(&app);
    let theme = app.theme;
    let squares: Vec<Bounds> = panels(&grid)
        .into_iter()
        .filter(|(bounds, _, border, _)| {
            border.is_some() && bounds.y >= i32::from(area.y) * UNIT && bounds.h < UNIT
        })
        .map(|(bounds, ..)| bounds)
        .collect();
    let left = px(squares.first().expect("the squares").x);
    let (top, high) = tall(*squares.last().unwrap());
    let under_squares = top + high;
    let panel = grid
        .paint
        .iter()
        .find_map(|p| match p {
            Paint::Panel {
                bounds,
                shadow: true,
                fill: Some(fill),
                ..
            } if *fill == theme.popup => Some(*bounds),
            _ => None,
        })
        .expect("the picker's panel");
    let (panel_top, panel_tall) = tall(panel);
    let foot = panel_top + panel_tall;
    // The label's ink, under the squares and inside the panel.
    let rows = picture(&grid, area.y..area.bottom());
    let (from, to) = (left as usize - 10, left as usize + 260);
    let inked: Vec<(f32, usize)> = (rows.iter().enumerate())
        .map(|(line, pixels)| (row_top(area.y) + line as f32, pixels))
        .filter(|(at, _)| *at > under_squares + 2.0 && *at < foot - 2.0)
        .filter_map(|(at, pixels)| {
            let first = (from..to).find(|x| apart(pixels[*x], theme.popup))?;
            Some((at, first))
        })
        .collect();
    let (ink_top, _) = *inked.first().expect("the label's words");
    let (ink_foot, _) = *inked.last().unwrap();
    let ink_left = inked.iter().map(|(_, x)| *x).min().unwrap() as f32;
    let over = ink_top - under_squares;
    let under = foot - (ink_foot + 1.0);
    assert!((over - 24.0).abs() <= 1.5, "{over} px under the squares");
    assert!((under - 22.0).abs() <= 2.0, "{under} px over the foot");
    assert!(
        (ink_left - left).abs() <= 3.0,
        "the words at {ink_left} px, the squares' edge at {left}"
    );
}

/// The tip on screen, the last raised panel painted, as its left and right edges and its top
/// and foot in pixels.
fn tip_pixels(grid: &Grid) -> (f32, f32, f32, f32) {
    let tip = grid.paint.iter().rev().find_map(|p| match p {
        Paint::Panel {
            bounds,
            shadow: true,
            ..
        } => Some(*bounds),
        _ => None,
    });
    let tip = tip.expect("a tip");
    let (top, high) = tall(tip);
    (px(tip.x), px(tip.x + tip.w), top, top + high)
}

/// Quit's tip, over the bar at the foot, ends 8 px or more from the line down the window's
/// side, as every tip ends at it: it stood 7 px from the pane's.
#[test]
fn a_tip_at_the_windows_side_keeps_clear_of_the_sheets_line() {
    let (_dir, mut app) = project((150, 40));
    for pane in [false, true] {
        if pane {
            app.run(Command::ListAgents);
        }
        let quit = app.quit_button().expect("Quit");
        app.handle(mouse(MouseKind::Move, quit.x, quit.y));
        let grid = screen(&app);
        let tip = tip_pixels(&grid);
        let line = f32::from(app.size().0) * CELL - measures::line_pixels(&*app.measure);
        assert!(
            line - tip.1 >= measures::LINE_AIR - 0.5,
            "pane {pane}: {} px from the side's line",
            line - tip.1
        );
        app.handle(mouse(MouseKind::Move, 1, 20));
    }
}

/// Every tip under a bar stands 10 px under the bar's line, under its control: a view's and
/// the sidebar button's under the views, a pane view's under the pane's strip, and the
/// sidebar button's under the tab row. It's a layer, and covers the top of what's under it,
/// the branch's chip or a minimap's first lines: moved past them, the rail's tip and Hide
/// Sidebar's stood 85 px under their icons and cut a folder's name in half, and Browser's
/// 60 px under its line, still on the minimap.
#[test]
fn every_tip_under_a_bar_stands_10_px_under_its_line_under_its_control() {
    let (dir, mut app) = project((150, 40));
    app.branch = Some("main".to_string());
    let long: String = (0..300).map(|i| format!("let line_{i} = {i};\n")).collect();
    let path = dir.path().join("long.rs");
    fs::write(&path, long).unwrap();
    app.open(&path).unwrap();
    let line = measures::line_pixels(&*app.measure);
    // Pointed at `(x, y)` on a bar's row, on a control from column `from` to `to`.
    let check = |app: &mut App, (x, y): (u16, u16), (from, to): (u16, u16), what: &str| {
        app.handle(mouse(MouseKind::Move, x, y));
        let grid = screen(app);
        let tip = tip_pixels(&grid);
        let under = tip.2 - (row_top(y + 1) + line);
        assert!(
            (8.5..=11.0).contains(&under),
            "{what}: {under} px under its line"
        );
        let (from, to) = (f32::from(from) * CELL, f32::from(to) * CELL);
        assert!(
            tip.0 < to && tip.1 > from,
            "{what}: the tip {tip:?} isn't under its control, {from} to {to}"
        );
        app.handle(mouse(MouseKind::Move, 60, 20));
    };
    let rail = app.activity_area();
    assert!(app.views_on_top(), "the views across the sidebar's top");
    for (view, name) in RAIL_NAMES.iter().enumerate() {
        let x = app.view_x(view, rail);
        check(&mut app, (x + 1, rail.y), (x, x + VIEW_STEP), name);
    }
    let hide = app.sidebar_hide().expect("Hide Sidebar");
    check(&mut app, (hide.x, hide.y), (hide.x, hide.right()), "Hide");
    // Over a minimap, under the icons at the top right.
    let actions = app.pane_actions();
    let browser = actions.iter().find(|(.., name)| *name == "Browser");
    let (cell, ..) = browser.expect("the Browser view's icon");
    let on = (cell.x + cell.w / 2, cell.y);
    check(&mut app, on, (cell.x, cell.right()), "Browser");
    // Under the tab row, with the sidebar hidden.
    app.run(Command::ToggleSidebar);
    let show = app.main_area().x;
    assert!(app.sidebar_button() && app.on_tabs_button(show));
    check(&mut app, (show, 0), (show, show + MENU_BUTTON), "Show");
}

/// An agent's tab's tip, what it says of the context, stands under the tab from where the
/// tab starts and inside the pane, `LINE_AIR` from its lines, and on a second line where it's
/// longer than that, broken at a " · ": from the pointer it ran over the file's sheet's line
/// and ended 9 px from the window's side, and cut with "…" on one line it lost what it said
/// last, "right-click to open the tra…", with 48 px empty after it (0.11.0's release check,
/// P7, 64-transcript-tab-hover).
#[test]
fn a_pane_tabs_tip_stands_inside_the_pane_from_the_tabs_start() {
    let (dir, mut app) = project((150, 40));
    let fake = dir.path().join("fake-agent");
    fs::write(&fake, "#!/bin/sh\nexec sleep 30\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&fake, fs::Permissions::from_mode(0o755)).unwrap();
    }
    app.agent_program = Some(fake.display().to_string());
    app.open_agent("claude");
    let index = app.dock.sessions.len() - 1;
    let usage = crate::transcript::Usage {
        context: 124_000,
        window: 1_000_000,
        input: 820_000,
        output: 1_000,
        model: "claude-opus-5-5".to_string(),
    };
    let said = usage.describe();
    app.dock.sessions[index].usage = Some(usage);
    let area = app.dock_area().expect("the pane");
    let title = app.title_of(area);
    let start = app.dock.tab_start_at(title, title.x + 3);
    assert_eq!(start, Some(title.x), "the agent's tab is the pane's first");
    app.handle(mouse(MouseKind::Move, title.x + 3, title.y));
    let grid = screen(&app);
    let tip = tip_pixels(&grid);
    // Its lines, each with where its words stand, in pixels from the window's top.
    let lines: Vec<(String, f32)> = (grid.paint.iter())
        .filter_map(|p| match p {
            Paint::Text { text, y, font, .. }
                if font.size == TOOLTIP_SIZE && said.contains(text.as_str()) =>
            {
                Some((text.clone(), *y as f32 / UNIT as f32))
            }
            _ => None,
        })
        .collect();
    // Too long for the pane at this width, it's on two lines, all of it, broken at a " · ".
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert_eq!(
        format!("{} · {}", lines[0].0, lines[1].0),
        said,
        "{lines:?}"
    );
    assert!(!lines[1].0.ends_with('…'), "{lines:?}");
    // The second line a line of the terminal's under the first, both inside the tip with the
    // air a tip of one line has over and under its words.
    let step = (lines[1].1 - lines[0].1) * ROW;
    assert!(
        (step - LINE).abs() < 1.0,
        "{step} px from one line to the next"
    );
    let (first, last) = (lines[0].1 * ROW, lines[1].1 * ROW);
    let (over, under) = (
        first + (ROW - LINE) / 2.0 - tip.2,
        tip.3 - (last + (ROW + LINE) / 2.0),
    );
    // The box and its words each stand on a 64th of a row, 0.8 px here.
    assert!(
        over > 0.0 && (over - under).abs() <= 1.6,
        "{over} px over its first line's box, {under} under its last's"
    );
    // Inside the pane, clear of its lines, from where the tab starts.
    let near = measures::LINE_AIR + measures::line_pixels(&*app.measure);
    let lines = app.sheet_lines();
    for line in &lines {
        let line = line * CELL;
        for edge in [tip.0, tip.1] {
            assert!(
                (edge - line).abs() >= near - 0.5,
                "the tip {tip:?} {} px from the line at {line}",
                (edge - line).abs()
            );
        }
    }
    let side = (f32::from(area.x) + SHEET_EDGE) * CELL;
    assert!(
        tip.0 > side && tip.0 - side <= near + 1.0,
        "starts {} px into the pane",
        tip.0 - side
    );
    let end = f32::from(area.right()) * CELL;
    assert!(tip.1 < end, "{} px past the pane", tip.1 - end);
}

/// A formula's picture under its open source stands 16 px under the source's box: it stood
/// 4 px under it, and its numerator touched the box's thumb (0.11.0's final review, C14,
/// 46-formula-open).
#[test]
fn an_open_formulas_picture_stands_clear_under_its_source() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("note.md");
    let text = "Words before it.\n\n$$\n\\frac{a}{b} = c\n$$\n\nWords after it.\n";
    fs::write(&path, text).unwrap();
    let mut app = App::new(dir.path().to_path_buf(), (150, 40));
    airy(&mut app);
    app.open(&path).unwrap();
    let theme = app.theme;
    let picture = |grid: &Grid| {
        grid.paint.iter().find_map(|p| match p {
            Paint::Image { bounds, .. } => Some(*bounds),
            _ => None,
        })
    };
    assert!(
        picture(&screen(&app)).is_some(),
        "typeset with the caret away"
    );
    // The caret in its source opens it, with the picture under it.
    let inside = text.find("frac").unwrap();
    let editor = app.editor_mut().unwrap();
    editor.selections.set_single(Selection::cursor(inside));
    let grid = screen(&app);
    let code = panels(&grid)
        .into_iter()
        .find(|(_, fill, border, _)| *fill == Some(theme.code_bg) && *border == Some(theme.border))
        .expect("the source's box");
    let (top, height) = tall(code.0);
    let below = picture(&grid).expect("its picture under it");
    let gap = below.y as f32 / UNIT as f32 * ROW - (top + height);
    assert!((gap - 16.0).abs() <= 1.0, "{gap} px under the box");
}

/// A drawing's source, wider than the page, keeps its sideways thumb inside its box, 4 px over
/// its foot, and "Drawing with LaTeX…" stands 16 px under the box, the first thing under it.
/// The thumb ran 14 px under the box, through the note, which stood 10 px under the box and
/// read as its caption (0.11.0's release check, P4, 82-paper-changes-row).
#[test]
fn a_drawings_note_stands_clear_under_its_box_and_thumb() {
    let dir = tempfile::tempdir().unwrap();
    let long = "\\draw (0,0) node {a label long enough to run on past the edge of the page} \
        -- (4,0) node {and on past it again} -- (8,0) node {and once more};";
    let text = format!("Before it.\n\n```tikz\n{long}\n```\n\nAfter the drawing.\n");
    fs::write(dir.path().join("note.md"), text).unwrap();
    let mut app = App::new(dir.path().to_path_buf(), (150, 40));
    airy(&mut app);
    // Nothing built here and nothing polled, so nothing runs: it waits to be drawn.
    app.built_folder = dir.path().join("built");
    app.open(&dir.path().join("note.md")).unwrap();
    let grid = screen(&app);
    let theme = app.theme;
    let code = panels(&grid)
        .into_iter()
        .find(|(_, fill, border, _)| *fill == Some(theme.code_bg) && *border == Some(theme.border))
        .expect("the source's box");
    let (top, height) = tall(code.0);
    let foot = top + height;
    let thumb = panels(&grid)
        .into_iter()
        .find(|(_, fill, ..)| *fill == Some(theme.scrollbar))
        .expect("a sideways thumb: the source is wider than the page");
    let (thumb_top, thumb_tall) = tall(thumb.0);
    assert!(thumb_top > top, "inside the box");
    let (box_left, thumb_left) = (px(code.0.x), px(thumb.0.x));
    assert!(
        (thumb_left - box_left - 4.0).abs() <= 0.5,
        "{} px in from the box's side",
        thumb_left - box_left
    );
    let over = foot - (thumb_top + thumb_tall);
    assert!((over - 4.0).abs() <= 0.5, "{over} px over the box's foot");
    // The note, the first ink under the box across its words.
    let note = grid
        .paint
        .iter()
        .find_map(|p| match p {
            Paint::Text { x, y, text, .. } if text.starts_with("Drawing with LaTeX") => {
                Some((*x, *y))
            }
            _ => None,
        })
        .expect("the note");
    let row = (note.1 / UNIT) as u16;
    let first = row.saturating_sub(1);
    let rows = picture(&grid, first..row + 1);
    let from = px(note.0) as usize;
    let ground = theme.text.bg;
    let ink = (0..rows.len())
        .map(|line| row_top(first) + line as f32)
        .zip(&rows)
        .filter(|(at, _)| *at >= foot)
        .find(|(_, pixels)| pixels[from..from + 120].iter().any(|p| apart(*p, ground)))
        .map(|(at, _)| at)
        .expect("the note's words");
    let under = ink - foot;
    assert!((under - 16.0).abs() <= 1.5, "{under} px under the box");
}
