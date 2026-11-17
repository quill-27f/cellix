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

#[test]
fn how_long_ago_is_said_in_a_few_letters() {
    use crate::agents::ago;
    let now = Instant::now();
    assert_eq!(ago(now), "<1m");
    assert_eq!(ago(now - Duration::from_secs(5 * 60 + 3)), "5m");
    assert_eq!(ago(now - Duration::from_secs(2 * 3600 + 40)), "2h");
    assert_eq!(ago(now - Duration::from_secs(3 * 86_400)), "3d");
}

/// A helper an agent started stands under it, in its section, a step in: the list has the
/// room for that though nothing here starts one yet.
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
    ];
    let order: Vec<(&str, u8)> = ordered(&rows)
        .into_iter()
        .map(|(r, d)| (r.name.as_str(), d))
        .collect();
    assert_eq!(
        order,
        [("asking", 0), ("lead", 0), ("helper", 1), ("idle", 0)]
    );
}
