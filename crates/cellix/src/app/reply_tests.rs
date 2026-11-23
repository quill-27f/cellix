//! A reply on a comment's card can be changed or removed as the comment itself can: the
//! reviewer's own replies have Edit and Delete, an agent's only Delete. The user: "in comments,
//! current edit delete work for only the main comment not for replies, make it possible to edit
//! delete the reply".

use std::fs;

use cellix_input::{Command, Event, Key, KeyEvent, MouseEvent, MouseKind};
use cellix_render::Grid;
use cellix_review::threads::{ADDRESSED, Reply, YOU};
use tempfile::TempDir;

use super::{App, Choice, Overlay, ThreadHit};

fn project() -> (TempDir, App) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("notes.md");
    fs::write(&path, "# Notes\n\nThe cache stays warm across runs.\n").unwrap();
    let mut app = App::new(dir.path().to_path_buf(), (150, 40));
    app.open(&path).unwrap();
    (dir, app)
}

fn screen(app: &App) -> Grid {
    let mut grid = Grid::new(app.size().0, app.size().1, app.background());
    app.draw(&mut grid);
    grid
}

fn typed(app: &mut App, text: &str) {
    for c in text.chars() {
        app.handle(Event::Key(KeyEvent::plain(Key::Char(c))));
    }
}

fn key(app: &mut App, key: Key) {
    app.handle(Event::Key(KeyEvent::plain(key)));
}

/// The pointer resting on row `y`, at column `x`.
fn point(app: &mut App, x: u16, y: u16) {
    app.handle(Event::Mouse(MouseEvent {
        kind: MouseKind::Move,
        x,
        y,
        fine_x: f32::from(x),
        fine_y: f32::from(y) + 0.5,
        mods: Default::default(),
    }));
}

/// Where `words` are on screen, as (column, row).
fn find(grid: &Grid, words: &str) -> (u16, u16) {
    (0..grid.height())
        .find_map(|y| {
            let row = grid.row_text(y);
            row.find(words)
                .map(|at| (row[..at].chars().count() as u16, y))
        })
        .unwrap_or_else(|| panic!("{words:?} isn't on screen:\n{}", grid.text()))
}

/// A comment with the reviewer's reply on it, then an agent's, the agent told of both.
fn with_replies(app: &mut App) -> u32 {
    app.run(Command::AddComment);
    typed(app, "Is this true for cold starts?");
    key(app, Key::Enter);
    let id = app.threads.last().expect("the comment").id;
    app.thread_hit(ThreadHit::Reply(id));
    typed(app, "And after a deploy");
    key(app, Key::Enter);
    let thread = app.threads.iter_mut().find(|t| t.id == id).unwrap();
    thread.replies.push(Reply::new(
        "Claude",
        format!("{ADDRESSED} it stays warm now"),
    ));
    thread.told = Some(1);
    app.save_threads();
    id
}

/// What's saved for the project, as `cellix threads` and the prompt hook read it.
fn saved(dir: &TempDir) -> Vec<cellix_review::Thread> {
    cellix_review::threads::load(dir.path()).unwrap()
}

#[test]
fn the_reviewers_own_reply_can_be_changed_and_an_agents_cannot() {
    let (dir, mut app) = project();
    let id = with_replies(&mut app);
    // The pointer on the reviewer's reply shows its Edit and Delete on the line over it.
    let grid = screen(&app);
    let (x, y) = find(&grid, "And after a deploy");
    point(&mut app, x + 2, y);
    let grid = screen(&app);
    // On the line over its words, which says who wrote it.
    let row = grid.row_text(y - 1);
    assert!(row.contains("Edit") && row.contains("Delete"), "{row}");
    // On the agent's, only Delete.
    let (x, y) = find(&grid, "stays warm now");
    point(&mut app, x + 2, y);
    let grid = screen(&app);
    let row = grid.row_text(y - 1);
    assert!(row.contains("Claude"), "{row}");
    assert!(row.contains("Delete") && !row.contains("Edit"), "{row}");
    app.thread_hit(ThreadHit::EditReply(id, 1));
    assert!(
        app.dock.draft.is_none(),
        "an agent's words aren't the reviewer's to change"
    );

    // Edit opens the reply's words in the box replies are written in, the caret after them.
    app.thread_hit(ThreadHit::EditReply(id, 0));
    let draft = app.dock.draft.as_ref().expect("the reply being changed");
    assert_eq!(
        (draft.text.text(), draft.text.caret()),
        ("And after a deploy", 18)
    );
    typed(&mut app, " too");
    let text = screen(&app).text();
    assert!(text.contains("And after a deploy too"), "{text}");
    // Save and Cancel take the card's buttons' place, and the hint says Enter saves.
    assert!(text.contains(" Save "), "{text}");
    assert!(text.contains("Enter saves."), "{text}");
    key(&mut app, Key::Enter);
    assert!(app.dock.draft.is_none());
    let thread = &saved(&dir)[0];
    assert_eq!(thread.replies[0].text, "And after a deploy too");
    assert_eq!(thread.replies[0].author, YOU);
    // The agent was told of it already, so it isn't handed over again.
    assert!(!thread.waiting());

    // Esc leaves it as it was.
    app.thread_hit(ThreadHit::EditReply(id, 0));
    typed(&mut app, "!!");
    key(&mut app, Key::Esc);
    assert_eq!(saved(&dir)[0].replies[0].text, "And after a deploy too");
}

#[test]
fn a_reply_is_deleted_and_the_agent_is_told_what_it_hasnt_had() {
    let (dir, mut app) = project();
    let id = with_replies(&mut app);
    assert!(
        app.threads[0].addressed_by().is_some(),
        "the agent said it's done"
    );
    // Delete asks first, as deleting a comment does.
    app.thread_hit(ThreadHit::DeleteReply(id, 1));
    assert!(matches!(app.overlay, Overlay::Confirm(_)));
    assert!(screen(&app).text().contains("Delete this reply?"));
    app.overlay = Overlay::None;
    app.choose(Choice::DeleteReply(id, 1));
    let thread = &saved(&dir)[0];
    assert_eq!(thread.replies.len(), 1);
    assert_eq!(
        thread.addressed_by(),
        None,
        "its word it was done went with it"
    );
    assert!(!screen(&app).text().contains("stays warm now"));
    // The reviewer's reply was told already: the agent isn't asked again for it.
    assert_eq!(thread.told, Some(1));
    assert!(!thread.waiting());

    // The reviewer's own reply, changed before an agent heard of it, goes with its new words.
    app.thread_hit(ThreadHit::Reply(id));
    typed(&mut app, "Also on Linux?");
    key(&mut app, Key::Enter);
    app.thread_hit(ThreadHit::EditReply(id, 1));
    typed(&mut app, " And macOS?");
    key(&mut app, Key::Enter);
    let thread = &saved(&dir)[0];
    assert!(thread.waiting(), "it's new to the agent");
    assert!(
        thread.message().contains("Also on Linux? And macOS?"),
        "{}",
        thread.message()
    );
    // Deleted, it's gone from what the agent is told, and nothing waits for it.
    app.choose(Choice::DeleteReply(id, 1));
    let thread = &saved(&dir)[0];
    assert!(!thread.message().contains("Linux"), "{}", thread.message());
    assert!(!thread.waiting());
    // The review's Markdown has what's left.
    let markdown = super::report::markdown(&app.threads).expect("the review");
    assert!(markdown.contains("And after a deploy"), "{markdown}");
    assert!(!markdown.contains("Linux"), "{markdown}");
}
