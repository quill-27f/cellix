//! Selecting as in any document: a click sets where the selection starts, and a Shift+click
//! elsewhere, however far it has scrolled since, selects everything between, across every kind
//! of block a page has. Driven through the app with the pointer on Markdown, LaTeX, a notebook,
//! the source and a terminal, and read off the selection, the drawn grid and the comment made.

use std::fs;
use std::rc::Rc;

use cellix_input::{Event, Key, KeyEvent, Mods, MouseEvent, MouseKind};
use cellix_render::{Grid, Paint};
use tempfile::TempDir;

use super::{App, Command};

/// A file opened in a window 150 by 40, on the cell tier or, with `pixels`, in measured type.
fn open(name: &str, text: &str, pixels: bool) -> (TempDir, App) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(name);
    fs::write(&path, text).unwrap();
    let mut app = App::new(dir.path().to_path_buf(), (150, 40));
    if pixels {
        app.set_measure(Rc::new(cellix_render::FontMeasure::new(17, 37)));
    }
    app.open(&path).unwrap();
    (dir, app)
}

fn screen(app: &App) -> Grid {
    let mut grid = Grid::new(app.size().0, app.size().1, app.background());
    app.draw(&mut grid);
    grid
}

fn pointer(kind: MouseKind, fine_x: f32, y: u16, shift: bool) -> Event {
    Event::Mouse(MouseEvent {
        kind,
        x: fine_x as u16,
        y,
        fine_x,
        fine_y: f32::from(y) + 0.5,
        mods: Mods {
            shift,
            ..Mods::default()
        },
    })
}

/// Where the file's text has `words`, counting in chars as the selection does.
fn at(app: &App, words: &str) -> usize {
    let text = app.active_text().unwrap();
    let byte = text
        .find(words)
        .unwrap_or_else(|| panic!("no {words:?} in {text}"));
    text[..byte].chars().count()
}

/// The same, just past the end of `words`.
fn after(app: &App, words: &str) -> usize {
    at(app, words) + words.chars().count()
}

/// Where a place in the text is drawn, while it's in view.
fn spot(app: &App, place: usize) -> Option<(f32, u16)> {
    let area = app.focused_area();
    app.editors[app.active()].cell_of(area, place)
}

/// A press and its release on a place in the text, as a terminal reports a click, a little
/// inside the letter there.
fn click(app: &mut App, place: usize, shift: bool) {
    let (x, y) = spot(app, place).unwrap_or_else(|| panic!("{place} isn't in view"));
    app.handle(pointer(MouseKind::Left, x + 0.1, y, shift));
    app.handle(pointer(MouseKind::Up, x + 0.1, y, shift));
}

/// Turns the wheel over the page until a place in the text is in view.
fn scroll_to(app: &mut App, place: usize) {
    let area = app.focused_area();
    let over = f32::from(area.x + area.w / 2);
    let down = place > app.editors[app.active()].selections.primary().head;
    let kind = if down {
        MouseKind::ScrollDown
    } else {
        MouseKind::ScrollUp
    };
    for _ in 0..400 {
        if spot(app, place).is_some() {
            return;
        }
        app.handle(pointer(kind, over, area.y + area.h / 2, false));
    }
    panic!("{place} never came into view");
}

fn selected(app: &App) -> String {
    app.selected_text().unwrap_or_default()
}

/// The file's text from one place to another.
fn source(app: &App, from: usize, to: usize) -> String {
    let text = app.active_text().unwrap();
    text.chars().skip(from).take(to - from).collect()
}

fn typed(app: &mut App, text: &str) {
    for c in text.chars() {
        app.handle(Event::Key(KeyEvent::plain(Key::Char(c))));
    }
}

fn filler(n: usize) -> String {
    (0..n)
        .map(|i| format!("Filler paragraph {i} keeps the end of the page out of view.\n\n"))
        .collect()
}

/// A page with a block of every kind before a long way down to its last paragraph.
fn markdown() -> String {
    let blocks = "# Start of the scan\n\n\
        The first paragraph begins the scan.\n\n\
        ## A heading inside\n\n\
        - an item in a list\n\
        - another item\n\n\
        > a quoted line\n\n\
        | name | value |\n\
        |------|-------|\n\
        | cell | 42 |\n\n\
        ```rust\n\
        let code = 1;\n\
        ```\n\n\
        $$\n\
        x^2 + y^2 = z^2\n\
        $$\n\n";
    format!(
        "{blocks}{}The last paragraph ends the scan here.\n",
        filler(40)
    )
}

/// A click, then the wheel until the start is long out of view, then a Shift+click at the end.
fn scan(app: &mut App) -> (usize, usize) {
    let (start, end) = (at(app, "first paragraph"), after(app, "ends the scan here"));
    click(app, start, false);
    scroll_to(app, end);
    assert!(
        spot(app, start).is_none(),
        "the start has scrolled out of view"
    );
    click(app, end, true);
    (start, end)
}

#[test]
fn a_shift_click_selects_everything_from_the_click_down_a_page() {
    for pixels in [false, true] {
        let (_dir, mut app) = open("scan.md", &markdown(), pixels);
        let (start, end) = scan(&mut app);
        let text = selected(&app);
        assert_eq!(text, source(&app, start, end), "pixels: {pixels}");
        for block in [
            "## A heading inside",
            "- another item",
            "> a quoted line",
            "| cell | 42 |",
            "let code = 1;",
            "x^2 + y^2 = z^2",
            "Filler paragraph 39",
        ] {
            assert!(text.contains(block), "{block:?} in {text:?}");
        }
        if !pixels {
            // And it's drawn as selected, to the last word.
            let grid = screen(&app);
            let (x, y) = spot(&app, at(&app, "Filler paragraph 39")).unwrap();
            let theme = crate::theme::Theme::default();
            assert_eq!(grid.cell(x as u16, y).style.bg, theme.selection);
            let (x, y) = spot(&app, end - 1).unwrap();
            assert_eq!(grid.cell(x as u16, y).style.bg, theme.selection);
        }
    }
}

#[test]
fn a_shift_click_again_moves_the_end_and_keeps_the_start() {
    let (_dir, mut app) = open("scan.md", &markdown(), false);
    let (start, _) = scan(&mut app);
    let nearer = at(&app, "Filler paragraph 30");
    scroll_to(&mut app, nearer);
    click(&mut app, nearer, true);
    assert_eq!(selected(&app), source(&app, start, nearer));
    // Before where it started, it runs back from there.
    let top = at(&app, "Start of the scan");
    scroll_to(&mut app, top);
    click(&mut app, top, true);
    assert_eq!(selected(&app), source(&app, top, start));
    assert_eq!(app.editors[0].selections.primary().anchor, start);
}

#[test]
fn a_shift_click_extends_a_selection_already_made_from_its_anchor() {
    let (_dir, mut app) = open("scan.md", &markdown(), false);
    // Dragged over "begins the".
    let (from, to) = (at(&app, "begins"), after(&app, "begins the"));
    let (x, y) = spot(&app, from).unwrap();
    app.handle(pointer(MouseKind::Left, x + 0.1, y, false));
    let (x, y) = spot(&app, to).unwrap();
    app.handle(pointer(MouseKind::Drag, x + 0.1, y, false));
    app.handle(pointer(MouseKind::Up, x + 0.1, y, false));
    assert_eq!(selected(&app), "begins the");
    let end = after(&app, "another item");
    click(&mut app, end, true);
    assert_eq!(selected(&app), source(&app, from, end));
    // A word a double click selected keeps its start too.
    let word = at(&app, "quoted");
    let (x, y) = spot(&app, word).unwrap();
    for _ in 0..2 {
        app.handle(pointer(MouseKind::Left, x + 1.1, y, false));
        app.handle(pointer(MouseKind::Up, x + 1.1, y, false));
    }
    assert_eq!(selected(&app), "quoted");
    let end = after(&app, "let code");
    click(&mut app, end, true);
    assert_eq!(selected(&app), source(&app, word, end));
}

#[test]
fn a_shift_click_takes_a_formula_whole_and_ticks_no_box() {
    let text = "Before the task.\n\n- [ ] a task to do\n\nThen a formula:\n\n$$\nx^2 + y^2 = z^2\n$$\n\nAfter it.\n";
    let (_dir, mut app) = open("marks.md", text, false);
    let start = at(&app, "Before");
    click(&mut app, start, false);
    // The box is where the list's marker is: the Shift+click selects up to it and leaves it be.
    let item = at(&app, "a task");
    let (x, y) = spot(&app, item).unwrap();
    let box_x = x - 2.0;
    app.handle(pointer(MouseKind::Left, box_x, y, true));
    app.handle(pointer(MouseKind::Up, box_x, y, true));
    assert_eq!(app.active_text().unwrap(), text, "the box is still empty");
    assert!(
        selected(&app).starts_with("Before the task."),
        "{:?}",
        selected(&app)
    );
    // On a formula, the selection takes all of its source, leaving no caret inside it to open
    // it: in measured type it stays a picture.
    let end = after(&app, "z^2\n$$\n");
    let inside = at(&app, "y^2");
    let (x, y) = spot(&app, inside).unwrap();
    app.handle(pointer(MouseKind::Left, x, y, true));
    app.handle(pointer(MouseKind::Up, x, y, true));
    assert_eq!(selected(&app), source(&app, start, end));
    // A plain click on the box still ticks it.
    let (x, y) = spot(&app, item).unwrap();
    app.handle(pointer(MouseKind::Left, x - 2.0, y, false));
    app.handle(pointer(MouseKind::Up, x - 2.0, y, false));
    assert!(app.active_text().unwrap().contains("- [x] a task"));

    let (_dir, mut app) = open("marks.md", text, true);
    click(&mut app, start, false);
    assert!(!screen(&app).text().contains("x^2"), "a picture");
    assert!(!picture_selected(&screen(&app)), "nothing selected yet");
    let (x, y) = spot(&app, inside).expect("the formula in view");
    app.handle(pointer(MouseKind::Left, x, y, true));
    app.handle(pointer(MouseKind::Up, x, y, true));
    assert_eq!(selected(&app), source(&app, start, end));
    let grid = screen(&app);
    assert!(!grid.text().contains("x^2"), "still a picture");
    // Its ink stands on the selection's colour, as the words' before it do.
    assert!(picture_selected(&grid), "the formula drawn as selected");
}

/// Whether a picture on the page stands in the selection's colour, its own shape of it.
fn picture_selected(grid: &Grid) -> bool {
    let selection = crate::theme::Theme::default().selection;
    let pictures: Vec<cellix_render::Bounds> = (grid.paint.iter())
        .filter_map(|paint| match paint {
            Paint::Image { bounds, .. } => Some(*bounds),
            _ => None,
        })
        .collect();
    // Ink stands on the colour with room round it; a wash lies on the picture's edges.
    let round = |outer: &cellix_render::Bounds| {
        pictures.iter().any(|inner| {
            outer.x < inner.x
                && outer.y < inner.y
                && outer.x + outer.w > inner.x + inner.w
                && outer.y + outer.h > inner.y + inner.h
        })
    };
    grid.paint.iter().any(|paint| match paint {
        Paint::Panel { bounds, fill, .. } => *fill == Some(selection) && round(bounds),
        Paint::Wash { bounds, color, .. } => *color == selection && pictures.contains(bounds),
        _ => false,
    })
}

#[test]
fn a_shift_click_selection_gets_the_comment_bar_and_its_comment_holds_all_of_it() {
    let (_dir, mut app) = open("scan.md", &markdown(), false);
    scan(&mut app);
    let grid = screen(&app);
    assert!(grid.text().contains("Ask Agent"), "{}", grid.text());
    let (comment, _) = app.selection_bar().expect("the buttons by the selection");
    app.handle(pointer(
        MouseKind::Left,
        f32::from(comment.x + 1),
        comment.y,
        false,
    ));
    app.handle(pointer(
        MouseKind::Up,
        f32::from(comment.x + 1),
        comment.y,
        false,
    ));
    typed(&mut app, "Why so long");
    app.handle(Event::Key(KeyEvent::plain(Key::Enter)));
    let thread = &app.threads[0];
    assert_eq!(thread.text, "Why so long");
    assert_eq!(thread.line, 3, "on the line it starts on");
    assert_eq!(thread.quote, "first paragraph begins the scan.");
    for words in ["| cell | 42 |", "x^2 + y^2 = z^2", "ends the scan here"] {
        assert!(
            thread.selection.contains(words),
            "{words:?}: {}",
            thread.selection
        );
        assert!(thread.message().contains(words), "{}", thread.message());
    }
}

#[test]
fn a_shift_click_selects_across_a_paper() {
    let blocks = "\\documentclass{article}\n\\begin{document}\n\
        \\section{Start of the scan}\n\n\
        The first paragraph begins the scan.\n\n\
        \\subsection{A heading inside}\n\n\
        \\begin{itemize}\n\\item an item in a list\n\\item another item\n\\end{itemize}\n\n\
        \\begin{quote}\na quoted line\n\\end{quote}\n\n\
        \\begin{tabular}{ll}\nname & value \\\\\ncell & 42 \\\\\n\\end{tabular}\n\n\
        \\begin{verbatim}\nlet code = 1;\n\\end{verbatim}\n\n\
        \\begin{equation}\nx^2 + y^2 = z^2\n\\end{equation}\n\n";
    let text = format!(
        "{blocks}{}The last paragraph ends the scan here.\n\n\\end{{document}}\n",
        filler(40)
    );
    for pixels in [false, true] {
        let (_dir, mut app) = open("paper.tex", &text, pixels);
        let (start, end) = scan(&mut app);
        let got = selected(&app);
        assert_eq!(got, source(&app, start, end), "pixels: {pixels}");
        for block in [
            "\\subsection{A heading inside}",
            "\\item another item",
            "a quoted line",
            "cell & 42",
            "let code = 1;",
            "x^2 + y^2 = z^2",
        ] {
            assert!(got.contains(block), "{block:?} in {got:?}");
        }
        let grid = screen(&app);
        assert!(grid.text().contains("Ask Agent"));
        // The words selected are counted as the paper's are, so never more than it has.
        let foot = grid.row_text(grid.height() - 1);
        let counts: Vec<usize> = foot
            .split(" words")
            .next()
            .unwrap()
            .split(" of ")
            .map(|n| n.split_whitespace().last().unwrap().parse().unwrap())
            .collect();
        assert!(counts.len() == 2 && counts[0] < counts[1], "{foot}");
    }
}

#[test]
fn a_shift_click_selects_across_a_notebooks_cells() {
    let notebook = r##"{
 "cells": [
  {"cell_type": "markdown", "metadata": {}, "source": ["# Training run\n", "\n", "Some words first.\n"]},
  {"cell_type": "code", "execution_count": 1, "metadata": {}, "outputs": [],
   "source": ["import torch\n", "train()"]},
  {"cell_type": "markdown", "metadata": {}, "source": ["Words at the end.\n"]}
 ],
 "metadata": {"kernelspec": {"language": "python", "name": "python3"}},
 "nbformat": 4, "nbformat_minor": 5
}"##;
    let (_dir, mut app) = open("run.ipynb", notebook, false);
    let (start, end) = (at(&app, "words first"), after(&app, "Words at the end"));
    click(&mut app, start, false);
    click(&mut app, end, true);
    let got = selected(&app);
    assert_eq!(got, source(&app, start, end));
    assert!(
        got.contains("import torch") && got.contains("train()"),
        "{got}"
    );
}

#[test]
fn a_shift_click_in_the_source_selects_the_same_way() {
    let (_dir, mut app) = open("scan.md", &markdown(), false);
    app.run(Command::MarkdownPreview);
    assert!(
        screen(&app).text().contains("# Start of the scan"),
        "the source"
    );
    let (start, end) = scan(&mut app);
    assert_eq!(selected(&app), source(&app, start, end));
    assert!(screen(&app).text().contains("Ask Agent"));
}

/// Advances the clock the loop would, a twentieth of a second at a time, until `done`.
fn hold(app: &mut App, mut done: impl FnMut(&App) -> bool) {
    let mut now = std::time::Instant::now();
    for _ in 0..400 {
        if done(app) {
            return;
        }
        now += std::time::Duration::from_millis(50);
        app.step_autoscroll(now);
    }
    panic!("it never got there");
}

#[test]
fn a_drag_held_past_the_foot_scrolls_the_page_and_the_selection_follows() {
    let (_dir, mut app) = open("scan.md", &markdown(), false);
    let start = at(&app, "first paragraph");
    let (x, y) = spot(&app, start).unwrap();
    app.handle(pointer(MouseKind::Left, x + 0.1, y, false));
    let last = at(&app, "ends the scan here");
    assert!(spot(&app, last).is_none(), "the end is out of view");
    // Onto the bar at the foot, under the page, and held there.
    let foot = app.size().1 - 1;
    app.handle(pointer(MouseKind::Drag, 60.0, foot, false));
    assert!(
        app.autoscroll.is_some() && app.soon(),
        "timed while it's held there"
    );
    let top = app.editors[0].top();
    hold(&mut app, |app| {
        let head = app.editors[0].selections.primary().head;
        assert!(
            spot(app, head).is_some(),
            "the end stays in view as it scrolls"
        );
        spot(app, last).is_some()
    });
    assert!(app.editors[0].top() > top, "the page scrolled");
    let area = app.focused_area();
    let edge = app.editors[0].scroll_region(area).bottom() - 1;
    let under = app.editors[0].position_at(area, 60.0, edge);
    let primary = app.editors[0].selections.primary();
    assert_eq!((primary.anchor, primary.head), (start, under));
    assert!(
        selected(&app).contains("Filler paragraph 39"),
        "{}",
        selected(&app)
    );
    // Back up past the top, over the tabs, it scrolls the other way.
    let top = app.editors[0].top();
    app.handle(pointer(MouseKind::Drag, 60.0, 1, false));
    hold(&mut app, |app| app.editors[0].top() + 10 < top);
    assert_eq!(app.editors[0].selections.primary().anchor, start);
    // Back inside it stops, and let go nothing is timed.
    app.handle(pointer(MouseKind::Drag, 60.0, 20, false));
    assert!(app.autoscroll.is_none());
    app.handle(pointer(MouseKind::Drag, 60.0, foot, false));
    app.handle(pointer(MouseKind::Up, 60.0, foot, false));
    assert!(app.autoscroll.is_none(), "nothing left timed");
    assert!(
        screen(&app).text().contains("Ask Agent"),
        "the bar, as after any drag"
    );
}

#[test]
fn turning_the_wheel_while_dragging_scrolls_and_the_selection_follows() {
    for source_view in [false, true] {
        let (_dir, mut app) = open("scan.md", &markdown(), false);
        if source_view {
            app.run(Command::MarkdownPreview);
        }
        let start = at(&app, "first paragraph");
        let (x, y) = spot(&app, start).unwrap();
        app.handle(pointer(MouseKind::Left, x + 0.1, y, false));
        app.handle(pointer(MouseKind::Drag, 60.0, 20, false));
        let top = app.editors[0].top();
        for _ in 0..8 {
            app.handle(pointer(MouseKind::ScrollDown, 60.0, 20, false));
        }
        assert!(app.editors[0].top() > top, "scrolled");
        let area = app.focused_area();
        let under = app.editors[0].position_at(area, 60.0, 20);
        let primary = app.editors[0].selections.primary();
        assert_eq!(
            (primary.anchor, primary.head),
            (start, under),
            "source: {source_view}"
        );
        assert!(app.autoscroll.is_none(), "inside, nothing's timed");
        app.handle(pointer(MouseKind::Up, 60.0, 20, false));
        assert_eq!(selected(&app), source(&app, start, under));
    }
}

#[test]
fn nothing_is_timed_unless_a_drag_is_held_past_an_edge() {
    let (_dir, mut app) = open("scan.md", &markdown(), false);
    assert!(app.autoscroll.is_none());
    let start = at(&app, "first paragraph");
    let (x, y) = spot(&app, start).unwrap();
    app.handle(pointer(MouseKind::Left, x + 0.1, y, false));
    app.handle(pointer(MouseKind::Drag, 60.0, 20, false));
    app.handle(pointer(MouseKind::Up, 60.0, 20, false));
    assert!(app.autoscroll.is_none(), "a drag inside the page");
    // Over the bar at the foot with no drag, nothing.
    let foot = app.size().1 - 1;
    app.handle(pointer(MouseKind::Move, 60.0, foot, false));
    assert!(app.autoscroll.is_none());
    assert!(
        !app.step_autoscroll(std::time::Instant::now()),
        "nothing to step"
    );
}

#[cfg(unix)]
mod terminal {
    use super::*;
    use std::time::{Duration, Instant};

    /// A program in the side pane's terminal, once it has printed `ready`.
    fn run(app: &mut App, script: &str) {
        let argv = ["sh".to_string(), "-c".to_string(), script.to_string()];
        let root = app.workspace.root().to_path_buf();
        app.start_session("Stand-in", &argv, &[], &root);
        let deadline = Instant::now() + Duration::from_secs(120);
        while !screen(app).text().contains("ready") {
            assert!(Instant::now() < deadline, "it never started");
            app.poll();
            app.fit_dock();
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// Where `words` are in the terminal, on the screen.
    fn find(app: &App, words: &str) -> Option<(u16, u16)> {
        let pane = app.dock_screen().unwrap();
        let grid = screen(app);
        (pane.y..pane.bottom()).find_map(|y| {
            let row: String = (grid.row_text(y).chars())
                .skip(usize::from(pane.x))
                .take(usize::from(pane.w))
                .collect();
            let at = row.find(words)?;
            Some((pane.x + row[..at].chars().count() as u16, y))
        })
    }

    fn press(app: &mut App, kind: MouseKind, (x, y): (u16, u16), shift: bool) {
        app.handle(pointer(kind, f32::from(x), y, shift));
    }

    fn wheel_to(app: &mut App, words: &str, up: bool) -> (u16, u16) {
        let pane = app.dock_screen().unwrap();
        let over = (pane.x + pane.w / 2, pane.y + pane.h / 2);
        let kind = if up {
            MouseKind::ScrollUp
        } else {
            MouseKind::ScrollDown
        };
        for _ in 0..400 {
            if let Some(found) = find(app, words) {
                return found;
            }
            press(app, kind, over, false);
        }
        panic!("{words:?} never showed");
    }

    #[test]
    fn a_shift_click_in_a_terminal_selects_from_the_click_however_far_it_scrolled() {
        let (_dir, mut app) = open("a.md", "# A page\n", false);
        let lines = "i=1; while [ $i -le 120 ]; do echo \"out $i.\"; i=$((i+1)); done";
        run(&mut app, &format!("{lines}; echo ready; sleep 30"));
        // A click at the end of a line near the foot, and a Shift+click far up the scrollback.
        let (x, y) = wheel_to(&mut app, "out 118.", false);
        let end = (x + "out 118.".len() as u16 - 1, y);
        press(&mut app, MouseKind::Left, end, false);
        press(&mut app, MouseKind::Up, end, false);
        let start = wheel_to(&mut app, "out 7.", true);
        assert!(find(&app, "out 118.").is_none(), "the click is out of view");
        press(&mut app, MouseKind::Left, start, true);
        press(&mut app, MouseKind::Up, start, true);
        let want: Vec<String> = (7..=118).map(|i| format!("out {i}.")).collect();
        assert_eq!(app.clipboard, want.join("\n"), "copied as a terminal does");
        // Again, nearer: the end moves and the start stays.
        let nearer = wheel_to(&mut app, "out 20.", false);
        press(&mut app, MouseKind::Left, nearer, true);
        press(&mut app, MouseKind::Up, nearer, true);
        let want: Vec<String> = (20..=118).map(|i| format!("out {i}.")).collect();
        assert_eq!(app.clipboard, want.join("\n"));
        // Scrolled, what's selected keeps to its words.
        let session = app.dock.session().unwrap();
        assert_eq!(session.selection().unwrap().0, want.join("\n"));
    }

    #[test]
    fn a_drag_held_past_a_terminals_top_scrolls_its_scrollback_and_the_wheel_does_too() {
        let (_dir, mut app) = open("a.md", "# A page\n", false);
        let lines = "i=1; while [ $i -le 200 ]; do echo \"out $i.\"; i=$((i+1)); done";
        run(&mut app, &format!("{lines}; echo ready; sleep 30"));
        let (x, y) = find(&app, "out 199.").unwrap();
        let end = (x + "out 199.".len() as u16 - 1, y);
        press(&mut app, MouseKind::Left, end, false);
        // Over the pane's tabs, above its screen, and held there.
        let pane = app.dock_screen().unwrap();
        let above = (pane.x + 4, pane.y - 1);
        press(&mut app, MouseKind::Drag, above, false);
        assert!(app.autoscroll.is_some());
        super::hold(&mut app, |app| find(app, "out 100.").is_some());
        press(&mut app, MouseKind::Up, above, false);
        assert!(app.autoscroll.is_none());
        let copied = app.clipboard.clone();
        let first = copied.lines().next().unwrap_or_default().to_string();
        let n: usize = first
            .trim_start_matches("out ")
            .trim_end_matches('.')
            .parse()
            .unwrap_or_else(|_| panic!("{copied}"));
        assert!(n <= 100, "from what came into view: {first}");
        assert!(
            copied.ends_with("out 199."),
            "to where it started: {copied}"
        );
        // Turned with the button held, the scrollback moves and the selection follows.
        let (x, y) = find(&app, "out 120.").unwrap();
        let end = (x + "out 120.".len() as u16 - 1, y);
        press(&mut app, MouseKind::Left, end, false);
        let middle = (pane.x, pane.y + pane.h / 2);
        press(&mut app, MouseKind::Drag, middle, false);
        for _ in 0..10 {
            press(&mut app, MouseKind::ScrollUp, middle, false);
        }
        press(&mut app, MouseKind::Up, middle, false);
        let first = app.clipboard.lines().next().unwrap_or_default().to_string();
        let n: usize = first
            .trim_start_matches("out ")
            .trim_end_matches('.')
            .parse()
            .unwrap_or_else(|_| panic!("{}", app.clipboard));
        assert!(n < 100, "{first}");
        assert!(app.clipboard.ends_with("out 120."), "{}", app.clipboard);
    }

    #[test]
    fn shift_selects_in_a_terminal_whose_program_takes_the_mouse() {
        let (_dir, mut app) = open("a.md", "# A page\n", false);
        // Mouse reporting on, as a program that uses the mouse asks.
        run(&mut app, "printf '\\033[?1000h'; echo ready; sleep 30");
        let (x, y) = find(&app, "ready").unwrap();
        press(&mut app, MouseKind::Left, (x, y), false);
        press(&mut app, MouseKind::Drag, (x + 4, y), false);
        press(&mut app, MouseKind::Up, (x + 4, y), false);
        assert_eq!(app.clipboard, "", "the program had that press");
        press(&mut app, MouseKind::Left, (x, y), true);
        press(&mut app, MouseKind::Drag, (x + 4, y), true);
        press(&mut app, MouseKind::Up, (x + 4, y), true);
        assert_eq!(app.clipboard, "ready");
    }
}
