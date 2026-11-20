//! Ask: a side question about a selection, answered by a stand-in for the agent's command line
//! that writes down what it was given and prints what the real one prints. Driven through the
//! App's input and the Grid it draws. No test here runs a real agent: the App finds command
//! lines only on the PATH a test names (`side_path`), and these name a folder of stand-ins.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use cellix_input::{Command, Event, Key, KeyEvent, Mods, MouseEvent, MouseKind};
use cellix_render::Grid;
use tempfile::TempDir;

use cellix_agent::{AgentEvent, AgentState, Request};

use super::App;
use super::side::{DISMISS, FOLLOW_UP, MAKE_COMMENT, SET_UP, WITH_AGENT};

/// How long a test waits for a stand-in, on a loaded machine.
const PATIENCE: Duration = Duration::from_secs(120);

/// The document every test asks about: a heading, a section under it, and the words.
const NOTE: &str =
    "# Notes\n\n## Method\n\nWe rely on ICL to adapt the model.\n\nThe cache goes first.\n";

/// What Claude Code prints, recorded from a real run and cut to what matters: its session,
/// a message streamed in two pieces, and its result.
const CLAUDE: &[&str] = &[
    r#"{"type":"system","subtype":"init","session_id":"side-1","cwd":"/p"}"#,
    r#"{"type":"stream_event","event":{"type":"message_start"}}"#,
    r#"{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"In-context"}}}"#,
    "WAIT",
    r#"{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":" learning: the model learns from examples in its prompt."}}}"#,
    r#"{"type":"result","subtype":"success","is_error":false,"result":"In-context learning: the model learns from examples in its prompt."}"#,
];

/// A project with the note open, the App finding command lines only in its `bin` folder.
fn page() -> (TempDir, App) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("notes.md");
    fs::write(&path, NOTE).unwrap();
    fs::create_dir(dir.path().join("bin")).unwrap();
    let mut app = App::new(dir.path().to_path_buf(), (150, 40));
    app.open(&path).unwrap();
    app.side_path = Some(dir.path().join("bin").into_os_string());
    (dir, app)
}

/// A stand-in for `program`: it writes its arguments, one a line, its input and whether it
/// was told it's a side question beside itself, then prints `lines`. At a line "WAIT" it waits
/// for a file named `go` before printing the rest, so a test can see what came before.
fn stand_in(dir: &Path, program: &str, lines: &[&str]) {
    let bin = dir.join("bin");
    let out = bin.join(format!("{program}.out"));
    fs::write(&out, lines.join("\n") + "\n").unwrap();
    let record = bin.join(program);
    let script = format!(
        "#!/bin/sh\n\
         n=$(/bin/cat '{rec}.runs' 2>/dev/null || echo 0); n=$((n + 1)); echo $n > '{rec}.runs'\n\
         printf '%s\\n' \"$@\" > '{rec}.args.'$n\n\
         printf '%s' \"$CELLIX_SIDE_QUESTION\" > '{rec}.side'\n\
         /bin/cat > '{rec}.prompt.'$n\n\
         while IFS= read -r line; do\n\
           if [ \"$line\" = WAIT ]; then\n\
             while [ ! -e '{go}' ]; do /bin/sleep 0.05; done\n\
           else\n\
             printf '%s\\n' \"$line\"\n\
           fi\n\
         done < '{out}'\n",
        rec = record.with_extension("").display(),
        go = dir.join("go").display(),
        out = out.display(),
    );
    let path = bin.join(program);
    fs::write(&path, script).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    }
}

/// What the stand-in for `program` was given on its `run`th run: its arguments and its input.
fn given(dir: &Path, program: &str, run: usize) -> (Vec<String>, String) {
    let base = dir.join("bin").join(program);
    let args = fs::read_to_string(format!("{}.args.{run}", base.display())).unwrap_or_default();
    let prompt = fs::read_to_string(format!("{}.prompt.{run}", base.display())).unwrap_or_default();
    (args.lines().map(str::to_string).collect(), prompt)
}

fn runs(dir: &Path, program: &str) -> usize {
    let base = dir.join("bin").join(program);
    fs::read_to_string(format!("{}.runs", base.display()))
        .ok()
        .and_then(|n| n.trim().parse().ok())
        .unwrap_or(0)
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

fn click(app: &mut App, x: u16, y: u16) {
    app.handle(mouse(MouseKind::Left, x, y));
    app.handle(mouse(MouseKind::Up, x, y));
}

fn typed(app: &mut App, text: &str) {
    for c in text.chars() {
        app.handle(Event::Key(KeyEvent::plain(Key::Char(c))));
    }
}

fn key(app: &mut App, key: Key) {
    app.handle(Event::Key(KeyEvent::plain(key)));
}

fn find(grid: &Grid, text: &str) -> (u16, u16) {
    for y in 0..grid.height() {
        if let Some(i) = grid.row_text(y).find(text) {
            return (grid.row_text(y)[..i].chars().count() as u16, y);
        }
    }
    panic!("{text:?} not on screen:\n{}", grid.text());
}

/// Selects a word on the page with a double click.
fn select_word(app: &mut App, word: &str) {
    let (x, y) = find(&screen(app), word);
    click(app, x + 1, y);
    click(app, x + 1, y);
}

/// Selects `word` in the file in front, as a double click on it does, without the clicks:
/// two selections made by double clicks in a row count as a triple click.
fn select(app: &mut App, word: &str) {
    let editor = app.editor_mut().unwrap();
    let text = editor.buffer.text();
    let start = text[..text.find(word).unwrap()].chars().count();
    editor.selections.set_single(cellix_core::Selection {
        anchor: start,
        head: start + word.chars().count(),
        goal: None,
    });
}

/// Presses the button that says `words` on the card.
fn press(app: &mut App, words: &str) {
    let (x, y) = find(&screen(app), words);
    click(app, x + 1, y);
}

/// Polls until the screen shows `want`.
fn shows(app: &mut App, want: &str) -> Grid {
    let deadline = Instant::now() + PATIENCE;
    loop {
        app.poll();
        let grid = screen(app);
        if grid.text().contains(want) {
            return grid;
        }
        assert!(
            Instant::now() < deadline,
            "never showed {want:?}:\n{}",
            grid.text()
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Polls until the card has its answer whole.
fn answered(app: &mut App) -> Grid {
    let deadline = Instant::now() + PATIENCE;
    loop {
        app.poll();
        if app.side.as_ref().is_some_and(|s| s.running.is_none()) {
            return screen(app);
        }
        assert!(
            Instant::now() < deadline,
            "never answered:\n{}",
            screen(app).text()
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// What the prompt hook would be handed now, and what waits.
fn handed(app: &mut App) -> (String, String) {
    let (sender, answer) = std::sync::mpsc::channel();
    app.serve(Request::TakeThreads(sender));
    let taken = answer.recv().unwrap();
    let (sender, answer) = std::sync::mpsc::channel();
    app.serve(Request::Waiting(sender));
    (taken, answer.recv().unwrap())
}

/// Ask on a selection opens a field under it with the quick questions; Enter runs Claude Code
/// once, read-only, given the whole document with the selection marked and the headings it's
/// under, and its answer streams onto the card under the words. Nothing of it is a comment:
/// none is made, the comment waiting isn't handed over, and the hook would hand nothing.
#[test]
fn ask_runs_the_agent_read_only_on_the_whole_document_and_streams_the_answer() {
    let (dir, mut app) = page();
    stand_in(dir.path(), "claude", CLAUDE);
    // A comment waits for the agent, which a side question must leave alone.
    select_word(&mut app, "cache");
    app.run(Command::AddComment);
    typed(&mut app, "Why first?");
    key(&mut app, Key::Enter);
    app.dock.open = false;
    select_word(&mut app, "ICL");
    let grid = screen(&app);
    assert!(grid.text().contains("Ask (Alt"), "{}", grid.text());
    assert!(!grid.text().contains("Ask Agent"), "{}", grid.text());
    app.run(Command::AskAgent);
    let grid = screen(&app);
    let text = grid.text();
    assert!(
        text.contains("Ask about ‘ICL’…"),
        "the field's hint: {text}"
    );
    for quick in ["Explain", "Define", "Simplify"] {
        assert!(text.contains(quick), "{quick}: {text}");
    }
    // The card stands under the words.
    let (_, words) = find(&grid, "We rely on ICL");
    let (_, field) = find(&grid, "Ask about ‘ICL’…");
    assert!(field > words, "under the words: {field} over {words}");
    typed(&mut app, "What is it?");
    key(&mut app, Key::Enter);
    // The first piece shows while the rest is still to come, with the spinner.
    let grid = shows(&mut app, "In-context");
    assert!(!grid.text().contains("learning"), "{}", grid.text());
    assert!(app.side.as_ref().unwrap().running.is_some());
    fs::write(dir.path().join("go"), "").unwrap();
    let grid = answered(&mut app);
    let text = grid.text();
    assert!(text.contains("learns from"), "{text}");
    assert!(
        text.contains("What is it?"),
        "the question heads it: {text}"
    );
    for button in ["Copy", FOLLOW_UP, MAKE_COMMENT, DISMISS] {
        assert!(text.contains(button), "{button}: {text}");
    }
    // What it was given: read-only tools, its answer streamed, the prompt on its input.
    assert_eq!(runs(dir.path(), "claude"), 1);
    let (args, prompt) = given(dir.path(), "claude", 1);
    for want in [
        "-p",
        "stream-json",
        "--verbose",
        "--include-partial-messages",
        "Read,Grep,Glob",
        "Bash,Edit,Write,MultiEdit,NotebookEdit",
    ] {
        assert!(args.iter().any(|a| a == want), "{want}: {args:?}");
    }
    let allowed = args.iter().position(|a| a == "--allowedTools").unwrap();
    assert_eq!(args[allowed + 1], "Read,Grep,Glob");
    let denied = args.iter().position(|a| a == "--disallowedTools").unwrap();
    assert_eq!(args[denied + 1], "Bash,Edit,Write,MultiEdit,NotebookEdit");
    assert!(!args.iter().any(|a| a == "--resume"), "{args:?}");
    let side = fs::read_to_string(dir.path().join("bin/claude.side")).unwrap();
    assert_eq!(
        side, "1",
        "told it's a side question, which the hooks leave alone"
    );
    // The whole document, the selection marked, its headings, its path, the question.
    assert!(
        prompt.contains("We rely on ⟦ICL⟧ to adapt the model."),
        "{prompt}"
    );
    assert!(
        prompt.contains("The cache goes first."),
        "the whole of it: {prompt}"
    );
    assert!(
        prompt.contains("The selection is under: Notes › Method"),
        "{prompt}"
    );
    assert!(prompt.contains("notes.md"), "{prompt}");
    assert!(prompt.contains("What is it?"), "{prompt}");
    assert!(prompt.contains("Answer only this question"), "{prompt}");
    // No comment is handed over, made or counted.
    assert!(
        !prompt.contains("Why first?"),
        "the comment went with it: {prompt}"
    );
    assert_eq!(app.threads.len(), 1, "no comment made of the question");
    let (taken, waiting) = handed(&mut app);
    assert!(
        taken.contains("Why first?"),
        "the comment still waits: {taken}"
    );
    assert!(!taken.contains("What is it?") && !waiting.contains("What is it?"));
    // Dismissed, it's gone.
    key(&mut app, Key::Esc);
    assert!(app.side.is_none());
    assert!(!screen(&app).text().contains("learns from"));
}

/// The next question about the same document goes on in its side conversation, by the session
/// id the first run gave, and doesn't send the document again; Follow Up on the card goes on
/// in it too. The ids are kept with Cellix's own state, not in the project.
#[test]
fn later_questions_and_follow_ups_resume_the_documents_side_conversation() {
    let (dir, mut app) = page();
    let quick: Vec<&str> = CLAUDE.iter().copied().filter(|l| *l != "WAIT").collect();
    stand_in(dir.path(), "claude", &quick);
    select(&mut app, "ICL");
    app.run(Command::AskAgent);
    press(&mut app, "Define");
    answered(&mut app);
    let (args, prompt) = given(dir.path(), "claude", 1);
    assert!(!args.iter().any(|a| a == "--resume"), "{args:?}");
    assert!(prompt.contains("Define this term"), "{prompt}");
    // Follow Up: a field in the card, which goes on in the same conversation.
    press(&mut app, FOLLOW_UP);
    typed(&mut app, "And in one word?");
    key(&mut app, Key::Enter);
    answered(&mut app);
    let (args, prompt) = given(dir.path(), "claude", 2);
    let resume = args.iter().position(|a| a == "--resume").expect("resumed");
    assert_eq!(args[resume + 1], "side-1");
    assert!(!args.iter().any(|a| a == "--fork-session"), "{args:?}");
    assert!(prompt.contains("And in one word?"), "{prompt}");
    assert!(
        !prompt.contains("The cache goes first."),
        "not the document again: {prompt}"
    );
    key(&mut app, Key::Esc);
    // Another question on other words of the same document: the same conversation.
    select(&mut app, "cache");
    app.run(Command::AskAgent);
    typed(&mut app, "Why?");
    key(&mut app, Key::Enter);
    answered(&mut app);
    let (args, prompt) = given(dir.path(), "claude", 3);
    let resume = args.iter().position(|a| a == "--resume").expect("resumed");
    assert_eq!(args[resume + 1], "side-1");
    assert!(prompt.contains("⟦cache⟧"), "{prompt}");
    assert!(
        !prompt.contains("We rely on ICL"),
        "the document unchanged isn't sent: {prompt}"
    );
    // Kept outside the project.
    let kept = crate::ask::load(app.workspace.root());
    assert_eq!(
        kept.get("notes.md").map(|k| k.session.as_str()),
        Some("side-1")
    );
    // Nothing in the project says it, but the stand-in's own lines.
    let stand_ins = dir.path().join("bin");
    let told: Vec<PathBuf> = walk(dir.path())
        .into_iter()
        .filter(|p| !p.starts_with(&stand_ins))
        .filter(|p| fs::read_to_string(p).is_ok_and(|t| t.contains("side-1")))
        .collect();
    assert!(told.is_empty(), "the id is in the project: {told:?}");
}

fn walk(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for entry in fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            found.extend(walk(&path));
        } else {
            found.push(path);
        }
    }
    found
}

/// Make a Comment turns the question and its answer into a review comment on the same words:
/// the question is the comment, Clarity its kind, the answer its first reply, by the agent.
#[test]
fn make_a_comment_turns_the_question_and_answer_into_a_comment() {
    let (dir, mut app) = page();
    let quick: Vec<&str> = CLAUDE.iter().copied().filter(|l| *l != "WAIT").collect();
    stand_in(dir.path(), "claude", &quick);
    select(&mut app, "ICL");
    app.run(Command::AskAgent);
    typed(&mut app, "What is it?");
    key(&mut app, Key::Enter);
    answered(&mut app);
    assert!(app.threads.is_empty());
    press(&mut app, MAKE_COMMENT);
    assert!(app.side.is_none(), "the card goes");
    assert_eq!(app.threads.len(), 1);
    let thread = &app.threads[0];
    assert_eq!(thread.quote, "ICL");
    assert_eq!(thread.text, "What is it?");
    assert_eq!(thread.kind, "Clarity");
    assert_eq!(thread.line, 5);
    assert_eq!(thread.replies.len(), 1);
    assert_eq!(thread.replies[0].author, "Claude Code");
    assert!(
        thread.replies[0].text.contains("learns from examples"),
        "{:?}",
        thread.replies
    );
    assert!(!thread.waiting(), "the reviewer sends it on with a reply");
    let text = screen(&app).text();
    assert!(text.contains("What is it?"), "in the Comments pane: {text}");
}

/// Codex and Gemini CLI are each given their own read-only, non-interactive arguments, and
/// their answers fill the card the same way; a second question resumes their conversation.
#[test]
fn codex_and_gemini_answer_read_only_and_resume_their_own_conversations() {
    let (dir, mut app) = page();
    stand_in(
        dir.path(),
        "codex",
        &[
            r#"{"type":"thread.started","thread_id":"t-9"}"#,
            r#"{"type":"item.started","item":{"type":"command_execution","command":"rg ICL"}}"#,
            r#"{"type":"item.completed","item":{"type":"agent_message","text":"Codex says: in-context learning."}}"#,
            r#"{"type":"turn.completed","usage":{}}"#,
        ],
    );
    stand_in(
        dir.path(),
        "gemini",
        &[
            r#"{"type":"init","session_id":"g-2","model":"m"}"#,
            r#"{"type":"message","role":"assistant","content":"Gemini says: ","delta":true}"#,
            r#"{"type":"message","role":"assistant","content":"in-context learning.","delta":true}"#,
            r#"{"type":"result","status":"success","stats":{}}"#,
        ],
    );
    let root = app.workspace.root().display().to_string();
    for (setting, program, says) in [
        ("codex", "codex", "Codex says"),
        ("gemini", "gemini", "Gemini says"),
    ] {
        app.settings.ask_agent = Some(setting.to_string());
        for run in 1..=2 {
            select(&mut app, "ICL");
            app.run(Command::AskAgent);
            key(&mut app, Key::Enter);
            let text = answered(&mut app).text();
            assert!(text.contains(says), "{text}");
            assert!(text.contains("in-context learning."), "{text}");
            key(&mut app, Key::Esc);
            let (args, prompt) = given(dir.path(), program, run);
            assert!(prompt.contains("⟦ICL⟧"), "{prompt}");
            assert!(
                prompt.contains("Explain this"),
                "Enter with nothing typed explains: {prompt}"
            );
            match (program, run) {
                ("codex", 1) => assert_eq!(
                    args,
                    [
                        "exec",
                        "--sandbox",
                        "read-only",
                        "--json",
                        "--skip-git-repo-check",
                        "-C",
                        &root,
                        "-"
                    ]
                ),
                ("codex", _) => assert_eq!(
                    args,
                    [
                        "exec",
                        "resume",
                        "--json",
                        "--skip-git-repo-check",
                        "-c",
                        "sandbox_mode=\"read-only\"",
                        "t-9",
                        "-"
                    ]
                ),
                (_, 1) => assert_eq!(
                    args,
                    [
                        "--output-format",
                        "stream-json",
                        "--approval-mode",
                        "default",
                        "-p",
                        "Answer the question above."
                    ]
                ),
                _ => assert_eq!(
                    args,
                    [
                        "--output-format",
                        "stream-json",
                        "--approval-mode",
                        "default",
                        "--resume",
                        "g-2",
                        "-p",
                        "Answer the question above."
                    ]
                ),
            }
        }
    }
}

/// With no agent's command line installed, the card says what Ask needs and has a button to
/// Settings › Integrations; with the setting naming one not installed, it says so.
#[test]
fn ask_with_no_agent_installed_says_what_it_needs() {
    let (_dir, mut app) = page();
    select(&mut app, "ICL");
    app.run(Command::AskAgent);
    let text = screen(&app).text();
    assert!(text.contains("Ask needs an agent"), "{text}");
    assert!(text.contains("none of them"), "{text}");
    app.settings.ask_agent = Some("codex".to_string());
    key(&mut app, Key::Esc);
    select(&mut app, "ICL");
    app.run(Command::AskAgent);
    let text = screen(&app).text();
    assert!(text.contains("names Codex"), "{text}");
    press(&mut app, SET_UP);
    assert!(app.side.is_none());
    let page = app.settings_page.as_ref().expect("Settings opened");
    assert_eq!(page.section, crate::settings_page::Section::Integrations);
}

/// What goes wrong says so on the card in words: not signed in, a failure the command line
/// gave, and an answer that took too long, which is stopped.
#[test]
fn a_failed_answer_says_why_on_the_card() {
    let (dir, mut app) = page();
    stand_in(
        dir.path(),
        "claude",
        &[
            r#"{"type":"system","subtype":"init","session_id":"s"}"#,
            r#"{"type":"assistant","error":"authentication_failed","message":{"content":[{"type":"text","text":"Not logged in · Please run /login"}]}}"#,
            r#"{"type":"result","subtype":"success","is_error":true,"result":"Not logged in · Please run /login"}"#,
        ],
    );
    select(&mut app, "ICL");
    app.run(Command::AskAgent);
    key(&mut app, Key::Enter);
    let text = answered(&mut app).text();
    assert!(text.contains("isn't signed in"), "{text}");
    assert!(text.contains("Try Again"), "{text}");
    key(&mut app, Key::Esc);
    // One that never answers is stopped at the limit.
    stand_in(dir.path(), "claude", &["WAIT"]);
    app.side_limit = Duration::from_millis(400);
    select(&mut app, "ICL");
    app.run(Command::AskAgent);
    key(&mut app, Key::Enter);
    let text = answered(&mut app).text();
    assert!(text.contains("took too long"), "{text}");
}

/// Where a Claude Code agent works here with a session its hooks have named, Ask offers to ask
/// in a copy of that agent's conversation (layer 4): the run resumes the agent's session with
/// `--fork-session`, so the agent never sees the question, and a follow-up goes on in the copy,
/// not in the agent's. With no such agent it isn't offered.
#[test]
fn with_the_agents_conversation_asks_in_a_copy_of_it() {
    let (dir, mut app) = page();
    let quick: Vec<&str> = CLAUDE.iter().copied().filter(|l| *l != "WAIT").collect();
    stand_in(dir.path(), "claude", &quick);
    select(&mut app, "ICL");
    app.run(Command::AskAgent);
    let text = screen(&app).text();
    assert!(text.contains("Simplify"), "{text}");
    assert!(!text.contains(WITH_AGENT), "no agent to copy: {text}");
    key(&mut app, Key::Esc);
    // A Claude Code agent in the side pane, its session said by its hooks.
    let script = "printf ready; exec /bin/cat > /dev/null".to_string();
    let argv = ["sh".to_string(), "-c".to_string(), script];
    let root = app.workspace.root().to_path_buf();
    app.start_session("Claude Code", &argv, &[], &root);
    app.serve(Request::Agent(AgentEvent {
        session: "main-7".into(),
        agent: "Claude Code".into(),
        state: AgentState::Working,
        detail: "Working".into(),
        folder: root.display().to_string(),
        pid: None,
        transcript: String::new(),
        name: String::new(),
        prompt: String::new(),
    }));
    select(&mut app, "ICL");
    app.run(Command::AskAgent);
    press(&mut app, WITH_AGENT);
    assert!(app.side.as_ref().unwrap().copy, "chosen");
    typed(&mut app, "How does the work use it?");
    key(&mut app, Key::Enter);
    answered(&mut app);
    let (args, prompt) = given(dir.path(), "claude", 1);
    let resume = args.iter().position(|a| a == "--resume").expect("resumed");
    assert_eq!(args[resume + 1], "main-7");
    assert!(args.iter().any(|a| a == "--fork-session"), "{args:?}");
    assert!(
        args.iter().any(|a| a == "Read,Grep,Glob"),
        "read-only: {args:?}"
    );
    assert!(prompt.contains("copy of the conversation"), "{prompt}");
    assert!(prompt.contains("⟦ICL⟧"), "{prompt}");
    // Its follow-up goes on in the copy, which the stand-in named side-1.
    press(&mut app, FOLLOW_UP);
    typed(&mut app, "And briefly?");
    key(&mut app, Key::Enter);
    answered(&mut app);
    let (args, _) = given(dir.path(), "claude", 2);
    let resume = args.iter().position(|a| a == "--resume").expect("resumed");
    assert_eq!(args[resume + 1], "side-1");
    assert!(!args.iter().any(|a| a == "--fork-session"), "{args:?}");
    // The copy isn't kept as the document's own side conversation.
    let kept = crate::ask::load(app.workspace.root());
    assert!(!kept.contains_key("notes.md"), "{kept:?}");
}
