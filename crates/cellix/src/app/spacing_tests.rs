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
    // starts, and Check my review ends where one ends. They stood 8 px inside on the left
    // and 17 on the right.
    app.add_comment("alpha.txt".to_string(), 0, "and this".to_string());
    let grid = screen(&app);
    let found = panels(&grid)
        .into_iter()
        .find(|(_, fill, ..)| *fill == Some(card))
        .expect("a card")
        .0;
    let quiet = app.theme.quiet_button(app.theme.sidebar.bg);
    let order = panels(&grid)
        .into_iter()
        .find(|(b, fill, ..)| *fill == Some(quiet) && b.y < found.y && b.x + UNIT >= found.x)
        .expect("the order's chip")
        .0;
    assert!(
        (px(order.x) - px(found.x)).abs() <= 1.0,
        "the order at {} px, the card at {}",
        px(order.x),
        px(found.x)
    );
    let check = panels(&grid)
        .into_iter()
        .find(|(b, fill, ..)| *fill == Some(app.theme.button) && b.y < found.y && b.x >= found.x)
        .expect("Check my review")
        .0;
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

/// Show Source stands on the row under the tab row at the line's height, 8 px or more from
/// the tab row's line, where as tall as the row's control it stood 4 px under it.
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
    assert!(chip.3, "kept at the line's height");
    let (top, _) = cellix_render::inset(chip.0.y, chip.0.h, ROW, LINE);
    assert!(
        top - row_top(1) >= 8.0,
        "{} px under the line",
        top - row_top(1)
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
