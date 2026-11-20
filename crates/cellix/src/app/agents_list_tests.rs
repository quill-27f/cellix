//! The Agents list by urgency, driven through the app and read off the drawn grid.

use std::fs;
use std::time::{Duration, Instant};

use cellix_agent::{AgentEvent, AgentState, Request};
use cellix_input::{Event, Mods, MouseEvent, MouseKind};
use cellix_render::Grid;
use tempfile::TempDir;

use super::App;

fn project() -> (TempDir, App) {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("alpha.txt"), "first line\n").unwrap();
    let app = App::new(dir.path().to_path_buf(), (130, 36));
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

/// The Agents list by urgency: who needs you, then who's done and not yet looked at, then
/// who's working, then who's idle, each section with how many are in it and none that's
/// empty; a row is the agent's mark, its name, how long since what it's doing changed, and
/// that.
#[test]
fn the_agents_list_puts_who_needs_you_first() {
    let (_dir, mut app) = project();
    app.turn_at = Some(2);
    let event = |session: &str, agent: &str, state: AgentState, detail: &str| {
        Request::Agent(AgentEvent {
            session: session.into(),
            agent: agent.into(),
            state,
            detail: detail.into(),
            folder: "/w".into(),
            pid: None,
            transcript: String::new(),
            name: String::new(),
            prompt: String::new(),
        })
    };
    // Each with a tab here: the list shows no agent running outside Cellix.
    for (agent, session) in [
        ("Claude Code", "a"),
        ("Codex", "b"),
        ("Gemini", "c"),
        ("Codex", "d"),
    ] {
        app.tab_for(agent, session);
    }
    app.serve(event("a", "Claude Code", AgentState::Working, "Working"));
    app.serve(event("b", "Codex", AgentState::Working, "Working"));
    app.serve(event("b", "Codex", AgentState::Idle, "Finished"));
    app.serve(event("c", "Gemini", AgentState::NeedsYou, "Allow Bash?"));
    app.serve(event("d", "Codex", AgentState::Idle, "Ready"));
    app.run(cellix_input::Command::ListAgents);
    let grid = screen(&app);
    let text = grid.text();
    let lines: Vec<&str> = text.lines().collect();
    let at = |words: &str| {
        (lines.iter().position(|line| line.contains(words)))
            .unwrap_or_else(|| panic!("no {words}: {text}"))
    };
    let (needs, done, working, idle) = (
        at("NEEDS YOU 1"),
        at("DONE 1"),
        at("WORKING 1"),
        at("IDLE 1"),
    );
    assert!(needs < done && done < working && working < idle, "{text}");
    // What it's asking is on the row under its own; its chip says it needs you.
    assert!(lines[needs + 1].contains("✦ Gemini CLI") && lines[needs + 1].contains("needs you"));
    assert!(lines[needs + 2].contains("Allow Bash?"), "{text}");
    assert!(lines[needs + 1].contains("<1m"), "how long since: {text}");
    assert!(lines[done + 1].contains("⬡ Codex") && lines[done + 1].contains('●'));
    assert!(lines[working + 1].contains("✻ Claude Code"));
    assert!(
        lines[working + 1].contains(cellix_render::spinner::FRAMES[2]),
        "turning: {text}"
    );
    assert!(lines[idle + 1].contains("Codex") && !lines[idle + 1].contains('●'));
    // A section with nobody in it isn't there.
    app.serve(event("c", "Gemini", AgentState::Working, "Working"));
    let text = screen(&app).text();
    assert!(
        !text.contains("NEEDS YOU") && text.contains("WORKING 2"),
        "{text}"
    );
    // Done stays done until the agent is looked at: a press on its row brings its tab forward.
    let grid = screen(&app);
    let y = (0..grid.height())
        .find(|y| grid.row_text(*y).contains("⬡ Codex") && grid.row_text(*y).contains('●'));
    let y = y.expect("the done row");
    let row = grid.row_text(y);
    let x = row[..row.find("Codex").unwrap()].chars().count() as u16;
    app.handle(mouse(MouseKind::Left, x, y));
    assert_eq!(app.dock.active, Some(1), "its tab, in front");
    app.run(cellix_input::Command::ListAgents);
    let text = screen(&app).text();
    assert!(!text.contains("DONE") && text.contains("IDLE 2"), "{text}");
}

/// How long ago stands in one column down the rows that have something at their end: each
/// keeps room there as wide as the "needs you" chip, where the row that needs you had its time
/// pushed six columns left of the others'. An idle row has nothing at its end, so its time is
/// its last word and ends where every row's last does, the chip's end: kept in the chip's
/// empty room, it stood 139 px short of every other row's end (the release review of 0.12.0,
/// 14-agents). This test had every row's time in the one column, idle ones too.
#[test]
fn the_agents_lists_times_stand_in_one_column() {
    let (_dir, mut app) = project();
    let event = |session: &str, agent: &str, state: AgentState, detail: &str| {
        Request::Agent(AgentEvent {
            session: session.into(),
            agent: agent.into(),
            state,
            detail: detail.into(),
            folder: "/w".into(),
            pid: None,
            transcript: String::new(),
            name: String::new(),
            prompt: String::new(),
        })
    };
    for (agent, session) in [
        ("Claude Code", "a"),
        ("Codex", "b"),
        ("Gemini", "c"),
        ("Codex", "d"),
    ] {
        app.tab_for(agent, session);
    }
    app.serve(event("a", "Claude Code", AgentState::Working, "Working"));
    app.serve(event("b", "Codex", AgentState::Idle, "Finished"));
    app.serve(event("c", "Gemini", AgentState::NeedsYou, "Allow Bash?"));
    app.serve(event("d", "Codex", AgentState::Idle, "Ready"));
    app.run(cellix_input::Command::ListAgents);
    // On cells: where each row's "<1m" ends, the idle rows' apart, and where the chip ends.
    let grid = screen(&app);
    let (mut busy, mut idle, mut chip) = (Vec::new(), Vec::new(), None);
    // An idle row has nothing after its time; the others their state.
    let is_idle = |row: &str, at: usize| row[at + 3..].trim().is_empty();
    for y in 0..grid.height() {
        let row = grid.row_text(y);
        let Some(at) = row.find("<1m") else { continue };
        if let Some(chip_at) = row.find("needs you") {
            chip = Some(row[..chip_at].chars().count() + "needs you".len());
        }
        let end = row[..at].chars().count() + 3;
        if is_idle(&row, at) {
            idle.push(end)
        } else {
            busy.push(end)
        }
    }
    // Gemini asking, a Codex done unseen and Claude Code working; the other Codex idle.
    assert_eq!((busy.len(), idle.len()), (3, 1), "{}", grid.text());
    assert!(
        busy.windows(2).all(|w| w[0] == w[1]),
        "{busy:?}\n{}",
        grid.text()
    );
    let chip = chip.expect("the chip");
    assert!(
        idle.iter().all(|end| *end == chip),
        "{idle:?} against the chip's end {chip}\n{}",
        grid.text()
    );
    // With pictures: the same right edge for the times of rows with a state, and an idle
    // row's at the end of the chip's.
    app.set_measure(std::rc::Rc::new(cellix_render::FontMeasure::new(17, 37)));
    let grid = screen(&app);
    let unit = cellix_render::UNIT;
    let row_of = |y: i32| grid.row_text((y / unit) as u16);
    let times: Vec<(i32, bool)> = (grid.paint.iter())
        .filter_map(|p| match p {
            cellix_render::Paint::Text { x, y, text, .. } if text == "<1m" => {
                let row = row_of(*y);
                let at = row.find("<1m")?;
                Some((*x, is_idle(&row, at)))
            }
            _ => None,
        })
        .collect();
    assert_eq!(times.len(), 4, "{times:?}");
    let chip_end = (grid.paint.iter())
        .find_map(|p| match p {
            cellix_render::Paint::Panel {
                bounds,
                fill: Some(fill),
                ..
            } if *fill != app.theme().sidebar.bg
                && row_of(bounds.y + bounds.h / 2).contains("needs you")
                && bounds.w < unit * 12
                && bounds.w > unit * 3 =>
            {
                Some(bounds.x + bounds.w)
            }
            _ => None,
        })
        .expect("the chip's wash");
    let busy: Vec<i32> = (times.iter()).filter(|t| !t.1).map(|t| t.0).collect();
    assert!(busy.windows(2).all(|w| w[0] == w[1]), "{times:?}");
    for (x, idle) in times {
        if idle {
            assert!(
                (x - chip_end).abs() <= unit / 17,
                "an idle row's time ends at {x}, the chip at {chip_end}"
            );
        }
    }
}

#[test]
fn how_long_ago_is_said_in_a_few_letters() {
    use crate::agents::ago;
    let now = Instant::now();
    assert_eq!(ago(now), "<1m");
    assert_eq!(ago(now - Duration::from_secs(5 * 60 + 3)), "5m");
    assert_eq!(ago(now - Duration::from_secs(2 * 3600 + 40)), "2h");
    assert_eq!(ago(now - Duration::from_secs(3 * 86_400)), "3d");
}

/// A helper an agent started always stands under it, a step in, whatever it's doing: in a
/// section of its own it read as one more agent (0.11.0's release review, 172). One asking the
/// person brings its agent and the agent's other helpers up to "NEEDS YOU", where agents come
/// in the order they came.
#[test]
fn a_helper_stands_under_the_agent_that_started_it() {
    use crate::agents::{Row, Urgency, ordered};
    let row = |session: &str, urgency, parent: Option<&str>| Row {
        icon: cellix_render::Icon::AgentCodex,
        name: session.to_string(),
        urgency,
        since: None,
        detail: String::new(),
        session: session.to_string(),
        parent: parent.map(str::to_string),
    };
    let rows = vec![
        row("idle", Urgency::Idle, None),
        row("helper", Urgency::NeedsYou, Some("lead")),
        row("lead", Urgency::Working, None),
        row("asking", Urgency::NeedsYou, None),
        row("worker", Urgency::Working, Some("lead")),
    ];
    let order: Vec<(&str, u8)> = ordered(&rows)
        .into_iter()
        .map(|(r, d)| (r.name.as_str(), d))
        .collect();
    assert_eq!(
        order,
        [
            ("lead", 0),
            ("helper", 1),
            ("worker", 1),
            ("asking", 0),
            ("idle", 0)
        ]
    );
    // Answered, the lead stands with the working, its helpers under it whatever they do.
    let mut rows = rows;
    rows[1].urgency = Urgency::Done;
    let order: Vec<(&str, u8)> = ordered(&rows)
        .into_iter()
        .map(|(r, d)| (r.name.as_str(), d))
        .collect();
    assert_eq!(
        order,
        [
            ("asking", 0),
            ("lead", 0),
            ("helper", 1),
            ("worker", 1),
            ("idle", 0)
        ]
    );
}

/// The chip that says an agent needs you is a chip as the design has it, 20/28 of a taller
/// row, 36 px on the tour's, with room under its words' tails, and stands in its row's
/// middle with its words: at 24 px the "y" of "you" reached a pixel short of its foot
/// (0.11.0's final review, m2).
#[test]
fn the_needs_you_chip_has_room_for_its_words_tails() {
    use cellix_render::{FontMeasure, Paint, UNIT, stood};
    let (_dir, mut app) = project();
    let (row, line) = (50.0, 36.0);
    app.set_measure(std::rc::Rc::new(FontMeasure::new(17, 50).with_type_row(36)));
    app.tab_for("Gemini", "c");
    app.serve(Request::Agent(AgentEvent {
        session: "c".into(),
        agent: "Gemini".into(),
        state: AgentState::NeedsYou,
        detail: "Allow Bash?".into(),
        folder: "/w".into(),
        pid: None,
        transcript: String::new(),
        name: String::new(),
        prompt: String::new(),
    }));
    app.run(cellix_input::Command::ListAgents);
    let grid = screen(&app);
    let y = (0..grid.height())
        .find(|y| grid.row_text(*y).contains("needs you") && grid.row_text(*y).contains("Gemini"))
        .expect("the row that needs you");
    let (top, bottom) = (i32::from(y) * UNIT, i32::from(y + 1) * UNIT);
    let attention = app.theme().attention;
    let chip = grid.paint.iter().find_map(|p| match p {
        Paint::Panel {
            bounds,
            fill: Some(fill),
            ..
        } if bounds.y >= top
            && bounds.y < bottom
            && *fill != attention
            && {
                // Its wash is the attention colour's, faint.
                let near = |a: u8, b: u8| a.abs_diff(b) < 120;
                near(fill.0, attention.0) || near(fill.1, attention.1)
            }
            && bounds.w > UNIT * 3 =>
        {
            Some(*bounds)
        }
        _ => None,
    });
    let chip = chip.expect("the chip");
    let (chip_top, chip_high) = stood(chip.y, chip.h, row, line);
    let high = chip_high;
    assert!((34.0..=37.0).contains(&high), "{high} px tall");
    // In its row's middle, where its words are.
    let middle = f32::from(y) * row + row / 2.0;
    assert!(
        (chip_top + high / 2.0 - middle).abs() <= 1.0,
        "its middle at {} where the row's is {middle}",
        chip_top + high / 2.0
    );
    // Its words' ink in its middle, measured in the picture: the "d" as far from its top as
    // the "y"'s tail from its foot. Words stand with their capitals in their row's middle, and
    // had 11 px over them and 7 under (0.11.0's final review, T9).
    let rows = super::strip_tests::picture(&grid, y);
    let (from, to) = (
        (chip.x as f32 / UNIT as f32 * 17.0) as usize + 2,
        ((chip.x + chip.w) as f32 / UNIT as f32 * 17.0) as usize - 2,
    );
    // The picture is the row's own, from its top.
    let top = chip_top - f32::from(y) * row;
    // Its wash, at its middle before its words.
    let wash = rows[(top + high / 2.0) as usize][from];
    let inked: Vec<usize> = (top.ceil() as usize + 1..(top + high) as usize - 1)
        // Clear of its round corners.
        .filter(|line| {
            rows[*line][from + 10..to - 10]
                .iter()
                .any(|p| super::strip_tests::apart(*p, wash))
        })
        .collect();
    let (ink_top, ink_foot) = (inked[0] as f32, inked[inked.len() - 1] as f32 + 1.0);
    let (over, under) = (ink_top - top, top + high - ink_foot);
    assert!(
        (over - under).abs() <= 1.5,
        "{over} px over its words' ink and {under} under it"
    );
}

/// A step that failed, marked `==failed==` as a transcript has it, is a chip in the failed
/// colour's tint, its word in that colour with 12 px either side: it was a highlighter's
/// amber, 2 px past the word (0.11.0's final review, T9, 65-67).
#[test]
fn a_failed_step_is_a_chip_of_the_failed_colour() {
    use cellix_render::{FontMeasure, Paint, UNIT};
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("transcript.md");
    fs::write(&page, "- Bash `cargo test orders` · ==failed==\n").unwrap();
    let mut app = App::new(dir.path().to_path_buf(), (130, 36));
    app.set_measure(std::rc::Rc::new(FontMeasure::new(17, 50).with_type_row(36)));
    app.open(&page).unwrap();
    let grid = screen(&app);
    let theme = *app.theme();
    let tint = theme.failed_tint();
    let chip = grid.paint.iter().find_map(|p| match p {
        Paint::Panel {
            bounds,
            fill: Some(fill),
            ..
        } if *fill == tint => Some(*bounds),
        _ => None,
    });
    let chip = chip.expect("a chip in the failed colour's tint");
    assert!(
        !grid.paint.iter().any(|p| matches!(p,
            Paint::Panel { fill: Some(fill), .. } if *fill == theme.highlighter())),
        "no highlighter's amber"
    );
    let word = grid.paint.iter().find_map(|p| match p {
        Paint::Text { x, text, color, .. } if text.trim() == "failed" => {
            Some((*x, text.clone(), *color))
        }
        _ => None,
    });
    let (x, text, color) = word.expect("the word, painted on its own");
    assert_eq!(color, theme.deleted(), "in the failed colour");
    let px = |units: i32| units as f32 / UNIT as f32 * 17.0;
    let measure = FontMeasure::new(17, 50).with_type_row(36);
    let wide = cellix_render::Measure::width(&measure, &text, cellix_render::Font::default());
    let lead = text.len() - text.trim_start().len();
    let spaces =
        cellix_render::Measure::width(&measure, &text[..lead], cellix_render::Font::default());
    let start = px(x) + spaces * 17.0;
    let end = px(x) + wide * 17.0
        - cellix_render::Measure::width(
            &measure,
            &text[text.trim_end().len()..],
            cellix_render::Font::default(),
        ) * 17.0;
    let (before, after) = (start - px(chip.x), px(chip.x + chip.w) - end);
    assert!(
        (11.0..=13.0).contains(&before) && (11.0..=13.0).contains(&after),
        "{before} px before the word, {after} after it"
    );
    // On cells, where nothing is painted, the word stands on the tint itself.
    let mut cells = App::new(dir.path().to_path_buf(), (130, 36));
    cells.open(&page).unwrap();
    let grid = screen(&cells);
    let y = (0..grid.height())
        .find(|y| grid.row_text(*y).contains("failed"))
        .expect("its row");
    let at = grid.row_text(y).find("failed").unwrap();
    let column = grid.row_text(y)[..at].chars().count() as u16;
    assert_eq!(grid.cell(column, y).style.bg, cells.theme().failed_tint());
}

/// A line that parts a list in the side pane runs from where its rows start to where they
/// end, as far in from the pane's right edge as from its left, measured in what's painted:
/// the Agents list's under its agents, beside START's "shell" and a "needs you" chip, and the
/// Inbox's when it's empty. They ended 6 to 9 px short of the rows, two columns in from the
/// pane's edge where the rows' ends are a column and a half in (the release review of 0.12.0,
/// 14-agents, 51-agent-copilot-asks, 51-inbox-empty).
#[test]
fn a_lists_lines_end_where_its_rows_end() {
    use cellix_render::{FontMeasure, Paint, UNIT};
    let (_dir, mut app) = project();
    app.set_measure(std::rc::Rc::new(FontMeasure::new(17, 50).with_type_row(36)));
    app.tab_for("Gemini", "c");
    app.serve(Request::Agent(AgentEvent {
        session: "c".into(),
        agent: "Gemini".into(),
        state: AgentState::NeedsYou,
        detail: "Allow Bash?".into(),
        folder: "/w".into(),
        pid: None,
        transcript: String::new(),
        name: String::new(),
        prompt: String::new(),
    }));
    app.run(cellix_input::Command::ListAgents);
    let px = |units: i32| units as f32 / UNIT as f32 * 17.0;
    let border = app.theme().border;
    let area = app.dock_area().expect("the pane");
    let (pane_left, pane_right) = (
        (f32::from(area.x) + app.pane_edge()) * 17.0,
        f32::from(area.right()) * 17.0,
    );
    // The lines across the pane under its tabs, as (start, end) in pixels.
    let lines = |grid: &Grid| -> Vec<(f32, f32)> {
        (grid.paint.iter())
            .filter_map(|p| match p {
                Paint::Hairline { bounds, color }
                    if *color == border
                        && bounds.h == 0
                        && bounds.y >= i32::from(area.y + 1) * UNIT =>
                {
                    Some((px(bounds.x), px(bounds.x + bounds.w)))
                }
                _ => None,
            })
            .filter(|(start, end)| {
                end - start > 17.0 * 10.0 && *start > pane_left && *end < pane_right
            })
            .collect()
    };
    let grid = screen(&app);
    let found = lines(&grid);
    let rule = *found
        .last()
        .unwrap_or_else(|| panic!("the line under START: {found:?}"));
    // Where the rows end: START's "shell", and the chip that says Gemini needs you.
    let shell = (grid.paint.iter())
        .find_map(|p| match p {
            Paint::Text { x, text, .. } if text == "shell" => Some(px(*x)),
            _ => None,
        })
        .expect("shell");
    assert!(
        (rule.1 - shell).abs() < 0.5,
        "the line ends at {}, shell at {shell}",
        rule.1
    );
    let (left, right) = (rule.0 - pane_left, pane_right - rule.1);
    assert!(
        (left - right).abs() <= 1.0,
        "{left} px from the pane's left to the line, {right} from its end to the right"
    );
    // The Inbox: Gemini asking is in it, its age ending where the Agents list's rows end.
    app.run(cellix_input::Command::ShowInbox);
    let grid = screen(&app);
    let small = cellix_render::Font {
        size: crate::threads::SMALL,
        ..cellix_render::Font::default()
    };
    let (age, room) = (grid.paint.iter())
        .find_map(|p| match p {
            Paint::Text {
                x, text, max_width, ..
            } if text == "now" => {
                let wide = cellix_render::Measure::width(&*app.measure, text, small) * 17.0;
                Some((px(*x) + wide, px(*x + *max_width)))
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("the asking agent's age: {}", grid.text()));
    assert!(
        (age - shell).abs() < 0.5,
        "its age ends at {age}, the rows' end at {shell}"
    );
    // And all of it is painted: stopped at the cells' end, "now" lost its last letter.
    assert!(room >= age - 0.5, "painted up to {room}, its end at {age}");
    // Empty, its line: the same.
    let (_dir, mut app) = project();
    app.set_measure(std::rc::Rc::new(FontMeasure::new(17, 50).with_type_row(36)));
    app.run(cellix_input::Command::ShowInbox);
    let grid = screen(&app);
    let found = lines(&grid);
    let rule = *found
        .last()
        .unwrap_or_else(|| panic!("the Inbox's line: {found:?}"));
    let (left, right) = (rule.0 - pane_left, pane_right - rule.1);
    assert!(
        (left - right).abs() <= 1.0,
        "the Inbox: {left} px from the pane's left to the line, {right} from its end"
    );
}

/// The words on a chip of their own colour's wash read at 4.5 to 1 or better on it, in both
/// themes and on a light terminal's white: in the light theme "needs you" read at 4.28 and a
/// transcript's "failed" at 4.4 (the release review of 0.12.0, m2-agents-urgency-light and
/// 66-transcript-light). Read off what's painted: the chip's fill and its words' colour.
#[test]
fn a_chips_words_read_at_four_and_a_half_to_one_in_every_theme() {
    use crate::theme::{Theme, contrast};
    use cellix_render::{FontMeasure, Paint, Rgb};
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("transcript.md");
    fs::write(&page, "- Bash `cargo test orders` · ==failed==\n").unwrap();
    let themes = [
        Theme::named(Some("dark")),
        Theme::named(Some("light")),
        Theme::named(Some("light")).on_background(Rgb(255, 255, 255)),
    ];
    // The colour of the painted words `word`, and the fill of the panel under them.
    let chip = |grid: &Grid, word: &str| {
        let (x, color) = (grid.paint.iter())
            .find_map(|p| match p {
                Paint::Text { x, text, color, .. } if text.trim() == word => Some((*x, *color)),
                _ => None,
            })
            .unwrap_or_else(|| panic!("{word}, painted"));
        // Inside the word, past any spaces it starts with.
        let x = x + cellix_render::UNIT * 3 / 2;
        let fill = (grid.paint.iter()).rev().find_map(|p| match p {
            Paint::Panel {
                bounds,
                fill: Some(fill),
                ..
            } if bounds.x <= x
                && x < bounds.x + bounds.w
                && bounds.w < 50 * cellix_render::UNIT =>
            {
                Some(*fill)
            }
            _ => None,
        });
        (color, fill.unwrap_or_else(|| panic!("{word}'s chip")))
    };
    for theme in themes {
        let measure = std::rc::Rc::new(FontMeasure::new(17, 50).with_type_row(36));
        let mut app = App::new(dir.path().to_path_buf(), (130, 36));
        app.set_measure(measure.clone());
        app.theme = theme;
        app.open(&page).unwrap();
        let (ink, ground) = chip(&screen(&app), "failed");
        assert_eq!(ground, theme.failed_tint(), "the failed colour's tint");
        let ratio = contrast(ink, ground);
        assert!(ratio >= 4.5, "failed: {ink:?} on {ground:?} is {ratio}");
        let (_dir, mut app) = project();
        app.set_measure(measure);
        app.theme = theme;
        app.tab_for("Gemini", "c");
        app.serve(Request::Agent(AgentEvent {
            session: "c".into(),
            agent: "Gemini".into(),
            state: AgentState::NeedsYou,
            detail: "Allow Bash?".into(),
            folder: "/w".into(),
            pid: None,
            transcript: String::new(),
            name: String::new(),
            prompt: String::new(),
        }));
        app.run(cellix_input::Command::ListAgents);
        let (ink, ground) = chip(&screen(&app), "needs you");
        let ratio = contrast(ink, ground);
        assert!(ratio >= 4.5, "needs you: {ink:?} on {ground:?} is {ratio}");
        // Still the attention colour's, only as much deeper as it takes.
        assert!(
            contrast(ink, theme.attention) < 1.3,
            "{ink:?} against {:?}",
            theme.attention
        );
    }
}

/// A row's end is as far in from the pane's right edge as its mark is from its left, so the
/// band under the pointer has as much air round what it holds on both sides, measured in the
/// picture: what stood at the row's end ended 36 px from the band's end, against 15 from its
/// start to the mark (0.11.0's review of `next`, m2-agents-urgency-hover).
#[test]
fn a_rows_band_has_the_same_air_either_side_of_what_it_holds() {
    use cellix_render::FontMeasure;
    let (_dir, mut app) = project();
    app.set_measure(std::rc::Rc::new(FontMeasure::new(17, 50).with_type_row(36)));
    let event = |state: AgentState, detail: &str| {
        Request::Agent(AgentEvent {
            session: "c".into(),
            agent: "Gemini".into(),
            state,
            detail: detail.into(),
            folder: "/w".into(),
            pid: None,
            transcript: String::new(),
            name: String::new(),
            prompt: String::new(),
        })
    };
    // Done behind another tab, so its row ends in the green dot.
    app.tab_for("Gemini", "c");
    app.tab_for("Codex", "d");
    app.serve(event(AgentState::Working, "Working"));
    app.serve(event(AgentState::Idle, "Finished"));
    app.run(cellix_input::Command::ListAgents);
    let grid = screen(&app);
    let y = (1..grid.height())
        .find(|y| grid.row_text(*y).contains("Gemini") && grid.row_text(*y).contains('●'))
        .unwrap_or_else(|| panic!("the done row: {}", grid.text()));
    let row = grid.row_text(y);
    let x = row[..row.find("Gemini").unwrap()].chars().count() as u16;
    app.handle(mouse(MouseKind::Move, x, y));
    let grid = screen(&app);
    let rows = super::strip_tests::picture(&grid, y);
    let band = app.theme().row_hover;
    // The band: the longest run of its colour across the row, over its words, whose round
    // corners take as much off both its ends.
    let middle = &rows[9];
    let mut best = (0, 0);
    let mut start = None;
    for (at, pixel) in middle
        .iter()
        .enumerate()
        .chain([(middle.len(), &cellix_render::Rgb(0, 0, 0))])
    {
        match (start, *pixel == band) {
            (None, true) => start = Some(at),
            (Some(from), false) => {
                if at - from > best.1 - best.0 {
                    best = (from, at);
                }
                start = None;
            }
            _ => {}
        }
    }
    let (from, to) = best;
    assert!(to - from > 400, "the band: {from}..{to}");
    // What it holds: anything off its colour inside it, its round ends left out.
    let inked: Vec<usize> = (from + 6..to - 6)
        .filter(|x| (12..38).any(|line| super::strip_tests::apart(rows[line][*x], band)))
        .collect();
    let (first, last) = (inked[0], inked[inked.len() - 1]);
    let (left, right) = (first - from, to - 1 - last);
    assert!(
        left.abs_diff(right) <= 2,
        "{left} px from the band's start to its mark, {right} from its end to the dot"
    );
}
