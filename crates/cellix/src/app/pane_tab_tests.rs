//! The side pane's row of tabs where there are pictures: each tab as wide as what it paints,
//! the + after the last of them while they fit, and the parts of a tab close together and
//! apart from the next one's. Driven through the app and read off the paint it draws.

use std::fs;
use std::path::Path;
use std::rc::Rc;
use std::time::{Duration, Instant};

use cellix_agent::{AgentEvent, AgentState, Request};
use cellix_input::{Command, Event, Mods, MouseEvent, MouseKind};
use cellix_render::{Font, Grid, Icon, Measure, Paint, UNIT};
use tempfile::TempDir;

use super::{App, Overlay};
use crate::dock::{Fit, Front, TabHit};

/// The tour's cells: 17 px wide, lines of 36.
fn measure() -> cellix_render::FontMeasure {
    cellix_render::FontMeasure::new(17, 36)
}

fn project(size: (u16, u16)) -> (TempDir, App) {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("alpha.txt"), "first line\n").unwrap();
    let mut app = App::new(dir.path().to_path_buf(), size);
    app.set_measure(Rc::new(measure()));
    (dir, app)
}

fn screen(app: &App) -> Grid {
    let mut grid = Grid::new(app.size().0, app.size().1, app.background());
    app.draw(&mut grid);
    grid
}

fn mouse(app: &mut App, kind: MouseKind, x: u16, y: u16) {
    app.handle(Event::Mouse(MouseEvent {
        kind,
        x,
        y,
        fine_x: x as f32,
        fine_y: y as f32 + 0.5,
        mods: Mods::default(),
    }));
}

/// A stand-in for an agent that names itself in its terminal's title, as Claude Code does,
/// and waits.
fn stand_in(dir: &Path, title: &str) -> String {
    let fake = dir.join("fake-agent");
    let script = format!("#!/bin/sh\nprintf '\\033]0;{title}\\007'\nexec sleep 30\n");
    fs::write(&fake, script).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&fake, fs::Permissions::from_mode(0o755)).unwrap();
    }
    fake.display().to_string()
}

/// Starts the stand-in as Claude Code in the side pane, and waits until it has named itself.
fn start_agent(dir: &Path, app: &mut App, title: &str) -> usize {
    app.agent_program = Some(stand_in(dir, title));
    app.open_agent("claude");
    let index = app.dock.sessions.len() - 1;
    let deadline = Instant::now() + Duration::from_secs(30);
    while app.dock.sessions[index].title.is_none() {
        assert!(Instant::now() < deadline, "the stand-in never named itself");
        app.poll();
        std::thread::sleep(Duration::from_millis(20));
    }
    index
}

fn working() -> Request {
    Request::Agent(AgentEvent {
        session: "s1".into(),
        agent: "Claude Code".into(),
        state: AgentState::Working,
        detail: "Working".into(),
        folder: "/w".into(),
        pid: None,
        transcript: String::new(),
        name: String::new(),
    })
}

/// The pane's row of tabs, as the pane's own part of it.
fn title(app: &App) -> cellix_render::Rect {
    app.title_of(app.dock_area().expect("the pane is open"))
}

/// Words painted on row `y`, with where they start and how far they may run, in cells.
fn words(grid: &Grid, y: u16) -> Vec<(String, f32, f32, cellix_render::Rgb)> {
    let unit = UNIT as f32;
    let words = grid.paint.iter().filter_map(|paint| match paint {
        Paint::Text {
            x,
            y: top,
            max_width,
            text,
            color,
            ..
        } if *top == i32::from(y) * UNIT => Some((
            text.clone(),
            *x as f32 / unit,
            *max_width as f32 / unit,
            *color,
        )),
        _ => None,
    });
    words.collect()
}

/// Where an icon is painted on row `y`, as (left, right) in cells.
fn icons(grid: &Grid, y: u16, which: Icon) -> Vec<(f32, f32)> {
    let unit = UNIT as f32;
    let (top, bottom) = (i32::from(y) * UNIT, i32::from(y + 1) * UNIT);
    let found = grid.paint.iter().filter_map(|paint| match paint {
        Paint::Icon { bounds, icon, .. }
            if *icon == which && bounds.y >= top && bounds.y < bottom =>
        {
            Some((bounds.x as f32 / unit, (bounds.x + bounds.w) as f32 / unit))
        }
        _ => None,
    });
    found.collect()
}

/// Where spinners and dots stand on row `y`, as their left edge in cells.
fn states(grid: &Grid, y: u16) -> Vec<f32> {
    let (top, bottom) = (i32::from(y) * UNIT, i32::from(y + 1) * UNIT);
    let found = grid.paint.iter().filter_map(|paint| match paint {
        Paint::Spinner { bounds, .. } if bounds.y >= top && bounds.y < bottom => {
            Some(bounds.x as f32 / UNIT as f32)
        }
        _ => None,
    });
    found.collect()
}

/// An agent's tab and a terminal's beside it leave the + standing after them, a cell past the
/// last, while they and it fit before the icons at the top right; and the + opens what starts
/// things. With two such tabs it went, though the row had 200 px to spare: each tab was as
/// wide as its name in letters, which is twice what it paints.
#[test]
#[cfg(unix)]
fn the_plus_stays_after_an_agents_tab_and_another() {
    let (dir, mut app) = project((150, 44));
    start_agent(dir.path(), &mut app, "✳ pricing page");
    app.open_agent(crate::agents::SHELL.1);
    let grid = screen(&app);
    let row = title(&app);
    assert_eq!(app.dock.fit(row), Fit::Whole, "{}", grid.row_text(row.y));
    // The + stands right after the last tab: the + is the cell after it.
    let plus = app.dock.plus_x(row);
    let last = (row.x..plus)
        .filter(|x| {
            matches!(
                app.dock.hit(row, *x),
                Some(TabHit::Tab(1) | TabHit::Close(1))
            )
        })
        .max();
    assert_eq!(
        last.map(|x| x + 1),
        Some(plus),
        "the + follows the last tab"
    );
    let drawn = words(&grid, row.y).into_iter().any(|(text, x, ..)| {
        text == "+" && x >= f32::from(plus) + 1.0 && x < f32::from(plus) + 2.0
    });
    assert!(drawn, "the + is drawn: {:?}", words(&grid, row.y));
    assert_eq!(app.dock.hit(row, plus + 1), Some(TabHit::New));
    assert!(
        plus + 3 <= app.dock.maximize_button(row),
        "before the button that maximizes the pane"
    );
    // A click on it opens what starts things, dropping from it.
    mouse(&mut app, MouseKind::Left, plus + 1, row.y);
    assert!(matches!(app.overlay, Overlay::Menu(_)), "a menu");
    let text = screen(&app).text();
    for name in ["Terminal", "Claude Code", "New Worktree"] {
        assert!(text.contains(name), "{name} is offered: {text}");
    }
    app.overlay = Overlay::None;

    // Only when the tabs truly don't fit does it give way.
    app.dock.width = Some(24);
    app.handle(Event::Resize(150, 44));
    let row = title(&app);
    assert_ne!(app.dock.fit(row), Fit::Whole);
    let grid = screen(&app);
    let drawn = words(&grid, row.y)
        .into_iter()
        .any(|(text, ..)| text == "+");
    assert!(!drawn, "out of room it gives way");
}

/// A tab is as wide as what it paints: its name whole, with no letter cut and no "…" while
/// it has room; what its agent is doing half a cell after the name; its × 12 px after that,
/// with nothing idle before it; and the tab after it clearly apart.
#[test]
#[cfg(unix)]
fn a_tab_is_as_wide_as_what_it_paints() {
    let (dir, mut app) = project((150, 44));
    let measure = measure();
    let font = Font::default();
    start_agent(dir.path(), &mut app, "✳ pricing page");
    app.turn_at = Some(0);
    let row = title(&app);
    // Idle, in front: its × right after its name.
    let grid = screen(&app);
    let (_, name_x, room, colour) = (words(&grid, row.y).into_iter())
        .find(|(text, ..)| text == "pricing page")
        .expect("its whole name");
    let painted = measure.width("pricing page", font);
    assert!(room >= painted, "never a letter cut: {room} for {painted}");
    assert_eq!(
        colour,
        app.theme().text.fg,
        "in front, in the text's colour"
    );
    let close = icons(&grid, row.y, Icon::Close);
    let close = *close
        .iter()
        .rfind(|c| c.0 >= f32::from(row.x))
        .expect("its ×");
    let gap = close.0 - (name_x + painted);
    assert!(gap > 0.0 && gap < 1.0, "the × follows the name: {gap}");

    // Working: the spinner half a cell after the name, and the × close after the spinner.
    app.serve(working());
    let grid = screen(&app);
    let spin = states(&grid, row.y);
    let spin = *spin.first().expect("a spinner");
    let after = spin - (name_x + painted);
    assert!(
        (after - 0.5).abs() < 0.05,
        "half a cell after its name: {after}"
    );
    let close = icons(&grid, row.y, Icon::Close);
    let close = *close
        .iter()
        .rfind(|c| c.0 >= f32::from(row.x))
        .expect("its ×");
    let wide = 0.65 * 36.0 / 17.0;
    let gap = close.0 - (spin + wide);
    assert!(gap > 0.0 && gap < 1.0, "the × after the spinner: {gap}");

    // Another tab after it: they stand clearly apart, the spinner never nearer the next
    // tab's mark than its own name.
    app.open_agent(crate::agents::SHELL.1);
    app.dock.sessions[1].renamed = Some("docs".into());
    let grid = screen(&app);
    let row = title(&app);
    assert_eq!(app.dock.fit(row), Fit::Whole);
    let spin = *states(&grid, row.y).first().expect("still turning");
    let next = (words(&grid, row.y).into_iter())
        .find(|(text, ..)| text == "docs")
        .expect("the next tab");
    let apart = next.1 - (spin + wide);
    assert!(apart * 17.0 >= 34.0, "{} px apart", apart * 17.0);

    // Behind another, one that ended has its words as any other has; its mark says so. In
    // front, its words are the text's colour.
    app.dock.sessions[0].exited = true;
    app.dock.active = Some(0);
    app.dock.front = Front::Session;
    let grid = screen(&app);
    let (.., colour) = (words(&grid, row.y).into_iter())
        .find(|(text, ..)| text == "pricing page")
        .expect("its name");
    assert_eq!(colour, app.theme().text.fg, "bright in front, ended or not");
}

/// A row too short for every tab whole cuts those behind the one in front, and two of them
/// side by side have a hairline between them, so a state at the end of one isn't read as
/// the next one's. The Agents tab has its mark, and cut short keeps it.
#[test]
#[cfg(unix)]
fn tabs_cut_short_stand_apart_and_the_agents_tab_keeps_its_mark() {
    let (dir, mut app) = project((150, 44));
    start_agent(dir.path(), &mut app, "✳ pricing page");
    app.serve(working());
    app.run(Command::ListAgents);
    // With room, the Agents tab is whole, its mark before its name, and the + after them.
    let grid = screen(&app);
    let row = title(&app);
    assert_eq!(app.dock.fit(row), Fit::Whole, "{}", grid.row_text(row.y));
    let has_mark = |app: &App, grid: &Grid, row: cellix_render::Rect| {
        let tabs = app.dock.view_tabs_for_test(row);
        let tab = tabs.iter().find(|t| t.0 == Front::Agents).unwrap().2;
        let marks = icons(grid, row.y, Icon::Agents);
        marks
            .iter()
            .any(|(left, right)| *left >= f32::from(tab.x) && *right <= f32::from(tab.right()))
    };
    assert!(has_mark(&app, &grid, row), "its mark");
    let (agents, ..) = (words(&grid, row.y).into_iter())
        .find(|(text, ..)| text.starts_with("Agents"))
        .expect("its name");
    assert_eq!(agents, "Agents", "whole, with room");
    // A terminal in front, and the pane narrowed until the others are cut short.
    app.open_agent(crate::agents::SHELL.1);
    app.run(Command::ListAgents);
    app.dock.active = Some(1);
    app.dock.front = Front::Session;
    let cut = (24..90).rev().find(|width| {
        app.dock.width = Some(*width);
        app.handle(Event::Resize(150, 44));
        app.dock.fit(title(&app)) == Fit::Cut
    });
    assert!(cut.is_some(), "some width cuts them");
    let grid = screen(&app);
    let row = title(&app);
    assert!(has_mark(&app, &grid, row), "cut short, it keeps its mark");
    let (top, bottom) = (i32::from(row.y) * UNIT, i32::from(row.y + 1) * UNIT);
    let lines = grid.paint.iter().filter(|paint| {
        matches!(paint, Paint::Hairline { bounds, color }
            if *color == app.theme().border && bounds.w == 0
                && bounds.y >= top && bounds.y + bounds.h <= bottom)
    });
    assert!(lines.count() >= 1, "a hairline between the two cut short");
}

/// One meaning per mark. An agent with no mark of its own has a filled square with its
/// initial, any other program a filled square with its own name's initial laid over it, a
/// shell and a task the prompt; nothing is the hollow square, which meant three things and
/// read as a box to tick. Not installed, an agent keeps its own mark in the + menu, dimmed.
#[test]
#[cfg(unix)]
fn every_program_has_one_mark_and_none_is_a_box_to_tick() {
    let (_dir, mut app) = project((200, 44));
    let root = app.workspace.root().to_path_buf();
    let wait = ["sleep".to_string(), "30".to_string()];
    for name in [
        "OpenCode",
        "Copilot CLI",
        "Cursor CLI",
        "deploy",
        "make build",
    ] {
        app.start_session(name, &wait, &[], &root);
    }
    // The last is a task, whose output is kept.
    let task = app.dock.sessions.len() - 1;
    app.dock.sessions[task].printed = Some(Vec::new());
    app.dock.width = Some(120);
    app.handle(Event::Resize(200, 44));
    let grid = screen(&app);
    let row = title(&app);
    assert_eq!(app.dock.fit(row), Fit::Whole, "{}", grid.row_text(row.y));
    for (icon, what) in [
        (Icon::AgentOpenCode, "OpenCode"),
        (Icon::AgentCopilot, "Copilot CLI"),
        (Icon::AgentCursor, "Cursor CLI"),
        (Icon::Terminal, "a task"),
        (Icon::Program, "any other program"),
    ] {
        assert!(!icons(&grid, row.y, icon).is_empty(), "{what}'s mark");
    }
    // Any other program's square has its initial cut out of it, in the colour it stands on.
    let (program, _) = icons(&grid, row.y, Icon::Program)[0];
    let letter = (words(&grid, row.y).into_iter())
        .find(|(text, x, ..)| text == "D" && (*x - program).abs() < 1.5)
        .expect("its initial on its square");
    assert_eq!(letter.3, app.theme().tab.bg, "cut out of it: {letter:?}");
    // Each one's mark is drawn filled: inked at its middle, where a hollow square isn't.
    for icon in [Icon::AgentOpenCode, Icon::AgentCopilot, Icon::Program] {
        assert!(cellix_render::inked(icon, 0.25, 0.5), "{icon:?} is filled");
    }
    assert!(!cellix_render::inked(
        Icon::AgentOpenCode,
        0.5,
        0.5 - 3.6 / 24.0
    ));
    // On cells each is one letter, the prompt's ">" for the task.
    app.set_measure(Rc::new(cellix_render::CellMeasure));
    app.dock.width = Some(200);
    app.handle(Event::Resize(260, 44));
    let grid = screen(&app);
    let text = grid.row_text(title(&app).y);
    for shown in ["O OpenCode", "C Copilot", "C Cursor", "D deploy", "> make"] {
        assert!(text.contains(shown), "{shown}: {text}");
    }
    assert!(!text.contains('▫'), "no hollow square: {text}");

    // In the + menu an agent that isn't installed keeps its own mark, dimmed.
    app.set_measure(Rc::new(measure()));
    app.dock.width = Some(120);
    app.handle(Event::Resize(200, 44));
    let _ = screen(&app);
    let row = title(&app);
    let plus = app.dock.plus_x(row);
    mouse(&mut app, MouseKind::Left, plus + 1, row.y);
    let grid = screen(&app);
    let dimmed = |which: Icon| {
        grid.paint.iter().any(|paint| {
            matches!(paint, Paint::Icon { icon, color, .. }
                if *icon == which && *color == app.theme().disabled)
        })
    };
    let menu = grid.text();
    for (icon, name) in [
        (Icon::AgentOpenCode, "OpenCode"),
        (Icon::AgentCopilot, "Copilot CLI"),
        (Icon::AgentCursor, "Cursor CLI"),
    ] {
        if menu.contains(name)
            && !app
                .agents
                .installed_programs()
                .iter()
                .any(|p| crate::agents::display_name(p) == name)
        {
            assert!(dimmed(icon), "{name} not installed: its own mark, dimmed");
        }
    }
    let under = i32::from(row.y + 1) * UNIT;
    let hollow = grid.paint.iter().any(|paint| {
        matches!(paint, Paint::Icon { icon: Icon::Program, bounds, .. } if bounds.y >= under)
    });
    assert!(!hollow, "no agent in the menu is a plain square");
}
