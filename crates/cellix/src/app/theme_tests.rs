//! The dark theme as the app draws it: herdr's graphite on the frame, the sheet, a menu and an
//! agent's marks, read off the drawn grid.

use std::fs;
use std::path::Path;
use std::time::{Duration, Instant};

use cellix_agent::{AgentEvent, AgentState, Request};
use cellix_input::Command;
use cellix_render::{Grid, Rgb};
use tempfile::TempDir;

use super::App;

/// herdr's chrome, graphite's background and herdr's popover.
const FRAME: Rgb = Rgb(0x10, 0x11, 0x14);
const SHEET: Rgb = Rgb(0x17, 0x19, 0x1d);
const MENU: Rgb = Rgb(0x25, 0x27, 0x2b);

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

/// Where words first stand on the screen, as (column, row).
fn place_of(grid: &Grid, words: &str) -> Option<(u16, u16)> {
    (0..grid.height()).find_map(|y| {
        let text = grid.row_text(y);
        let at = text.find(words)?;
        Some((text[..at].chars().count() as u16, y))
    })
}

/// How many cells stand on herdr's popover.
fn menu_cells(grid: &Grid) -> usize {
    (0..grid.height())
        .flat_map(|y| (0..grid.width()).map(move |x| (x, y)))
        .filter(|(x, y)| grid.cell(*x, *y).style.bg == MENU)
        .count()
}

/// A stand-in for an agent's program that names itself in its terminal's title and waits.
fn stand_in(dir: &Path) -> String {
    let fake = dir.join("fake-agent");
    fs::write(
        &fake,
        "#!/bin/sh\nprintf '\\033]0;pricing page\\007'\nexec sleep 30\n",
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&fake, fs::Permissions::from_mode(0o755)).unwrap();
    }
    fake.display().to_string()
}

fn said(state: AgentState) -> Request {
    Request::Agent(AgentEvent {
        session: "s1".into(),
        agent: "Claude Code".into(),
        state,
        detail: String::new(),
        folder: "/w".into(),
        pid: None,
        transcript: String::new(),
        name: String::new(),
        prompt: String::new(),
    })
}

/// The window is herdr's: the foot on herdr's chrome, the file on graphite's background, a
/// menu on herdr's popover, and an agent's spinner and its asking in the colours herdr.dev
/// gives them.
#[test]
#[cfg(unix)]
fn a_dark_window_wears_herdrs_graphite() {
    let (dir, mut app) = project();
    app.open(&dir.path().join("alpha.txt")).unwrap();
    let grid = screen(&app);
    let foot = grid.height() - 1;
    let on_frame = (0..grid.width())
        .filter(|x| grid.cell(*x, foot).style.bg == FRAME)
        .count();
    assert!(
        on_frame > usize::from(grid.width()) / 2,
        "the foot is the frame: {on_frame} cells"
    );
    // The caret's line is washed, so the line under it shows the sheet.
    let (x, y) = place_of(&grid, "second line").expect("the file's second line");
    assert_eq!(
        grid.cell(x, y).style.bg,
        SHEET,
        "the file stands on a sheet"
    );
    assert_eq!(
        grid.cell(x, y).style.fg,
        Rgb(0xee, 0xee, 0xef),
        "herdr's words"
    );

    let before = menu_cells(&grid);
    app.run(Command::CommandPalette);
    let menu = menu_cells(&screen(&app));
    assert!(
        menu > before + 200,
        "the palette stands on herdr's popover: {menu} cells, {before} before it"
    );
    app.handle(cellix_input::Event::Key(cellix_input::KeyEvent {
        key: cellix_input::Key::Esc,
        mods: cellix_input::Mods::default(),
    }));

    app.agent_program = Some(stand_in(dir.path()));
    app.open_agent("claude");
    let index = app.dock.sessions.len() - 1;
    let deadline = Instant::now() + Duration::from_secs(30);
    while app.dock.sessions[index].title.is_none() {
        assert!(Instant::now() < deadline, "the stand-in never named itself");
        app.poll();
        std::thread::sleep(Duration::from_millis(20));
    }
    let row = app.dock_area().unwrap().y;
    // The spinner's frame on cells is a braille letter, and a dot is a dot.
    let mark = |grid: &Grid, wanted: fn(char) -> bool| {
        (0..grid.width())
            .find(|x| grid.cell(*x, row).symbol.chars().next().is_some_and(wanted))
            .map(|x| grid.cell(x, row).style.fg)
    };
    app.serve(said(AgentState::Working));
    let grid = screen(&app);
    let braille = |c: char| ('\u{2801}'..='\u{28ff}').contains(&c);
    let spinner = mark(&grid, braille);
    let spinner = spinner.unwrap_or_else(|| panic!("no spinner on {:?}", grid.row_text(row)));
    assert_eq!(spinner, Rgb(0x76, 0xa8, 0xff), "herdr.dev's blue");
    app.serve(said(AgentState::NeedsYou));
    let grid = screen(&app);
    let asking = mark(&grid, |c| c == '●');
    let asking = asking.unwrap_or_else(|| panic!("no dot on {:?}", grid.row_text(row)));
    assert_eq!(asking, Rgb(0xf0, 0x70, 0x5f), "herdr's blocked");

    // At the foot, once the hint about the pane has gone, what says so is in that colour, and
    // the file's details stay quiet.
    app.say("");
    let grid = screen(&app);
    let foot = grid.height() - 1;
    let said = place_of(&grid, "An agent needs you")
        .unwrap_or_else(|| panic!("nothing says so on {:?}", grid.row_text(foot)));
    assert_eq!(grid.cell(said.0, said.1).style.fg, Rgb(0xf0, 0x70, 0x5f));
    let (x, y) = place_of(&grid, "Ln 1").expect("the caret's place at the foot");
    assert_eq!(
        grid.cell(x, y).style.fg,
        Rgb(0x98, 0x99, 0x9b),
        "herdr's muted, not the attention colour"
    );
}

/// Where there are pictures the commit message's box, without the keyboard, has the line every
/// text box has (`Theme::field_line`), which shows on the sidebar: it had a menu's, 1.7 to 1.
/// And the page picked in Settings' list keeps the bright text colour on its wash, as every
/// other picked row's words do; they were the accent.
#[test]
fn the_commit_box_and_settings_picked_page_read_as_their_kind() {
    use cellix_render::Paint;
    let (dir, mut app) = project();
    let made = std::process::Command::new("git")
        .args(["init", "-q", "-b", "main"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert!(made.status.success());
    app.set_measure(std::rc::Rc::new(
        cellix_render::FontMeasure::new(17, 50).with_type_row(36),
    ));
    app.run(Command::ShowSourceControl);
    let grid = screen(&app);
    let theme = app.theme();
    let line = theme.field_line();
    let boxed = grid.paint.iter().any(|paint| {
        matches!(paint, Paint::Panel { fill: Some(fill), border: Some(edge), .. }
            if *fill == theme.text.bg && *edge == line)
    });
    assert!(boxed, "the message box has a text box's line");
    let ratio = crate::theme::contrast(line, theme.sidebar.bg);
    assert!(ratio >= 1.8, "{ratio:.2} to 1 on the sidebar");

    app.run(Command::OpenSettings);
    let grid = screen(&app);
    let theme = app.theme();
    let picked = grid.paint.iter().find_map(|paint| match paint {
        Paint::Text { text, color, .. } if text == "Appearance" => Some(*color),
        _ => None,
    });
    assert_eq!(picked, Some(theme.text.fg), "the picked page's words");
}
