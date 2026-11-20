//! Words on a comment's card and its answers are selected with the mouse as a page's are: a
//! drag, a double click for a word, a triple click for a line, Shift+click to reach further.
//! The copy key copies them, and a right click on a card has Copy, Copy Comment and Copy Quote,
//! each through the clipboard every copy goes by (OSC 52, over SSH too). What's pasted into a
//! comment being written goes in whole, its lines and all. Driven through the app with the
//! mouse and keys, and read off the grid and the clipboard handed to the terminal.

use std::fs;

use cellix_input::{Command, Event, Key, KeyEvent, Mods, MouseEvent, MouseKind};
use cellix_render::Grid;
use tempfile::TempDir;

use super::{App, Overlay};

/// A file with a comment of two lines on its first line, answered by an agent.
fn commented() -> (TempDir, App) {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("alpha.txt"), "first line\nsecond line\n").unwrap();
    let mut app = App::new(dir.path().to_path_buf(), (130, 30));
    app.show_sidebar = false;
    app.open(&dir.path().join("alpha.txt")).unwrap();
    let thread = r#"{"id": 4, "file": "alpha.txt", "line": 1, "quote": "first line",
        "text": "Rename the thing here.\nThen say why.",
        "replies": [{"author": "Claude", "text": "Agree: the answer words."}]}"#;
    app.threads = vec![serde_json::from_str(thread).unwrap()];
    app.run(Command::ShowComments);
    (dir, app)
}

fn screen(app: &App) -> Grid {
    let mut grid = Grid::new(app.size().0, app.size().1, app.background());
    app.draw(&mut grid);
    grid
}

fn press(app: &mut App, kind: MouseKind, (x, y): (u16, u16), mods: Mods) {
    app.handle(Event::Mouse(MouseEvent {
        kind,
        x,
        y,
        fine_x: f32::from(x) + 0.5,
        fine_y: f32::from(y) + 0.5,
        mods,
    }));
}

fn click(app: &mut App, at: (u16, u16)) {
    press(app, MouseKind::Left, at, Mods::default());
    press(app, MouseKind::Up, at, Mods::default());
}

/// Presses at `from`, drags to `to` and lets go there.
fn drag(app: &mut App, from: (u16, u16), to: (u16, u16)) {
    press(app, MouseKind::Left, from, Mods::default());
    press(app, MouseKind::Drag, to, Mods::default());
    press(app, MouseKind::Up, to, Mods::default());
}

/// Where `text` starts on the screen, on the first row that has it.
fn find(grid: &Grid, text: &str) -> (u16, u16) {
    (0..grid.height())
        .find_map(|y| {
            let row = grid.row_text(y);
            let i = row.find(text)?;
            Some((row[..i].chars().count() as u16, y))
        })
        .unwrap_or_else(|| panic!("{text:?} not on screen:\n{}", grid.text()))
}

/// Where `text` ends on the screen: the column after its last letter.
fn end_of(grid: &Grid, text: &str) -> (u16, u16) {
    let (x, y) = find(grid, text);
    (x + text.chars().count() as u16, y)
}

/// What the copy key put on the clipboard and handed to the terminal to send on.
fn copied(app: &mut App) -> Option<String> {
    let sent = app.outgoing_clipboard.take();
    if let Some(sent) = &sent {
        assert_eq!(&app.clipboard, sent, "Cellix's own clipboard has it too");
    }
    sent
}

fn ctrl(c: char, shift: bool) -> Event {
    Event::Key(KeyEvent {
        key: Key::Char(c),
        mods: Mods {
            ctrl: true,
            shift,
            ..Mods::default()
        },
    })
}

/// A drag across a card's words selects them, tinted as a page's selection is, and the copy
/// key copies them: Command::Copy, and Cmd+C or Ctrl+Shift+C as the terminal sends them. A
/// selection over two lines keeps the new line between them, and one reaching into an answer
/// has the answer's words on a line of their own. A double click selects a word, a triple
/// click a line, and Shift+click reaches from where it began. Any other key lets it go.
#[test]
fn a_cards_words_are_selected_with_the_mouse_and_copied() {
    let (_dir, mut app) = commented();
    let grid = screen(&app);
    let from = find(&grid, "thing here");
    let to = end_of(&grid, "thing");
    drag(&mut app, from, to);
    let grid = screen(&app);
    let panel = app.dock_panel().unwrap();
    assert_eq!(
        grid.cell(from.0, from.1).style.bg,
        app.theme.selection,
        "tinted: {:?} shown {:?} at {from:?} in {:?}\n{}",
        app.card_selection,
        app.card_selected(panel),
        app.dock.tops.of(crate::dock::Front::Comments),
        grid.text()
    );
    assert_ne!(
        grid.cell(to.0, to.1).style.bg,
        app.theme.selection,
        "to there"
    );
    app.run(Command::Copy);
    assert_eq!(copied(&mut app).as_deref(), Some("thing"));
    // As the terminal sends Cmd+C, and Ctrl+Shift+C.
    app.handle(ctrl('c', false));
    assert_eq!(copied(&mut app).as_deref(), Some("thing"));
    app.handle(ctrl('C', true));
    assert_eq!(copied(&mut app).as_deref(), Some("thing"));

    // Over two lines, with the line between them; and into the answer.
    let grid = screen(&app);
    let from = find(&grid, "here.");
    drag(&mut app, from, end_of(&grid, "Then"));
    app.run(Command::Copy);
    assert_eq!(copied(&mut app).as_deref(), Some("here.\nThen"));
    drag(&mut app, find(&grid, "why."), end_of(&grid, "The answer"));
    app.run(Command::Copy);
    assert_eq!(copied(&mut app).as_deref(), Some("why.\nThe answer"));

    // A word, and a line.
    let (x, y) = find(&grid, "thing");
    click(&mut app, (x + 2, y));
    click(&mut app, (x + 2, y));
    app.run(Command::Copy);
    assert_eq!(copied(&mut app).as_deref(), Some("thing"));
    let (x, y) = find(&grid, "say why");
    for _ in 0..3 {
        click(&mut app, (x + 1, y));
    }
    app.run(Command::Copy);
    assert_eq!(copied(&mut app).as_deref(), Some("Then say why."));

    // Shift+click reaches from where the last press began.
    click(&mut app, find(&grid, "Rename"));
    let shift = Mods {
        shift: true,
        ..Mods::default()
    };
    press(
        &mut app,
        MouseKind::Left,
        end_of(&grid, "Rename the"),
        shift,
    );
    press(&mut app, MouseKind::Up, end_of(&grid, "Rename the"), shift);
    app.run(Command::Copy);
    assert_eq!(copied(&mut app).as_deref(), Some("Rename the"));

    // Another key, and it's let go: the copy key is the file's again.
    app.handle(Event::Key(KeyEvent::plain(Key::Right)));
    assert!(app.card_selection.is_none());
    assert_ne!(
        screen(&app)
            .cell(find(&grid, "Rename").0, find(&grid, "Rename").1)
            .style
            .bg,
        app.theme.selection
    );
}

/// A right click on a card has Copy Comment and Copy Quote, and Copy over them once words on
/// the cards are selected; on a link, Open Link and Copy Link over those. Each puts its words
/// on the clipboard and says so.
#[test]
fn a_right_click_on_a_card_copies_it_its_quote_or_whats_selected() {
    let (_dir, mut app) = commented();
    let grid = screen(&app);
    let at = find(&grid, "Rename");
    press(
        &mut app,
        MouseKind::Right,
        (at.0 + 1, at.1),
        Mods::default(),
    );
    let labels = match &app.overlay {
        Overlay::Menu(menu) => menu.labels(),
        _ => panic!("no menu:\n{}", screen(&app).text()),
    };
    assert_eq!(labels, ["Copy Comment", "Copy Quote"]);
    choose(&mut app, "Copy Comment");
    assert_eq!(
        copied(&mut app).as_deref(),
        Some("Rename the thing here.\nThen say why.")
    );
    assert_eq!(*app.message.borrow(), "Copied the comment");
    press(
        &mut app,
        MouseKind::Right,
        (at.0 + 1, at.1),
        Mods::default(),
    );
    choose(&mut app, "Copy Quote");
    assert_eq!(copied(&mut app).as_deref(), Some("first line"));

    // With a word selected, Copy comes first.
    let (x, y) = find(&grid, "thing");
    click(&mut app, (x + 2, y));
    click(&mut app, (x + 2, y));
    press(&mut app, MouseKind::Right, (x + 2, y), Mods::default());
    let labels = match &app.overlay {
        Overlay::Menu(menu) => menu.labels(),
        _ => panic!("no menu"),
    };
    assert_eq!(labels, ["Copy", "Copy Comment", "Copy Quote"]);
    choose(&mut app, "Copy");
    assert_eq!(copied(&mut app).as_deref(), Some("thing"));

    // On the answer's words too, the same card's.
    let (x, y) = find(&screen(&app), "answer words");
    press(&mut app, MouseKind::Right, (x + 1, y), Mods::default());
    choose(&mut app, "Copy Comment");
    assert!(copied(&mut app).is_some_and(|c| c.starts_with("Rename")));
}

/// Where there are pictures, what's selected on a card is tinted under its letters, set at
/// their own widths, and copied as the cells' tier copies it.
#[test]
fn where_there_are_pictures_a_cards_selection_is_tinted_under_its_words() {
    let (_dir, mut app) = commented();
    app.set_measure(std::rc::Rc::new(
        cellix_render::FontMeasure::new(17, 50).with_type_row(36),
    ));
    let grid = screen(&app);
    let from = find(&grid, "Rename");
    drag(&mut app, from, (from.0 + 12, from.1));
    let grid = screen(&app);
    let panel = app.dock_panel().unwrap();
    let tinted = grid.paint.iter().any(|p| {
        matches!(p, cellix_render::Paint::Panel { fill: Some(fill), .. } if *fill == app.theme.selection)
    });
    assert!(
        tinted,
        "{:?} shown {:?}",
        app.card_selection,
        app.card_selected(panel)
    );
    app.run(Command::Copy);
    let words = copied(&mut app).unwrap_or_default();
    assert!(words.contains("ename the"), "{words:?}");
}

/// Picks a row of the menu that's open.
fn choose(app: &mut App, label: &str) {
    let (x, y) = find(&screen(app), label);
    click(app, (x + 1, y));
    assert!(matches!(app.overlay, Overlay::None), "the menu stayed open");
}

/// What's pasted into a comment being written goes in whole, its lines and all: as the
/// terminal pastes it (bracketed), and by Cmd+V from the clipboard Cellix copied to.
#[test]
fn a_paste_into_a_comment_being_written_keeps_its_lines() {
    let (_dir, mut app) = commented();
    let reply = find(&screen(&app), "Reply");
    click(&mut app, reply);
    assert!(app.dock.draft.is_some(), "a reply begun");
    app.handle(Event::Paste("one line\nand the next".into()));
    app.clipboard = "\nthen a third".into();
    app.handle(ctrl('v', false));
    let text = app.dock.draft.as_ref().map(|d| d.text.clone());
    assert_eq!(
        text.as_deref(),
        Some("one line\nand the next\nthen a third")
    );
    let grid = screen(&app);
    let first = find(&grid, "one line").1;
    assert_eq!(
        find(&grid, "and the next").1,
        first + 1,
        "on rows of their own"
    );
    app.handle(Event::Key(KeyEvent::plain(Key::Enter)));
    let reply = app.threads[0].replies.last().map(|r| r.text.clone());
    assert_eq!(
        reply.as_deref(),
        Some("one line\nand the next\nthen a third")
    );
}
