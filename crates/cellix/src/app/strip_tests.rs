//! The wheel over a strip of tabs, driven through the app and read off the drawn grid.

use std::fs;

use cellix_input::{Command, Event, Mods, MouseEvent, MouseKind};
use cellix_render::Grid;
use tempfile::TempDir;

use super::App;

fn screen(app: &App) -> Grid {
    let mut grid = Grid::new(app.size().0, app.size().1, app.background());
    app.draw(&mut grid);
    grid
}

fn wheel(app: &mut App, kind: MouseKind, x: u16, y: u16) {
    app.handle(Event::Mouse(MouseEvent {
        kind,
        x,
        y,
        fine_x: x as f32,
        fine_y: y as f32 + 0.5,
        mods: Mods::default(),
    }));
}

/// Files with long names, more than the strip holds.
fn many_files(count: usize, size: (u16, u16)) -> (TempDir, App) {
    let dir = tempfile::tempdir().unwrap();
    let mut app = App::new(dir.path().to_path_buf(), size);
    for n in 0..count {
        let path = dir.path().join(format!("file_number_{n}.txt"));
        fs::write(&path, format!("{n}\n")).unwrap();
        app.open(&path).unwrap();
    }
    (dir, app)
}

/// As in VS Code: with more tabs than fit, the wheel over the strip moves it sideways, a tab
/// a notch, the counts at its ends following, and the tab in front stays in front; with
/// every tab in view, it does nothing.
#[test]
fn the_wheel_over_the_tabs_moves_them_sideways_when_they_dont_all_fit() {
    let (_dir, mut app) = many_files(8, (100, 24));
    let area = app.tabs_area();
    let first_shown = |app: &App| app.tab_strip().0.first().map(|t| t.0);
    app.show_tab(7);
    let row = screen(&app).row_text(area.y);
    assert!(row.contains("more"), "{row}");
    let at_start = first_shown(&app).unwrap();
    assert!(
        at_start > 0,
        "the last one in front, the first ones are out of view"
    );
    // Up, back towards the first tab.
    wheel(&mut app, MouseKind::ScrollUp, area.x + 10, area.y);
    assert_eq!(first_shown(&app), Some(at_start - 1));
    assert_eq!(app.active(), 7, "the tab in front stays in front");
    for _ in 0..20 {
        wheel(&mut app, MouseKind::ScrollUp, area.x + 10, area.y);
    }
    assert_eq!(first_shown(&app), Some(0), "and no further than the first");
    let row = screen(&app).row_text(area.y);
    assert!(
        row.contains("file_number_0") && row.contains("more ›"),
        "{row}"
    );
    assert!(!row.contains("‹"), "{row}");
    // Down, and sideways by a trackpad, the other way.
    wheel(&mut app, MouseKind::ScrollDown, area.x + 10, area.y);
    wheel(&mut app, MouseKind::ScrollRight, area.x + 10, area.y);
    assert_eq!(first_shown(&app), Some(2));
    // Choosing a tab brings the strip to it again.
    app.show_tab(7);
    assert_eq!(first_shown(&app), Some(at_start));

    // With every tab in view, the wheel over them does nothing.
    let (_dir, mut few) = many_files(2, (100, 24));
    let before = few.tab_strip().0;
    let top = few.editors[1].top;
    wheel(&mut few, MouseKind::ScrollDown, area.x + 10, area.y);
    assert_eq!(few.tab_strip().0, before);
    assert!(few.tab_strip().1.is_none() && few.tab_strip().2.is_none());
    assert_eq!(few.editors[1].top, top, "nor scrolls the file under it");
}

/// The side pane keeps every tab in view, cut short as it must; only where it shows the tab
/// in front alone does the wheel over it step to the next.
#[test]
fn the_wheel_over_the_side_panes_tabs_steps_through_them_when_one_shows() {
    let (_dir, mut app) = many_files(1, (110, 30));
    app.run(Command::ShowComments);
    app.run(Command::ListAgents);
    let front = |app: &App| {
        let tabs = app.dock.every_tab();
        tabs.into_iter().find(|t| t.2).map(|t| t.0)
    };
    // Every tab in view: the wheel over them moves nothing. Where the tabs stop is known
    // once the row is drawn.
    let _ = screen(&app);
    let area = app.dock_area().unwrap();
    assert!(!app.dock.picking(app.title_of(area)));
    let before = front(&app);
    wheel(&mut app, MouseKind::ScrollUp, area.x + 3, area.y);
    assert_eq!(front(&app), before);
    app.open_agent(crate::agents::SHELL.1);
    app.open_agent(crate::agents::SHELL.1);
    app.dock.width = Some(24);
    app.handle(Event::Resize(110, 30));
    let _ = screen(&app);
    let area = app.dock_area().unwrap();
    assert!(
        app.dock.picking(app.title_of(area)),
        "only the tab in front shows"
    );
    let before = front(&app);
    wheel(&mut app, MouseKind::ScrollUp, area.x + 3, area.y);
    let after = front(&app);
    assert_ne!(after, before);
    wheel(&mut app, MouseKind::ScrollDown, area.x + 3, area.y);
    assert_eq!(front(&app), before, "and back");
}

/// The count of tabs out of view at the strip's start is a chip 8 px in from the sheet's
/// edge, with as much air after its words as before them: it stood 3 px from the edge, with
/// 14 px before its words and 24 after.
#[test]
fn the_count_at_the_strips_start_keeps_clear_of_the_edge() {
    use cellix_render::{Font, Measure, Paint, UNIT};
    let (_dir, mut app) = many_files(8, (84, 40));
    let measure = cellix_render::FontMeasure::new(17, 36);
    app.set_measure(std::rc::Rc::new(cellix_render::FontMeasure::new(17, 36)));
    app.show_tab(7);
    let area = app.tabs_area();
    let grid = screen(&app);
    let px = |units: i32| units as f32 * 17.0 / UNIT as f32;
    let (x, text) = (grid.paint.iter())
        .find_map(|p| match p {
            Paint::Text { x, text, y, .. } if *y == 0 && text.starts_with('‹') => {
                Some((*x, text.clone()))
            }
            _ => None,
        })
        .expect("a count at the start");
    // Its words are set a little under the tabs' size.
    let font = Font {
        size: 92,
        ..Font::default()
    };
    let words = measure.width(&text, font) * UNIT as f32;
    let chip = (grid.paint.iter())
        .find_map(|p| match p {
            Paint::Panel {
                bounds,
                fill: Some(fill),
                ..
            } if *fill == app.theme().row_hover
                && bounds.y < UNIT
                && bounds.x <= x
                && bounds.x + bounds.w >= x =>
            {
                Some(*bounds)
            }
            _ => None,
        })
        .expect("its chip");
    let edge = i32::from(area.x) * UNIT;
    assert!(
        px(chip.x - edge) >= 8.0,
        "{} px from the edge",
        px(chip.x - edge)
    );
    let before = px(x - chip.x);
    let after = px(chip.x + chip.w) - px(x) - words * 17.0 / UNIT as f32;
    assert!(
        (before - after).abs() <= 1.0,
        "{before} px before its words, {after} after"
    );
}
