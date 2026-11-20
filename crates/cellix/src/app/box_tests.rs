//! Every box of one line edits text as the comment composer, the palette and the side
//! pane's field do: a caret that moves by letter, word and line, the deletes a Mac or a
//! shell has taught, what's pasted going in at the caret, and a press putting the caret at
//! the letter pressed. Driven through the app and read off the grid.

use std::fs;
use std::rc::Rc;

use cellix_input::{Command, Event, Key, KeyEvent, Mods, MouseEvent, MouseKind};
use cellix_render::{Grid, Paint};
use tempfile::TempDir;

use super::{App, Overlay};

const ALT: Mods = Mods {
    alt: true,
    ctrl: false,
    shift: false,
};
const CTRL: Mods = Mods {
    alt: false,
    ctrl: true,
    shift: false,
};

fn project() -> (TempDir, App) {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("alpha.txt"), "first line\nsecond line\n").unwrap();
    let app = App::new(dir.path().to_path_buf(), (130, 30));
    (dir, app)
}

fn screen(app: &App) -> Grid {
    let mut grid = Grid::new(app.size().0, app.size().1, app.background());
    app.draw(&mut grid);
    grid
}

fn press(app: &mut App, key: Key, mods: Mods) {
    app.handle(Event::Key(KeyEvent { key, mods }));
}

fn plain(app: &mut App, key: Key) {
    press(app, key, Mods::default());
}

fn typed(app: &mut App, text: &str) {
    for c in text.chars() {
        plain(app, Key::Char(c));
    }
}

fn click(app: &mut App, x: f32, y: u16) {
    app.handle(Event::Mouse(MouseEvent {
        kind: MouseKind::Left,
        x: x as u16,
        y,
        fine_x: x,
        fine_y: f32::from(y) + 0.5,
        mods: Mods::default(),
    }));
}

/// Where `words` start on row `y`, in cells.
fn on_row(grid: &Grid, y: u16, words: &str) -> Option<u16> {
    let row = grid.row_text(y);
    let at = row.find(words)?;
    Some(row[..at].chars().count() as u16)
}

/// What's typed in the prompt that's open.
fn prompted(app: &App) -> String {
    match &app.overlay {
        Overlay::Prompt(prompt) => prompt.input.clone(),
        _ => panic!("no prompt open"),
    }
}

/// A tab in the side pane running a stand-in that only waits, so nothing of the person's own
/// shell or its files is read.
fn waiting_tab(app: &mut App) -> usize {
    let root = app.workspace.root().to_path_buf();
    let argv = ["sleep".to_string(), "30".to_string()];
    app.start_session(crate::agents::SHELL.0, &argv, &[], &root);
    app.dock.sessions.len() - 1
}

/// Rename tab held only letters and Backspace (the user: "Rename tab is not supporting
/// natural text writing as in other places (word deletion etc)").
#[test]
#[cfg(unix)]
fn rename_tab_edits_its_name_as_any_text_box_does() {
    let (dir, mut app) = project();
    app.open(&dir.path().join("alpha.txt")).unwrap();
    let index = waiting_tab(&mut app);
    let name = app.dock.sessions[index].label();
    app.rename_session(index);
    // It opens with the name the tab has, the caret after it.
    assert_eq!(prompted(&app), name);
    match &app.overlay {
        Overlay::Prompt(prompt) => assert_eq!(prompt.caret(), name.len()),
        _ => unreachable!(),
    }
    // Cmd+Backspace, as Ghostty sends it, takes everything before the caret.
    press(&mut app, Key::Char('u'), CTRL);
    assert_eq!(prompted(&app), "");
    typed(&mut app, "fix the login page now");
    // Option+Backspace takes a word, and so does Ctrl+W, as in a shell.
    press(&mut app, Key::Backspace, ALT);
    assert_eq!(prompted(&app), "fix the login page ");
    press(&mut app, Key::Char('w'), CTRL);
    assert_eq!(prompted(&app), "fix the login ");
    assert!(
        app.active_text().is_some(),
        "Ctrl+W in the box leaves the file under it open"
    );
    typed(&mut app, "redirect");
    // Option+Left goes back a word, and typing goes in there.
    press(&mut app, Key::Left, ALT);
    typed(&mut app, "page ");
    assert_eq!(prompted(&app), "fix the login page redirect");
    // Home, then Delete takes the letter after the caret.
    plain(&mut app, Key::Home);
    plain(&mut app, Key::Delete);
    typed(&mut app, "F");
    assert_eq!(prompted(&app), "Fix the login page redirect");
    plain(&mut app, Key::End);
    typed(&mut app, " flow");
    // Ctrl+Backspace takes a word too.
    press(&mut app, Key::Backspace, CTRL);
    typed(&mut app, "loop");
    assert_eq!(prompted(&app), "Fix the login page redirect loop");
    // What's pasted goes in at the caret.
    press(&mut app, Key::Char('b'), ALT);
    app.handle(Event::Paste("refresh ".into()));
    assert_eq!(prompted(&app), "Fix the login page redirect refresh loop");
    plain(&mut app, Key::Enter);
    assert_eq!(
        app.dock.sessions[index].label(),
        "Fix the login page redirect refresh loop"
    );

    // A press on a letter in the box puts the caret there.
    app.rename_session(index);
    let (_, line) = app.prompt_area();
    let grid = screen(&app);
    let x = on_row(&grid, line.y, "Fix the").expect("the name in the box");
    click(&mut app, f32::from(x) + 4.0, line.y);
    typed(&mut app, "all ");
    assert_eq!(
        prompted(&app),
        "Fix all the login page redirect refresh loop"
    );
    // Past the words is their end.
    click(&mut app, f32::from(line.right()) - 2.0, line.y);
    typed(&mut app, "s");
    assert_eq!(
        prompted(&app),
        "Fix all the login page redirect refresh loops"
    );
    plain(&mut app, Key::Esc);
    assert_eq!(
        app.dock.sessions[index].label(),
        "Fix the login page redirect refresh loop",
        "Escape keeps the name it had"
    );
}

/// The caret was drawn after the last letter wherever it was: moved, it's drawn where it is,
/// in the cells and in the pictures.
#[test]
#[cfg(unix)]
fn a_boxs_caret_is_drawn_where_it_stands() {
    let (_dir, mut app) = project();
    let index = waiting_tab(&mut app);
    app.rename_session(index);
    press(&mut app, Key::Char('u'), CTRL);
    typed(&mut app, "pricing page");
    for _ in 0.."page".len() {
        plain(&mut app, Key::Left);
    }
    let (_, line) = app.prompt_area();
    let grid = screen(&app);
    let x = on_row(&grid, line.y, "pricing page").expect("the name in the box");
    assert_eq!(grid.cursor, Some((x + 8, line.y)), "in the cell of the p");

    // Painted, it stands in the middle of the space before "page", clear of the g and the
    // p: 0.1 of a column before the space's end, it stood 1 px from the p's stem.
    let (cell, row) = (17, 37);
    app.set_measure(Rc::new(cellix_render::FontMeasure::new(cell, row)));
    let grid = screen(&app);
    let painted = grid.paint.iter().rev().find_map(|p| match p {
        Paint::Text { text, x, .. } if text == "pricing page" => Some(*x),
        _ => None,
    });
    let painted = painted.expect("the name is painted") as f32;
    let font = cellix_render::Font {
        size: 95,
        ..cellix_render::Font::default()
    };
    let unit = cellix_render::UNIT as f32;
    let px = |units: f32| units * cell as f32 / unit;
    let measure = app.measure.clone();
    let wide = |words: &str| measure.width(words, font) * unit;
    // The caret is painted a ninth of a column wide, or 2 px.
    let half = (cell as f32 / 9.0).max(2.0) / 2.0;
    let caret = grid.caret.expect("a caret").x as f32;
    let middle = painted + wide("pricing") + wide(" ") / 2.0;
    let off = px(caret - middle) + half;
    assert!(off.abs() <= 1.0, "{off} px from the space's middle");
    // At the end it stands just past the last letter, 1 to 3 px from where it ends.
    plain(&mut app, Key::End);
    let grid = screen(&app);
    let caret = grid.caret.expect("a caret").x as f32;
    let past = px(caret - (painted + wide("pricing page")));
    assert!((1.0..3.0).contains(&past), "{past} px past the e");
    // And between two letters, on the edge between them.
    press(&mut app, Key::Left, ALT);
    plain(&mut app, Key::Right);
    let grid = screen(&app);
    let caret = grid.caret.expect("a caret").x as f32;
    let off = px(caret - (painted + wide("pricing p"))) + half;
    assert!(off.abs() <= 1.0, "{off} px from between the p and the a");
}

/// A name longer than its box shows the part its caret is in, as the palette's field does.
#[test]
#[cfg(unix)]
fn a_long_name_shows_the_part_the_caret_is_in() {
    let (_dir, mut app) = project();
    let index = waiting_tab(&mut app);
    app.rename_session(index);
    press(&mut app, Key::Char('u'), CTRL);
    let long = "the start of it ".to_string() + &"and more ".repeat(20) + "the end";
    app.handle(Event::Paste(long.clone()));
    let (_, line) = app.prompt_area();
    let row = screen(&app).row_text(line.y);
    assert!(
        row.contains("the end") && !row.contains("the start"),
        "{row}"
    );
    let (caret, _) = screen(&app).cursor.expect("a caret");
    assert!(
        caret < line.right() && caret > line.x,
        "{caret} in {line:?}"
    );
    plain(&mut app, Key::Home);
    let grid = screen(&app);
    let row = grid.row_text(line.y);
    assert!(
        row.contains("the start") && !row.contains("the end"),
        "{row}"
    );
    assert_eq!(
        grid.cursor,
        Some((line.x + 1, line.y)),
        "before its first letter"
    );
}

/// Find's query and its replacement were the same: letters pushed, Backspace popped.
#[test]
fn the_find_bars_fields_edit_as_any_text_box_does() {
    let (dir, mut app) = project();
    app.open(&dir.path().join("alpha.txt")).unwrap();
    app.run(Command::FindInFile);
    let query = |app: &App| match &app.overlay {
        Overlay::Find(find) => (find.query.clone(), find.matches.len()),
        _ => panic!("Find closed"),
    };
    typed(&mut app, "secnd line");
    assert_eq!(query(&app), ("secnd line".into(), 0));
    // Back two words, along three letters, and the one left out goes in: found at once.
    press(&mut app, Key::Left, ALT);
    press(&mut app, Key::Left, ALT);
    for _ in 0..3 {
        plain(&mut app, Key::Right);
    }
    typed(&mut app, "o");
    assert_eq!(query(&app), ("second line".into(), 1));
    // Ctrl+W takes the word before the caret, and leaves the file be.
    plain(&mut app, Key::End);
    press(&mut app, Key::Char('w'), CTRL);
    assert_eq!(query(&app), ("second ".into(), 1));
    assert!(app.active_text().is_some(), "the file is open");
    // Home and Delete, and Option+Backspace.
    plain(&mut app, Key::Home);
    plain(&mut app, Key::Delete);
    assert_eq!(query(&app).0, "econd ");
    plain(&mut app, Key::End);
    press(&mut app, Key::Backspace, ALT);
    typed(&mut app, "line");
    assert_eq!(query(&app), ("line".into(), 2));
    // Cmd+Backspace, as Ghostty sends it.
    press(&mut app, Key::Char('u'), CTRL);
    assert_eq!(query(&app).0, "");
    typed(&mut app, "first");
    // What's pasted goes in at the caret.
    plain(&mut app, Key::Home);
    app.handle(Event::Paste("^".into()));
    assert_eq!(query(&app).0, "^first");

    // The replace row's field is a box like any other.
    press(&mut app, Key::Char('r'), CTRL);
    typed(&mut app, "the 1st one");
    press(&mut app, Key::Backspace, ALT);
    press(&mut app, Key::Left, ALT);
    typed(&mut app, "very ");
    let replacement = |app: &App| match &app.overlay {
        Overlay::Find(find) => find.replace.clone().unwrap_or_default(),
        _ => panic!("Find closed"),
    };
    assert_eq!(replacement(&app), "the very 1st ");
    assert_eq!(query(&app).0, "^first", "the query is as it was");

    // A press in the query's box puts the caret there and gives the box the keyboard.
    let grid = screen(&app);
    let y = app.find_bar().y;
    let x = on_row(&grid, y, "^first").expect("the query in its box");
    click(&mut app, f32::from(x) + 1.0, y);
    typed(&mut app, "(");
    assert_eq!(query(&app).0, "^(first");
    let grid = screen(&app);
    assert_eq!(grid.cursor, Some((x + 2, y)), "after the (");
    // And one in the replacement's, there.
    let x = on_row(&grid, y + 1, "the very").expect("the replacement in its box");
    click(&mut app, f32::from(x) + 4.0, y + 1);
    press(&mut app, Key::Char('w'), CTRL);
    assert_eq!(replacement(&app), "very 1st ");
}

/// The Search sidebar's boxes, Settings' filter and the emoji picker's field took letters and
/// Backspace only, and what was pasted with the Search box's keyboard went into the file.
#[test]
fn the_other_boxes_edit_as_any_text_box_does() {
    let (dir, mut app) = project();
    app.open(&dir.path().join("alpha.txt")).unwrap();
    app.run(Command::FindInProject);
    let searched = |app: &App| app.search.query.clone();
    typed(&mut app, "first lne");
    plain(&mut app, Key::Left);
    plain(&mut app, Key::Left);
    typed(&mut app, "i");
    assert_eq!(searched(&app), "first line");
    press(&mut app, Key::Char('e'), CTRL);
    press(&mut app, Key::Char('w'), CTRL);
    assert_eq!(searched(&app), "first ");
    assert!(app.active_text().is_some(), "the file is open");
    press(&mut app, Key::Char('a'), CTRL);
    app.handle(Event::Paste("the ".into()));
    assert_eq!(searched(&app), "the first ");
    assert_eq!(
        app.active_text().as_deref(),
        Some("first line\nsecond line\n")
    );
    // A press in the box puts the caret at the letter pressed.
    let area = app.sidebar_area().unwrap();
    let grid = screen(&app);
    let x = on_row(&grid, area.y + 1, "the first").expect("the query in its box");
    click(&mut app, f32::from(x) + 3.0, area.y + 1);
    press(&mut app, Key::Char('k'), CTRL);
    assert_eq!(searched(&app), "the");
    plain(&mut app, Key::Esc);

    // Settings' filter: typing anywhere there goes to it, and so do the deletes; the arrows
    // go along it while no control has the keyboard. Ctrl+W still closes Settings.
    app.run(Command::OpenSettings);
    let filter = |app: &App| app.settings_page.as_ref().unwrap().filter.clone();
    typed(&mut app, "word wrp");
    plain(&mut app, Key::Left);
    typed(&mut app, "a");
    assert_eq!(filter(&app), "word wrap");
    press(&mut app, Key::Left, ALT);
    press(&mut app, Key::Backspace, ALT);
    assert_eq!(filter(&app), "wrap");
    plain(&mut app, Key::End);
    typed(&mut app, "x");
    plain(&mut app, Key::Backspace);
    assert_eq!(filter(&app), "wrap");
    press(&mut app, Key::Char('w'), CTRL);
    assert!(app.settings_page.is_none(), "Ctrl+W closes Settings");

    // The emoji picker's arrows move among the emoji, so its deletes go from its end.
    app.overlay = Overlay::Emoji(crate::emoji::Picker::new(
        (10, 5),
        cellix_render::Rect::new(0, 0, 130, 30),
        &[],
    ));
    typed(&mut app, "red heart");
    press(&mut app, Key::Char('w'), CTRL);
    let query = |app: &App| match &app.overlay {
        Overlay::Emoji(picker) => picker.query.clone(),
        _ => panic!("the picker closed"),
    };
    assert_eq!(query(&app), "red ");
    press(&mut app, Key::Char('u'), CTRL);
    assert_eq!(query(&app), "");
    assert!(app.active_text().is_some(), "the file is open");
}
