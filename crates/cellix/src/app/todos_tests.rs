//! The To-dos view in the sidebar: what is left in the workspace, in progress first, then to
//! do, then done, and who owns each row. Driven through the app and read off the drawn grid.

use std::fs;
use std::rc::Rc;

use cellix_input::{Command, Event, Key, KeyEvent, Mods, MouseEvent};
use cellix_render::{FontMeasure, Grid, Icon, Paint, Piece};
use cellix_review::todos::{self, State};
use tempfile::TempDir;

use super::{App, SidebarView};

/// A workspace with a parent and three children, one of them done; an agent's item under
/// way with its active form; one open item another agent owns; and two finished.
fn seeded() -> (TempDir, App) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::write(root.join("alpha.txt"), "first line\n").unwrap();
    let parent = todos::add(root, "Fix the order", "you", "you", None, "").unwrap();
    let read = todos::add(root, "Read the sort", "you", "you", Some(&parent), "").unwrap();
    todos::set(root, &read, State::Done).unwrap();
    todos::add(root, "Swap the lists", "you", "you", Some(&parent), "").unwrap();
    todos::add(root, "Run the tests", "you", "you", Some(&parent), "").unwrap();
    let tour = todos::add(
        root,
        "Run the tour",
        "Claude Code",
        "claude:s1",
        None,
        "Running the tour",
    )
    .unwrap();
    todos::set(root, &tour, State::Doing).unwrap();
    todos::add(root, "Add export button", "Codex", "codex:s2", None, "").unwrap();
    let old = todos::add(root, "Read old tests", "you", "you", None, "").unwrap();
    todos::set(root, &old, State::Done).unwrap();
    let ask = todos::add(
        root,
        "Ask about the sort",
        "Claude Code",
        "claude:s1",
        None,
        "",
    )
    .unwrap();
    todos::set(root, &ask, State::Done).unwrap();
    // Wide enough for the sidebar's full width, so its rows are not cut for room.
    let mut app = App::new(root.to_path_buf(), (160, 30));
    app.run(Command::ShowTodos);
    (dir, app)
}

fn screen(app: &App) -> Grid {
    let mut grid = Grid::new(app.size().0, app.size().1, app.background());
    app.draw(&mut grid);
    grid
}

/// The first row holding `words`, with the grid it is on when it is not.
fn row_of(grid: &Grid, words: &str) -> u16 {
    (0..grid.height())
        .find(|y| grid.row_text(*y).contains(words))
        .unwrap_or_else(|| panic!("{words:?} isn't drawn:\n{}", grid.text()))
}

fn mouse(kind: cellix_input::MouseKind, x: u16, y: u16) -> Event {
    Event::Mouse(MouseEvent {
        kind,
        x,
        y,
        fine_x: x as f32 + 0.5,
        fine_y: y as f32 + 0.5,
        mods: Mods::default(),
    })
}

fn click(app: &mut App, x: u16, y: u16) {
    app.handle(mouse(cellix_input::MouseKind::Left, x, y));
    app.handle(mouse(cellix_input::MouseKind::Up, x, y));
}

fn typed(app: &mut App, text: &str) {
    for c in text.chars() {
        app.handle(Event::Key(KeyEvent::plain(Key::Char(c))));
    }
}

fn key(app: &mut App, key: Key) {
    app.handle(Event::Key(KeyEvent::plain(key)));
}

#[test]
fn the_todos_view_lists_what_is_left_by_state_and_who_owns_it() {
    let (dir, mut app) = seeded();
    assert!(app.sidebar_view == SidebarView::Todos);
    assert!(app.rail_on(Command::ShowTodos));
    let grid = screen(&app);
    let text = grid.text();
    for words in [
        "TO-DOS",
        "2 of 5",
        "IN PROGRESS",
        "TO DO",
        "Show 2 done",
        "Fix the order",
        "Read the sort",
        "Swap the lists",
        "Run the tests",
        "Run the tour",
        "Running the tour…",
        "Add export button",
    ] {
        assert!(text.contains(words), "{words:?} missing:\n{text}");
    }
    // Done folds away: its rows are not in the list until the fold is pressed.
    for words in ["Read old tests", "Ask about the sort"] {
        assert!(
            !text.contains(words),
            "{words:?} shows though the fold is closed:\n{text}"
        );
    }

    // In progress first, then to do, then the fold; a parent's children follow it.
    let order = [
        row_of(&grid, "IN PROGRESS"),
        row_of(&grid, "Fix the order"),
        row_of(&grid, "Read the sort"),
        row_of(&grid, "Swap the lists"),
        row_of(&grid, "Run the tests"),
        row_of(&grid, "Run the tour"),
        row_of(&grid, "TO DO"),
        row_of(&grid, "Add export button"),
        row_of(&grid, "Show 2 done"),
    ];
    assert!(
        order.windows(2).all(|pair| pair[0] < pair[1]),
        "the list's order: {order:?}\n{text}"
    );

    // The doing item's active form stands under it, quiet.
    let tour = row_of(&grid, "Run the tour");
    assert!(
        grid.row_text(tour + 1).contains("Running the tour…"),
        "the active form under its item:\n{text}"
    );

    // Every state has a mark: a half box for the parent part-done, boxes, ticks, and the
    // spinner for what's under way.
    let parent = grid.row_text(row_of(&grid, "Fix the order"));
    assert!(parent.contains('◐'), "the parent's half box: {parent:?}");
    assert!(parent.contains("1/3"), "its progress: {parent:?}");
    let read = grid.row_text(row_of(&grid, "Read the sort"));
    assert!(read.contains('☑'), "a child done: {read:?}");
    let swap = grid.row_text(row_of(&grid, "Swap the lists"));
    assert!(swap.contains('☐'), "a child to do: {swap:?}");
    let tour = grid.row_text(tour);
    assert!(
        cellix_render::spinner::FRAMES
            .iter()
            .any(|f| tour.contains(f)),
        "the item under way turns: {tour:?}"
    );

    // Who owns it, marked at the row's end: the agent's mark, and the person's quiet word.
    assert!(grid.row_text(row_of(&grid, "Run the tour")).contains('✻'));
    assert!(
        grid.row_text(row_of(&grid, "Add export button"))
            .contains('⬡'),
        "Codex's mark:\n{}",
        grid.row_text(row_of(&grid, "Add export button"))
    );

    // The fold opens where it stands, and the finished rows come out under it, ticked.
    let fold = row_of(&grid, "Show 2 done");
    let area = app.sidebar_area().unwrap();
    click(&mut app, area.x + 5, fold);
    let grid = screen(&app);
    let text = grid.text();
    assert!(text.contains("Hide 2 done"), "{text}");
    let fold = row_of(&grid, "Hide 2 done");
    let done = [
        row_of(&grid, "Read old tests"),
        row_of(&grid, "Ask about the sort"),
    ];
    assert!(done[0] > fold && done[1] > fold, "under the fold:\n{text}");
    for at in done {
        let row = grid.row_text(at);
        assert!(row.contains('☑'), "a finished row ticked: {row:?}");
    }

    // The views' rail carries the checklist, not the Task page's checked box.
    let rail = app.activity_area();
    let icon = app.rail_row(5, rail);
    let piece = Piece::decode(&grid.cell(rail.x + 1, icon).symbol);
    assert_eq!(
        piece.map(|piece| piece.icon),
        Some(Icon::ListTodo),
        "the rail's sixth view is To-dos"
    );

    // Where there are pictures, the same list in the same places: marks and owners painted.
    app.set_measure(Rc::new(FontMeasure::new(17, 37)));
    let grid = screen(&app);
    assert!(
        grid.paint.iter().any(
            |paint| matches!(paint, Paint::Text { text, x, .. } if text == "TO-DOS"
                && *x == cellix_render::units(2.0))
        ),
        "the title two columns in"
    );
    for words in ["Fix the order", "Running the tour…", "1/3"] {
        assert!(
            grid.paint
                .iter()
                .any(|paint| matches!(paint, Paint::Text { text, .. } if text == words)),
            "{words:?} painted:\n{:?}",
            grid.paint
        );
    }
    assert!(
        grid.paint.iter().any(|paint| matches!(
            paint,
            Paint::Icon {
                icon: Icon::Check,
                ..
            }
        )),
        "a tick, painted"
    );
    assert!(
        grid.paint.iter().any(|paint| matches!(
            paint,
            Paint::Icon {
                icon: Icon::AgentClaude,
                ..
            }
        )),
        "Claude's mark, painted"
    );
    assert!(
        grid.paint.iter().any(|paint| matches!(
            paint,
            Paint::Panel {
                border: Some(_),
                ..
            }
        )),
        "a box to tick, outlined"
    );
    assert!(
        grid.paint
            .iter()
            .any(|paint| matches!(paint, Paint::Spinner { .. })),
        "the spinner, painted"
    );
    // The views' icon carries how many are under way.
    assert!(
        grid.paint
            .iter()
            .any(|paint| matches!(paint, Paint::Text { text, y, .. } if text == "1" && *y < 0)),
        "the badge over the To-dos view"
    );

    // The log is read again on the next poll, with no message from anyone.
    todos::add(dir.path(), "A new one", "you", "you", None, "").unwrap();
    assert!(app.poll(), "the log changed, so the view reads it again");
    let grid = screen(&app);
    assert!(grid.text().contains("A new one"), "{}", grid.text());
}

/// A list with a to-do, an item under way, and one finished: enough to tick, unfold and
/// clear.
fn worked() -> (TempDir, App) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    todos::add(root, "Write notes", "you", "you", None, "").unwrap();
    let run = todos::add(root, "Run the tests", "you", "you", None, "").unwrap();
    todos::set(root, &run, State::Doing).unwrap();
    let read = todos::add(root, "Read the sort", "you", "you", None, "").unwrap();
    todos::set(root, &read, State::Done).unwrap();
    let mut app = App::new(root.to_path_buf(), (160, 30));
    app.run(Command::ShowTodos);
    (dir, app)
}

#[test]
fn a_press_marks_a_to_do_done_and_it_stays_done() {
    let (dir, mut app) = worked();
    let grid = screen(&app);
    let text = grid.text();
    for words in ["1 of 3", "Write notes", "Show 1 done"] {
        assert!(text.contains(words), "{words:?} missing:\n{text}");
    }
    assert!(
        !text.contains("Read the sort"),
        "done stays folded:\n{text}"
    );

    // The box at the start of a to-do's row ticks it: it goes to done and folds away.
    let area = app.sidebar_area().unwrap();
    let notes = row_of(&grid, "Write notes");
    click(&mut app, area.x + 3, notes);
    let grid = screen(&app);
    let text = grid.text();
    assert!(text.contains("2 of 3"), "{text}");
    assert!(text.contains("Show 2 done"), "{text}");
    assert!(
        !text.contains("Write notes"),
        "ticked, so folded away:\n{text}"
    );

    // A list opened after that reads the same: the tick is in the log, not in this screen.
    let mut again = App::new(dir.path().to_path_buf(), (160, 30));
    again.run(Command::ShowTodos);
    let grid = screen(&again);
    let text = grid.text();
    assert!(
        text.contains("2 of 3") && text.contains("Show 2 done"),
        "{text}"
    );
    assert!(!text.contains("Write notes"), "{text}");

    // The fold shows the finished rows, ticked; a press on a tick puts it back to do.
    let area = again.sidebar_area().unwrap();
    let fold = row_of(&grid, "Show 2 done");
    click(&mut again, area.x + 5, fold);
    let grid = screen(&again);
    let text = grid.text();
    assert!(
        text.contains("Write notes") && text.contains("Read the sort"),
        "{text}"
    );
    let notes = row_of(&grid, "Write notes");
    assert!(
        grid.row_text(notes).contains('☑'),
        "{}",
        grid.row_text(notes)
    );
    click(&mut again, area.x + 3, notes);
    let grid = screen(&again);
    let text = grid.text();
    assert!(text.contains("1 of 3"), "{text}");
    assert!(text.contains("Write notes"), "back to do:\n{text}");
    assert!(
        grid.row_text(row_of(&grid, "Write notes")).contains('☐'),
        "{}",
        grid.row_text(row_of(&grid, "Write notes"))
    );

    // What is under way goes to done with a press on its mark too.
    let run = row_of(&grid, "Run the tests");
    click(&mut again, area.x + 3, run);
    let grid = screen(&again);
    let text = grid.text();
    assert!(text.contains("2 of 3"), "{text}");
    assert!(
        grid.row_text(row_of(&grid, "Run the tests")).contains('☑'),
        "{}",
        grid.row_text(row_of(&grid, "Run the tests"))
    );

    // Clear done takes the finished rows, and nothing else.
    let clear = row_of(&grid, "Clear");
    click(&mut again, area.right() - 2, clear);
    let grid = screen(&again);
    let text = grid.text();
    assert!(text.contains("0 of 1"), "{text}");
    assert!(text.contains("Write notes"), "{text}");
    for words in ["Run the tests", "Read the sort", "Clear"] {
        assert!(!text.contains(words), "{words:?} stays:\n{text}");
    }

    // And it stays cleared.
    let mut later = App::new(dir.path().to_path_buf(), (160, 30));
    later.run(Command::ShowTodos);
    let grid = screen(&later);
    let text = grid.text();
    assert!(
        text.contains("0 of 1") && text.contains("Write notes"),
        "{text}"
    );
    assert!(
        !text.contains("Run the tests") && !text.contains("Read the sort"),
        "{text}"
    );
}

#[test]
fn a_to_do_can_be_added_renamed_and_removed() {
    let (dir, mut app) = worked();
    let area = app.sidebar_area().unwrap();

    // The control at the list's end opens a box; Enter writes the to-do and keeps the box
    // open for another.
    let grid = screen(&app);
    let add = row_of(&grid, "+ Add a To-do");
    click(&mut app, area.x + 5, add);
    typed(&mut app, "Plan the release");
    key(&mut app, Key::Enter);
    let grid = screen(&app);
    let text = grid.text();
    assert!(text.contains("Plan the release"), "{text}");
    assert!(text.contains("1 of 4"), "the new to-do counted:\n{text}");
    assert!(
        text.contains("+ Add a To-do"),
        "the box stays open:\n{text}"
    );
    key(&mut app, Key::Esc);

    // A press on the words opens the row's box; a press in the box puts the caret where it
    // lands, and Enter writes the new words.
    let grid = screen(&app);
    let notes = row_of(&grid, "Write notes");
    click(&mut app, area.x + 5, notes);
    click(&mut app, area.x + 4, notes);
    typed(&mut app, "New ");
    key(&mut app, Key::Enter);
    let grid = screen(&app);
    assert!(
        grid.text().contains("New Write notes"),
        "the words renamed:\n{}",
        grid.text()
    );

    // The × at the row's end removes it, leaving the rest.
    let notes = row_of(&grid, "New Write notes");
    click(&mut app, area.right() - 2, notes);
    let grid = screen(&app);
    let text = grid.text();
    assert!(!text.contains("Write notes"), "removed:\n{text}");
    assert!(text.contains("1 of 3"), "{text}");

    // Escape leaves the words as they were.
    let plan = row_of(&grid, "Plan the release");
    click(&mut app, area.x + 5, plan);
    typed(&mut app, " changed");
    key(&mut app, Key::Esc);
    let grid = screen(&app);
    let text = grid.text();
    assert!(text.contains("Plan the release"), "{text}");
    assert!(!text.contains("Plan the release changed"), "{text}");

    // The log has all of it: the added to-do, the new words gone, the plan as it was.
    let mut later = App::new(dir.path().to_path_buf(), (160, 30));
    later.run(Command::ShowTodos);
    let grid = screen(&later);
    let text = grid.text();
    assert!(text.contains("Plan the release"), "{text}");
    assert!(!text.contains("Write notes"), "{text}");
    assert!(text.contains("1 of 3"), "{text}");
}

/// A parent with two children, one of them done.
fn parented() -> (TempDir, App) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let parent = todos::add(root, "Ship the release", "you", "you", None, "").unwrap();
    let notes = todos::add(root, "Write the notes", "you", "you", Some(&parent), "").unwrap();
    todos::set(root, &notes, State::Done).unwrap();
    todos::add(root, "Run the checks", "you", "you", Some(&parent), "").unwrap();
    let mut app = App::new(root.to_path_buf(), (160, 30));
    app.run(Command::ShowTodos);
    (dir, app)
}

#[test]
fn a_parent_is_done_only_when_all_its_children_are() {
    let (dir, mut app) = parented();
    let grid = screen(&app);
    let text = grid.text();
    assert!(
        text.contains("Ship the release") && text.contains("0 of 1"),
        "{text}"
    );
    let parent = row_of(&grid, "Ship the release");
    assert!(
        grid.row_text(parent).contains('◐'),
        "the parent part done: {}",
        grid.row_text(parent)
    );
    assert!(
        grid.row_text(parent).contains("1/2"),
        "{}",
        grid.row_text(parent)
    );
    assert!(
        grid.row_text(row_of(&grid, "Write the notes"))
            .contains('☑'),
        "{}",
        grid.row_text(row_of(&grid, "Write the notes"))
    );
    assert!(
        grid.row_text(row_of(&grid, "Run the checks")).contains('☐'),
        "{}",
        grid.row_text(row_of(&grid, "Run the checks"))
    );

    // The chevron folds the children away and brings them back where it stands.
    let area = app.sidebar_area().unwrap();
    click(&mut app, area.x + 2, parent);
    let grid = screen(&app);
    assert!(
        !grid.text().contains("Write the notes"),
        "folded:\n{}",
        grid.text()
    );
    assert!(grid.text().contains("Ship the release"), "{}", grid.text());
    click(&mut app, area.x + 2, row_of(&grid, "Ship the release"));
    let grid = screen(&app);
    assert!(grid.text().contains("Write the notes"), "{}", grid.text());

    // The box groups: every child done makes the parent done, so it folds away.
    click(&mut app, area.x + 3, row_of(&grid, "Ship the release"));
    let grid = screen(&app);
    let text = grid.text();
    assert!(text.contains("1 of 1"), "{text}");
    assert!(text.contains("Show 1 done"), "{text}");
    let fold = row_of(&grid, "Show 1 done");
    click(&mut app, area.x + 5, fold);
    let grid = screen(&app);
    let parent = row_of(&grid, "Ship the release");
    assert!(
        grid.row_text(parent).contains('☑'),
        "the parent ticked: {}",
        grid.row_text(parent)
    );
    assert!(
        grid.row_text(parent).contains("2/2"),
        "{}",
        grid.row_text(parent)
    );
    assert!(
        grid.row_text(row_of(&grid, "Write the notes"))
            .contains('☑'),
        "{}",
        grid.row_text(row_of(&grid, "Write the notes"))
    );

    // A second press makes the whole group to do again, and the parent is not done.
    click(&mut app, area.x + 3, parent);
    let grid = screen(&app);
    let text = grid.text();
    assert!(text.contains("0 of 1"), "{text}");
    assert!(
        grid.row_text(row_of(&grid, "Ship the release"))
            .contains('☐'),
        "{}",
        grid.row_text(row_of(&grid, "Ship the release"))
    );

    // The + under the pointer adds a child, under the parent, and the progress follows.
    let parent = row_of(&grid, "Ship the release");
    click(&mut app, area.right() - 4, parent);
    typed(&mut app, "Check the docs");
    key(&mut app, Key::Enter);
    let grid = screen(&app);
    let text = grid.text();
    assert!(text.contains("Check the docs"), "{text}");
    let parent = row_of(&grid, "Ship the release");
    assert!(
        grid.row_text(parent).contains("0/3"),
        "a third child: {}",
        grid.row_text(parent)
    );
    key(&mut app, Key::Esc);

    // The × removes the parent, and its children with it.
    let parent = row_of(&grid, "Ship the release");
    click(&mut app, area.right() - 2, parent);
    let grid = screen(&app);
    let text = grid.text();
    assert!(text.contains("No to-dos yet."), "{text}");
    assert!(
        !text.contains("Write the notes") && !text.contains("Check the docs"),
        "the children went with it:\n{text}"
    );

    // And the log says the same to the next Cellix.
    let mut later = App::new(dir.path().to_path_buf(), (160, 30));
    later.run(Command::ShowTodos);
    assert!(screen(&later).text().contains("No to-dos yet."));
}

/// Claude Code's `TodoWrite`, as its PostToolUse hook sees it, for the project at `root`.
fn todo_write(root: &std::path::Path, session: &str, todos: serde_json::Value) {
    let call = serde_json::json!({
        "cwd": root,
        "session_id": session,
        "tool_name": "TodoWrite",
        "tool_input": {"todos": todos},
    });
    cellix_agent::hook::handle("tool-done", &call.to_string());
}

#[test]
fn claude_codes_to_dos_appear_and_change_as_it_works() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    // A repository's top: where the hook's rule keeps the log with no Cellix serving it.
    std::fs::create_dir(root.join(".git")).unwrap();
    let mut app = App::new(root.clone(), (160, 30));
    app.run(Command::ShowTodos);
    assert!(
        screen(&app).text().contains("No to-dos yet."),
        "{}",
        screen(&app).text()
    );

    // The hook writes the whole list as Claude Code's session's own.
    todo_write(
        &root,
        "s9",
        serde_json::json!([
            {"content": "Read the sort", "status": "completed", "activeForm": "Reading the sort"},
            {"content": "Swap the lists", "status": "in_progress", "activeForm": "Swapping the lists"},
            {"content": "Run the tests", "status": "pending", "activeForm": "Running the tests"},
        ]),
    );
    assert!(app.poll(), "the log changed, so the view reads it again");
    let grid = screen(&app);
    let text = grid.text();
    for words in [
        "1 of 3",
        "Swap the lists",
        "Swapping the lists…",
        "Run the tests",
        "Show 1 done",
        "✻",
    ] {
        assert!(text.contains(words), "{words:?} missing:\n{text}");
    }

    // The next call completes one and adds one: the rows change as the agent works, and the
    // active form of the one it finished goes.
    todo_write(
        &root,
        "s9",
        serde_json::json!([
            {"content": "Read the sort", "status": "completed", "activeForm": "Reading the sort"},
            {"content": "Swap the lists", "status": "completed", "activeForm": "Swapping the lists"},
            {"content": "Run the tests", "status": "pending", "activeForm": "Running the tests"},
            {"content": "Write the notes", "status": "pending", "activeForm": "Writing the notes"},
        ]),
    );
    assert!(app.poll());
    let grid = screen(&app);
    let text = grid.text();
    assert!(text.contains("2 of 4"), "{text}");
    assert!(text.contains("Show 2 done"), "{text}");
    assert!(text.contains("Write the notes"), "{text}");
    assert!(
        !text.contains("Swapping the lists…"),
        "the finished item's form is gone:\n{text}"
    );
}
