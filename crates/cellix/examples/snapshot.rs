//! Renders the real app through the real rasterizer to a PNG, with no terminal or window.
//!
//!   snapshot <workspace> <out.png> [view] [columns] [rows]
//!
//! `view` is `review`, `comments` for the Comments pane with no file open, or a file to open.
//! Agents use this to see the interface.
//! `select:<file>:<word>` opens a file and double-clicks a word; `ask:<file>:<word>` then
//! presses Ask Agent. `across:<file>:<word>:<notches>` opens a file and tips the wheel
//! sideways that many notches over the row that word is on, or over that row of the screen
//! when it's a number, and `pane:<file>:<columns>` opens it beside the side pane dragged that
//! wide. `tabs:<file>:<columns>:<front>` does the same with a terminal's tab as well, and
//! `comments`, `agents` or `terminal` in front. `CELLIX_TIER=cells` lays it out on the cell
//! tier, as a terminal without pictures shows it, and `CELLIX_PANE=1` as the pane inside
//! Claude Code. `CELLIX_HOVER=<column>,<row>` rests the pointer there before the picture.

use std::fs::File;
use std::io::BufWriter;
use std::path::PathBuf;

use cellix::App;
use cellix_input::{Command, Event, Key, KeyEvent, Mods, MouseEvent, MouseKind};
use cellix_render::{FontMeasure, Grid, Rasterizer, SoftwareRasterizer};

const DEFAULT_SIZE: (u16, u16) = (120, 36);
// The cell size of a Retina terminal, so the picture matches what people see.
const CELL: (usize, usize) = (17, 37);

fn type_text(app: &mut App, text: &str) {
    for c in text.chars() {
        app.handle(Event::Key(KeyEvent::plain(Key::Char(c))));
    }
}

/// Where a word is drawn, as a column and row.
fn find(app: &App, word: &str) -> Option<(u16, u16)> {
    let (columns, rows) = app.size();
    let mut grid = Grid::new(columns, rows, app.background());
    app.draw(&mut grid);
    (0..rows).find_map(|y| {
        let row = grid.row_text(y);
        let at = row.find(word)?;
        Some((row[..at].chars().count() as u16, y))
    })
}

fn click(app: &mut App, (x, y): (u16, u16)) {
    for kind in [MouseKind::Left, MouseKind::Up] {
        app.handle(Event::Mouse(MouseEvent {
            kind,
            x,
            y,
            fine_x: x as f32 + 0.5,
            fine_y: y as f32 + 0.5,
            mods: Mods::default(),
        }));
    }
}

/// Opens a file and double-clicks a word in it.
fn select(app: &mut App, root: &std::path::Path, spec: &str) {
    let (file, word) = spec.split_once(':').expect("<file>:<word>");
    app.open(&root.join(file)).unwrap();
    let (x, y) = find(app, word).expect("the word on screen");
    click(app, (x + 1, y));
    click(app, (x + 1, y));
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [root, out, rest @ ..] = args.as_slice() else {
        panic!("usage: snapshot <workspace> <out.png> [review|file]");
    };
    let root = PathBuf::from(root).canonicalize().unwrap();
    let number = |i: usize, default| rest.get(i).and_then(|v| v.parse().ok()).unwrap_or(default);
    let size = (number(1, DEFAULT_SIZE.0), number(2, DEFAULT_SIZE.1));
    let mut app = App::new(root.clone(), size);
    if std::env::var("CELLIX_TIER").as_deref() != Ok("cells") {
        app.set_measure(std::rc::Rc::new(FontMeasure::new(CELL.0, CELL.1)));
    }
    if std::env::var_os("CELLIX_PANE").is_some() {
        app.in_pane = true;
        app.compact_width = 110;
    }
    if let Ok(theme) = std::env::var("CELLIX_THEME") {
        app.follow_terminal_theme(Some(theme == "light"));
    }
    // As a terminal would report it, in hex.
    if let Ok(hex) = std::env::var("CELLIX_BACKGROUND") {
        let channel = |at: usize| u8::from_str_radix(&hex[at..at + 2], 16).unwrap();
        app.match_terminal_background(Some((channel(0), channel(2), channel(4))));
    }
    match rest.first().map(String::as_str) {
        Some("review") => app.run(Command::OpenDiff),
        Some("welcome") => {}
        Some("palette") => app.run(Command::CommandPalette),
        // The Comments pane with no file in view, where it lists every file's.
        Some("comments") => app.run(Command::ShowComments),
        // Quick Open a moment after it opens, while a big tree is still being walked.
        Some("quick-open") => {
            app.finding_wait = std::time::Duration::ZERO;
            app.run(Command::QuickOpen);
            std::thread::sleep(std::time::Duration::from_millis(60));
            app.poll();
        }
        Some("plan") => {
            let path = root.join("plan.md");
            let (reply, _decision) = std::sync::mpsc::channel();
            app.serve(cellix_agent::Request::Plan { path, reply });
        }
        Some("close") => {
            app.open(&root.join("README.md")).unwrap();
            type_text(&mut app, "x");
            app.run(Command::CloseTab);
        }
        Some("comment") => {
            app.open(&root.join("README.md")).unwrap();
            app.run(Command::AddComment);
            type_text(&mut app, "Tighten this");
        }
        Some(view) if view.starts_with("find:") => {
            app.open(&root.join("README.md")).unwrap();
            app.run(Command::FindInFile);
            type_text(&mut app, &view["find:".len()..]);
        }
        Some(view) if view.starts_with("select:") => select(&mut app, &root, &view[7..]),
        Some(view) if view.starts_with("ask:") => {
            select(&mut app, &root, &view[4..]);
            let ask = find(&app, "Ask Agent").expect("the Ask Agent button");
            click(&mut app, (ask.0 + 1, ask.1));
        }
        Some(view) if view.starts_with("across:") => {
            let (rest, notches) = view[7..].rsplit_once(':').expect("<file>:<word>:<notches>");
            let (file, word) = rest.split_once(':').expect("<file>:<word>:<notches>");
            app.open(&root.join(file)).unwrap();
            // Proportional text isn't in the cells to be found, so a row can be named.
            let (x, y) = match word.parse() {
                Ok(row) => (size.0 * 2 / 3, row),
                Err(_) => find(&app, word).expect("the word on screen"),
            };
            for _ in 0..notches.parse().expect("a number of notches") {
                app.handle(Event::Mouse(MouseEvent {
                    kind: MouseKind::ScrollRight,
                    x,
                    y,
                    fine_x: f32::from(x),
                    fine_y: f32::from(y),
                    mods: Mods::default(),
                }));
            }
        }
        Some(view) if view.starts_with("pane:") => {
            let (file, columns) = view[5..].rsplit_once(':').expect("<file>:<columns>");
            app.open(&root.join(file)).unwrap();
            // Two tabs, so a narrow pane has more than it has room for.
            app.run(Command::ShowComments);
            app.run(Command::ListAgents);
            app.set_pane_width(columns.parse().expect("a number of columns"));
        }
        Some(view) if view.starts_with("tabs:") => {
            let mut parts = view[5..].rsplitn(3, ':');
            let front = parts.next().expect("<file>:<columns>:<front>");
            let columns = parts.next().expect("<file>:<columns>:<front>");
            let file = parts.next().expect("<file>:<columns>:<front>");
            app.open(&root.join(file)).unwrap();
            // Three tabs, with the named one in front, so the others have to give way.
            let order = match front {
                "comments" => [
                    Command::NewTerminal,
                    Command::ListAgents,
                    Command::ShowComments,
                ],
                "agents" => [
                    Command::NewTerminal,
                    Command::ShowComments,
                    Command::ListAgents,
                ],
                _ => [
                    Command::ShowComments,
                    Command::ListAgents,
                    Command::NewTerminal,
                ],
            };
            for command in order {
                app.run(command);
            }
            app.set_pane_width(columns.parse().expect("a number of columns"));
        }
        Some(view) if view.starts_with("search:") => {
            app.run(Command::FindInProject);
            type_text(&mut app, &view["search:".len()..]);
            std::thread::sleep(std::time::Duration::from_millis(800));
            app.poll();
        }
        Some(file) => app.open(&root.join(file)).unwrap(),
        None => {}
    }
    if let Ok(at) = std::env::var("CELLIX_HOVER") {
        let (x, y) = at.split_once(',').expect("<column>,<row>");
        let (x, y): (u16, u16) = (x.parse().unwrap(), y.parse().unwrap());
        app.handle(Event::Mouse(MouseEvent {
            kind: MouseKind::Move,
            x,
            y,
            fine_x: x as f32 + 0.5,
            fine_y: y as f32 + 0.5,
            mods: Mods::default(),
        }));
    }
    let mut grid = Grid::new(size.0, size.1, app.background());
    app.draw(&mut grid);

    let mut rasterizer = SoftwareRasterizer::new(CELL.0, CELL.1).expect("no fonts on this host");
    let mut rgb = Vec::new();
    for y in 0..grid.height() {
        rgb.extend(rasterizer.row(&grid, y).rgb);
    }
    let width = size.0 as u32 * CELL.0 as u32;
    let height = size.1 as u32 * CELL.1 as u32;
    let mut encoder = png::Encoder::new(BufWriter::new(File::create(out).unwrap()), width, height);
    encoder.set_color(png::ColorType::Rgb);
    let mut writer = encoder.write_header().unwrap();
    writer.write_image_data(&rgb).unwrap();
}
