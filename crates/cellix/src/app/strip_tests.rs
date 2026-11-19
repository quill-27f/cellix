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

/// Row `y` as the tour's terminal shows it, rows of 50 px with words set for 36: each pixel
/// row's colours, from the window's left edge.
pub(super) fn picture(grid: &Grid, y: u16) -> Vec<Vec<cellix_render::Rgb>> {
    use cellix_render::{Rasterizer, SoftwareRasterizer};
    let mut rasterizer = SoftwareRasterizer::new(17, 50)
        .expect("fonts are in the binary")
        .with_type_row(36);
    let image = rasterizer.row(grid, y);
    (0..image.height)
        .map(|line| {
            (0..image.width)
                .map(|x| {
                    let at = (line * image.width + x) * 3;
                    cellix_render::Rgb(image.rgb[at], image.rgb[at + 1], image.rgb[at + 2])
                })
                .collect()
        })
        .collect()
}

/// Whether two colours are apart by more than a hair.
pub(super) fn apart(a: cellix_render::Rgb, b: cellix_render::Rgb) -> bool {
    let d = |p: u8, q: u8| p.abs_diff(q) > 24;
    d(a.0, b.0) || d(a.1, b.1) || d(a.2, b.2)
}

/// The runs of columns with ink in them, from pixel `from` to `to`, on a picture of a row:
/// what isn't `ground` in its middle, clear of the accent's line at its foot. Runs closer
/// than 4 px are one.
pub(super) fn ink_runs(
    rows: &[Vec<cellix_render::Rgb>],
    ground: cellix_render::Rgb,
    (from, to): (usize, usize),
) -> Vec<(usize, usize)> {
    let inked = |x: usize| (10..40).any(|line| apart(rows[line][x], ground));
    let mut runs: Vec<(usize, usize)> = Vec::new();
    for x in from..to.min(rows[0].len()) {
        if !inked(x) {
            continue;
        }
        match runs.last_mut() {
            Some(run) if x - run.1 <= 4 => run.1 = x,
            _ => runs.push((x, x)),
        }
    }
    runs
}

/// Each line between two tabs on row `y`, drawn as the tour's terminal shows it, with the air
/// either side of it in pixels: from the last ink before it to the line, and from the line to
/// the first ink after it. `ground` is the row's colour where nothing is drawn.
pub(super) fn line_air(
    grid: &Grid,
    y: u16,
    border: cellix_render::Rgb,
    ground: cellix_render::Rgb,
) -> Vec<(usize, usize)> {
    use cellix_render::{Paint, UNIT};
    let rows = picture(grid, y);
    let width = rows[0].len();
    // Words and marks stand in the row's middle, clear of the accent's line at its foot.
    let inked = |x: usize| (10..40).any(|line| apart(rows[line][x], ground));
    let lines = grid.paint.iter().filter_map(|p| match p {
        Paint::Hairline { bounds, color }
            if *color == border
                && bounds.w == 0
                && bounds.y >= i32::from(y) * UNIT
                && bounds.y < i32::from(y + 1) * UNIT =>
        {
            Some((bounds.x as f32 / UNIT as f32 * 17.0).round() as usize)
        }
        _ => None,
    });
    lines
        .map(|line| {
            // The line is two pixels from its place on, on a Retina screen.
            let left = (0..line).rev().find(|x| inked(*x));
            let right = (line + 2..width).find(|x| inked(*x));
            let (left, right) = (left.expect("ink before"), right.expect("ink after"));
            (line - left - 1, right - line - 2)
        })
        .collect()
}

/// The line between two file tabs behind the one in front stands in the middle of the air
/// between what the first paints, its name while its × shows only under the pointer, and the
/// second one's mark, measured to the ink: at their edge it stood 55 px after README.md and 16
/// before PLAN.md (0.11.0's release review, A7), and in the middle of their measures 37 and 32
/// (its release check, m4-tabs-wheel). A tab behind keeps its ×'s room, as VS Code's do, so
/// that it doesn't move under the pointer: the line can't stand nearer either than about 34.
#[test]
fn the_line_between_two_file_tabs_stands_in_the_middle_of_their_air() {
    use cellix_render::{Paint, UNIT};
    let dir = tempfile::tempdir().unwrap();
    let mut app = App::new(dir.path().to_path_buf(), (150, 30));
    let airy = cellix_render::FontMeasure::new(17, 50).with_type_row(36);
    app.set_measure(std::rc::Rc::new(airy));
    for name in ["queue.rs", "README.md", "PLAN.md"] {
        let path = dir.path().join(name);
        fs::write(&path, "text\n").unwrap();
        app.open(&path).unwrap();
    }
    app.show_tab(0);
    let grid = screen(&app);
    let theme = *app.theme();
    let lines = line_air(&grid, 0, theme.border, theme.tab.bg);
    assert_eq!(
        lines.len(),
        1,
        "one line, between the two behind: {lines:?}"
    );
    let (before, after) = lines[0];
    assert!(
        before.abs_diff(after) <= 2,
        "{before} px after README.md's ink, {after} before PLAN.md's mark's"
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

/// Every chip in the row of a file's tabs is one height, 34 px of a row of 50, with 8 px over
/// and under it and so 8 px to the sheet's line under the row: the count of tabs out of view,
/// and under the pointer the sidebar's button and the icons at the top right. The count's was
/// 35 px and the others' a control's 39, 5 px over the line (0.11.0's release check, m4).
#[test]
fn every_chip_in_the_tab_row_is_one_height() {
    use cellix_render::{Paint, UNIT, stood};
    let (_dir, mut app) = many_files(8, (84, 40));
    let (row, line) = (50.0, 36.0);
    app.set_measure(std::rc::Rc::new(
        cellix_render::FontMeasure::new(17, 50).with_type_row(36),
    ));
    app.show_tab(7);
    let theme = *app.theme();
    // The chips on row 0 in the hover colour over column `x`, or all of them, as a top and a
    // height in pixels.
    let chips = |app: &App, x: Option<u16>| -> Vec<(f32, f32)> {
        let grid = screen(app);
        let over = |bounds: &cellix_render::Bounds| {
            x.is_none_or(|x| {
                let x = i32::from(x) * UNIT + UNIT / 2;
                bounds.x <= x && bounds.x + bounds.w >= x
            })
        };
        let found = grid.paint.iter().filter_map(|p| match p {
            Paint::Panel {
                bounds,
                fill: Some(fill),
                inset: kept,
                ..
            } if *fill == theme.row_hover && bounds.y < UNIT && over(bounds) => {
                // Kept at the terminal's line's height, or stood as a taller row stands it.
                Some(if *kept {
                    cellix_render::inset(bounds.y, bounds.h, row, line)
                } else {
                    stood(bounds.y, bounds.h, row, line)
                })
            }
            _ => None,
        });
        found.collect()
    };
    let count = chips(&app, None);
    assert_eq!(count.len(), 1, "the count of tabs out of view: {count:?}");
    let mut seen = vec![("the count", count[0])];
    // Each icon at the top right, under the pointer, and then, the sidebar hidden, the
    // button at the start of the tabs that shows it.
    let mut spots = Vec::new();
    for (cell, ..) in app.pane_actions() {
        if cell.y == 0 {
            spots.push(("an icon at the top right", cell.x + 1));
        }
    }
    assert!(spots.len() > 1, "{spots:?}");
    let mut pointed = |app: &mut App, what: &'static str, x: u16| {
        wheel(app, MouseKind::Move, x, 0);
        let found = chips(app, Some(x));
        assert_eq!(found.len(), 1, "{what} at {x}: {found:?}");
        seen.push((what, found[0]));
    };
    for (what, x) in spots {
        pointed(&mut app, what, x);
    }
    app.handle(Event::Key(cellix_input::KeyEvent::ctrl('b')));
    assert!(app.sidebar_button(), "the button at the start of the tabs");
    let button = app.main_area().x + 1;
    pointed(&mut app, "the sidebar's button", button);
    for (what, (top, tall)) in &seen {
        assert!(
            (33.5..=34.5).contains(tall),
            "{what}: {tall} px tall: {seen:?}"
        );
        // 8 px over and under, to a 64th of a row.
        assert!(
            *top >= 7.4 && row - (top + tall) >= 7.4,
            "{what}: {top} to {} of a row of {row}",
            top + tall
        );
    }
}

/// A tab dragged to the start of the strip has its bar in the middle of the air between the
/// sheet's line and the first tab's mark, 8 px or more from each: it stood on the sheet's line
/// and read as that line lit (0.11.0's release check, R5, 97-tab-drag-reorder).
#[test]
fn a_dragged_tabs_bar_stands_clear_of_the_sheets_line() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = App::new(dir.path().to_path_buf(), (150, 30));
    app.set_measure(std::rc::Rc::new(
        cellix_render::FontMeasure::new(17, 50).with_type_row(36),
    ));
    for name in ["queue.rs", "results.md", "notes.txt"] {
        let path = dir.path().join(name);
        fs::write(&path, "text\n").unwrap();
        app.open(&path).unwrap();
    }
    let tabs = app.visible_tabs();
    let y = app.tabs_area().y;
    let (last, first) = (tabs[2].1 + 3, tabs[0].1);
    wheel(&mut app, MouseKind::Left, last, y);
    wheel(&mut app, MouseKind::Drag, first, y);
    let grid = screen(&app);
    let theme = *app.theme();
    let rows = picture(&grid, y);
    // The bar: the accent, upright, in the row's middle.
    let accent = |x: usize| (20..30).all(|line| !apart(rows[line][x], theme.accent));
    let edge = usize::from(first) * 17;
    let bar: Vec<usize> = (edge.saturating_sub(17)..edge + 40)
        .filter(|x| accent(*x))
        .collect();
    assert!(!bar.is_empty(), "the bar at the strip's start");
    let (from, to) = (bar[0], bar[bar.len() - 1]);
    // What's either side of it on the row's ground: the sheet's line, and the first mark.
    // Past the pixel either side the bar shades. The sheet's line is faint, 22 from the ground
    // in each of its colours, so anything 12 away counts.
    let ground = theme.tab.bg;
    let faint = |a: cellix_render::Rgb| {
        a.0.abs_diff(ground.0) > 12 || a.1.abs_diff(ground.1) > 12 || a.2.abs_diff(ground.2) > 12
    };
    let inked = |x: usize| (10..40).any(|l| faint(rows[l][x]));
    let left = (0..from.saturating_sub(1)).rev().find(|x| inked(*x));
    let right = (to + 2..rows[0].len()).find(|x| inked(*x));
    let (before, after) = (from - left.unwrap() - 1, right.unwrap() - to - 1);
    assert!(
        before >= 7 && after >= 7 && before.abs_diff(after) <= 2,
        "{before} px from the line to the bar at {from}..{to}, {after} from the bar to the mark"
    );
}
