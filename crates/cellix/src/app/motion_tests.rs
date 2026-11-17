//! What turns, and what asks for frames: the spinner as the app places it, driven through the
//! app and read off the drawn grid.

use std::fs;

use cellix_input::{Command, Event, Mods, MouseEvent, MouseKind};
use cellix_render::Grid;
use tempfile::TempDir;

use super::App;
use crate::settings_page::Control;

fn project() -> (TempDir, App) {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("alpha.txt"), "first line\nsecond line\n").unwrap();
    let app = App::new(dir.path().to_path_buf(), (120, 36));
    (dir, app)
}

fn screen(app: &App) -> Grid {
    let mut grid = Grid::new(app.size().0, app.size().1, app.background());
    app.draw(&mut grid);
    grid
}

fn click(app: &mut App, x: u16, y: u16) {
    for kind in [MouseKind::Left, MouseKind::Up] {
        app.handle(Event::Mouse(MouseEvent {
            kind,
            x,
            y,
            fine_x: x as f32,
            fine_y: y as f32 + 0.5,
            mods: Mods::default(),
        }));
    }
}

/// With nothing under way, a frame asks for no other: the loop waits as it always did, and
/// nothing is drawn or sent until something happens.
#[test]
fn nothing_under_way_asks_for_no_frames() {
    let (dir, mut app) = project();
    app.open(&dir.path().join("alpha.txt")).unwrap();
    app.run(Command::ShowAgents);
    let grid = screen(&app);
    assert!(grid.turn.is_some(), "motion is on unless turned off");
    assert!(!grid.spinning, "but nothing turns");
}

/// Inside Claude Code's pane nothing turns: the mod keeps only the pictures on screen, so
/// each step would be its row's picture crossing again, or by file the whole pane's.
#[test]
fn inside_claude_codes_pane_a_spinner_stands_still() {
    let (_dir, mut app) = project();
    assert!(app.turn().is_some());
    app.in_pane = true;
    assert_eq!(app.turn(), None);
    let mut grid = screen(&app);
    grid.spinner(4.0, 3, 0.65, cellix_render::Rgb(1, 2, 3), app.background());
    assert!(!grid.spinning, "so the pane asks for no frames");
}

/// Motion is a switch on Settings' Appearance page. Off, every spinner is a still dot: the
/// grid has no step for one to stand at, and nothing on it turns.
#[test]
fn motion_is_a_switch_in_settings() {
    let (_dir, mut app) = project();
    app.run(Command::OpenSettings);
    let text = screen(&app).text();
    assert!(text.contains("Motion"), "{text}");
    let area = app.sheet_area();
    let spot = |app: &App| {
        let page = app.settings_page.as_ref().unwrap();
        let spots = page.controls(area);
        spots.iter().find(|(_, c)| *c == Control::Motion).unwrap().0
    };
    let switch = spot(&app);
    assert!(app.turn().is_some());
    click(&mut app, switch.x + 1, switch.y);
    assert_eq!(app.settings.motion, Some(false));
    assert_eq!(app.turn(), None);
    let mut grid = screen(&app);
    assert_eq!(grid.turn, None);
    // A spinner placed now is a still dot, on cells as on pictures, and asks for nothing.
    let style = app.background();
    grid.spinner(4.0, 3, 0.65, cellix_render::Rgb(1, 2, 3), style);
    assert_eq!(grid.cell(4, 3).symbol, cellix_render::spinner::STILL);
    assert!(!grid.spinning);
    click(&mut app, switch.x + 1, switch.y);
    assert_eq!(app.settings.motion, Some(true));
    let mut grid = screen(&app);
    grid.spinner(4.0, 3, 0.65, cellix_render::Rgb(1, 2, 3), style);
    assert!(grid.spinning, "on again, it turns");
    let step = grid.turn.unwrap();
    assert_eq!(
        grid.cell(4, 3).symbol,
        cellix_render::spinner::FRAMES[step as usize]
    );
}
