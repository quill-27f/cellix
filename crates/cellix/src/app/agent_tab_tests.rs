//! An agent's tab in the side pane: its program's mark, the name its own title gives it, how
//! full its context is, and what it's doing, driven through the app and read off the grid.

use std::fs;
use std::path::Path;
use std::time::{Duration, Instant};

use cellix_agent::{AgentEvent, AgentState, Request};
use cellix_input::{Event, Key, KeyEvent, Mods, MouseEvent, MouseKind};
use cellix_render::Grid;
use tempfile::TempDir;

use super::App;
use crate::dock::Front;

fn project() -> (TempDir, App) {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("alpha.txt"), "first line\n").unwrap();
    let app = App::new(dir.path().to_path_buf(), (130, 30));
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

/// A stand-in for an agent's program that names itself in its terminal's title, as Claude
/// Code does with what it's working on, and waits.
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

/// Starts the stand-in as Claude Code in the side pane, and waits for its title.
fn start(dir: &Path, app: &mut App, title: &str) -> usize {
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

fn said(state: AgentState, detail: &str) -> Request {
    Request::Agent(AgentEvent {
        session: "s1".into(),
        agent: "Claude Code".into(),
        state,
        detail: detail.into(),
        folder: "/w".into(),
        pid: None,
        transcript: String::new(),
        name: String::new(),
        prompt: String::new(),
    })
}

/// The pane's tab row, and where a word stands on it.
fn tab_row(app: &App) -> (Grid, u16) {
    let grid = screen(app);
    let y = app.dock_area().unwrap().y;
    (grid, y)
}

fn column_of(grid: &Grid, y: u16, symbol: &str) -> Option<u16> {
    (0..grid.width()).find(|x| grid.cell(*x, y).symbol == symbol)
}

#[test]
#[cfg(unix)]
fn an_agents_tab_shows_its_mark_its_own_name_and_what_it_is_doing() {
    let (dir, mut app) = project();
    app.turn_at = Some(3);
    let index = start(dir.path(), &mut app, "✳ pricing page");
    let (grid, y) = tab_row(&app);
    let row = grid.row_text(y);
    // Claude Code's mark, one letter on cells, and the name its title gives it.
    assert!(row.contains("✻ pricing page"), "{row}");
    assert!(!row.contains("Claude Code"), "{row}");
    assert!(!grid.spinning, "idle: nothing after its name");

    // Working: the spinner, in the accent, between its name and its ×.
    app.serve(said(AgentState::Working, "Working"));
    let (grid, y) = tab_row(&app);
    let spin = column_of(&grid, y, "⠸").expect("the fourth braille frame");
    let close = column_of(&grid, y, "×").expect("its ×");
    let name = grid.row_text(y).find("pricing page").unwrap() as u16;
    assert!(name < spin && spin < close, "{}", grid.row_text(y));
    assert_eq!(grid.cell(spin, y).style.fg, app.theme().accent);
    assert!(grid.spinning, "and it asks for its next step");

    // Asking: a dot in the attention colour.
    app.serve(said(AgentState::NeedsYou, "Allow Bash?"));
    let (grid, y) = tab_row(&app);
    let dot = column_of(&grid, y, "●").expect("a dot");
    assert_eq!(grid.cell(dot, y).style.fg, app.theme().attention);
    assert!(!grid.spinning);

    // Finished with its tab in front: it's been seen, and nothing shows.
    app.serve(said(AgentState::Working, "Working"));
    app.serve(said(AgentState::Idle, "Finished"));
    let (grid, y) = tab_row(&app);
    assert_eq!(column_of(&grid, y, "●"), None, "{}", grid.row_text(y));

    // Finished behind another tab: a green dot, until its tab is in front.
    app.open_agent(crate::agents::SHELL.1);
    app.serve(said(AgentState::Working, "Working"));
    app.serve(said(AgentState::Idle, "Finished"));
    let (grid, y) = tab_row(&app);
    let dot = column_of(&grid, y, "●").expect("done, not yet seen");
    assert_eq!(grid.cell(dot, y).style.fg, app.theme().git_added);
    app.dock.active = Some(index);
    app.dock.front = Front::Session;
    app.poll();
    let (grid, y) = tab_row(&app);
    assert_eq!(column_of(&grid, y, "●"), None, "seen once it's in front");

    // Working out of sight asks for nothing.
    app.serve(said(AgentState::Working, "Working"));
    assert!(screen(&app).spinning);
    app.dock.open = false;
    assert!(
        !screen(&app).spinning,
        "a hidden pane's spinner isn't drawn"
    );
}

#[test]
#[cfg(unix)]
fn a_tab_renamed_keeps_its_name_over_its_title() {
    let (dir, mut app) = project();
    let index = start(dir.path(), &mut app, "⠙ pricing page");
    assert_eq!(app.dock.sessions[index].label(), "pricing page");
    // The tab's menu has Rename, which asks with the name it has.
    let area = app.dock_area().unwrap();
    let (grid, y) = tab_row(&app);
    let x = grid.row_text(y).find("pricing").unwrap() as u16;
    app.handle(mouse(MouseKind::Right, x, area.y));
    assert!(screen(&app).text().contains("Rename…"));
    app.rename_session(index);
    for _ in 0.."pricing page".len() {
        app.handle(Event::Key(KeyEvent::plain(Key::Backspace)));
    }
    for c in "billing".chars() {
        app.handle(Event::Key(KeyEvent::plain(Key::Char(c))));
    }
    app.handle(Event::Key(KeyEvent::plain(Key::Enter)));
    assert_eq!(app.dock.sessions[index].label(), "billing");
    let (grid, y) = tab_row(&app);
    assert!(
        grid.row_text(y).contains("✻ billing"),
        "{}",
        grid.row_text(y)
    );
    // What the program calls itself after doesn't change it.
    app.dock.sessions[index].title = Some("✶ checkout".into());
    assert_eq!(app.dock.sessions[index].label(), "billing");
    // Named nothing, it takes its own name again; and from the palette as well.
    app.run(cellix_input::Command::RenamePaneTab);
    for _ in 0.."billing".len() {
        app.handle(Event::Key(KeyEvent::plain(Key::Backspace)));
    }
    app.handle(Event::Key(KeyEvent::plain(Key::Enter)));
    assert_eq!(app.dock.sessions[index].label(), "checkout");
}

/// The name is kept with the pane's other state, by the session's host, so the tab has it
/// again when the next run joins it.
#[test]
#[cfg(unix)]
fn a_tabs_name_is_kept_for_the_next_run() {
    let (dir, mut app) = project();
    let folder = crate::hold::folder_of(dir.path());
    let aside = folder.join("stopped");
    fs::create_dir_all(&aside).unwrap();
    let job = crate::hold::Job {
        name: "Codex".to_string(),
        argv: vec!["codex".to_string()],
        env: Vec::new(),
        folder: dir.path().to_path_buf(),
        size: (80, 24),
        pixels: (0, 0),
        member: None,
    };
    fs::write(aside.join("1-5.json"), serde_json::to_vec(&job).unwrap()).unwrap();
    fs::write(aside.join("1-5.log"), "said\r\n").unwrap();
    app.rejoin_sessions();
    assert_eq!(app.dock.sessions.len(), 1);
    app.dock.sessions[0].renamed = Some("refunds".into());
    app.keep_state();
    let mut next = App::new(dir.path().to_path_buf(), (130, 30));
    next.restore_state();
    next.rejoin_sessions();
    let name = next.dock.sessions.first().map(|s| s.label());
    let _ = fs::remove_dir_all(&folder);
    assert_eq!(name.as_deref(), Some("refunds"));
}

/// A pane too narrow for every tab whole cuts those behind the one in front to their mark
/// and what they're doing, not to three letters and a "…".
#[test]
#[cfg(unix)]
fn a_narrow_pane_keeps_each_agents_mark_and_state() {
    let (dir, mut app) = project();
    app.turn_at = Some(0);
    let first = start(dir.path(), &mut app, "✳ pricing page");
    app.serve(said(AgentState::NeedsYou, "Allow Bash?"));
    app.open_agent(crate::agents::SHELL.1);
    app.open_agent(crate::agents::SHELL.1);
    app.dock.width = Some(30);
    app.handle(Event::Resize(130, 30));
    let (grid, y) = tab_row(&app);
    let row = grid.row_text(y);
    assert!(
        app.dock.fit(app.title_of(app.dock_area().unwrap())) == crate::dock::Fit::Cut,
        "{row}"
    );
    let mark = column_of(&grid, y, "✻").expect("its mark");
    let dot = column_of(&grid, y, "●").expect("and that it's asking");
    assert!(dot > mark && dot - mark <= 3, "{row}");
    assert!(!row.contains("pri…"), "no letters cut short: {row}");
    assert_eq!(first, 0);
}

/// The workspace's tile says the most urgent thing its tabs say. A tab asking stood beside a
/// tile that said "done just now", with no badge and no message, once another agent's news
/// from the same process had taken the asking one's place among the agents (m1-agent-tabs).
#[test]
#[cfg(unix)]
fn the_tile_says_what_the_most_urgent_tab_says() {
    use super::TileState;
    let (dir, mut app) = project();
    start(dir.path(), &mut app, "✳ pricing page");
    let pid = Some(std::process::id());
    let news = |session: &str, agent: &str, state, detail: &str| {
        Request::Agent(AgentEvent {
            session: session.into(),
            agent: agent.into(),
            state,
            detail: detail.into(),
            folder: "/w".into(),
            pid,
            transcript: String::new(),
            name: String::new(),
            prompt: String::new(),
        })
    };
    app.serve(news(
        "s1",
        "Claude Code",
        AgentState::NeedsYou,
        "Allow Bash?",
    ));
    assert_eq!(app.tile_state(), TileState::NeedsYou);
    // Another agent's news from the same process, which works and then finishes.
    app.serve(news("s2", "Gemini CLI", AgentState::Working, "Working"));
    app.serve(news("s2", "Gemini CLI", AgentState::Idle, "Finished"));
    assert_eq!(app.agents.needing_you(), 0, "its news is gone");
    let (grid, y) = tab_row(&app);
    let dot = column_of(&grid, y, "●").expect("the tab still asks");
    assert_eq!(grid.cell(dot, y).style.fg, app.theme().attention);
    assert_eq!(
        app.tile_state(),
        TileState::NeedsYou,
        "and so does the tile"
    );
    assert_eq!(app.needing_you(), 1, "for the badge and the status bar too");
    // Once it's answered, working is what the tile says while its tab turns.
    app.serve(news("s1", "Claude Code", AgentState::Working, "Working"));
    assert_eq!(app.tile_state(), TileState::Working);
}

/// The Agents list shows what a press acts on: an agent with a tab here, which a press brings
/// forward. One running outside Cellix, idle or asking, whether its hooks report or a look at
/// the folder found it, isn't listed, counted, rung for or noted in the bell (the user, 19
/// November: "There are idle agents session list in the agents tab but they are not
/// actionable"). It still hears about comments.
#[test]
#[cfg(unix)]
fn the_agents_list_shows_only_agents_with_a_tab_here() {
    use super::TileState;
    let (dir, mut app) = project();
    let news = |session: &str, agent: &str, state, detail: &str| {
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
    // In other terminals: Codex idle and Gemini CLI asking, by their hooks, and OpenCode,
    // which a look at the folder found and which says nothing.
    app.serve(news("outside-idle", "Codex", AgentState::Idle, "Ready"));
    app.serve(news(
        "outside-asking",
        "Gemini CLI",
        AgentState::NeedsYou,
        "Allow Bash?",
    ));
    app.agents
        .found_by_looking(&["Codex", "Gemini CLI", "OpenCode"]);
    assert!(app.agent_rows().is_empty(), "nothing here to act on");
    assert_eq!(app.needing_you(), 0, "nor for the badge or the status bar");
    assert!(app.inbox_listed().is_empty(), "nor the bell");
    // A finished turn outside doesn't make the tile say done.
    app.serve(news("outside-idle", "Codex", AgentState::Idle, "Finished"));
    assert_eq!(app.tile_state(), TileState::Nothing);
    assert!(app.inbox_listed().is_empty());
    // Who hears about comments is unchanged: an agent working on this project outside.
    let outside = app.agents.outside();
    assert!(
        matches!(outside.as_deref(), Some("Codex" | "Gemini CLI")),
        "{outside:?}"
    );
    app.run(cellix_input::Command::ListAgents);
    let text = screen(&app).text();
    assert!(
        text.contains("HOW IT WORKS"),
        "an empty list says what it's for: {text}"
    );

    // Beside an agent with a tab here, whose hooks report for it.
    let tab = start(dir.path(), &mut app, "✳ pricing page");
    app.serve(said(AgentState::Working, "Working"));
    let rows = app.agent_rows();
    assert_eq!(
        rows.len(),
        1,
        "the one with a tab: {:?}",
        rows.iter().map(|r| &r.name).collect::<Vec<_>>()
    );
    assert_eq!(rows[0].session, "s1");
    assert_eq!(
        app.tile_state(),
        TileState::Working,
        "not the asking one outside"
    );
    app.run(cellix_input::Command::ListAgents);
    let grid = screen(&app);
    let text = grid.text();
    assert!(text.contains("WORKING"), "{text}");
    assert!(!text.contains("NEEDS YOU"), "{text}");
    assert!(!text.contains("IDLE"), "{text}");
    assert!(!text.contains("HOW IT WORKS"), "{text}");
    assert!(!text.contains("An agent needs you"), "{text}");
    // Its row brings its tab forward.
    let heading = (0..grid.height())
        .find(|y| grid.row_text(*y).contains("WORKING"))
        .unwrap();
    let row = grid.row_text(heading + 1);
    assert!(row.contains("pricing page"), "{row}");
    let x = row
        .find("pricing")
        .map(|at| row[..at].chars().count() as u16)
        .unwrap();
    app.handle(mouse(MouseKind::Left, x, heading + 1));
    assert_eq!(app.dock.active, Some(tab));
    assert_eq!(app.dock.front, Front::Session);
}

/// Agents' tabs whose own titles say only which program they are go by their conversation's
/// title, which Claude Code keeps in its transcript, or else by the first thing asked of them,
/// cut short, with the whole name in the tab's tip; the Agents list calls them the same. Two
/// that would still read alike are told apart. A real title and a Rename still win (the user,
/// 20 November: "All the tabs say claude code and it's hard to distinguish them").
#[test]
#[cfg(unix)]
fn agents_tabs_with_no_title_go_by_their_topic_or_what_was_asked() {
    let (dir, mut app) = project();
    app.dock.width = Some(100);
    app.handle(Event::Resize(130, 30));
    let first = start(dir.path(), &mut app, "✳ Claude Code");
    let second = start(dir.path(), &mut app, "Claude Code");
    let third = start(dir.path(), &mut app, "claude");
    let shown = |app: &App| -> Vec<String> {
        let names = app.dock.session_names().into_iter();
        names.map(|(shown, _)| shown).collect()
    };
    assert_eq!(
        shown(&app),
        ["Claude Code", "Claude Code (2)", "Claude Code (3)"],
        "told apart before anything names them"
    );
    // The tab in front whole; one behind cut short keeps its number, and has the whole in
    // its tip.
    let (grid, y) = tab_row(&app);
    assert!(
        grid.row_text(y).contains("Claude Code (3)"),
        "{}",
        grid.row_text(y)
    );
    assert!(grid.row_text(y).contains("… (2)"), "{}", grid.row_text(y));
    let title = app.title_of(app.dock_area().unwrap());
    let tips: Vec<String> = (title.x..title.right())
        .filter_map(|x| app.dock.cut_name(title, x))
        .collect();
    assert!(
        tips.iter().any(|tip| tip == "Claude Code (2)"),
        "{tips:?} {}",
        grid.row_text(y)
    );

    // Each linked to its own session, as the hooks link them.
    for (index, id) in [(first, "s1"), (second, "s2"), (third, "s3")] {
        app.dock.sessions[index].agent_session = Some(id.into());
    }
    let transcript = dir.path().join("s1.jsonl");
    fs::write(
        &transcript,
        "{\"type\":\"ai-title\",\"aiTitle\":\"Refund route and its tests\",\"sessionId\":\"s1\"}\n",
    )
    .unwrap();
    let asked = |session: &str, prompt: &str, transcript: &Path| {
        Request::Agent(AgentEvent {
            session: session.into(),
            agent: "Claude Code".into(),
            state: AgentState::Working,
            detail: "Working".into(),
            folder: "/w".into(),
            pid: None,
            transcript: transcript.display().to_string(),
            name: String::new(),
            prompt: prompt.into(),
        })
    };
    let nowhere = Path::new("");
    app.serve(asked("s1", "Add a refund route", &transcript));
    app.serve(asked(
        "s2",
        "Fix the login redirect after a password reset, and add a test for it",
        nowhere,
    ));
    app.serve(asked(
        "s3",
        "Fix the login redirect after a password change",
        nowhere,
    ));
    // Only the first prompt names a tab.
    app.serve(asked("s2", "And the logout too", nowhere));
    // The transcript is read away from the screen, a little at a time.
    let deadline = Instant::now() + Duration::from_secs(20);
    while app.dock.sessions[first].topic.is_none() {
        assert!(
            Instant::now() < deadline,
            "the transcript's title was never read"
        );
        app.follow_transcripts();
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(
        shown(&app),
        [
            "Refund route and its tests",
            "Fix the login redirect…",
            "Fix the login redirect… (2)",
        ]
    );

    // The tab's tip has its whole name.
    let area = app.dock_area().unwrap();
    let title = app.title_of(area);
    let whole = "Fix the login redirect after a password reset, and add a test for it";
    let x = (title.x..title.right())
        .find(|&x| app.dock.cut_name(title, x).as_deref() == Some(whole))
        .expect("the second tab's tip has its whole name");
    app.handle(mouse(MouseKind::Move, x, area.y));
    let text = screen(&app).text();
    assert!(text.contains("password reset"), "{text}");

    // The Agents list calls each what its tab does.
    app.run(cellix_input::Command::ListAgents);
    let text = screen(&app).text();
    assert!(text.contains("Refund route and its tests"), "{text}");
    assert!(text.contains("Fix the login redirect… (2)"), "{text}");

    // A title that says what it works on wins over both, and a Rename over that.
    app.dock.sessions[first].title = Some("✳ pricing page".into());
    assert_eq!(app.dock.sessions[first].label(), "pricing page");
    app.dock.sessions[second].renamed = Some("login".into());
    assert_eq!(shown(&app)[1], "login");
    assert_eq!(
        shown(&app)[2],
        "Fix the login redirect…",
        "nothing reads like it now"
    );
}
