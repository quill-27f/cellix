//! The review as Markdown: what Copy puts on the clipboard and Save writes and opens.

use std::fs;

use cellix_input::{Command, Event, Key, KeyEvent, Mods, MouseEvent, MouseKind};
use cellix_render::Grid;
use cellix_review::Thread;
use tempfile::TempDir;

use super::{App, Front, Overlay};

fn screen(app: &App) -> Grid {
    let mut grid = Grid::new(app.size().0, app.size().1, app.background());
    app.draw(&mut grid);
    grid
}

fn find(grid: &Grid, text: &str) -> (u16, u16) {
    for y in 0..grid.height() {
        if let Some(i) = grid.row_text(y).find(text) {
            return (grid.row_text(y)[..i].chars().count() as u16, y);
        }
    }
    panic!("{text:?} not on screen:\n{}", grid.text());
}

fn mouse(kind: MouseKind, x: u16, y: u16) -> Event {
    Event::Mouse(MouseEvent {
        kind,
        x,
        y,
        fine_x: f32::from(x),
        fine_y: f32::from(y) + 0.5,
        mods: Mods::default(),
    })
}

fn key(app: &mut App, key: Key) {
    app.handle(Event::Key(KeyEvent::plain(key)));
}

fn typed(app: &mut App, text: &str) {
    for c in text.chars() {
        key(app, Key::Char(c));
    }
}

/// Two files of a paper, with a comment of each kind but two, open and resolved, a verdict
/// each way, the reviewer's own answer, a suggestion kept and two not.
const THREADS: &str = r#"[
  {"id": 1, "file": "Results.md", "line": 9, "quote": "a gain of 4 points",
   "text": "Over which baseline?", "kind": "Evidence",
   "replies": [{"author": "Claude", "text": "Disagree (Evidence): table 2 names the baseline."}]},
  {"id": 2, "file": "intro.md", "line": 12, "quote": "is robust", "text": "Robust to what?",
   "kind": "Clarity", "resolved": true,
   "replies": [{"author": "Claude", "text": "Agree (Clarity): the checks should be named."},
               {"author": "You", "text": "Done in the next draft."}]},
  {"id": 3, "file": "intro.md", "line": 4, "quote": "We propose", "text": "Say what's new here.",
   "kind": "Clarity", "at": 1763470800},
  {"id": 4, "file": "intro.md", "line": 20, "quote": "", "text": "Add a limitations paragraph."},
  {"id": 5, "file": "intro.md", "line": 7, "quote": "three datasets",
   "text": "Name them.", "kind": "Reproducibility", "suggested_by": "Claude"},
  {"id": 6, "file": "Results.md", "line": 3, "quote": "significant",
   "text": "Give the test.", "kind": "Correctness", "suggested_by": "Claude", "resolved": true},
  {"id": 7, "file": "Results.md", "line": 2, "quote": "The first table",
   "selection": "The first table\nshows the runs.", "text": "Move the table up.",
   "kind": "Structure"}
]"#;

/// The review those make: a file at a time by name whatever its case, a kind at a time in
/// the strip's order with none last, down each file, and the two suggestions not kept left out.
const REVIEW: &str = "# Review

5 comments on 2 files, 1 resolved.

- Clarity: 2
- Evidence: 1
- Structure: 1
- Other: 1

## intro.md

### Clarity

#### Line 4 · Open · 2025-11-18 13:00 UTC

> We propose

Say what's new here.

#### Line 12 · Resolved

> is robust

Robust to what?

**Agrees · Claude:** The checks should be named.

**You:** Done in the next draft.

### Other

#### Line 20 · Open

Add a limitations paragraph.

## Results.md

### Evidence

#### Line 9 · Open

> a gain of 4 points

Over which baseline?

**Pushes back · Claude:** Table 2 names the baseline.

### Structure

#### Line 2 · Open

> The first table
> shows the runs.

Move the table up.
";

fn reviewed() -> (TempDir, App) {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("intro.md"), "# Intro\n").unwrap();
    fs::write(dir.path().join("Results.md"), "# Results\n").unwrap();
    let mut app = App::new(dir.path().to_path_buf(), (120, 36));
    app.threads = serde_json::from_str::<Vec<Thread>>(THREADS).unwrap();
    (dir, app)
}

#[test]
fn the_review_is_markdown_a_file_and_a_kind_at_a_time() {
    let (_dir, app) = reviewed();
    let text = super::report::markdown(&app.threads).expect("a review");
    assert_eq!(text, REVIEW);
}

/// Copy puts the review on the clipboard, as every copy goes out (OSC 52), and says so; the
/// Comments tab's own menu does the same. With nothing kept there's nothing to copy.
#[test]
fn copy_review_puts_the_markdown_on_the_clipboard() {
    let (_dir, mut app) = reviewed();
    assert!(app.can_run(Command::CopyReview));
    app.run(Command::CopyReview);
    assert_eq!(app.outgoing_clipboard.take().as_deref(), Some(REVIEW));
    assert_eq!(app.clipboard, REVIEW, "Paste in Cellix has it too");
    let grid = screen(&app);
    assert!(
        grid.text()
            .contains("Copied the review as Markdown: 5 comments"),
        "{}",
        grid.text()
    );

    // The Comments tab's menu, a right click on the tab.
    app.run(Command::ShowComments);
    assert_eq!(app.dock.front, Front::Comments);
    let grid = screen(&app);
    let area = app.dock_area().expect("the pane");
    let row = grid.row_text(area.y);
    let at = row.find("Comments").expect("the Comments tab");
    let x = row[..at].chars().count() as u16 + 2;
    app.handle(mouse(MouseKind::Right, x, area.y));
    let grid = screen(&app);
    assert!(
        grid.text().contains("Save Review as Markdown"),
        "{}",
        grid.text()
    );
    let (x, y) = find(&grid, "Copy Review as Markdown");
    app.handle(mouse(MouseKind::Left, x + 1, y));
    assert_eq!(app.outgoing_clipboard.take().as_deref(), Some(REVIEW));

    // Suggestions alone aren't a review.
    app.threads.retain(|t| !t.suggested_by.is_empty());
    assert!(!app.can_run(Command::CopyReview));
    app.run(Command::CopyReview);
    assert_eq!(app.outgoing_clipboard.take(), None);
    assert!(screen(&app).text().contains("No comments to copy"));
}

/// Save asks for a name, offering one the project doesn't have, writes the review there and
/// opens it. A name that's taken is asked about before it's replaced.
#[test]
fn save_review_writes_it_into_the_project_and_opens_it() {
    let (dir, mut app) = reviewed();
    app.run(Command::SaveReview);
    let grid = screen(&app);
    assert!(grid.text().contains("Save review as"), "{}", grid.text());
    assert!(grid.text().contains("review.md"), "{}", grid.text());
    key(&mut app, Key::Enter);
    let path = dir.path().join("review.md");
    assert_eq!(fs::read_to_string(&path).unwrap(), REVIEW);
    let open = app.editors.get(app.active()).and_then(|e| e.buffer.path());
    assert_eq!(open, Some(path.as_path()), "it opens");
    assert!(
        screen(&app)
            .text()
            .contains("Saved the review to review.md")
    );

    // Again: review.md is taken, so it offers review-2.md.
    app.threads[2].text = "Say what's new in the first paragraph.".into();
    app.run(Command::SaveReview);
    let Overlay::Prompt(prompt) = &app.overlay else {
        panic!("no prompt");
    };
    assert_eq!(prompt.input, "review-2.md");
    // Named review, it's review.md, which is asked about; Cancel leaves it as it was.
    for _ in 0.."-2.md".len() {
        key(&mut app, Key::Backspace);
    }
    key(&mut app, Key::Enter);
    let grid = screen(&app);
    assert!(
        grid.text().contains("Replace review.md?"),
        "{}",
        grid.text()
    );
    key(&mut app, Key::Enter);
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        REVIEW,
        "Cancel is first"
    );
    // Replace takes its place, and the tab open on it shows what's there now.
    app.run(Command::SaveReview);
    for _ in 0.."review-2.md".len() {
        key(&mut app, Key::Backspace);
    }
    typed(&mut app, "review");
    key(&mut app, Key::Enter);
    let grid = screen(&app);
    let buttons = (0..grid.height())
        .find(|y| grid.row_text(*y).contains("Cancel"))
        .expect("the dialog's buttons");
    let row = grid.row_text(buttons);
    let x = row[..row.find("Replace").expect("Replace")].chars().count() as u16;
    app.handle(mouse(MouseKind::Left, x + 2, buttons));
    let saved = fs::read_to_string(&path).unwrap();
    assert!(
        saved.contains("Say what's new in the first paragraph."),
        "{saved}"
    );
    let shown = app.editors[app.active()].buffer.text();
    assert_eq!(shown, saved, "the open tab shows it");
    assert!(!dir.path().join("review-2.md").exists());
}
