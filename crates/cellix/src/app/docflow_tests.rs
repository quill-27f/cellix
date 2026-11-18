//! The way a comment on a document goes: written beside its words, delivered to the agent,
//! answered, followed as the words change, and settled. Driven as a person would, through the
//! App's input and the Grid it draws, with stand-in agents that write down what they hear.

use std::fs;
use std::path::Path;
use std::time::{Duration, Instant};

use cellix_input::{Command, Event, Key, KeyEvent, Mods, MouseEvent, MouseKind};
use cellix_render::Grid;
use tempfile::TempDir;

use cellix_agent::Request;

use super::App;

/// How long a test waits for a stand-in to hear something, on a loaded machine.
const PATIENCE: Duration = Duration::from_secs(120);

fn page(text: &str) -> (TempDir, App) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("note.md");
    fs::write(&path, text).unwrap();
    let mut app = App::new(dir.path().to_path_buf(), (150, 40));
    app.open(&path).unwrap();
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

fn click(app: &mut App, x: u16, y: u16) {
    app.handle(mouse(MouseKind::Left, x, y));
    app.handle(mouse(MouseKind::Up, x, y));
}

fn typed(app: &mut App, text: &str) {
    for c in text.chars() {
        app.handle(Event::Key(KeyEvent::plain(Key::Char(c))));
    }
}

fn enter(app: &mut App) {
    app.handle(Event::Key(KeyEvent::plain(Key::Enter)));
}

/// Where `text` first shows on the screen, as its column and row.
fn find(grid: &Grid, text: &str) -> (u16, u16) {
    for y in 0..grid.height() {
        if let Some(i) = grid.row_text(y).find(text) {
            return (grid.row_text(y)[..i].chars().count() as u16, y);
        }
    }
    panic!("{text:?} not on screen:\n{}", grid.text());
}

/// Selects a word on the page with a double click, as a person picks one to comment on.
fn select_word(app: &mut App, word: &str) {
    let (x, y) = find(&screen(app), word);
    click(app, x + 1, y);
    click(app, x + 1, y);
}

/// Comments on the word, as Alt+C and typing do.
fn comment(app: &mut App, word: &str, text: &str) {
    select_word(app, word);
    app.run(Command::AddComment);
    typed(app, text);
    enter(app);
}

/// Starts a stand-in agent named `name` in the side pane, which writes what it's sent to
/// `log` in the project, and waits until it's ready.
fn stand_in(app: &mut App, name: &str, log: &str) {
    let script = format!("printf ready; cat > {log}");
    let argv = ["sh".to_string(), "-c".to_string(), script];
    let root = app.workspace.root().to_path_buf();
    app.start_session(name, &argv, &[], &root);
    let deadline = Instant::now() + PATIENCE;
    while !screen(app).text().contains("ready") {
        assert!(Instant::now() < deadline, "{name} never started");
        app.poll();
        app.fit_dock();
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// What the stand-in wrote down, once it has `want`.
fn heard(app: &mut App, log: &Path, want: &str) -> String {
    let deadline = Instant::now() + PATIENCE;
    loop {
        let text = fs::read_to_string(log).unwrap_or_default();
        if text.contains(want) {
            return text;
        }
        assert!(Instant::now() < deadline, "it never heard {want:?}: {text}");
        app.poll();
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Presses the first place on screen that says `words`.
fn press(app: &mut App, words: &str) {
    let (x, y) = find(&screen(app), words);
    click(app, x + 1, y);
}

/// Presses a card's button: `words` on the row of a card's buttons, which has Resolve, and
/// not where a card's words name it.
fn press_button(app: &mut App, words: &str) {
    let grid = screen(app);
    let y = (0..grid.height())
        .find(|y| {
            let row = grid.row_text(*y);
            row.contains(words) && row.contains("Resolve")
        })
        .unwrap_or_else(|| panic!("no button {words:?}:\n{}", grid.text()));
    let row = grid.row_text(y);
    let x = row[..row.find(words).unwrap()].chars().count() as u16;
    click(app, x + 1, y);
}

/// With two agents in the side pane, what's sent goes to the one in front, and the card
/// waiting on its answer names that one: it named the first agent while the one in front
/// was asked.
#[test]
fn a_comment_waits_on_the_agent_it_was_sent_to() {
    let (dir, mut app) = page("The cache goes first.\n");
    stand_in(&mut app, "Codex", "codex.txt");
    stand_in(&mut app, "Claude Code", "claude.txt");
    comment(&mut app, "cache", "Why first?");
    app.dock.show_comments();
    press_button(&mut app, "Ask agent");
    let said = heard(&mut app, &dir.path().join("claude.txt"), "Why first?");
    assert!(said.contains("cellix reply"), "{said}");
    let codex = fs::read_to_string(dir.path().join("codex.txt")).unwrap_or_default();
    assert!(codex.is_empty(), "the one behind heard it: {codex}");
    let text = screen(&app).text();
    assert!(text.contains("Waiting for Claude Code…"), "{text}");
    assert!(!text.contains("Waiting for Codex"), "{text}");
}

/// What the prompt hook would take now: the threads no agent has had, as JSON.
fn take_threads(app: &mut App) -> String {
    let (sender, answer) = std::sync::mpsc::channel();
    app.serve(Request::TakeThreads(sender));
    answer.recv().unwrap()
}

/// Send Comments to Agent sends a document's comments, which are threads, and not only
/// the review's comments on code: on a page it said there were none to send. They go in one
/// message, each with how to answer it, and are told, so the prompt hook doesn't send them
/// again and their cards wait on the agent.
#[test]
fn sending_comments_sends_a_documents_comments_once() {
    let (dir, mut app) = page("The cache goes first.\n\nThen the tests run.\n");
    comment(&mut app, "cache", "Why first?");
    comment(&mut app, "tests", "Which tests?");
    stand_in(&mut app, "Claude Code", "heard.txt");
    app.dock.show_comments();
    let text = screen(&app).text();
    assert!(text.contains("Goes to Claude Code"), "{text}");
    app.run(Command::SendToAgent);
    let said = heard(&mut app, &dir.path().join("heard.txt"), "Which tests?");
    assert!(said.contains("Why first?"), "{said}");
    assert_eq!(said.matches("cellix reply").count(), 2, "{said}");
    assert_eq!(said.lines().count(), 1, "one message: {said}");
    assert!(app.threads.iter().all(|t| t.told == Some(0)));
    let kept = cellix_review::threads::load(dir.path()).unwrap();
    assert!(kept.iter().all(|t| t.told == Some(0)), "{kept:?}");
    assert_eq!(take_threads(&mut app), "[]", "the hook has nothing to add");
    let text = screen(&app).text();
    assert!(text.contains("Waiting for Claude Code…"), "{text}");
    assert!(!text.contains("Goes to"), "{text}");
}

/// Inside Claude Code, sending a document's comments starts a turn, and the prompt hook
/// takes them: the mod counted only the review's comments on code, so none went.
#[test]
fn inside_claude_code_a_documents_comments_start_a_turn() {
    let (_dir, mut app) = page("The cache goes first.\n");
    app.in_pane = true;
    comment(&mut app, "cache", "Why first?");
    app.run(Command::SendToAgent);
    assert_eq!(
        app.take_outgoing_prompt().as_deref(),
        Some(super::SEND_PROMPT)
    );
    assert!(take_threads(&mut app).contains("Why first?"));
    // Ask agent on a card starts one too.
    comment(&mut app, "goes", "And this?");
    app.dock.show_comments();
    let text = screen(&app).text();
    assert!(text.contains("Goes to Claude Code"), "{text}");
    press_button(&mut app, "Ask agent");
    assert_eq!(
        app.take_outgoing_prompt().as_deref(),
        Some(super::SEND_PROMPT)
    );
}

/// An agent's suggestion on a document, kept, is the person's comment, and goes to the
/// agent as theirs do. It stayed marked as told, so it never went.
#[test]
fn a_kept_suggestion_on_a_document_goes_to_the_agent() {
    let (_dir, mut app) = page("Our method is better on every benchmark.\n");
    let (made, number) = std::sync::mpsc::channel();
    app.serve(Request::Suggest {
        file: "note.md".into(),
        line: 1,
        kind: "Evidence".into(),
        quote: "every benchmark".into(),
        text: "Name the benchmarks.".into(),
        author: "Claude".into(),
        made,
    });
    number.recv().unwrap();
    app.run(Command::ShowComments);
    assert_eq!(
        take_threads(&mut app),
        "[]",
        "the agent's own, not sent back"
    );
    press(&mut app, "Keep");
    let text = screen(&app).text();
    assert!(text.contains("Goes to an agent"), "{text}");
    let taken = take_threads(&mut app);
    assert!(taken.contains("Name the benchmarks."), "{taken}");
}

/// Check my review asks for a verdict on each comment, so each is told: the prompt hook
/// added the same comments to that prompt again, asking for a plain answer.
#[test]
fn check_my_review_hands_each_comment_over_once() {
    let (dir, mut app) = page("The cache goes first.\n");
    comment(&mut app, "cache", "Why first?");
    stand_in(&mut app, "Claude Code", "heard.txt");
    app.dock.show_comments();
    press(&mut app, "Check my review");
    let said = heard(&mut app, &dir.path().join("heard.txt"), "Why first?");
    assert!(said.contains("Agree (<kind>)"), "{said}");
    assert_eq!(said.matches("Why first?").count(), 1, "{said}");
    assert_eq!(take_threads(&mut app), "[]", "nothing for the hook to add");
}

/// A waiting card says how it reaches the agent, for each way there is: the hooks of
/// Claude Code, Codex and Gemini CLI hand comments over with a prompt, other agents' don't,
/// and with none here it waits for one.
#[test]
fn a_waiting_comment_says_how_it_reaches_the_agent() {
    let (_dir, mut app) = page("The cache goes first.\n");
    comment(&mut app, "cache", "Why first?");
    let text = screen(&app).text();
    assert!(text.contains("Goes to an agent"), "{text}");
    assert!(text.contains("starts one"), "{text}");
    stand_in(&mut app, "OpenCode", "heard.txt");
    app.dock.show_comments();
    let text = screen(&app).text();
    assert!(text.contains("OpenCode isn't"), "{text}");
    assert!(text.contains("sends it"), "{text}");
}
