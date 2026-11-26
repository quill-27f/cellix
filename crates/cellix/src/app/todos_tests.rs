//! The To-dos view in the sidebar: what is left in the workspace, in progress first, then to
//! do, then done, and who owns each row. Driven through the app and read off the drawn grid.

use std::fs;
use std::rc::Rc;

use cellix_input::Command;
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
        "DONE",
        "Fix the order",
        "Read the sort",
        "Swap the lists",
        "Run the tests",
        "Run the tour",
        "Running the tour…",
        "Add export button",
        "Read old tests",
        "Ask about the sort",
    ] {
        assert!(text.contains(words), "{words:?} missing:\n{text}");
    }

    // In progress first, then to do, then done; a parent's children follow it.
    let order = [
        row_of(&grid, "IN PROGRESS"),
        row_of(&grid, "Fix the order"),
        row_of(&grid, "Read the sort"),
        row_of(&grid, "Swap the lists"),
        row_of(&grid, "Run the tests"),
        row_of(&grid, "Run the tour"),
        row_of(&grid, "TO DO"),
        row_of(&grid, "Add export button"),
        row_of(&grid, "DONE"),
        row_of(&grid, "Read old tests"),
        row_of(&grid, "Ask about the sort"),
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
