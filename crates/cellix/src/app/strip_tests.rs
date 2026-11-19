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

/// The line between two file tabs behind the one in front stands in the middle of the air
/// between what the first paints, its name while its × shows only under the pointer, and the
/// second one's mark: at their edge it stood 55 px after README.md and 16 before PLAN.md
/// (0.11.0's release review, A7, m4-tabs-wheel).
#[test]
fn the_line_between_two_file_tabs_stands_in_the_middle_of_their_air() {
    use cellix_render::{Font, Measure, Paint, UNIT};
    let dir = tempfile::tempdir().unwrap();
    let mut app = App::new(dir.path().to_path_buf(), (150, 30));
    let measure = cellix_render::FontMeasure::new(17, 36);
    app.set_measure(std::rc::Rc::new(cellix_render::FontMeasure::new(17, 36)));
    for name in ["queue.rs", "README.md", "PLAN.md"] {
        let path = dir.path().join(name);
        fs::write(&path, "text\n").unwrap();
        app.open(&path).unwrap();
    }
    app.show_tab(0);
    let grid = screen(&app);
    let unit = UNIT as f32;
    let lines: Vec<f32> = (grid.paint.iter())
        .filter_map(|p| match p {
            Paint::Hairline { bounds, color }
                if *color == app.theme().border
                    && bounds.w == 0
                    && bounds.y < UNIT
                    && bounds.h < UNIT =>
            {
                Some(bounds.x as f32 / unit)
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        lines.len(),
        1,
        "one line, between the two behind: {lines:?}"
    );
    let line = lines[0];
    let name = (grid.paint.iter())
        .find_map(|p| match p {
            Paint::Text { x, y: 0, text, .. } if text == "README.md" => Some(*x as f32 / unit),
            _ => None,
        })
        .expect("README.md");
    let name_end = name + measure.width("README.md", Font::default());
    let mark = (grid.paint.iter())
        .filter_map(|p| match p {
            Paint::Icon { bounds, .. } if bounds.y < UNIT => Some(bounds.x as f32 / unit),
            _ => None,
        })
        .find(|x| *x > line)
        .expect("PLAN.md's mark");
    // Its ink about 2 px into its box.
    let (before, after) = ((line - name_end) * 17.0, (mark - line) * 17.0 + 2.0);
    assert!(
        (before - after).abs() <= 4.0,
        "{before} px after README.md, {after} before PLAN.md's mark"
    );
    // Beside a tab under the pointer, whose × and its chip stand there, there's none.
    let (_, x, _) = app.visible_tabs()[1];
    wheel(&mut app, MouseKind::Move, x + 2, 0);
    let grid = screen(&app);
    let lines = (grid.paint.iter()).filter(|p| {
        matches!(p, Paint::Hairline { bounds, color }
            if *color == app.theme().border && bounds.w == 0 && bounds.y < UNIT
                && bounds.h < UNIT)
    });
    assert_eq!(lines.count(), 0);
}

/// The chip under the pointer on a file tab's × is a chip, not the row: about 28 px tall on
/// the tour's rows of 50 with words set for 36, centred on the ×, and 8 px clear of the
/// accent's line under the tab in front. It was 37 px, its foot 3 px over that line (29, 41).
#[test]
fn a_tabs_close_chip_stands_clear_of_its_line() {
    use cellix_render::{Icon, Paint, UNIT, inset, stood};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("alpha.txt");
    fs::write(&path, "first line\n").unwrap();
    let mut app = App::new(dir.path().to_path_buf(), (120, 30));
    let (row, line) = (50.0, 36.0);
    let airy = cellix_render::FontMeasure::new(17, 50).with_type_row(36);
    app.set_measure(std::rc::Rc::new(airy));
    app.open(&path).unwrap();
    let theme = *app.theme();
    let (_, x, width) = app.visible_tabs()[0];
    // Across the tab until the pointer is on its ×, where the chip shows.
    let mut found = None;
    for at in x..x + width {
        wheel(&mut app, MouseKind::Move, at, 0);
        let grid = screen(&app);
        let chip = grid.paint.iter().find_map(|p| match p {
            Paint::Panel {
                bounds,
                fill: Some(fill),
                inset: on_wash,
                ..
            } if *fill == theme.row_hover && bounds.y < UNIT => Some((*bounds, *on_wash)),
            _ => None,
        });
        if let Some(chip) = chip {
            found = Some((chip, grid));
            break;
        }
    }
    let ((chip, on_wash), grid) = found.expect("a chip on the ×");
    let (top, tall) = if on_wash {
        inset(chip.y, chip.h, row, line)
    } else {
        stood(chip.y, chip.h, row, line)
    };
    assert!((26.0..=30.0).contains(&tall), "{tall} px tall");
    assert!(
        (top + tall / 2.0 - row / 2.0).abs() <= 1.0,
        "in the row's middle: {top} and {tall}"
    );
    // Across, centred on the ×: its box is a square as tall as its bounds, in the type's
    // rows, from their left edge.
    let close = grid.paint.iter().find_map(|p| match p {
        Paint::Icon {
            bounds,
            icon: Icon::Close,
            ..
        } if bounds.y < UNIT => Some(*bounds),
        _ => None,
    });
    let close = close.expect("its ×");
    let px = |units: i32| units as f32 / UNIT as f32 * 17.0;
    let middle = px(close.x) + close.h as f32 / UNIT as f32 * line / 2.0;
    let chip_middle = px(chip.x) + px(chip.w) / 2.0;
    assert!(
        (middle - chip_middle).abs() <= 1.5,
        "the × at {middle}, the chip's middle at {chip_middle}"
    );
    // The accent's line under the tab in front, 8 px or more under the chip's foot.
    let under = grid.paint.iter().find_map(|p| match p {
        Paint::Panel {
            bounds,
            fill: Some(fill),
            ..
        } if *fill == theme.accent && bounds.y < UNIT => Some(*bounds),
        _ => None,
    });
    let under = under.expect("the line under the tab in front");
    let (line_top, _) = stood(under.y, under.h, row, line);
    let clear = line_top - (top + tall);
    assert!(clear >= 8.0, "{clear} px over the line");
}
