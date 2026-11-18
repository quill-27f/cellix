//! Suggestions on a page: Accept and Reject by the one at the caret or under the pointer, the
//! row in the Comments pane that counts them and settles them all, and an agent's rewrite taken
//! in one step. Driven through the app and read off the drawn grid.

use std::fs;

use cellix_input::{Event, Key, KeyEvent, Mods, MouseEvent, MouseKind};
use cellix_render::Grid;
use tempfile::TempDir;

use super::{App, Command};

fn page(name: &str, text: &str) -> (TempDir, App) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(name);
    fs::write(&path, text).unwrap();
    let mut app = App::new(dir.path().to_path_buf(), (150, 40));
    app.open(&path).unwrap();
    (dir, app)
}

/// A paper loading the package its suggestions print with, around `body`.
fn paper(body: &str) -> String {
    format!(
        "\\documentclass{{article}}\n\\usepackage{{changes}}\n\\begin{{document}}\n\n{body}\n\n\\end{{document}}\n"
    )
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
        fine_x: x as f32 + 0.5,
        fine_y: y as f32 + 0.5,
        mods: Mods::default(),
    })
}

/// A press and its release, as a click is.
fn click(app: &mut App, (x, y): (u16, u16)) {
    app.handle(mouse(MouseKind::Left, x, y));
    app.handle(mouse(MouseKind::Up, x, y));
}

fn point(app: &mut App, (x, y): (u16, u16)) {
    app.handle(mouse(MouseKind::Move, x, y));
}

/// Where `text` is first drawn, under the tabs.
fn find(grid: &Grid, text: &str) -> Option<(u16, u16)> {
    (1..grid.height()).find_map(|y| {
        let row = grid.row_text(y);
        let at = row.find(text)?;
        Some((row[..at].chars().count() as u16, y))
    })
}

fn spot(app: &App, text: &str) -> (u16, u16) {
    let grid = screen(app);
    find(&grid, text).unwrap_or_else(|| panic!("{text:?} isn't drawn:\n{}", grid.text()))
}

/// Accept and Reject by a suggestion, as Review has them: where each stands, if they do.
fn chips(app: &App) -> Option<((u16, u16), (u16, u16))> {
    let grid = screen(app);
    let (x, y) = find(&grid, " Reject  Accept ")?;
    Some(((x + 2, y), (x + 10, y)))
}

fn said(app: &App) -> String {
    app.message.borrow().clone()
}

fn undo(app: &mut App) {
    app.handle(Event::Key(KeyEvent {
        key: Key::Char('z'),
        mods: Mods {
            ctrl: true,
            ..Mods::default()
        },
    }));
}

#[test]
fn a_suggestion_at_the_caret_is_accepted_or_rejected_by_its_chips() {
    let (_dir, mut app) = page(
        "note.md",
        "# Notes\n\nKeep {++added++} and {--gone--} end.\n",
    );
    assert!(chips(&app).is_none(), "the caret is in no suggestion");
    let (x, y) = spot(&app, "added");
    click(&mut app, (x + 2, y));
    let (_, accept) = chips(&app).expect("Accept and Reject by it");
    assert!(
        accept.1 < y,
        "over the row it's on, as a selection's buttons are"
    );
    click(&mut app, accept);
    assert_eq!(
        app.active_text().unwrap(),
        "# Notes\n\nKeep added and {--gone--} end.\n"
    );
    assert_eq!(said(&app), "Accepted. 1 suggestion left");
    assert!(
        chips(&app).is_none(),
        "the caret is after it, in plain words now"
    );
    let (x, y) = spot(&app, "gone");
    click(&mut app, (x + 1, y));
    let (reject, _) = chips(&app).expect("by the other one");
    click(&mut app, reject);
    assert_eq!(
        app.active_text().unwrap(),
        "# Notes\n\nKeep added and gone end.\n"
    );
    assert_eq!(said(&app), "Rejected. No suggestions left");
    undo(&mut app);
    assert_eq!(
        app.active_text().unwrap(),
        "# Notes\n\nKeep added and {--gone--} end.\n",
        "one edit to undo"
    );
}

#[test]
fn the_chips_stand_by_the_suggestion_under_the_pointer_and_stay_on_the_way_to_them() {
    let (_dir, mut app) = page(
        "note.md",
        "# Notes\n\nThe {~~tests~>experiments~~} show it, and {++clearly++} so.\n",
    );
    assert!(chips(&app).is_none());
    let (x, y) = spot(&app, "clearly");
    point(&mut app, (x + 3, y));
    let (_, accept) = chips(&app).expect("chips by what's pointed at");
    // Up from the words to the chips, by way of the row's other words.
    point(&mut app, (accept.0, y));
    point(&mut app, accept);
    let (_, still) = chips(&app).expect("still there on the chips");
    assert_eq!(still, accept);
    click(&mut app, accept);
    assert_eq!(
        app.active_text().unwrap(),
        "# Notes\n\nThe {~~tests~>experiments~~} show it, and clearly so.\n",
        "the one pointed at, not another"
    );
    // Off the page, nothing is pointed at, and the caret's in no suggestion.
    point(&mut app, (2, 2));
    assert!(chips(&app).is_none());
    let (x, y) = spot(&app, "experiments");
    point(&mut app, (x + 2, y));
    let (reject, _) = chips(&app).expect("by a replacement too");
    click(&mut app, reject);
    assert_eq!(
        app.active_text().unwrap(),
        "# Notes\n\nThe tests show it, and clearly so.\n"
    );
}

#[test]
fn a_selection_keeps_its_own_buttons_over_a_suggestion() {
    let (_dir, mut app) = page("note.md", "# Notes\n\nKeep {++added words++} here.\n");
    let (x, y) = spot(&app, "added");
    for _ in 0..2 {
        click(&mut app, (x + 1, y));
    }
    let text = screen(&app).text();
    assert!(text.contains("Comment"), "{text}");
    assert!(chips(&app).is_none(), "{text}");
}

#[test]
fn a_paper_s_suggestions_have_the_same_chips() {
    let source = paper(
        "This sentence \\added{gains words}, \\deleted{loses these}, and \
         \\replaced{a new word}{an old word}.",
    );
    let (_dir, mut app) = page("paper.tex", &source);
    let (x, y) = spot(&app, "gains");
    click(&mut app, (x + 2, y));
    let (_, accept) = chips(&app).expect("by \\added");
    click(&mut app, accept);
    let text = app.active_text().unwrap();
    assert!(
        text.contains("This sentence gains words, \\deleted"),
        "{text}"
    );
    let (x, y) = spot(&app, "new word");
    click(&mut app, (x + 2, y));
    let (reject, _) = chips(&app).expect("by \\replaced");
    click(&mut app, reject);
    let text = app.active_text().unwrap();
    assert!(text.contains(", and an old word."), "{text}");
    assert_eq!(said(&app), "Rejected. 1 suggestion left");
}

#[test]
fn the_right_click_menu_and_the_commands_still_settle_one() {
    let (_dir, mut app) = page("note.md", "# Notes\n\nKeep {++added++} end.\n");
    let (x, y) = spot(&app, "added");
    click(&mut app, (x + 2, y));
    app.run(Command::AcceptChange);
    assert_eq!(app.active_text().unwrap(), "# Notes\n\nKeep added end.\n");
}

#[test]
fn the_comments_pane_counts_the_suggestions_and_next_steps_through_them() {
    let (_dir, mut app) = page(
        "note.md",
        "# Notes\n\nOne {++alpha++} two {--beta--} three {~~gamma~>delta~~} four.\n",
    );
    app.run(Command::ShowComments);
    let text = screen(&app).text();
    assert!(text.contains("3 suggestions"), "{text}");
    assert!(
        text.contains("Accept All") && text.contains("Reject All"),
        "{text}"
    );
    let next = spot(&app, "Next");
    click(&mut app, (next.0 + 1, next.1));
    assert_eq!(said(&app), "Suggestion 1 of 3");
    assert!(screen(&app).text().contains("Suggestion 1 of 3"));
    assert!(chips(&app).is_some(), "Accept and Reject stand by it");
    assert!(!app.dock.focused, "the keyboard stays in the file");
    for says in ["Suggestion 2 of 3", "Suggestion 3 of 3"] {
        let next = spot(&app, "Next");
        click(&mut app, (next.0 + 1, next.1));
        assert_eq!(said(&app), says);
        assert!(screen(&app).text().contains(says), "{says}");
    }
    let next = spot(&app, "Next");
    click(&mut app, (next.0 + 1, next.1));
    assert_eq!(said(&app), "Suggestion 1 of 3", "round again");
    // Settled at the caret by its chips, the count goes down.
    let (_, accept) = chips(&app).expect("chips");
    click(&mut app, accept);
    assert!(screen(&app).text().contains("2 suggestions"));
}

#[test]
fn accept_all_and_reject_all_in_the_comments_pane_settle_every_suggestion() {
    let (_dir, mut app) = page(
        "note.md",
        "# Notes\n\nOne {++alpha++} two {--beta--} three {~~gamma~>delta~~} four.\n",
    );
    app.run(Command::ShowComments);
    let all = spot(&app, "Accept All");
    click(&mut app, (all.0 + 2, all.1));
    assert_eq!(
        app.active_text().unwrap(),
        "# Notes\n\nOne alpha two  three delta four.\n"
    );
    assert_eq!(said(&app), "Accepted 3 changes");
    let text = screen(&app).text();
    assert!(
        !text.contains("Accept All"),
        "no row once none are left: {text}"
    );

    let (_dir, mut app) = page(
        "paper.tex",
        &paper(
            "This sentence \\added{gains words}, \\deleted{loses these}, and \\replaced{a new word}{an old word}.",
        ),
    );
    app.run(Command::ShowComments);
    assert!(screen(&app).text().contains("3 suggestions"));
    let all = spot(&app, "Reject All");
    click(&mut app, (all.0 + 2, all.1));
    let text = app.active_text().unwrap();
    assert!(
        text.contains("This sentence , loses these, and an old word."),
        "{text}"
    );
    assert_eq!(said(&app), "Rejected 3 changes");
}

fn typed(app: &mut App, text: &str) {
    for c in text.chars() {
        app.handle(Event::Key(KeyEvent::plain(Key::Char(c))));
    }
    app.handle(Event::Key(KeyEvent::plain(Key::Enter)));
}

/// Comments on what's selected, from `from` to the end of `to`, and has an agent answer with
/// a rewrite.
fn rewrite_asked(app: &mut App, (from, to): (&str, &str), rewrite: &str) -> u32 {
    let text = app.active_text().unwrap();
    let start = text[..text.find(from).unwrap()].chars().count();
    let end = text[..text.find(to).unwrap()].chars().count() + to.chars().count();
    let editor = app.editor_mut().unwrap();
    editor.selections.set_single(cellix_core::Selection {
        anchor: start,
        head: end,
        goal: None,
    });
    app.run(Command::AddComment);
    typed(app, "Reword this");
    let id = app.threads[0].id;
    let (found, _) = std::sync::mpsc::channel();
    app.serve(cellix_agent::Request::Reply {
        id,
        author: "Claude".into(),
        text: format!("Rewrite: {rewrite}"),
        found,
    });
    id
}

#[test]
fn accept_rewrite_takes_the_words_across_a_hard_wrapped_paragraph_in_one_step() {
    let source = "# Notes\n\nThe cache goes first,\nthen the index is built\nfrom the log.\n";
    let (_dir, mut app) = page("note.md", source);
    let id = rewrite_asked(&mut app, ("goes", "index"), "comes last, after the index");
    let text = screen(&app).text();
    assert!(
        text.contains("Accept Rewrite") && text.contains("Apply as Suggestion"),
        "{text}"
    );
    let button = spot(&app, "Accept Rewrite");
    click(&mut app, (button.0 + 2, button.1));
    assert_eq!(
        app.active_text().unwrap(),
        "# Notes\n\nThe cache comes last, after the index is built\nfrom the log.\n",
        "every line of what was commented on, and nothing else"
    );
    let thread = app.threads.iter().find(|t| t.id == id).unwrap();
    assert!(thread.resolved, "its ask is met");
    assert!(!app.dock.focused);
    undo(&mut app);
    assert_eq!(app.active_text().unwrap(), source, "one edit to undo");
}

#[test]
fn a_rewrite_applied_as_a_suggestion_in_a_paper_settles_by_its_chips() {
    let source = paper("The cache goes first,\nthen the index is built\nfrom the log.");
    let (_dir, mut app) = page("paper.tex", &source);
    let id = rewrite_asked(&mut app, ("goes", "index"), "comes last, after the index");
    let button = spot(&app, "Apply as Suggestion");
    click(&mut app, (button.0 + 2, button.1));
    let text = app.active_text().unwrap();
    assert!(
        text.contains(
            "The cache \\replaced{comes last, after the index}{goes first,\nthen the index} is"
        ),
        "{text}"
    );
    let thread = app.threads.iter().find(|t| t.id == id).unwrap();
    assert!(!thread.resolved, "still to be settled on the page");
    let (x, y) = spot(&app, "comes last");
    click(&mut app, (x + 2, y));
    let (_, accept) = chips(&app).unwrap_or_else(|| {
        let grid = screen(&app);
        panic!("no chips by the rewrite at {x},{y}:\n{}", grid.text())
    });
    click(&mut app, accept);
    let text = app.active_text().unwrap();
    assert!(
        text.contains("The cache comes last, after the index is built\nfrom the log."),
        "{text}"
    );
}
