//! An agent's tab is named by the conversation it holds, from sources that don't depend on
//! which `cellix` its hooks run: Claude Code's own record of its process
//! (`sessions/<pid>.json` in its folder) leads to the conversation's transcript, whose title
//! or first prompt names the tab, and a new conversation in the same tab (/clear, /resume) is
//! named afresh. An agent typed at a Terminal tab's prompt is named and marked as an agent's tab
//! is. On Amazon Linux, Claude Code's tabs stayed "Claude Code" though their spinners turned
//! (the user, 20 November: "Claude code titles are not changing automatically").
//!
//! Claude Code's folder here is a temporary one, given to the app: no test reads the person's.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use cellix_agent::{AgentEvent, AgentState, Request};
use cellix_render::Grid;
use serde_json::json;
use tempfile::TempDir;

use super::App;

/// A project, and a Claude Code folder of its own that the app reads.
fn project() -> (TempDir, TempDir, App) {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("alpha.txt"), "first line\n").unwrap();
    let home = tempfile::tempdir().unwrap();
    let mut app = App::new(dir.path().to_path_buf(), (130, 30));
    app.claude_home = Some(home.path().to_path_buf());
    app.dock.width = Some(90);
    app.handle(cellix_input::Event::Resize(130, 30));
    (dir, home, app)
}

fn screen(app: &App) -> Grid {
    let mut grid = Grid::new(app.size().0, app.size().1, app.background());
    app.draw(&mut grid);
    grid
}

/// The side pane's row of tabs, as text.
fn tabs(app: &App) -> String {
    let grid = screen(app);
    grid.row_text(app.dock_area().unwrap().y)
}

/// Polls the app, and reads what Claude Code and the transcripts say as it does every couple
/// of seconds, until `done` holds or a minute has gone.
fn until(app: &mut App, what: &str, done: impl Fn(&App) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(60);
    while !done(app) {
        assert!(Instant::now() < deadline, "never {what}: {}", tabs(app));
        app.poll();
        app.follow_transcripts();
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// What Claude Code writes of process `pid`: the conversation it holds, and its folder's name,
/// which it makes up and which isn't a title.
fn register(home: &Path, pid: u32, session: &str, cwd: &Path) {
    let sessions = home.join("sessions");
    fs::create_dir_all(&sessions).unwrap();
    let record = json!({
        "pid": pid, "sessionId": session, "cwd": cwd.display().to_string(),
        "startedAt": 1, "kind": "interactive", "entrypoint": "cli",
        "name": "shop", "nameSource": "derived", "status": "idle", "updatedAt": 2,
    });
    let file = sessions.join(format!("{pid}.json"));
    // Written beside and moved over, as a file that changes is, so a read never finds half.
    let draft = file.with_extension("new");
    fs::write(&draft, record.to_string()).unwrap();
    fs::rename(draft, file).unwrap();
}

/// Where Claude Code keeps a conversation's transcript: under its project's folder.
fn transcript(home: &Path, cwd: &Path, session: &str) -> PathBuf {
    let folder = crate::transcript::project_folder(&cwd.display().to_string());
    let folder = home.join("projects").join(folder);
    fs::create_dir_all(&folder).unwrap();
    folder.join(format!("{session}.jsonl"))
}

/// Adds lines to a transcript: the person's prompt, and the title Claude Code gives it.
fn write_lines(path: &Path, lines: &[serde_json::Value]) {
    use std::io::Write;
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .unwrap();
    for line in lines {
        writeln!(file, "{line}").unwrap();
    }
}

fn prompt(words: &str) -> serde_json::Value {
    json!({"type": "user", "message": {"role": "user", "content": words}})
}

fn titled(words: &str, session: &str) -> serde_json::Value {
    json!({"type": "ai-title", "aiTitle": words, "sessionId": session})
}

/// Hooks' news of the kind an older `cellix` sends: the state, but no prompt and no
/// transcript.
fn news(session: &str, pid: Option<u32>, state: AgentState, detail: &str) -> Request {
    Request::Agent(AgentEvent {
        session: session.into(),
        agent: "Claude Code".into(),
        state,
        detail: detail.into(),
        folder: "/w".into(),
        pid,
        transcript: String::new(),
        name: String::new(),
        prompt: String::new(),
    })
}

/// A stand-in for Claude Code that titles its terminal as Claude Code does before it has a
/// title of its own, and waits.
fn stand_in(dir: &Path) -> String {
    let fake = dir.join("fake-claude");
    let script = "#!/bin/sh\nprintf '\\033]0;\\342\\234\\263 Claude Code\\007'\nexec sleep 60\n";
    fs::write(&fake, script).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&fake, fs::Permissions::from_mode(0o755)).unwrap();
    }
    fake.display().to_string()
}

/// A Claude Code tab whose hooks say nothing of its prompt or transcript is named from what
/// Claude Code says of its process: its conversation's first prompt, then its title. And when
/// Claude Code says the process holds another conversation, as after /clear, the tab drops the
/// last one's name and takes the new one's.
#[test]
#[cfg(unix)]
fn an_agents_tab_is_named_from_what_claude_code_says_of_its_process() {
    let (dir, home, mut app) = project();
    let root = app.workspace.root().to_path_buf();
    app.agent_program = Some(stand_in(dir.path()));
    app.open_agent("claude");
    let index = app.dock.sessions.len() - 1;
    until(&mut app, "its title", |app| {
        app.dock.sessions[index].title.is_some()
    });
    assert_eq!(app.dock.sessions[index].whole_label(), "Claude Code");
    let pid = app.dock.sessions[index].agent_pid().expect("its process");

    // Its hooks report its state, and nothing else.
    app.serve(news("conv-1", None, AgentState::Working, "Working"));
    register(home.path(), pid, "conv-1", &root);
    let first = transcript(home.path(), &root, "conv-1");
    write_lines(&first, &[prompt("Add a refund route for unpaid orders")]);
    until(&mut app, "named by its first prompt", |app| {
        app.dock.sessions[index].whole_label() == "Add a refund route for unpaid orders"
    });
    assert!(
        tabs(&app).contains("Add a refund route for unpaid…"),
        "cut short on the tab: {}",
        tabs(&app)
    );
    // The name made from its folder isn't a title.
    assert!(!tabs(&app).contains("shop"), "{}", tabs(&app));

    // Then by the title Claude Code gives the conversation.
    write_lines(&first, &[titled("Refund route", "conv-1")]);
    until(&mut app, "named by its title", |app| {
        app.dock.sessions[index].label() == "Refund route"
    });

    // /clear: another conversation, not yet written. The last one's name goes.
    register(home.path(), pid, "conv-2", &root);
    until(&mut app, "a new conversation", |app| {
        app.dock.sessions[index].agent_session.as_deref() == Some("conv-2")
    });
    assert_eq!(app.dock.sessions[index].whole_label(), "Claude Code");
    let second = transcript(home.path(), &root, "conv-2");
    write_lines(
        &second,
        &[prompt("/clear"), prompt("Fix the login redirect")],
    );
    until(&mut app, "named by the new conversation", |app| {
        app.dock.sessions[index].label() == "Fix the login redirect"
    });
    let row = tabs(&app);
    assert!(row.contains("✻ Fix the login redirect"), "{row}");
    assert!(!row.contains("Refund"), "{row}");
}

/// With hooks that say the prompt, a new conversation in the same tab is named by its own
/// first prompt and title, not the last one's, which stayed on the tab after /clear.
#[test]
#[cfg(unix)]
fn a_new_conversation_in_a_tab_is_named_afresh() {
    let (dir, _home, mut app) = project();
    app.agent_program = Some(stand_in(dir.path()));
    app.open_agent("claude");
    let index = app.dock.sessions.len() - 1;
    until(&mut app, "its title", |app| {
        app.dock.sessions[index].title.is_some()
    });
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
    let first = dir.path().join("s1.jsonl");
    write_lines(
        &first,
        &[prompt("Add a refund route"), titled("Refund route", "s1")],
    );
    app.serve(asked("s1", "Add a refund route", &first));
    assert_eq!(app.dock.sessions[index].label(), "Add a refund route");
    until(&mut app, "named by its title", |app| {
        app.dock.sessions[index].label() == "Refund route"
    });

    // A new session in the same tab, its transcript not titled yet: its prompt names it at
    // once, from the hook.
    let second = dir.path().join("s2.jsonl");
    write_lines(&second, &[prompt("Fix the login redirect")]);
    app.serve(asked("s2", "Fix the login redirect", &second));
    assert_eq!(app.dock.sessions[index].label(), "Fix the login redirect");
    for _ in 0..3 {
        app.follow_transcripts();
        std::thread::sleep(Duration::from_millis(50));
    }
    until(&mut app, "the new transcript read", |app| {
        app.dock.sessions[index].first_asked.is_some()
    });
    assert_eq!(app.dock.sessions[index].label(), "Fix the login redirect");
    write_lines(&second, &[titled("Login redirect", "s2")]);
    until(&mut app, "named by the new title", |app| {
        app.dock.sessions[index].label() == "Login redirect"
    });
    // Its first prompt names only its own conversation: news of the last one's doesn't bring
    // its name back.
    assert!(!tabs(&app).contains("Refund"), "{}", tabs(&app));
}

/// `claude` typed at a Terminal tab's prompt: the tab is named and marked as an agent's tab
/// is, "Claude Code" with its mark rather than "claude", then by what Claude Code says the
/// process holds, its conversation's title; its hooks' news, found by its process, says what
/// it's doing. It's still a shell, which comments never reach. Back at the prompt, it's a
/// Terminal again.
#[test]
#[cfg(unix)]
fn an_agent_typed_at_a_shells_prompt_is_named_by_its_conversation() {
    let (dir, home, mut app) = project();
    app.turn_at = Some(3);
    let root = app.workspace.root().to_path_buf();
    // A stand-in that runs as `claude`, as the system names its process: a script of that
    // name, which Linux names by the script and macOS by its shell. A link to `sleep` was one
    // until CI's Ubuntu took uutils' coreutils, whose one binary refuses to run under another
    // name ("Requested utility `claude` does not match executable name").
    let bin = dir.path().join("bin");
    fs::create_dir_all(&bin).unwrap();
    let claude = bin.join("claude");
    fs::write(&claude, "#!/bin/sh\nsleep \"$1\"\n").unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&claude, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let path = format!(
        "PATH={}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    // Interactive, a shell gives each command a process group of its own, as a person's does.
    let shell = ["sh".to_string(), "-i".to_string()];
    let env = ["PS1=ready$ ".to_string(), "ENV=".to_string(), path];
    app.start_session(crate::agents::SHELL.0, &shell, &env, &root);
    let printed = |app: &App, words: &str| {
        let lines = app.dock.sessions[0].lines();
        lines.iter().any(|line| line.contains(words))
    };
    until(&mut app, "a prompt", |app| printed(app, "ready$"));
    app.dock.sessions[0].send_line("claude 60");
    let said = |app: &App| app.dock.sessions[0].lines().join(" / ");
    let deadline = Instant::now() + Duration::from_secs(60);
    while app.dock.sessions[0].running_program().is_none() {
        assert!(
            Instant::now() < deadline,
            "never something in front: {}; the shell said: {}",
            tabs(&app),
            said(&app)
        );
        app.poll();
        app.follow_transcripts();
        std::thread::sleep(Duration::from_millis(20));
    }
    // Linux names a process by the link it was started by, as a real `claude` is one; macOS
    // by the file it leads to, so there the stand-in runs as `sleep`, as a real one can run as
    // its version's file, and only what Claude Code says names it.
    #[cfg(target_os = "linux")]
    {
        let row = tabs(&app);
        assert!(row.contains("✻ Claude Code"), "its agent's mark: {row}");
        assert!(!row.contains("> claude"), "not its command: {row}");
    }
    let pid = app.dock.sessions[0]
        .agent_pid()
        .expect("the command in front");

    // What Claude Code says of that process names it by its conversation.
    register(home.path(), pid, "conv-1", &root);
    write_lines(
        &transcript(home.path(), &root, "conv-1"),
        &[
            prompt("Add a refund route for unpaid orders"),
            titled("Refund route for unpaid orders", "conv-1"),
        ],
    );
    until(&mut app, "named by its conversation", |app| {
        app.dock.sessions[0].label() == "Refund route for unpaid orders"
    });
    assert!(tabs(&app).contains("✻ Refund route"), "{}", tabs(&app));
    assert!(
        !app.dock.sessions[0].agent(),
        "comments never reach a shell"
    );

    // Its hooks' news finds the tab by its process, and its mark says what they say.
    app.serve(news("conv-1", Some(pid), AgentState::Working, "Working"));
    assert!(screen(&app).spinning, "working: {}", tabs(&app));
    app.serve(news("conv-1", Some(pid), AgentState::Idle, "Finished"));
    let grid = screen(&app);
    assert!(
        !grid.spinning,
        "done and in front, it doesn't turn as a command running does: {}",
        tabs(&app)
    );
    // The Agents list calls it what its tab does.
    let rows = app.agent_rows();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].name, "Refund route for unpaid orders");

    // Back at the prompt, a Terminal again, and what it held is gone.
    app.dock.sessions[0].write(b"\x03");
    until(&mut app, "a Terminal again", |app| {
        tabs(app).contains("Terminal") && app.dock.sessions[0].agent_session.is_none()
    });
    assert!(!tabs(&app).contains("Refund"), "{}", tabs(&app));
}
