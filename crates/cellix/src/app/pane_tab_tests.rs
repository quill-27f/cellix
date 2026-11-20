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
        prompt: String::new(),
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

    // Only when the tabs, each cut to its mark, leave it no room does it give way, to the
    // tab in front alone with its chevron, whose list starts the same things.
    let gone = (16..60).rev().find(|width| {
        app.dock.width = Some(*width);
        app.handle(Event::Resize(150, 44));
        let _ = screen(&app);
        !app.dock.plus_shows(title(&app))
    });
    assert!(gone.is_some(), "some width is too narrow for it");
    let row = title(&app);
    assert_eq!(app.dock.fit(row), Fit::Front);
    let grid = screen(&app);
    let drawn = words(&grid, row.y)
        .into_iter()
        .any(|(text, ..)| text == "+");
    assert!(!drawn, "out of room it gives way");
}

/// A row too short for every tab whole cuts the names behind the one in front further
/// rather than lose the +: it kept giving way the moment any name was cut, which left a row
/// of agents with nothing to start another from (m1-agent-tabs, 131, 132).
#[test]
#[cfg(unix)]
fn names_are_cut_further_before_the_plus_gives_way() {
    let (dir, mut app) = project((150, 44));
    start_agent(dir.path(), &mut app, "✳ pricing page");
    app.open_agent(crate::agents::SHELL.1);
    app.open_agent(crate::agents::SHELL.1);
    app.dock.sessions[1].renamed = Some("the documentation's navigation".into());
    app.dock.sessions[2].renamed = Some("the open graph tags".into());
    // Narrowed until the names behind the one in front are cut short.
    // The row's room is known once it's drawn.
    let cut = (24..90).rev().find(|width| {
        app.dock.width = Some(*width);
        app.handle(Event::Resize(150, 44));
        let _ = screen(&app);
        app.dock.fit(title(&app)) == Fit::Cut
    });
    assert!(cut.is_some(), "some width cuts them");
    let grid = screen(&app);
    let row = title(&app);
    let text = grid.row_text(row.y);
    assert_eq!(
        app.dock.fit(row),
        Fit::Cut,
        "those behind are names alone: {text}"
    );
    // The + stands after the last tab, before the button that maximizes the pane.
    assert!(app.dock.plus_shows(row), "{text}");
    let plus = app.dock.plus_x(row);
    let drawn = words(&grid, row.y).into_iter().any(|(text, x, ..)| {
        text == "+" && x >= f32::from(plus) + 1.0 && x < f32::from(plus) + 2.0
    });
    assert!(drawn, "the + is drawn: {text}");
    assert_eq!(app.dock.hit(row, plus + 1), Some(TabHit::New));
    let maximize = app.dock.maximize_button(row);
    assert!(plus + 3 <= maximize, "{plus} and {maximize}: {text}");
    assert_eq!(app.dock.hit(row, maximize), Some(TabHit::Maximize));
    // No tab reaches into it.
    for x in plus..plus + 3 {
        assert_eq!(app.dock.hit(row, x), Some(TabHit::New), "at {x}: {text}");
    }
    // A click on it opens what starts things.
    mouse(&mut app, MouseKind::Left, plus + 1, row.y);
    assert!(matches!(app.overlay, Overlay::Menu(_)), "a menu");
    app.overlay = Overlay::None;
    // One that ends points at it.
    app.dock.sessions[2].exited = true;
    let text = screen(&app).text();
    assert!(app.dock.plus_shows(title(&app)), "{text}");
    assert!(text.contains("ress + for another"), "{text}");
}

/// The columns a session's tab stands in, as clicks find them.
fn tab_span(app: &App, row: cellix_render::Rect, index: usize) -> (u16, u16) {
    let hits = (row.x..row.right()).filter(
        |x| matches!(app.dock.hit(row, *x), Some(TabHit::Tab(i) | TabHit::Close(i)) if i == index),
    );
    hits.fold((u16::MAX, 0), |(from, to), x| (from.min(x), to.max(x + 1)))
}

/// Whether the + is drawn after the tabs, where a click on it starts something.
fn plus_drawn(app: &App, grid: &Grid, row: cellix_render::Rect) -> bool {
    let plus = app.dock.plus_x(row);
    // Within its own columns: it stands in the middle of the air before the button after it.
    let drawn = words(grid, row.y)
        .into_iter()
        .any(|(text, x, ..)| text == "+" && x >= f32::from(plus) && x < f32::from(plus) + 3.0);
    drawn && app.dock.hit(row, plus + 1) == Some(TabHit::New)
}

/// A row too short for every tab whole cuts those behind the one in front to their marks
/// before the + or the button that maximizes the pane gives way: at every width the tabs
/// are cut rather than listed, both are there, the longest name gives way first, and a name
/// short of three letters goes, leaving its mark in the middle of its tab with the same air
/// either side and a chip under the pointer that's exactly the mark and that air. Every tab
/// has a mark, and marks tell tabs apart. The + went whenever the names had less than three
/// letters each (167, 131, m1-agent-tabs-narrow, 173-peek-narrow), and a tab cut to its mark
/// kept a chip five columns wide with the mark at its left, 16 px before it and 47 after (166).
#[test]
#[cfg(unix)]
fn tabs_are_cut_to_their_marks_before_the_plus_gives_way() {
    let (_dir, mut app) = project((150, 44));
    for name in ["the documentation's navigation", "docs", "og tags"] {
        app.open_agent(crate::agents::SHELL.1);
        let last = app.dock.sessions.len() - 1;
        app.dock.sessions[last].renamed = Some(name.to_string());
    }
    app.run(Command::ShowComments);
    app.dock.active = Some(2);
    app.dock.front = Front::Session;
    let (mut cut, mut marks_only, mut longest_first) = (0, false, false);
    let mut with_button = 0;
    for width in (16..90).rev() {
        app.dock.width = Some(width);
        app.handle(Event::Resize(150, 44));
        let grid = screen(&app);
        let row = title(&app);
        let text = grid.row_text(row.y);
        match app.dock.fit(row) {
            Fit::Whole => continue,
            Fit::Front => {
                // Only where even the marks don't fit: the tab in front, its chevron, no +.
                assert!(!app.dock.plus_shows(row), "{width}: {text}");
                assert!(cut > 0, "cut short before listed");
                break;
            }
            Fit::Cut => cut += 1,
        }
        assert!(plus_drawn(&app, &grid, row), "{width}: the + stays: {text}");
        // The button that maximizes the pane stays while the marks leave it room: it gives
        // way only before a tab would go into the list.
        let maximize = app.dock.maximize_button(row);
        if app.dock.maximize_shows(row) {
            assert_eq!(
                app.dock.hit(row, maximize + 1),
                Some(TabHit::Maximize),
                "{text}"
            );
            assert!(app.dock.plus_x(row) + 3 <= maximize, "{width}: {text}");
            with_button += 1;
        }
        // The long name is cut while the short one beside it is still whole.
        let names = words(&grid, row.y);
        let long = names
            .iter()
            .any(|w| w.0 == "the documentation's navigation");
        let short = names.iter().any(|w| w.0 == "docs");
        longest_first |= !long && short;
        // A tab whose name has gone is its mark alone, in its middle.
        let (from, to) = tab_span(&app, row, 0);
        // Any letter of its name and the "…" is a name: tabs behind share the row by letters.
        let named = shown_name(&app, &grid, row, 0).is_some();
        if from >= to || named {
            continue;
        }
        marks_only = true;
        let (start, end) = (f32::from(from), f32::from(to));
        let mark = icons(&grid, row.y, Icon::Terminal)
            .into_iter()
            .find(|(left, _)| *left >= start && *left < end)
            .expect("cut to its mark");
        // The prompt's ink starts 2.4 of its 24 in, and ends 2.4 before its box does.
        let ink = 2.4 / 24.0 * (mark.1 - mark.0);
        let (before, after) = ((mark.0 + ink - start) * 17.0, (end - mark.1 + ink) * 17.0);
        assert!(
            (before - after).abs() <= 3.0 && before >= 12.0,
            "{width}: {before} px before its mark, {after} after: {text}"
        );
        // Under the pointer, a chip that's the mark and the same air either side.
        mouse(&mut app, MouseKind::Move, from + 1, row.y);
        let grid = screen(&app);
        let (top, bottom) = (i32::from(row.y) * UNIT, i32::from(row.y + 1) * UNIT);
        let chip = grid.paint.iter().find_map(|paint| match paint {
            Paint::Panel {
                bounds,
                fill: Some(fill),
                ..
            } if *fill == app.theme().row_hover && bounds.y >= top && bounds.y < bottom => Some((
                bounds.x as f32 / UNIT as f32,
                (bounds.x + bounds.w) as f32 / UNIT as f32,
            )),
            _ => None,
        });
        let chip = chip.expect("a chip under the pointer");
        let (lead, tail) = (
            (mark.0 + ink - chip.0) * 17.0,
            (chip.1 - mark.1 + ink) * 17.0,
        );
        assert!(
            (lead - tail).abs() <= 2.0 && (14.0..=18.0).contains(&lead),
            "{width}: the chip has {lead} px before the mark and {tail} after"
        );
        assert!(
            chip.0 >= start - 0.01 && chip.1 <= end + 0.01,
            "inside its tab"
        );
        mouse(&mut app, MouseKind::Move, 0, 40);
    }
    assert!(cut > 3, "{cut} widths cut the tabs short");
    assert!(
        with_button * 2 > cut,
        "the button stays at most: {with_button} of {cut}"
    );
    assert!(longest_first, "the longest name gives way first");
    assert!(marks_only, "some width cuts a tab to its mark");
    three_agents_in_the_tours_narrow_window();
}

/// Three agents in the tour's narrow window (m1-agent-tabs-narrow, 96 columns): Codex in
/// front asking, Claude Code behind it working and Gemini CLI done. Each behind is cut to its
/// mark and what it's doing, and the + stays 20 px after the last; the button that maximizes
/// the pane gives way first. Once a tab's anatomy made the one in front two columns wider, the
/// row showed it alone with its chevron, with 154 px empty and the others' marks gone.
#[cfg(unix)]
fn three_agents_in_the_tours_narrow_window() {
    use crate::dock::Doing;
    let (dir, mut app) = project((96, 40));
    app.agent_program = Some(stand_in(dir.path(), "agent"));
    let agents = [
        ("claude", "pricing page"),
        ("gemini", "docs nav"),
        ("codex", "og tags"),
    ];
    for (agent, name) in agents {
        app.open_agent(agent);
        let last = app.dock.sessions.len() - 1;
        app.dock.sessions[last].renamed = Some(name.to_string());
    }
    app.dock.sessions[0].doing = Doing::Working;
    app.dock.sessions[1].done_unseen = true;
    app.dock.sessions[2].doing = Doing::NeedsYou;
    app.dock.active = Some(2);
    app.dock.front = Front::Session;
    let grid = screen(&app);
    let row = title(&app);
    let text = grid.row_text(row.y);
    let room = format!(
        "{text} (pane {:?}, row {:?})",
        app.dock_area(),
        app.dock.tabs_end.get()
    );
    assert_eq!(app.dock.fit(row), Fit::Cut, "{room}");
    assert!(plus_drawn(&app, &grid, row), "the + stays: {room}");
    for icon in [Icon::AgentClaude, Icon::AgentGemini, Icon::AgentCodex] {
        let mark = icons(&grid, row.y, icon);
        assert_eq!(mark.len(), 1, "{icon:?}'s mark: {room}");
    }
    assert_eq!(states(&grid, row.y).len(), 3, "what each is doing: {room}");
    // Behind, each is its mark and its state, no letters.
    let names = words(&grid, row.y);
    for name in ["pri", "doc"] {
        assert!(
            !names.iter().any(|w| w.0.starts_with(name)),
            "{name}: {names:?}"
        );
    }
    // The + stands about 20 px after the last tab.
    let (_, end) = tab_span(&app, row, 2);
    let plus = names.iter().find(|w| w.0 == "+").expect("the +");
    let ink = (plus.1 - f32::from(end)) * 17.0;
    assert!((14.0..=26.0).contains(&ink), "{ink} px to the +: {room}");
    // No tab reaches into it, and it reaches no icon after it: its last column of air may be
    // the one kept clear before them.
    let plus_x = app.dock.plus_x(row);
    for x in plus_x..plus_x + 3 {
        assert_eq!(app.dock.hit(row, x), Some(TabHit::New), "at {x}: {room}");
    }
    let icons = app.dock.tabs_end.get() + app.dock.icons_air.get();
    assert!(plus_x + 3 <= icons, "{room}");
    assert!(!app.dock.maximize_shows(row), "the button gave way first");
    // A press on it opens what starts things.
    mouse(&mut app, MouseKind::Left, plus_x + 1, row.y);
    assert!(matches!(app.overlay, Overlay::Menu(_)), "a menu");
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
    for icon in [
        Icon::AgentOpenCode,
        Icon::AgentCopilot,
        Icon::AgentCursor,
        Icon::Program,
    ] {
        assert!(cellix_render::inked(icon, 0.25, 0.5), "{icon:?} is filled");
    }
    // Copilot CLI's and Cursor's are told apart: a C, and a pointer, cut out of the square.
    // They were one C.
    let (x, y) = (10.0 / 24.0, 12.0 / 24.0);
    assert!(
        cellix_render::inked(Icon::AgentCopilot, x, y),
        "inside the C"
    );
    assert!(
        !cellix_render::inked(Icon::AgentCursor, x, y),
        "on the pointer"
    );
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
    for shown in ["O OpenCode", "C Copilot", "↖ Cursor", "D deploy", "> make"] {
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

/// A tip under the views' line has 8 px of air under it on rows of Cellix's own, the sidebar
/// button's and a view's in the air over the heading rather than on it, as the button's at the
/// start of the tabs does under the tab row's line.
#[test]
fn a_tip_stands_eight_pixels_under_its_line() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("alpha.txt"), "first line\n").unwrap();
    let (row, words) = (50, 36);
    let mut app = App::new(dir.path().to_path_buf(), (150, 44));
    let measure = cellix_render::FontMeasure::new(17, row).with_type_row(words);
    app.set_measure(Rc::new(measure));
    app.open(&dir.path().join("alpha.txt")).unwrap();
    let tip_top = |app: &App| -> f32 {
        let grid = screen(app);
        let tip = grid.paint.iter().find_map(|paint| match paint {
            Paint::Panel {
                bounds,
                shadow: true,
                fill: Some(fill),
                ..
            } if *fill == app.theme().popup => Some(*bounds),
            _ => None,
        });
        let tip = tip.expect("a tip");
        // Drawn exactly where it's placed (`Grid::exact_popup`), on a pixel.
        (tip.y as f32 * row as f32 / cellix_render::UNIT as f32).round()
    };
    let line = |at: u16| f32::from(at) * row as f32;
    // 8 px of air under a line a pixel thick.
    let under = |gap: f32| (9.0..=11.0).contains(&gap);
    // The button that hides the sidebar, at the end of the views' row.
    let views = app.activity_area();
    let hide = app.sidebar_hide().expect("its button");
    mouse(&mut app, MouseKind::Move, hide.x + 1, hide.y);
    let gap = tip_top(&app) - line(views.bottom());
    assert!(under(gap), "Hide Sidebar {gap} px under");
    // A view's.
    mouse(&mut app, MouseKind::Move, views.x + 2, views.y);
    let gap = tip_top(&app) - line(views.bottom());
    assert!(under(gap), "a view's {gap} px under");
    // Hidden, the button at the start of the tabs.
    app.handle(Event::Key(cellix_input::KeyEvent::ctrl('b')));
    mouse(&mut app, MouseKind::Move, 1, 0);
    let gap = tip_top(&app) - line(1);
    assert!(under(gap), "Show Sidebar {gap} px under");
}

/// Every tab has a mark: a shell at its prompt its prompt, as one running a command has, and
/// each view the one its icon has, the inbox its bell; they were names alone. A program that
/// says what it is (OSC 7501's `app`) names its tab until it takes that back, as it names its
/// row in the Agents list: its tab said "Terminal" beside a row that said "deploy".
#[test]
#[cfg(unix)]
fn every_tab_has_a_mark_and_a_program_names_its_tab() {
    use crate::program::{Report, Word};
    let (_dir, mut app) = project((150, 44));
    app.open_agent(crate::agents::SHELL.1);
    app.run(Command::ShowInbox);
    app.dock.active = Some(0);
    app.dock.front = Front::Session;
    let grid = screen(&app);
    let row = title(&app);
    let text = grid.row_text(row.y);
    assert!(text.contains("Terminal"), "{text}");
    assert!(
        !icons(&grid, row.y, Icon::Terminal).is_empty(),
        "the prompt: {text}"
    );
    assert!(
        !icons(&grid, row.y, Icon::Bell).is_empty(),
        "the bell: {text}"
    );
    // A report under a name of its own names the tab; taken back, the tab is the shell's.
    let said = |word: Word| Report {
        word,
        kind: None,
        progress: None,
        id: String::new(),
        app: "deploy".to_string(),
        title: String::new(),
        message: String::new(),
    };
    app.dock.sessions[0]
        .reports
        .push((said(Word::Working), false));
    app.hear_programs();
    let text = screen(&app).row_text(row.y);
    assert!(
        text.contains("deploy") && !text.contains("Terminal"),
        "{text}"
    );
    app.dock.sessions[0]
        .reports
        .push((said(Word::Clear), false));
    app.hear_programs();
    let text = screen(&app).row_text(row.y);
    assert!(text.contains("Terminal"), "{text}");
}

/// A tab has as much air before its mark as after its ×, and what its whole columns hold past
/// what it paints is shared by both ends, so the two don't change with how its columns
/// round: the air after the × ran from 9 px to 22 by tab, beside 21 to 27 before the mark.
#[test]
#[cfg(unix)]
fn a_tabs_two_ends_have_the_same_air() {
    let (_dir, mut app) = project((150, 44));
    app.dock.width = Some(70);
    app.handle(Event::Resize(150, 44));
    for name in ["a", "make", "og tags"] {
        app.open_agent(crate::agents::SHELL.1);
        let last = app.dock.sessions.len() - 1;
        app.dock.sessions[last].renamed = Some(name.to_string());
    }
    let mut ends = Vec::new();
    for index in 0..app.dock.sessions.len() {
        app.dock.active = Some(index);
        app.dock.front = Front::Session;
        let grid = screen(&app);
        let row = title(&app);
        assert_eq!(app.dock.fit(row), Fit::Whole, "{}", grid.row_text(row.y));
        let tab: Vec<u16> = (row.x..row.right())
            .filter(|x| {
                matches!(app.dock.hit(row, *x), Some(TabHit::Tab(i) | TabHit::Close(i)) if i == index)
            })
            .collect();
        let (start, end) = (f32::from(tab[0]), f32::from(tab[tab.len() - 1] + 1));
        let mark = icons(&grid, row.y, Icon::Terminal)
            .into_iter()
            .find(|(left, _)| *left >= start && *left < end)
            .expect("its mark");
        let close = icons(&grid, row.y, Icon::Close)
            .into_iter()
            .find(|(left, _)| *left >= start && *left < end)
            .expect("its ×");
        // From the tab's ends to the ink: the prompt's starts 2.4 of its 24 in, and the ×'s
        // ends 5.6 of its 24 before its box does (`icon.rs`).
        let lead = (mark.0 - start + 2.4 / 24.0 * (mark.1 - mark.0)) * 17.0;
        let tail = (end - close.1 + 5.6 / 24.0 * (close.1 - close.0)) * 17.0;
        ends.push((lead, tail));
    }
    // As much air at each end, to a few pixels, in every tab however its columns rounded.
    for (lead, tail) in &ends {
        assert!(
            (lead - tail).abs() <= 4.0,
            "{lead} px before the mark, {tail} after the ×: {ends:?}"
        );
    }
}

fn asking() -> Request {
    Request::Agent(AgentEvent {
        state: AgentState::NeedsYou,
        detail: "Allow Bash?".into(),
        ..match working() {
            Request::Agent(event) => event,
            _ => unreachable!(),
        }
    })
}

/// How far from the start of an icon's box its ink starts, and from its end its ink ends, as
/// parts of the box: a mark's about 2 of its 24, a ×'s 5.35 (`icon.rs`).
const MARK_INK: f32 = 2.2 / 24.0;
const CLOSE_INK: f32 = 5.35 / 24.0;

/// Where an icon is drawn on row `y`, as (left, right) in cells: a square as tall as its
/// bounds are, in the type's rows, from their left edge, whatever their width says.
fn squares(grid: &Grid, y: u16, which: Option<Icon>) -> Vec<(f32, f32)> {
    let unit = UNIT as f32;
    let (top, bottom) = (i32::from(y) * UNIT, i32::from(y + 1) * UNIT);
    let found = grid.paint.iter().filter_map(|paint| match paint {
        Paint::Icon { bounds, icon, .. }
            if which.is_none_or(|w| w == *icon) && bounds.y >= top && bounds.y < bottom =>
        {
            let left = bounds.x as f32 / unit;
            Some((left, left + bounds.h as f32 / unit * 36.0 / 17.0))
        }
        _ => None,
    });
    found.collect()
}

/// A tab's anatomy in pixels, to the ink: (before its mark, mark to name, name to ×, after ×).
fn anatomy(
    (start, end): (f32, f32),
    mark: (f32, f32),
    (name_x, name_w): (f32, f32),
    close: (f32, f32),
) -> (f32, f32, f32, f32) {
    let px = |cells: f32| cells * 17.0;
    let (mark_ink, close_ink) = (
        (mark.1 - mark.0) * MARK_INK,
        (close.1 - close.0) * CLOSE_INK,
    );
    (
        px(mark.0 + mark_ink - start),
        px(name_x - (mark.1 - mark_ink)),
        px(close.0 + close_ink - (name_x + name_w)),
        px(end - (close.1 - close_ink)),
    )
}

/// Every tab, a file's and the side pane's, idle, working or asking, has one anatomy, measured
/// to the ink: about 18 px before its mark, 8 from the mark to its name, 12 from its name to
/// its ×, and as much after the × as before the mark; what an agent is doing has room of its
/// own before the ×. And a file's tabs abut, each with its own air, as the pane's do. The air
/// after a pane tab's × ran from 9 px on a working tab to 25 on an idle one, and a file's tab
/// was 21 | icon | name | 51 to its × | 13, with a column between it and the next (0.11.0's
/// final review, P12).
#[test]
#[cfg(unix)]
fn every_tab_has_one_anatomy() {
    let (dir, mut app) = project((150, 44));
    fs::write(dir.path().join("beta.rs"), "fn main() {}\n").unwrap();
    app.open(&dir.path().join("alpha.txt")).unwrap();
    app.open(&dir.path().join("beta.rs")).unwrap();
    let mut measured = Vec::new();
    let tabs = app.visible_tabs();
    assert_eq!(tabs.len(), 2, "{tabs:?}");
    assert_eq!(tabs[0].1 + tabs[0].2, tabs[1].1, "they abut: {tabs:?}");
    for (index, x, width) in tabs.iter().copied() {
        // Pointed at, so a tab behind shows its × in the room it keeps for it.
        mouse(&mut app, MouseKind::Move, x + 1, 0);
        let grid = screen(&app);
        let (start, end) = (f32::from(x), f32::from(x + width));
        let name = app.tab_title(index);
        let (_, name_x, ..) = (words(&grid, 0).into_iter())
            .find(|w| w.0 == name)
            .unwrap_or_else(|| panic!("{name}: {:?}", words(&grid, 0)));
        let mark = (squares(&grid, 0, None).into_iter())
            .find(|(left, right)| *left >= start && *right <= name_x);
        let mark = mark.unwrap_or_else(|| panic!("{name}'s mark"));
        let close = squares(&grid, 0, Some(Icon::Close))
            .into_iter()
            .find(|c| c.0 >= start && c.0 < end)
            .unwrap_or_else(|| panic!("{name}'s ×"));
        let name_w = measure().width(&name, Font::default());
        let parts = anatomy((start, end), mark, (name_x, name_w), close);
        measured.push((name, parts));
    }
    // The side pane's: an agent's tab in front, idle, working and asking.
    start_agent(dir.path(), &mut app, "✳ pricing page");
    app.turn_at = Some(0);
    let states = [
        ("idle", None),
        ("working", Some(working())),
        ("asking", Some(asking())),
    ];
    for (state, said) in states {
        if let Some(said) = said {
            app.serve(said);
        }
        let grid = screen(&app);
        let row = title(&app);
        assert_eq!(app.dock.fit(row), Fit::Whole, "{}", grid.row_text(row.y));
        let (from, to) = tab_span(&app, row, 0);
        let (start, end) = (f32::from(from), f32::from(to));
        let mark = squares(&grid, row.y, Some(Icon::AgentClaude))
            .into_iter()
            .find(|m| m.0 >= start && m.0 < end)
            .expect("its mark");
        let close = squares(&grid, row.y, Some(Icon::Close))
            .into_iter()
            .find(|c| c.0 >= start && c.0 < end)
            .expect("its ×");
        let (_, name_x, ..) = (words(&grid, row.y).into_iter())
            .find(|w| w.0 == "pricing page")
            .expect("its name");
        let name_w = measure().width("pricing page", Font::default());
        let (lead, mark_name, name_close, tail) =
            anatomy((start, end), mark, (name_x, name_w), close);
        // What it's doing has its own room between the name and the ×.
        let doing = states_on(&grid, row.y, (name_x + name_w, close.0));
        let name_close = if doing { 12.0 } else { name_close };
        let what = format!("the agent's tab, {state}");
        measured.push((what, (lead, mark_name, name_close, tail)));
    }
    // Each of the three gets a third of what the tab's whole columns hold past what it paints,
    // up to 4 px.
    for (what, (lead, mark_name, name_close, tail)) in &measured {
        assert!(
            (15.0..=22.0).contains(lead),
            "{what}: {lead} px before its mark: {measured:?}"
        );
        assert!(
            (5.0..=11.0).contains(mark_name),
            "{what}: {mark_name} px from its mark to its name"
        );
        assert!(
            (10.0..=16.5).contains(name_close),
            "{what}: {name_close} px from its name to its ×: {measured:?}"
        );
        assert!(
            (lead - tail).abs() <= 3.0,
            "{what}: {lead} px before its mark and {tail} after its ×: {measured:?}"
        );
    }
}

/// Whether a spinner or a dot stands between `from` and `to` on row `y`, in cells.
fn states_on(grid: &Grid, y: u16, (from, to): (f32, f32)) -> bool {
    states(grid, y).iter().any(|x| *x >= from && *x < to)
}

/// Two tabs cut short side by side have the same air either side of the line between them,
/// about 16 px to the ink, as a whole tab has at its ends: the spinner at the end of one
/// stood 10 px from the line and the next one's mark 11 (0.11.0's release review, A7,
/// m1-agent-tabs).
#[test]
#[cfg(unix)]
fn the_line_between_two_tabs_cut_short_has_the_same_air_either_side() {
    let (dir, mut app) = project((150, 44));
    start_agent(dir.path(), &mut app, "✳ pricing page");
    app.serve(working());
    for name in ["documentation", "Terminal"] {
        app.open_agent(crate::agents::SHELL.1);
        let last = app.dock.sessions.len() - 1;
        app.dock.sessions[last].renamed = Some(name.to_string());
    }
    app.dock.active = Some(2);
    app.dock.front = Front::Session;
    let unit = UNIT as f32;
    let mut seen = 0;
    for width in (16..90).rev() {
        app.dock.width = Some(width);
        app.handle(Event::Resize(150, 44));
        let grid = screen(&app);
        let row = title(&app);
        if app.dock.fit(row) != Fit::Cut {
            continue;
        }
        // Both behind still named, the first with its spinner at its end.
        let names = words(&grid, row.y);
        let named = |start: &str| names.iter().any(|w| w.0.starts_with(start));
        if !named("pri") || !named("doc") {
            continue;
        }
        let (_, end) = tab_span(&app, row, 0);
        let (top, bottom) = (i32::from(row.y) * UNIT, i32::from(row.y + 1) * UNIT);
        let spin = grid.paint.iter().find_map(|paint| match paint {
            Paint::Spinner { bounds, .. } if bounds.y >= top && bounds.y < bottom => {
                let left = bounds.x as f32 / unit;
                Some((left, left + bounds.h as f32 / unit * 36.0 / 17.0))
            }
            _ => None,
        });
        let spin = spin.expect("the spinner at the first one's end");
        let line = grid.paint.iter().find_map(|paint| match paint {
            Paint::Hairline { bounds, color }
                if *color == app.theme().border
                    && bounds.w == 0
                    && bounds.y >= top
                    && bounds.y < bottom =>
            {
                Some(bounds.x as f32 / unit)
            }
            _ => None,
        });
        let line = line.expect("a line between the two");
        let mark = squares(&grid, row.y, Some(Icon::Terminal))
            .into_iter()
            .find(|m| m.0 >= f32::from(end) - 0.01)
            .expect("the second one's mark");
        // The ring's ink ends 1.4 of its 24 before its square does.
        let before = (line - (spin.1 - (spin.1 - spin.0) * 1.4 / 24.0)) * 17.0;
        let after = (mark.0 + (mark.1 - mark.0) * MARK_INK - line) * 17.0;
        assert!(
            (13.0..=20.0).contains(&before) && (13.0..=20.0).contains(&after),
            "{width}: {before} px before the line, {after} after: {}",
            grid.row_text(row.y)
        );
        assert!((before - after).abs() <= 3.0, "{before} and {after}");
        seen += 1;
    }
    assert!(seen > 0, "some width cuts both short and names them");
}

/// A tab behind whose whole name shows has its dot as close after the name as a whole tab's,
/// in the middle of the room a spinner would have there, about 14 px to the ink, and the room
/// its last column has to spare after the dot: put at the tab's end, "pricing page"'s dot
/// stood 33 px after the name and 17 px before the next tab, and read as that one's (0.11.0's
/// release check, R1, m1-agent-tab-done). It stood at the start of that room, 10 px from the
/// name, until the dot on a tab in front was centred in it, and this one with it.
#[test]
#[cfg(unix)]
fn a_dot_on_a_tab_behind_follows_its_name() {
    let (dir, mut app) = project((150, 44));
    start_agent(dir.path(), &mut app, "✳ pricing page");
    app.serve(asking());
    app.open_agent(crate::agents::SHELL.1);
    let last = app.dock.sessions.len() - 1;
    app.dock.sessions[last].renamed = Some("Terminal".to_string());
    app.dock.active = Some(last);
    app.dock.front = Front::Session;
    let unit = UNIT as f32;
    let mut seen = 0;
    for width in (16..90).rev() {
        app.dock.width = Some(width);
        app.handle(Event::Resize(150, 44));
        let grid = screen(&app);
        let row = title(&app);
        let names = words(&grid, row.y);
        let Some(name) = names.iter().find(|w| w.0 == "pricing page") else {
            continue;
        };
        let end = name.1 + measure().width(&name.0, Font::default());
        let (top, bottom) = (i32::from(row.y) * UNIT, i32::from(row.y + 1) * UNIT);
        let dot = grid.paint.iter().find_map(|paint| match paint {
            Paint::Spinner {
                bounds, step: None, ..
            } if bounds.y >= top && bounds.y < bottom => {
                let left = bounds.x as f32 / unit;
                let square = bounds.h as f32 / unit * 36.0 / 17.0;
                // A dot's ink is 12 of its square's 24 parts, from the 6th.
                Some((left + square * 6.0 / 24.0, left + square * 18.0 / 24.0))
            }
            _ => None,
        });
        let (ink, ink_end) = dot.expect("the dot after the name");
        let after = (ink - end) * 17.0;
        assert!(
            (12.0..=17.0).contains(&after),
            "{width}: the dot {after} px after the name: {}",
            grid.row_text(row.y)
        );
        // Nearer its own name than the next tab's mark.
        let next = squares(&grid, row.y, Some(Icon::Terminal))
            .into_iter()
            .find(|mark| mark.0 > ink_end)
            .expect("the terminal's mark after it");
        let before_next = (next.0 + (next.1 - next.0) * MARK_INK - ink_end) * 17.0;
        assert!(
            before_next > after + 8.0,
            "{width}: {after} px after the name, {before_next} before the next tab"
        );
        seen += 1;
    }
    assert!(seen > 0, "some width shows the whole name with its dot");
}

/// The middle of what the pane's first agent tab says it's doing, and where the ink of the
/// tab's × starts, in pixels.
fn doing_and_close(app: &App) -> (f32, f32) {
    let grid = screen(app);
    let row = title(app);
    let unit = UNIT as f32;
    let (top, bottom) = (i32::from(row.y) * UNIT, i32::from(row.y + 1) * UNIT);
    let middle = grid.paint.iter().find_map(|paint| match paint {
        Paint::Spinner { bounds, .. } if bounds.y >= top && bounds.y < bottom => {
            let left = bounds.x as f32 / unit;
            Some(left + bounds.h as f32 / unit * 36.0 / 17.0 / 2.0)
        }
        _ => None,
    });
    let middle = middle.expect("what it's doing");
    let close = squares(&grid, row.y, Some(Icon::Close))
        .into_iter()
        .find(|c| c.0 > middle)
        .expect("its ×");
    (
        middle * 17.0,
        (close.0 + (close.1 - close.0) * CLOSE_INK) * 17.0,
    )
}

/// What an agent's tab says it's doing stands in the room a spinner has between its name and
/// its ×, and a dot in the middle of that room: at its start, the needs-you dot on the tab in
/// front stood 24 to 28 px before the × against a spinner's 19 (0.11.0's release check,
/// m1-agent-tab-asking, 170), with the room the spinner would fill after it.
#[test]
#[cfg(unix)]
fn a_dot_stands_in_the_middle_of_a_spinners_room() {
    let (dir, mut app) = project((150, 44));
    start_agent(dir.path(), &mut app, "✳ pricing page");
    app.turn_at = Some(0);
    // Names of several widths, so the room their columns round to differs.
    for name in ["pricing page", "og tags", "a", "documentation", "refunds"] {
        app.dock.sessions[0].renamed = Some(name.to_string());
        app.serve(working());
        let (spinner, close) = doing_and_close(&app);
        app.serve(asking());
        let (dot, dot_close) = doing_and_close(&app);
        assert_eq!(close, dot_close, "{name}: the × stays where it was");
        assert!(
            (dot - spinner).abs() <= 1.0,
            "{name}: the dot's middle at {dot} px, the spinner's at {spinner}"
        );
        // A dot's ink is half its square's: a quarter of the square either side of its middle.
        let square = crate::dock::MARK_SIZE * 36.0;
        let before_close = close - (dot + square / 4.0);
        assert!(
            before_close <= 25.0,
            "{name}: the dot's ink {before_close} px before the ×"
        );
    }
}

/// A tab behind cut short has its mark, its name, and a dot that follows its whole name, where
/// they stand when it's brought forward, so none of them moves on a click: cut short, the first
/// tab's mark stood 5 px nearer the pane's edge than in front (0.11.0's release check, A7).
#[test]
#[cfg(unix)]
fn a_tab_brought_forward_keeps_its_mark_name_and_dot_in_place() {
    let (dir, mut app) = project((150, 44));
    start_agent(dir.path(), &mut app, "✳ pricing page");
    app.serve(asking());
    app.open_agent(crate::agents::SHELL.1);
    let last = app.dock.sessions.len() - 1;
    app.dock.sessions[last].renamed = Some("Terminal".to_string());
    app.dock.front = Front::Session;
    let unit = UNIT as f32;
    // How the row stands, and where the agent's mark, name and dot are, in pixels.
    let place = |app: &App| {
        let grid = screen(app);
        let row = title(app);
        let mark = squares(&grid, row.y, Some(Icon::AgentClaude));
        let name = words(&grid, row.y)
            .into_iter()
            .find(|w| w.0.starts_with("pri"))
            .map(|w| (w.0, w.1 * 17.0));
        let (top, bottom) = (i32::from(row.y) * UNIT, i32::from(row.y + 1) * UNIT);
        let dot = grid.paint.iter().find_map(|paint| match paint {
            Paint::Spinner {
                bounds, step: None, ..
            } if bounds.y >= top && bounds.y < bottom => Some(bounds.x as f32 / unit * 17.0),
            _ => None,
        });
        let text = grid.row_text(row.y);
        (
            app.dock.fit(row),
            mark.first().map(|m| m.0 * 17.0),
            name,
            dot,
            text,
        )
    };
    let (mut named, mut whole) = (0, 0);
    for width in (16..90).rev() {
        app.dock.width = Some(width);
        app.handle(Event::Resize(150, 44));
        app.dock.active = Some(last);
        let behind = place(&app);
        app.dock.active = Some(0);
        let front = place(&app);
        if behind.0 != Fit::Cut || front.0 != Fit::Cut {
            continue;
        }
        // Where it shows any of its name: cut to its mark it stands in its tab's middle.
        let (Some(was), Some(name)) = (&behind.2, &front.2) else {
            continue;
        };
        let (mark, then) = (behind.1.unwrap(), front.1.unwrap());
        assert!(
            (mark - then).abs() <= 0.5,
            "{width}: its mark at {mark} px behind and {then} in front: {} / {}",
            behind.4,
            front.4
        );
        assert!(
            (was.1 - name.1).abs() <= 0.5,
            "{width}: its name at {} px behind and {} in front",
            was.1,
            name.1
        );
        named += 1;
        if was.0 == name.0 {
            let (dot, then) = (behind.3.unwrap(), front.3.unwrap());
            assert!(
                (dot - then).abs() <= 0.5,
                "{width}: its dot at {dot} px behind and {then} in front"
            );
            whole += 1;
        }
    }
    assert!(
        named > 0 && whole > 0,
        "{named} widths named it, {whole} whole"
    );
}

/// In the light theme a tab behind reads as behind, a file's and the side pane's: its words a
/// step quieter than the one in front's, as VS Code's light theme has them, and still 4.5 to 1
/// on white and on Latte's base. In the light theme's quiet words they were 1.1 to 1 from the
/// front's (0.11.0's release check, m1-agent-tabs-light).
#[test]
#[cfg(unix)]
fn a_tab_behind_reads_as_behind_in_the_light_theme() {
    use crate::theme::Theme;
    let (dir, mut app) = project((150, 44));
    app.theme = Theme::named(Some("light"));
    fs::write(dir.path().join("beta.rs"), "fn main() {}\n").unwrap();
    app.open(&dir.path().join("alpha.txt")).unwrap();
    app.open(&dir.path().join("beta.rs")).unwrap();
    for name in ["make build", "og tags"] {
        app.open_agent(crate::agents::SHELL.1);
        let last = app.dock.sessions.len() - 1;
        app.dock.sessions[last].renamed = Some(name.to_string());
    }
    let grid = screen(&app);
    let theme = *app.theme();
    let colour = |y: u16, name: &str| {
        let found = words(&grid, y).into_iter().find(|w| w.0 == name);
        found.unwrap_or_else(|| panic!("{name}")).3
    };
    let row = title(&app);
    // The pane's: "og tags" in front, "make build" behind; the files': beta.rs in front.
    let pairs = [
        (row.y, "make build", "og tags"),
        (0, "alpha.txt", "beta.rs"),
    ];
    let luminance = |c: cellix_render::Rgb| {
        let channel = |v: u8| {
            let v = f32::from(v) / 255.0;
            if v <= 0.039_28 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * channel(c.0) + 0.7152 * channel(c.1) + 0.0722 * channel(c.2)
    };
    let contrast = |a: cellix_render::Rgb, b: cellix_render::Rgb| {
        let (a, b) = (luminance(a), luminance(b));
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    };
    for (y, behind, front) in pairs {
        let (behind, front) = (colour(y, behind), colour(y, front));
        assert!(
            contrast(behind, front) >= 1.4,
            "{behind:?} behind and {front:?} in front"
        );
        for ground in [cellix_render::Rgb(255, 255, 255), theme.tab.bg] {
            assert!(contrast(behind, ground) >= 4.5, "{behind:?} on {ground:?}");
        }
    }
}

/// What a tab behind shows of its name, painted in its columns, if any.
fn shown_name(app: &App, grid: &Grid, row: cellix_render::Rect, index: usize) -> Option<String> {
    let (from, to) = tab_span(app, row, index);
    let found = words(grid, row.y)
        .into_iter()
        .find(|w| w.1 >= f32::from(from) && w.1 < f32::from(to));
    found.map(|w| w.0)
}

/// In a row too short for every tab whole, those behind the one in front share the room
/// evenly, as VS Code's shrink mode does: each shows the same number of letters, as many as
/// fit, and never fewer than three; one whose whole name is shorter keeps it; no room stands
/// empty after the + while a name is cut; and none goes to its mark while another shows any of
/// its name, so where three letters each have no room, all go to their marks together. Gemini
/// CLI stood at its mark while Claude Code kept "pricing p…", and then "pri…" with the room it
/// freed left after the + (0.11.0's release check, R3, m1-agent-tabs; the user's choice). The
/// least was one letter, and "p…" and "pr…" said nothing a mark doesn't (the release review of
/// 0.12.0, 14-agents, m1-agent-tabs): this test let a name cut to one letter pass.
#[test]
#[cfg(unix)]
fn tabs_behind_share_the_room_evenly() {
    let (_dir, mut app) = project((150, 44));
    let names = [
        "the documentation's navigation",
        "docs",
        "pricing page and refunds",
        "og tags",
    ];
    for name in names {
        app.open_agent(crate::agents::SHELL.1);
        let last = app.dock.sessions.len() - 1;
        app.dock.sessions[last].renamed = Some(name.to_string());
    }
    app.dock.active = Some(3);
    app.dock.front = Front::Session;
    let (mut alike, mut whole, mut marks) = (0, 0, 0);
    for width in (16..100).rev() {
        app.dock.width = Some(width);
        app.handle(Event::Resize(150, 44));
        let grid = screen(&app);
        let row = title(&app);
        if app.dock.fit(row) != Fit::Cut {
            continue;
        }
        let text = grid.row_text(row.y);
        let shown: Vec<Option<String>> = (0..3).map(|i| shown_name(&app, &grid, row, i)).collect();
        // None at its mark while another shows any of its name: all of them or none.
        let at_marks = shown.iter().filter(|name| name.is_none()).count();
        assert!(
            at_marks == 0 || at_marks == shown.len(),
            "{width}: a tab at its mark beside a name: {text}"
        );
        if at_marks > 0 {
            marks += 1;
            continue;
        }
        let shown: Vec<&String> = shown.iter().flatten().collect();
        let letters = |name: &str| name.trim_end_matches('…').chars().count();
        let cut: Vec<usize> = (shown.iter())
            .filter(|name| name.ends_with('…'))
            .map(|name| letters(name))
            .collect();
        if cut.is_empty() {
            continue;
        }
        // The same number of letters each, but where a name's cut falls on a space, which
        // goes with it and leaves that one a letter fewer ("the documentation's…").
        let (least, most) = (*cut.iter().min().unwrap(), *cut.iter().max().unwrap());
        let spaced = (shown.iter().zip(names)).any(|(name, full)| {
            let kept = letters(name);
            name.ends_with('…') && full.chars().nth(kept) == Some(' ')
        });
        assert!(
            most == least || (spaced && most == least + 1),
            "{width}: {shown:?}: {text}"
        );
        // Three letters at the least, or two where the third is a space, which goes with the
        // cut ("og…").
        for (name, full) in shown.iter().zip(names) {
            let kept = letters(name);
            let spaced = full.chars().nth(kept) == Some(' ');
            assert!(
                !name.ends_with('…') || kept >= 3 || (spaced && kept >= 2),
                "{width}: {name} of {full}: {text}"
            );
        }
        // A name no wider than that cut, its "…" and a column it rounds to stays whole: "docs"
        // takes no more room whole than cut to "do…".
        let font = Font::default();
        let widest = (shown.iter())
            .filter(|name| name.ends_with('…'))
            .map(|name| measure().width(name, font))
            .fold(0.0, f32::max);
        for (name, full) in shown.iter().zip(names) {
            if !name.ends_with('…') {
                assert_eq!(name.as_str(), full, "{width}: {text}");
                assert!(
                    measure().width(full, font) <= widest + 1.0,
                    "{width}: {text}"
                );
                whole += 1;
            }
        }
        // No room after the + while a name is cut: the tabs take all of it, up to the button
        // that maximizes the pane, or where that gave way, up to its place.
        let plus = app.dock.plus_x(row);
        let button = app.dock.maximize_button(row);
        assert!(
            plus + 3 >= button,
            "{width}: the + at {plus}, the button at {button}: {text}"
        );
        if cut.len() >= 2 {
            alike += 1;
        }
    }
    assert!(
        alike > 3 && whole > 0 && marks > 0,
        "{alike} widths cut alike, {whole} names whole, {marks} all at their marks"
    );
}

/// A first tab cut to its mark has the room it needs for its mark to stand in the middle
/// between the sheet's line just before it and its own end, and for its chip under the
/// pointer to keep 8 px from that line: in three columns it stood 22 px from the line and 14
/// from its end, and centred, its chip came 4 px from the line (0.11.0's release review, A4,
/// 131, 173-peek-narrow; the user's choice, a wider tab).
#[test]
#[cfg(unix)]
fn a_first_tab_cut_to_its_mark_stands_in_the_middle_of_its_room() {
    let (dir, mut app) = project((150, 44));
    app.open_agent(crate::agents::SHELL.1);
    app.dock.sessions[0].renamed = Some("make build".to_string());
    start_agent(dir.path(), &mut app, "✳ pricing page");
    app.dock.active = Some(1);
    app.dock.front = Front::Session;
    let theme = *app.theme();
    let mut seen = 0;
    for width in (16..60).rev() {
        app.dock.width = Some(width);
        app.handle(Event::Resize(150, 44));
        mouse(&mut app, MouseKind::Move, 0, 40);
        let grid = screen(&app);
        let row = title(&app);
        if app.dock.fit(row) != Fit::Cut || shown_name(&app, &grid, row, 0).is_some() {
            continue;
        }
        let (from, to) = tab_span(&app, row, 0);
        // Where the row has the column for it: where it hasn't, every tab staying in the row
        // comes first, and the mark stands in the middle of its three.
        if to - from < 4 {
            continue;
        }
        // The sheet's line, half way into the column before the tabs, two pixels thick.
        let area = app.dock_area().unwrap();
        let line = (f32::from(area.x) + 0.5) * 17.0 + 2.0;
        let mark = squares(&grid, row.y, Some(Icon::Terminal))
            .into_iter()
            .find(|m| m.0 >= f32::from(from) - 1.0 && m.0 < f32::from(to))
            .expect("its mark");
        // The prompt's ink starts 2.4 of its 24 in, and ends 2.4 before its box does.
        let ink = 2.4 / 24.0 * (mark.1 - mark.0);
        let (before, after) = (
            (mark.0 + ink) * 17.0 - line,
            f32::from(to) * 17.0 - (mark.1 - ink) * 17.0,
        );
        assert!(
            (before - after).abs() <= 2.0,
            "{width}: {before} px from the line to its mark, {after} from it to its end"
        );
        // Under the pointer, its chip 8 px or more from the line.
        mouse(&mut app, MouseKind::Move, from + 1, row.y);
        let grid = screen(&app);
        let (top, bottom) = (i32::from(row.y) * UNIT, i32::from(row.y + 1) * UNIT);
        let chip = grid.paint.iter().find_map(|paint| match paint {
            Paint::Panel {
                bounds,
                fill: Some(fill),
                ..
            } if *fill == theme.row_hover && bounds.y >= top && bounds.y < bottom => {
                Some(bounds.x as f32 / UNIT as f32 * 17.0)
            }
            _ => None,
        });
        let chip = chip.expect("its chip");
        assert!(
            chip - line >= 8.0,
            "{width}: its chip {} px from the line",
            chip - line
        );
        seen += 1;
    }
    assert!(seen > 0, "some width cuts the first tab to its mark");
}

/// The side pane's terminal's cursor goes while a tip covers its cell: the tip covers the
/// terminal's first row but the top of it, and a sliver of the cursor stood on its top edge
/// (0.11.0's release check, R4, 166-pane-tabs-cut-hover). Without the tip it's there.
#[test]
#[cfg(unix)]
fn a_tip_over_the_terminals_cursor_hides_it() {
    let (dir, mut app) = project((150, 44));
    app.set_measure(Rc::new(
        cellix_render::FontMeasure::new(17, 50).with_type_row(36),
    ));
    app.open_agent(crate::agents::SHELL.1);
    app.dock.sessions[0].renamed = Some("the documentation's navigation".to_string());
    // An agent that prints nothing, so its cursor stays in its first cell.
    let agent = start_agent(dir.path(), &mut app, "✳ pricing page");
    app.dock.active = Some(agent);
    app.dock.front = Front::Session;
    app.dock.focused = true;
    let theme = *app.theme();
    let mut seen = false;
    let mut tried = Vec::new();
    for width in (30..90).rev() {
        app.dock.width = Some(width);
        app.handle(Event::Resize(150, 44));
        // Where the tabs stop is known once the row is drawn.
        let _ = screen(&app);
        let row = title(&app);
        let (from, _) = tab_span(&app, row, 0);
        if app.dock.cut_name(row, from).is_none() {
            tried.push((width, app.dock.fit(row), from));
            continue;
        }
        let screen_area = app.dock_screen().unwrap();
        let cursor = app.dock.sessions[agent].cursor_in(screen_area, true);
        let (x, y) = cursor.expect("its cursor, in its first cell");
        mouse(&mut app, MouseKind::Move, 0, 40);
        let grid = screen(&app);
        assert_eq!(grid.cell(x, y).style.bg, theme.text.fg, "the cursor shows");
        // Under the pointer, the tab cut short has its name in a tip under it.
        mouse(&mut app, MouseKind::Move, from + 1, row.y);
        let grid = screen(&app);
        let tip = grid.paint.iter().any(|paint| {
            matches!(paint, Paint::Panel { shadow: true, fill: Some(fill), .. }
                if *fill == theme.popup)
        });
        assert!(tip, "{width}: a tip under the tab");
        let cell = grid.cell(x, y);
        assert_eq!(
            (cell.style.fg, cell.style.bg),
            (theme.text.fg, theme.sidebar.bg),
            "{width}: the cursor under the tip goes"
        );
        seen = true;
        break;
    }
    assert!(seen, "some width cuts the first tab short: {tried:?}");
}

/// A stand-in that names itself and then prints `text`, and waits.
fn printing_stand_in(dir: &Path, title: &str, text: &str) -> String {
    let fake = dir.join("printing-agent");
    let script =
        format!("#!/bin/sh\nprintf '\\033]0;{title}\\007'\nprintf '{text}'\nexec sleep 30\n");
    fs::write(&fake, script).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&fake, fs::Permissions::from_mode(0o755)).unwrap();
    }
    fake.display().to_string()
}

/// Starts the printing stand-in in the side pane, in front, and waits until its first row
/// says `shows`.
fn start_printing(dir: &Path, app: &mut App, text: &str, shows: char) -> usize {
    app.agent_program = Some(printing_stand_in(dir, "✳ pricing page", text));
    app.open_agent("claude");
    let index = app.dock.sessions.len() - 1;
    app.dock.active = Some(index);
    app.dock.front = Front::Session;
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let area = app.dock_screen().expect("the pane is open");
        if screen(app).row_text(area.y).contains(shows) {
            return index;
        }
        assert!(Instant::now() < deadline, "the stand-in never printed");
        app.poll();
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// The tour's terminal, rows of 50 px with words set for 36.
fn tour_rows(app: &mut App) {
    app.set_measure(Rc::new(
        cellix_render::FontMeasure::new(17, 50).with_type_row(36),
    ));
}

/// A tip's rounded corners show the ground past them, never a piece of a letter under their
/// square: the prompt's "$" showed a pixel at the corner of the tip under a cut tab (166). The
/// program in the pane fills its first rows with full blocks, so a letter is under every
/// corner, and the picture is read in each corner, past the arc.
#[test]
#[cfg(unix)]
fn a_tips_round_corners_cover_the_letters_under_them() {
    let (dir, mut app) = project((150, 44));
    tour_rows(&mut app);
    app.open_agent(crate::agents::SHELL.1);
    app.dock.sessions[0].renamed = Some("the documentation's navigation".to_string());
    start_printing(dir.path(), &mut app, &"█".repeat(600), '█');
    let theme = *app.theme();
    let mut seen = false;
    for width in (30..90).rev() {
        app.dock.width = Some(width);
        app.handle(Event::Resize(150, 44));
        let _ = screen(&app);
        let row = title(&app);
        let (from, _) = tab_span(&app, row, 0);
        if app.dock.cut_name(row, from).is_none() {
            continue;
        }
        mouse(&mut app, MouseKind::Move, from + 1, row.y);
        let grid = screen(&app);
        let tip = grid.paint.iter().find_map(|paint| match paint {
            Paint::Panel {
                bounds,
                shadow: true,
                fill: Some(fill),
                inset,
                ..
            } if *fill == theme.popup => Some((*bounds, *inset)),
            _ => None,
        });
        let (tip, exact) = tip.unwrap_or_else(|| panic!("{width}: a tip under the cut tab"));
        // Where the rasterizer stands it: exactly where it's placed (`Grid::exact_popup`), or a
        // control's height, on whole pixels.
        let unit = UNIT as f32;
        let (top, high) = if exact {
            (tip.y as f32 * 50.0 / unit, tip.h as f32 * 50.0 / unit)
        } else {
            cellix_render::stood(tip.y, tip.h, 50.0, 36.0)
        };
        let left = (tip.x as f32 * 17.0 / unit).round();
        let right = ((tip.x + tip.w) as f32 * 17.0 / unit).round();
        let (top, bottom) = (top.round(), (top + high).round());
        // The rows of the picture it stands on, from the first.
        let first = (top / 50.0) as u16;
        let last = ((bottom - 1.0) / 50.0) as u16;
        let rows: Vec<Vec<cellix_render::Rgb>> = (first..=last)
            .flat_map(|y| super::strip_tests::picture(&grid, y))
            .collect();
        let pixel = |x: f32, y: f32| rows[(y as usize) - first as usize * 50][x as usize];
        let bright = |c: cellix_render::Rgb| c.0.max(c.1).max(c.2) > 128;
        // The blocks are there, beside the tip.
        assert!(
            bright(pixel(right + 30.0, top + 15.0)),
            "{width}: a block beside the tip"
        );
        let reach = 24.0 * 36.0 / 100.0;
        let corners = [
            (left, top, left + reach, top + reach),
            (right - reach, top, right - reach, top + reach),
            (left, bottom - reach, left + reach, bottom - reach),
            (right - reach, bottom - reach, right - reach, bottom - reach),
        ];
        for (x0, y0, cx, cy) in corners {
            for y in y0 as i32..(y0 + reach) as i32 {
                for x in x0 as i32..(x0 + reach) as i32 {
                    let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                    let past = ((px - cx).powi(2) + (py - cy).powi(2)).sqrt() > reach + 1.0;
                    if past {
                        let colour = pixel(x as f32, y as f32);
                        assert!(
                            !bright(colour),
                            "{width}: ({x}, {y}) past the corner at ({cx}, {cy}) is {colour:?}"
                        );
                    }
                }
            }
        }
        seen = true;
        break;
    }
    assert!(seen, "some width cuts the first tab short");
}

/// The program in the side pane stands clear of the line under the pane's tabs: a cell is its
/// row's whole height, so a cursor or a coloured cell on the first row stood on the line (58,
/// the final review's T8). Where a row has room, the first row's top is the pane's ground
/// `LINE_AIR` past the line. The cells are as they were, and on cells nothing is painted.
#[test]
#[cfg(unix)]
fn the_panes_program_stands_clear_of_the_line_under_its_tabs() {
    let (dir, mut app) = project((150, 44));
    tour_rows(&mut app);
    // Its first row in reverse video, coloured the row's whole height, and its cursor after.
    start_printing(dir.path(), &mut app, "\\033[7mINVERSE\\033[0m", 'I');
    app.dock.focused = true;
    let theme = *app.theme();
    let area = app.dock_screen().expect("the pane is open");
    let grid = screen(&app);
    let at = grid
        .row_text(area.y)
        .find("INVERSE")
        .expect("the program's words");
    let x = grid.row_text(area.y)[..at].chars().count() as u16;
    assert_eq!(
        grid.cell(x, area.y).style.bg,
        theme.text.fg,
        "a coloured cell"
    );
    assert_eq!(
        grid.cell(x + 7, area.y).style.bg,
        theme.text.fg,
        "the cursor"
    );
    let picture = super::strip_tests::picture(&grid, area.y);
    // The line under the tabs is the row's top 2 px; 8 px of the pane's ground under it, the
    // cells' colour after that.
    let columns = usize::from(x) * 17..usize::from(x + 8) * 17;
    for (y, pixels) in picture.iter().enumerate().take(12).skip(2) {
        let (want, said) = match y {
            10 => continue,
            11 => (theme.text.fg, "the cell's colour under the inset"),
            _ => (theme.sidebar.bg, "the pane's ground"),
        };
        for (column, pixel) in pixels[columns.clone()].iter().enumerate() {
            assert_eq!(*pixel, want, "({}, {y}) is {said}", columns.start + column);
        }
    }
    // Where there's no air to give, nothing is painted over the program.
    app.set_measure(Rc::new(cellix_render::CellMeasure));
    let grid = screen(&app);
    let fills = grid.paint.iter().any(|paint| {
        matches!(paint, Paint::Image { bounds, .. } if bounds.y == i32::from(area.y) * UNIT)
    });
    assert!(!fills, "nothing over the program on cells");
}

/// The air over and under a tip's words in the picture, as the pixels from the line over
/// them to their ink's top, and from their ink's foot to the line under them: the last tip
/// painted, drawn exactly where it's placed (`Grid::exact_popup`), its lines on the first
/// and last of its pixels.
fn tip_air(app: &App, grid: &Grid) -> (i32, i32) {
    let popup = app.theme().popup;
    let tip = (grid.paint.iter()).rev().find_map(|paint| match paint {
        Paint::Panel {
            bounds,
            shadow: true,
            inset: true,
            fill: Some(fill),
            ..
        } if *fill == popup => Some(*bounds),
        _ => None,
    });
    let tip = tip.expect("a tip, drawn where it's placed");
    let pixels = |units: i32, size: f32| (units as f32 * size / UNIT as f32).round() as i32;
    let (top, foot) = (pixels(tip.y, 50.0), pixels(tip.y + tip.h, 50.0) - 1);
    let first = (top / 50) as u16;
    let rows: Vec<Vec<cellix_render::Rgb>> = (first..=(foot / 50) as u16)
        .flat_map(|row| super::strip_tests::picture(grid, row))
        .collect();
    let at = |y: i32| &rows[(y - i32::from(first) * 50) as usize];
    // Clear of its round corners.
    let (left, right) = (pixels(tip.x, 17.0) + 12, pixels(tip.x + tip.w, 17.0) - 12);
    let inked: Vec<i32> = (top + 1..foot)
        .filter(|y| {
            at(*y)[left as usize..right as usize]
                .iter()
                .any(|p| super::strip_tests::apart(*p, popup))
        })
        .collect();
    let (ink_top, ink_foot) = (inked[0], inked[inked.len() - 1]);
    (ink_top - top - 1, foot - ink_foot - 1)
}

/// A tip's words keep 8 px of air over and under their ink, ascenders, brackets and tails, in
/// the middle of its box: as a control's 39 px a tip held them 7 px under its line and 6 over
/// its foot (the release review of 0.12.0, 64-transcript-tab-hover, 13-quit-hover). Quit's
/// over the bar at the foot, raised to the top of its row, and a pane tab's under the tab
/// row's line, over a terminal, measured in the picture.
#[test]
#[cfg(unix)]
fn a_tips_words_keep_eight_pixels_over_and_under_them() {
    let (dir, mut app) = project((150, 44));
    app.set_measure(Rc::new(
        cellix_render::FontMeasure::new(17, 50).with_type_row(36),
    ));
    let check = |app: &App, what: &str| {
        let (over, under) = tip_air(app, &screen(app));
        assert!(
            over >= 8 && under >= 8 && over.abs_diff(under) <= 1,
            "{what}: {over} px over its words and {under} under them"
        );
    };
    let quit = app.quit_button().expect("Quit");
    mouse(&mut app, MouseKind::Move, quit.x, quit.y);
    check(&app, "Quit");
    app.open_agent(crate::agents::SHELL.1);
    // Brackets, ascenders and tails: the font's whole height of ink.
    app.dock.sessions[0].renamed = Some("the documentation (pricing)".to_string());
    let agent = start_agent(dir.path(), &mut app, "✳ pricing page");
    app.dock.active = Some(agent);
    app.dock.front = Front::Session;
    // The pane's tab "the documentation (pricing)", cut short, has its name in a tip.
    let mut seen = false;
    for width in (30..90).rev() {
        app.dock.width = Some(width);
        app.handle(Event::Resize(150, 44));
        let _ = screen(&app);
        let row = title(&app);
        let (from, _) = tab_span(&app, row, 0);
        if app.dock.cut_name(row, from).is_none() {
            continue;
        }
        mouse(&mut app, MouseKind::Move, from + 1, row.y);
        check(&app, "the pane's tab");
        seen = true;
        break;
    }
    assert!(seen, "some width cuts the first tab short");
}

/// With the button that maximizes the pane right after it, the + stands in the middle of the
/// air from the last tab's ink to that button's, measured in the picture, whether the tabs are
/// whole or cut short: at the start of its columns it stood 39 px after the last tab's × and
/// 34 before the button (0.11.0's release check, m1-agent-tabs), and with a column between
/// them, 41 and 51.
#[test]
#[cfg(unix)]
fn the_plus_stands_in_the_middle_between_the_tabs_and_the_button() {
    let (dir, mut app) = project((150, 44));
    start_agent(dir.path(), &mut app, "✳ pricing page");
    app.serve(working());
    app.turn_at = Some(0);
    for name in ["documentation", "og tags"] {
        app.open_agent(crate::agents::SHELL.1);
        let last = app.dock.sessions.len() - 1;
        app.dock.sessions[last].renamed = Some(name.to_string());
    }
    app.dock.active = Some(2);
    app.dock.front = Front::Session;
    let (mut whole, mut cut) = (0, 0);
    for width in (30..90).rev() {
        app.dock.width = Some(width);
        app.handle(Event::Resize(150, 44));
        let grid = screen(&app);
        let row = title(&app);
        let (plus, button) = (app.dock.plus_x(row), app.dock.maximize_button(row));
        if !app.dock.plus_shows(row) || !app.dock.maximize_shows(row) {
            continue;
        }
        // Where it shows, the button follows the + with a column between at most.
        assert!(
            button <= plus + 4,
            "{width}: the + at {plus}, the button at {button}"
        );
        let (before, after) = plus_air(&app, &grid, row);
        assert!(
            before.abs_diff(after) <= 2,
            "{width}: {before} px before the +, {after} after it, the + at {plus} and the \
             button at {button}: {}",
            grid.row_text(row.y)
        );
        match app.dock.fit(row) {
            Fit::Whole => whole += 1,
            _ => cut += 1,
        }
    }
    assert!(whole > 0 && cut > 0, "{whole} widths whole, {cut} cut");
    // With a whole tab behind last, by what it draws: its ×, shown only under the pointer,
    // has the room the tab keeps, and by that room the + stood 79 px after the tab's ink and
    // 47 before the button (the release review of 0.12.0, 172-agents-helper).
    app.dock.active = Some(0);
    let mut behind = 0;
    for width in (30..90).rev() {
        app.dock.width = Some(width);
        app.handle(Event::Resize(150, 44));
        let grid = screen(&app);
        let row = title(&app);
        if app.dock.fit(row) != Fit::Whole || !app.dock.maximize_shows(row) {
            continue;
        }
        let (before, after) = plus_air(&app, &grid, row);
        assert!(
            before.abs_diff(after) <= 2,
            "{width}: {before} px after og tags, {after} before the button: {}",
            grid.row_text(row.y)
        );
        behind += 1;
    }
    assert!(behind > 0, "some width has every tab whole");
}

/// The air before the + and after it, to the last tab's ink and the button's after it, in
/// pixels, measured in the picture of the row.
fn plus_air(app: &App, grid: &Grid, row: cellix_render::Rect) -> (usize, usize) {
    let theme = *app.theme();
    let (plus, button) = (app.dock.plus_x(row), app.dock.maximize_button(row));
    let rows = super::strip_tests::picture(grid, row.y);
    // Far enough back for a tab behind's name, before the room it keeps for its ×.
    let from = usize::from(plus) * 17 - 85;
    let to = usize::from(button + 3) * 17;
    let runs = super::strip_tests::ink_runs(&rows, theme.tab.bg, (from, to));
    // The + in its columns, the last tab's last ink before it, and the button's after it.
    let own = usize::from(plus) * 17..usize::from(plus + 3) * 17;
    let found = runs.iter().position(|run| own.contains(&run.0));
    let (Some(at), true) = (found, found.is_some_and(|at| at > 0 && at + 1 < runs.len())) else {
        panic!("{runs:?}: {}", grid.row_text(row.y));
    };
    let (tab, sign, maximize) = (runs[at - 1], runs[at], runs[at + 1]);
    (sign.0 - tab.1, maximize.0 - sign.1)
}

/// Agents behind the Agents list's tab, cut to their marks with what each is doing, as the
/// tour's m2-agents-urgency has them: the + stands in the middle between the last one's ink
/// and the button that maximizes the pane, which follows the + where the marks leave room,
/// and the line between two has the same air either side on every one, measured in the
/// picture. The button kept the row's end, 70 px after the + against 39 before it; and the
/// air beside the lines ran 14 to 21 px, by how each tab's columns rounded (the release review
/// of 0.12.0, m2-agents-urgency, 71-sample-changes-row-narrow).
#[test]
#[cfg(unix)]
fn tabs_at_their_marks_have_even_air_and_the_plus_its_middle() {
    use crate::dock::Doing;
    let (dir, mut app) = project((150, 44));
    app.agent_program = Some(stand_in(dir.path(), "agent"));
    let agents = [
        ("claude", "pricing page"),
        ("claude", "release notes"),
        ("gemini", "docs nav"),
        ("codex", "og tags"),
    ];
    for (agent, name) in agents {
        app.open_agent(agent);
        let last = app.dock.sessions.len() - 1;
        app.dock.sessions[last].renamed = Some(name.to_string());
    }
    app.dock.sessions[0].doing = Doing::Working;
    app.dock.sessions[2].done_unseen = true;
    app.dock.sessions[3].doing = Doing::NeedsYou;
    app.run(Command::ListAgents);
    let theme = *app.theme();
    let mut seen = 0;
    for width in (20..90).rev() {
        app.dock.width = Some(width);
        app.handle(Event::Resize(150, 44));
        let grid = screen(&app);
        let row = title(&app);
        let text = grid.row_text(row.y);
        if app.dock.fit(row) != Fit::Cut || !app.dock.maximize_shows(row) {
            continue;
        }
        // Every agent behind at its marks.
        if (0..agents.len()).any(|index| shown_name(&app, &grid, row, index).is_some()) {
            continue;
        }
        let (plus, button) = (app.dock.plus_x(row), app.dock.maximize_button(row));
        assert!(
            button <= plus + 4,
            "{width}: the + at {plus}, the button at {button}: {text}"
        );
        let (before, after) = plus_air(&app, &grid, row);
        assert!(
            before.abs_diff(after) <= 2,
            "{width}: {before} px before the +, {after} after it: {text}"
        );
        let lines = super::strip_tests::line_air(&grid, row.y, theme.border, theme.tab.bg);
        assert_eq!(lines.len(), agents.len() - 1, "{width}: {lines:?}: {text}");
        let airs = lines.iter().flat_map(|(before, after)| [*before, *after]);
        let (least, most) = airs.fold((usize::MAX, 0), |(l, m), a| (l.min(a), m.max(a)));
        assert!(
            most - least <= 2 && least >= 12,
            "{width}: the air beside the lines {lines:?}: {text}"
        );
        seen += 1;
    }
    assert!(
        seen > 0,
        "some width cuts them all to their marks with the button"
    );
}

/// Two whole tabs behind the one in front have the line between them in the middle of the
/// air from the first one's ink to the next one's mark's, as two files' tabs do, measured in
/// the picture: at their edge, it stood after the room the first keeps for its ×, about 40 px
/// from its name and 17 from the next mark.
#[test]
#[cfg(unix)]
fn the_line_between_two_whole_tabs_behind_stands_in_their_air() {
    let (_dir, mut app) = project((150, 44));
    for name in ["make build", "Terminal"] {
        app.open_agent(crate::agents::SHELL.1);
        let last = app.dock.sessions.len() - 1;
        app.dock.sessions[last].renamed = Some(name.to_string());
    }
    app.run(Command::ShowComments);
    app.dock.width = Some(80);
    app.handle(Event::Resize(150, 44));
    let grid = screen(&app);
    let row = title(&app);
    assert_eq!(app.dock.fit(row), Fit::Whole, "{}", grid.row_text(row.y));
    let theme = *app.theme();
    let lines = super::strip_tests::line_air(&grid, row.y, theme.border, theme.tab.bg);
    assert_eq!(lines.len(), 1, "{lines:?}: {}", grid.row_text(row.y));
    let (before, after) = lines[0];
    eprintln!("whole pane tabs: {before} / {after}");
    assert!(
        before.abs_diff(after) <= 2,
        "{before} px after make build's ink, {after} before Terminal's mark's"
    );
}
