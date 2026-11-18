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
    let grid = screen(app);
    let (x, y) = on_page(app, &grid, word);
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

/// Presses `words` on the first row that also says `beside`, as a card's buttons do.
fn press_beside(app: &mut App, words: &str, beside: &str) {
    let grid = screen(app);
    let y = (0..grid.height())
        .find(|y| {
            let row = grid.row_text(*y);
            row.contains(words) && row.contains(beside)
        })
        .unwrap_or_else(|| panic!("no {words:?} beside {beside:?}:\n{}", grid.text()));
    let row = grid.row_text(y);
    let x = row[..row.find(words).unwrap()].chars().count() as u16;
    click(app, x + 1, y);
}

/// Where `text` first shows on the page, left of the side pane where it's open.
fn on_page(app: &App, grid: &Grid, text: &str) -> (u16, u16) {
    let pane = app.dock_area().map_or(grid.width(), |pane| pane.x);
    for y in 0..grid.height() {
        let row = grid.row_text(y);
        if let Some(i) = row.find(text) {
            let x = row[..i].chars().count() as u16;
            if x < pane {
                return (x, y);
            }
        }
    }
    panic!("{text:?} not on the page:\n{}", grid.text());
}

/// Drags over the page from where `from` starts to the end of `to`.
fn drag_over(app: &mut App, from: &str, to: &str) {
    let grid = screen(app);
    let (x, y) = on_page(app, &grid, from);
    let (end, end_y) = on_page(app, &grid, to);
    let end = end + to.chars().count() as u16;
    app.handle(mouse(MouseKind::Left, x, y));
    app.handle(mouse(MouseKind::Drag, end, end_y));
    app.handle(mouse(MouseKind::Up, end, end_y));
}

/// Whether the page marks `word` as words a comment is about.
fn marked(app: &App, word: &str) -> bool {
    let grid = screen(app);
    let (x, y) = on_page(app, &grid, word);
    grid.cell(x, y).style.bg == app.theme.tint
}

/// An agent changes the file on disk, and Cellix sees it on its next tick.
fn agent_writes(app: &mut App, dir: &TempDir, text: &str) {
    std::thread::sleep(Duration::from_millis(20));
    fs::write(dir.path().join("note.md"), text).unwrap();
    app.handle(Event::Tick);
}

/// Words selected over a hard-wrapped paragraph are the comment's, all of them: marked on
/// the page wherever they stand, quoted whole to an agent asked to check the review, and
/// replaced whole by a rewrite applied. The quote was their first line alone, so Apply put
/// the whole rewrite in place of that line and left the rest behind.
#[test]
fn a_comment_on_words_over_a_line_break_is_about_all_of_them() {
    let text =
        "# Plan\n\nThe cache goes first so that every read\nafter the start is warm and fast.\n";
    let (_dir, mut app) = page(text);
    // The page shows the line break as a space, so the words stand in one row.
    drag_over(&mut app, "every read", "warm");
    let selected = app.selected_text();
    assert_eq!(
        selected.as_deref(),
        Some("every read\nafter the start is warm")
    );
    app.run(Command::AddComment);
    typed(&mut app, "Warm how?");
    enter(&mut app);
    assert_eq!(app.threads[0].quote, "every read\nafter the start is warm");
    assert_eq!(app.threads[0].line, 3);
    // A click on the heading takes the selection off, so the mark shows.
    let (x, y) = on_page(&app, &screen(&app), "Plan");
    click(&mut app, x + 1, y);
    for word in ["every", "after the", "warm"] {
        assert!(marked(&app, word), "{word} isn't marked");
    }
    assert!(!marked(&app, "first"), "only what the comment is about");
    // The card quotes them as the page reads them.
    let text = screen(&app).text();
    assert!(text.contains("every read after"), "{text}");
    // Asked to check the review, the agent hears all of them.
    app.in_pane = true;
    press(&mut app, "Check my review");
    let prompt = app.take_outgoing_prompt().expect("the check's prompt");
    assert!(
        prompt.contains("\"every read after the start is warm\""),
        "{prompt}"
    );
    // A rewrite applied takes the place of all of them.
    let id = app.threads[0].id;
    let (found, _) = std::sync::mpsc::channel();
    app.serve(Request::Reply {
        id,
        author: "Claude".into(),
        text: "Rewrite: every later read is warm".into(),
        found,
    });
    press_beside(&mut app, "Apply", "Not this");
    assert_eq!(
        app.active_text().unwrap(),
        "# Plan\n\nThe cache goes first so that {~~every read\nafter the start is warm~>every later read is warm~~} and fast.\n"
    );
}

/// When an agent rewords what a comment is about, the card still quotes the words it was
/// made on and says they changed, and the page marks the words as they read now. Gone, the
/// card says so and nothing is marked; put back, it's as it was. The quote took the new
/// words, so the card showed words nobody had commented on, and when they went the card
/// said nothing and the page lost its mark.
#[test]
fn a_card_keeps_the_words_it_was_made_on_and_says_when_they_change() {
    let (dir, mut app) = page("The cache goes first.\n\nThe tests stay fast on every run.\n");
    drag_over(&mut app, "tests", "every");
    app.run(Command::AddComment);
    typed(&mut app, "Even on CI?");
    enter(&mut app);
    assert_eq!(app.threads[0].quote, "tests stay fast on every");
    agent_writes(
        &mut app,
        &dir,
        "The cache goes first.\n\nThe tests stay quick on every run.\n",
    );
    let (x, y) = on_page(&app, &screen(&app), "cache");
    click(&mut app, x + 1, y);
    let thread = &app.threads[0];
    assert_eq!(thread.quote, "tests stay fast on every", "as it was made");
    assert_eq!(thread.now, "tests stay quick on every");
    let text = screen(&app).text();
    assert!(text.contains(crate::threads::CHANGED_SINCE), "{text}");
    assert!(text.contains("tests stay fast on every"), "{text}");
    assert!(marked(&app, "quick"), "marked as they read now");
    // A click on the card goes to the words as they read now.
    press_beside(&mut app, "tests stay fast", "tests stay fast");
    assert_eq!(
        app.selected_text().as_deref(),
        Some("tests stay quick on every")
    );

    agent_writes(
        &mut app,
        &dir,
        "The cache goes first.\n\nNothing about that now.\n",
    );
    let (x, y) = on_page(&app, &screen(&app), "cache");
    click(&mut app, x + 1, y);
    let text = screen(&app).text();
    assert!(text.contains(crate::threads::WORDS_GONE), "{text}");
    assert!(app.threads[0].gone);
    assert!(!marked(&app, "Nothing"), "nothing to mark");

    agent_writes(
        &mut app,
        &dir,
        "The cache goes first.\n\nThe tests stay fast on every run.\n",
    );
    let text = screen(&app).text();
    assert!(!text.contains(crate::threads::WORDS_GONE), "{text}");
    assert!(!text.contains(crate::threads::CHANGED_SINCE), "{text}");
    assert!(marked(&app, "fast"));
}

/// Next Comment and Previous Comment go through the file's comments in the order their words
/// stand in, round at the ends: each selects its words, lights its card in the Comments pane,
/// which opens on it, and the status bar says which of how many. A press on a line's ◆ in
/// the page's margin opens the Comments on that line's comment; presses there did nothing.
/// Check My Review is in the palette too.
#[test]
fn next_comment_goes_through_the_files_comments_and_the_mark_opens_one() {
    let (_dir, mut app) = page("Alpha one here.\n\nBeta two here.\n\nGamma three here.\n");
    // Made out of the document's order, which is the order they're gone through in.
    comment(&mut app, "Gamma", "Third");
    comment(&mut app, "Alpha", "First");
    comment(&mut app, "Beta", "Second");
    let id = |app: &App, text: &str| app.threads.iter().find(|t| t.text == text).unwrap().id;
    let (x, y) = on_page(&app, &screen(&app), "Alpha");
    click(&mut app, x, y);
    app.dock.open = false;
    app.run(Command::NextComment);
    assert!(app.dock.open, "the pane opens on it");
    assert_eq!(app.lit_thread, Some(id(&app, "Second")));
    assert_eq!(app.selected_text().as_deref(), Some("Beta"));
    assert!(screen(&app).text().contains("Comment 2 of 3"));
    app.run(Command::NextComment);
    assert_eq!(app.selected_text().as_deref(), Some("Gamma"));
    assert!(screen(&app).text().contains("Comment 3 of 3"));
    app.run(Command::NextComment);
    assert_eq!(
        app.lit_thread,
        Some(id(&app, "First")),
        "round to the first"
    );
    assert!(screen(&app).text().contains("Comment 1 of 3"));
    app.run(Command::PreviousComment);
    assert_eq!(app.lit_thread, Some(id(&app, "Third")), "and back round");
    // The ◆ beside Beta's line, with the pane shut.
    app.dock.open = false;
    app.lit_thread = None;
    let grid = screen(&app);
    let (_, row) = on_page(&app, &grid, "Beta two");
    let line = grid.row_text(row);
    let mark = line[..line.find('◆').expect("a mark beside the line")]
        .chars()
        .count() as u16;
    click(&mut app, mark, row);
    assert!(app.dock.open);
    assert_eq!(app.dock.front, super::Front::Comments);
    assert_eq!(app.lit_thread, Some(id(&app, "Second")));
    assert_eq!(app.selected_text().as_deref(), Some("Beta"));
    // Check My Review from the palette asks the agent, as the pane's button does.
    app.in_pane = true;
    app.run(Command::CheckReview);
    let prompt = app.take_outgoing_prompt().expect("the check's prompt");
    assert!(prompt.contains("Second"), "{prompt}");
}
