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
        Overlay::Prompt(prompt) => prompt.input.to_string(),
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
    // Home, as Ghostty sends Cmd+Left. Ctrl+A went to the line's start too until every box
    // selected all of its words with it, as the file does.
    plain(&mut app, Key::Home);
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

/// What the file's words are in the table below, on one line.
const WORDS: &str = "fix the cache now";

/// Every kind of box, each opened over the one-line file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    File,
    Comment,
    Reply,
    Ask,
    Commit,
    NewWorktree,
    Palette,
    Emoji,
    Prompt,
    Find,
    FindReplace,
    Search,
    SearchReplace,
    Settings,
}

const KINDS: [Kind; 14] = [
    Kind::File,
    Kind::Comment,
    Kind::Reply,
    Kind::Ask,
    Kind::Commit,
    Kind::NewWorktree,
    Kind::Palette,
    Kind::Emoji,
    Kind::Prompt,
    Kind::Find,
    Kind::FindReplace,
    Kind::Search,
    Kind::SearchReplace,
    Kind::Settings,
];

/// A repository holding the one-line file, open, and no agent to be found for Ask.
fn one_line() -> (TempDir, App) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("one");
    fs::create_dir_all(root.join("bin")).unwrap();
    fs::write(root.join("one.txt"), WORDS).unwrap();
    let who = ["-c", "user.name=t", "-c", "user.email=t@example.com"];
    for args in [
        &["init", "-q", "-b", "main"][..],
        &["add", "one.txt"],
        &["commit", "-qm", "First"],
    ] {
        let done = std::process::Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(who)
            .args(args)
            .output()
            .unwrap();
        assert!(done.status.success(), "git {args:?}");
    }
    let root = dunce::canonicalize(&root).unwrap();
    let mut app = App::new(root.clone(), (130, 30));
    app.open(&root.join("one.txt")).unwrap();
    app.side_path = Some(root.join("bin").into_os_string());
    (dir, app)
}

/// Opens a box of `kind`, its words emptied and the table's words pasted in, its caret after
/// them; for the file, the same done to the file, so the undo behind each is alike.
fn opened(kind: Kind) -> (TempDir, App) {
    opened_with(kind, WORDS)
}

/// `opened`, with `words` pasted into the box in place of the table's.
fn opened_with(kind: Kind, words: &str) -> (TempDir, App) {
    let (dir, mut app) = one_line();
    match kind {
        Kind::File => {}
        Kind::Comment => app.run(Command::AddComment),
        Kind::Reply => {
            app.run(Command::AddComment);
            typed(&mut app, "first");
            plain(&mut app, Key::Enter);
            let id = app.threads.last().expect("the comment").id;
            app.thread_hit(super::ThreadHit::Reply(id));
        }
        Kind::Ask => {
            app.run(Command::AskAgent);
            let side = app.side.as_mut().expect("Ask's card");
            side.phase = super::side::Phase::Asking;
            side.field = Some(crate::field::TextBox::default());
        }
        Kind::Commit => {
            app.run(Command::ShowSourceControl);
            app.source.focused = true;
        }
        Kind::NewWorktree => app.run(Command::NewWorktree),
        Kind::Palette => app.run(Command::QuickOpen),
        Kind::Emoji => {
            app.overlay = Overlay::Emoji(crate::emoji::Picker::new(
                (10, 5),
                cellix_render::Rect::new(0, 0, 130, 30),
                &[],
            ));
        }
        Kind::Prompt => app.ask("Save review as", super::PromptAction::SaveReview),
        Kind::Find => app.run(Command::FindInFile),
        Kind::FindReplace => {
            app.run(Command::FindInFile);
            typed(&mut app, "x");
            app.run(Command::ReplaceInFile);
        }
        Kind::Search => app.run(Command::FindInProject),
        Kind::SearchReplace => {
            app.run(Command::FindInProject);
            plain(&mut app, Key::Tab);
        }
        Kind::Settings => app.run(Command::OpenSettings),
    }
    let empty = match kind {
        Kind::File => false,
        _ => app
            .focused_box()
            .unwrap_or_else(|| panic!("{kind:?} has no box"))
            .is_empty(),
    };
    if !empty {
        press(&mut app, Key::Char('a'), CTRL);
        plain(&mut app, Key::Backspace);
    }
    app.handle(Event::Paste(words.into()));
    (dir, app)
}

/// The words, the caret and what's selected, in bytes, of a box or of the file.
fn state(app: &App, kind: Kind) -> (String, usize, Option<(usize, usize)>) {
    if kind == Kind::File {
        let editor = &app.editors[app.active()];
        let rope = editor.buffer.rope();
        let s = editor.selections.primary();
        let byte = |at: usize| rope.char_to_byte(at);
        let selected = (!s.is_empty()).then(|| (byte(s.start()), byte(s.end())));
        return (editor.buffer.text(), byte(s.head), selected);
    }
    let field = app
        .focused_box()
        .unwrap_or_else(|| panic!("{kind:?} lost its box"));
    (field.text().to_string(), field.caret(), field.selection())
}

/// Every key does in a box what it does in the file, in each form a terminal sends it: the
/// same keys, sent to a one-line file and to each kind of box from the same words, leave the
/// same words, caret and selection. Enter, Esc and Tab are each box's own, Ctrl+W takes a
/// word in a box where it closes the file's tab, Up and Down step through a one-line box's
/// list, and the emoji picker's plain arrows move among the emoji: those aren't in the table.
#[test]
fn every_key_does_in_every_box_what_it_does_in_the_file() {
    let key = |key, ctrl, alt, shift| KeyEvent {
        key,
        mods: Mods { ctrl, alt, shift },
    };
    let k = |c| key(Key::Char(c), false, false, false);
    let ctrl = |c| key(Key::Char(c), true, false, false);
    let ctrl_shift = |c| key(Key::Char(c), true, false, true);
    let alt = |c| key(Key::Char(c), false, true, false);
    let bare = |k| key(k, false, false, false);
    let shift = |k| key(k, false, false, true);
    let alt_shift = |k| key(k, false, true, true);
    let ctrl_key = |k| key(k, true, false, false);
    let ctrl_shift_key = |k| key(k, true, false, true);
    let alt_key = |k| key(k, false, true, false);
    let queue: Vec<KeyEvent> = "queue".chars().map(k).collect();
    let typed_over = |then: &[KeyEvent]| {
        let mut keys = vec![alt_shift(Key::Left)];
        keys.extend(queue.iter().copied());
        keys.extend_from_slice(then);
        keys
    };
    let rows: Vec<(&str, Vec<KeyEvent>)> = vec![
        ("Shift+Left, three times", vec![shift(Key::Left); 3]),
        ("Shift+Option+Left (CSI 1;4D)", vec![alt_shift(Key::Left)]),
        (
            "Shift+Option+Left twice, Shift+Option+Right",
            vec![
                alt_shift(Key::Left),
                alt_shift(Key::Left),
                alt_shift(Key::Right),
            ],
        ),
        ("Option+Left as ESC b, a letter", vec![alt('b'), k('x')]),
        (
            "Option+Right as ESC f from Home",
            vec![bare(Key::Home), alt('f'), k('x')],
        ),
        ("Option+Left (CSI 1;3D)", vec![alt_key(Key::Left), k('x')]),
        (
            "Ctrl+Left twice, Ctrl+Shift+Right",
            vec![
                ctrl_key(Key::Left),
                ctrl_key(Key::Left),
                ctrl_shift_key(Key::Right),
            ],
        ),
        ("Home, Shift+End", vec![bare(Key::Home), shift(Key::End)]),
        (
            "Shift+Home, a letter over it",
            vec![shift(Key::Home), k('y')],
        ),
        (
            "Cmd+Right as Ghostty sends it (Ctrl+E), Shift with it",
            vec![bare(Key::Home), ctrl('e'), bare(Key::Home), ctrl_shift('E')],
        ),
        (
            "Ctrl+Home, Ctrl+Shift+End",
            vec![ctrl_key(Key::Home), ctrl_shift_key(Key::End)],
        ),
        ("Ctrl+A, a letter over all of it", vec![ctrl('a'), k('z')]),
        (
            "Shift+Left twice, Backspace",
            vec![shift(Key::Left), shift(Key::Left), bare(Key::Backspace)],
        ),
        (
            "Shift+Right from Home, Delete",
            vec![
                bare(Key::Home),
                shift(Key::Right),
                shift(Key::Right),
                bare(Key::Delete),
            ],
        ),
        ("Option+Backspace", vec![alt_key(Key::Backspace)]),
        ("Ctrl+Backspace", vec![ctrl_key(Key::Backspace)]),
        ("Cmd+Backspace as Ctrl+U", vec![alt('b'), ctrl('u')]),
        ("Ctrl+K", vec![alt('b'), alt('b'), ctrl('k')]),
        (
            "Option+Delete from Home",
            vec![bare(Key::Home), alt_key(Key::Delete)],
        ),
        (
            "Left takes a selection to its start",
            vec![alt_shift(Key::Left), alt_shift(Key::Left), bare(Key::Left)],
        ),
        (
            "Right takes a selection to its end",
            vec![bare(Key::Home), alt_shift(Key::Right), bare(Key::Right)],
        ),
        ("Shift+Right at the end", vec![shift(Key::Right)]),
        (
            "A word typed over a selection, Ctrl+Z",
            typed_over(&[ctrl('z')]),
        ),
        (
            "Ctrl+Z, Ctrl+Shift+Z",
            typed_over(&[ctrl('z'), ctrl_shift('Z')]),
        ),
        ("Ctrl+Z, Ctrl+Y", typed_over(&[ctrl('z'), ctrl('y')])),
        (
            "Ctrl+X, Home, Ctrl+V",
            vec![alt_shift(Key::Left), ctrl('x'), bare(Key::Home), ctrl('v')],
        ),
        (
            "Ctrl+C, a word selected, Ctrl+V over it",
            vec![
                alt_shift(Key::Left),
                ctrl('c'),
                bare(Key::Home),
                alt_shift(Key::Right),
                ctrl('v'),
            ],
        ),
        (
            "Ctrl+Shift+C, Ctrl+V",
            vec![
                alt_shift(Key::Left),
                ctrl_shift('C'),
                bare(Key::Home),
                ctrl('v'),
            ],
        ),
    ];
    let plain_arrow = |keys: &[KeyEvent]| {
        keys.iter()
            .any(|k| matches!(k.key, Key::Left | Key::Right) && k.mods == Mods::default())
    };
    for (name, keys) in &rows {
        let (_dir, mut file) = opened(Kind::File);
        for key in keys {
            file.handle(Event::Key(*key));
        }
        let want = state(&file, Kind::File);
        for kind in KINDS.into_iter().skip(1) {
            if kind == Kind::Emoji && plain_arrow(keys) {
                continue;
            }
            let (_dir, mut app) = opened(kind);
            assert_eq!(
                state(&app, kind),
                (WORDS.to_string(), WORDS.len(), None),
                "{kind:?} before {name}"
            );
            for key in keys {
                app.handle(Event::Key(*key));
            }
            assert_eq!(state(&app, kind), want, "{name}, in {kind:?}");
        }
    }
}

/// A press, a drag or a release of the left button at `x`, with its fraction, on row `y`.
fn pointer(app: &mut App, kind: MouseKind, x: f32, y: u16, shift: bool) {
    app.handle(Event::Mouse(MouseEvent {
        kind,
        x: x as u16,
        y,
        fine_x: x,
        fine_y: f32::from(y) + 0.5,
        mods: Mods {
            shift,
            ..Mods::default()
        },
    }));
}

/// The mouse does in every box what it does in the file: a press puts the caret at the letter
/// under it, a drag selects, a double click selects the word and a triple click the line, and
/// Shift with a press selects from the caret. Until now a comment's box and Ask's took no
/// press at all, and no box selected.
#[test]
fn the_mouse_does_in_every_box_what_it_does_in_the_file() {
    // Short, for the Search sidebar's box, which is narrow.
    const TEXT: &str = "ab cde fgh";
    for kind in KINDS.into_iter().skip(1) {
        let (_dir, mut app) = opened_with(kind, TEXT);
        let grid = screen(&app);
        let (x0, y) = (0..app.size().1)
            .find_map(|y| on_row(&grid, y, TEXT).map(|x| (x, y)))
            .unwrap_or_else(|| panic!("{kind:?}: the words aren't on screen"));
        let at = |letter: u16| f32::from(x0 + letter) + 0.2;
        let shown = |caret, selected| (TEXT.to_string(), caret, selected);
        pointer(&mut app, MouseKind::Left, at(3), y, false);
        assert_eq!(state(&app, kind), shown(3, None), "{kind:?}: a press");
        pointer(&mut app, MouseKind::Drag, at(5), y, false);
        pointer(&mut app, MouseKind::Drag, at(6), y, false);
        pointer(&mut app, MouseKind::Up, at(6), y, false);
        assert_eq!(
            state(&app, kind),
            shown(6, Some((3, 6))),
            "{kind:?}: a drag"
        );
        for _ in 0..2 {
            pointer(&mut app, MouseKind::Left, at(8), y, false);
            pointer(&mut app, MouseKind::Up, at(8), y, false);
        }
        let word = shown(10, Some((7, 10)));
        assert_eq!(state(&app, kind), word, "{kind:?}: a double click");
        pointer(&mut app, MouseKind::Left, at(8), y, false);
        pointer(&mut app, MouseKind::Up, at(8), y, false);
        let line = shown(10, Some((0, 10)));
        assert_eq!(state(&app, kind), line, "{kind:?}: a triple click");
        pointer(&mut app, MouseKind::Left, at(0), y, false);
        pointer(&mut app, MouseKind::Up, at(0), y, false);
        pointer(&mut app, MouseKind::Left, at(2), y, true);
        let extended = shown(2, Some((0, 2)));
        assert_eq!(state(&app, kind), extended, "{kind:?}: Shift with a press");
        // What's typed goes over what the mouse selected.
        typed(&mut app, "zz");
        assert_eq!(state(&app, kind).0, "zz cde fgh", "{kind:?}");
    }
}

/// Keys as Vim's Normal mode reads them, a letter at a time, and Esc.
fn vim_keys(app: &mut App, keys: &str) {
    for c in keys.chars() {
        match c {
            '\x1b' => plain(app, Key::Esc),
            c => plain(app, Key::Char(c)),
        }
    }
}

/// With the `vim` keymap the boxes written in follow Vim, as the file does: they open in
/// Insert, Esc goes to Normal, and motions, operators with motions and text objects, undo,
/// yank and put work on them; Normal's mode shows in the box, and Esc there closes the box as
/// it always has. The one-line boxes stay natural, and Esc closes them at once.
#[test]
fn vim_keys_work_in_the_boxes_written_in() {
    for kind in [Kind::Comment, Kind::Reply, Kind::Ask, Kind::Commit] {
        let (_dir, mut app) = opened(kind);
        app.settings.keymap = Some("vim".to_string());
        let shown = |app: &App| state(app, kind);
        // Typing is Insert's; Esc steps back onto the last letter, as Vim does.
        vim_keys(&mut app, "\x1b");
        assert_eq!(shown(&app), (WORDS.to_string(), 16, None), "{kind:?}: Esc");
        assert!(
            screen(&app).text().contains("NORMAL"),
            "{kind:?}: its mode shows"
        );
        if matches!(kind, Kind::Comment | Kind::Reply) {
            let text = screen(&app).text();
            assert!(text.contains("i to type. Esc cancels."), "{kind:?}: {text}");
        }
        vim_keys(&mut app, "b");
        assert_eq!(shown(&app).1, 14, "{kind:?}: b, to the word's start");
        vim_keys(&mut app, "dw");
        assert_eq!(shown(&app).0, "fix the cache ", "{kind:?}: dw");
        vim_keys(&mut app, "u");
        assert_eq!(shown(&app).0, WORDS, "{kind:?}: u");
        vim_keys(&mut app, "0x");
        assert_eq!(shown(&app).0, "ix the cache now", "{kind:?}: 0 and x");
        vim_keys(&mut app, "u");
        // A text object, then Insert, typing, and Normal again.
        vim_keys(&mut app, "0ciwmend\x1b");
        assert_eq!(shown(&app).0, "mend the cache now", "{kind:?}: ciw");
        // Yank a word and put it after the last letter.
        vim_keys(&mut app, "0yiw$p");
        assert_eq!(
            shown(&app).0,
            "mend the cache nowmend",
            "{kind:?}: yiw and p"
        );
        // Ctrl+R takes back what u undid.
        vim_keys(&mut app, "u");
        assert_eq!(shown(&app).0, "mend the cache now", "{kind:?}: u");
        press(&mut app, Key::Char('r'), CTRL);
        assert_eq!(shown(&app).0, "mend the cache nowmend", "{kind:?}: Ctrl+R");
        // Visual: v, a motion, d, the letter after the motion taken too, as in the file.
        vim_keys(&mut app, "0ved");
        assert_eq!(shown(&app).0, "the cache nowmend", "{kind:?}: ved");
        // Esc in Normal does what Esc does to the box today: it's closed, or let go.
        vim_keys(&mut app, "\x1b");
        assert!(
            app.focused_box().is_none(),
            "{kind:?}: Esc in Normal closes it"
        );
    }

    // A one-line box stays natural: the letters are typed, and Esc closes it at once.
    for kind in [Kind::Palette, Kind::Find, Kind::Prompt] {
        let (_dir, mut app) = opened(kind);
        app.settings.keymap = Some("vim".to_string());
        vim_keys(&mut app, "dw");
        assert_eq!(state(&app, kind).0, format!("{WORDS}dw"), "{kind:?}");
        assert!(!screen(&app).text().contains("NORMAL"), "{kind:?}");
        vim_keys(&mut app, "\x1b");
        assert!(
            matches!(app.overlay, Overlay::None),
            "{kind:?}: Esc closes it"
        );
    }
}
