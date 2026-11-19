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
        })
    };
    app.serve(event("a", "Claude Code", AgentState::Working, "Working"));
    app.serve(event("b", "Codex", AgentState::Working, "Working"));
    app.serve(event("b", "Codex", AgentState::Idle, "Finished"));
    app.serve(event("c", "Gemini", AgentState::NeedsYou, "Allow Bash?"));
    app.serve(event("d", "Codex", AgentState::Idle, "Ready"));
    app.run(cellix_input::Command::ShowAgents);
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
    // Done stays done until the agent is looked at: a press on its row, for one with no tab.
    let grid = screen(&app);
    let y = (0..grid.height())
        .find(|y| grid.row_text(*y).contains("⬡ Codex") && grid.row_text(*y).contains('●'));
    let y = y.expect("the done row");
    let row = grid.row_text(y);
    let x = row[..row.find("Codex").unwrap()].chars().count() as u16;
    app.handle(mouse(MouseKind::Left, x, y));
    let text = screen(&app).text();
    assert!(!text.contains("DONE") && text.contains("IDLE 2"), "{text}");
}

/// How long ago stands in one column down the list: every row keeps room at its end as wide
/// as the "needs you" chip, whatever its state, where the row that needs you had its time
/// pushed six columns left of the others'.
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
        })
    };
    app.serve(event("a", "Claude Code", AgentState::Working, "Working"));
    app.serve(event("b", "Codex", AgentState::Idle, "Finished"));
    app.serve(event("c", "Gemini", AgentState::NeedsYou, "Allow Bash?"));
    app.serve(event("d", "Codex", AgentState::Idle, "Ready"));
    app.run(cellix_input::Command::ShowAgents);
    // On cells: where each row's "<1m" ends.
    let grid = screen(&app);
    let ends: Vec<usize> = (0..grid.height())
        .filter_map(|y| {
            let row = grid.row_text(y);
            let at = row.find("<1m")?;
            Some(row[..at].chars().count() + 3)
        })
        .collect();
    assert_eq!(ends.len(), 4, "{}", grid.text());
    assert!(
        ends.windows(2).all(|w| w[0] == w[1]),
        "{ends:?}\n{}",
        grid.text()
    );
    // With pictures: the same right edge for every painted time.
    app.set_measure(std::rc::Rc::new(cellix_render::FontMeasure::new(17, 37)));
    let grid = screen(&app);
    let edges: Vec<i32> = grid
        .paint
        .iter()
        .filter_map(|p| match p {
            cellix_render::Paint::Text { x, text, .. } if text == "<1m" => Some(*x),
            _ => None,
        })
        .collect();
    assert_eq!(edges.len(), 4, "{edges:?}");
    assert!(edges.windows(2).all(|w| w[0] == w[1]), "{edges:?}");
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
    app.serve(Request::Agent(AgentEvent {
        session: "c".into(),
        agent: "Gemini".into(),
        state: AgentState::NeedsYou,
        detail: "Allow Bash?".into(),
        folder: "/w".into(),
        pid: None,
        transcript: String::new(),
        name: String::new(),
    }));
    app.run(cellix_input::Command::ShowAgents);
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
}
