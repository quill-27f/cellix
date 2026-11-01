//! Renders the real app through the real rasterizer to a PNG, with no terminal or window.
//!
//!   snapshot <workspace> <out.png> [view] [columns] [rows]
//!
//! `view` is `review`, or a file to open. Agents use this to see the interface.

use std::fs::File;
use std::io::BufWriter;
use std::path::PathBuf;

use cellix::App;
use cellix_input::{Command, Event, Key, KeyEvent};
use cellix_render::{FontMeasure, Grid, Rasterizer, SoftwareRasterizer};

const DEFAULT_SIZE: (u16, u16) = (120, 36);
// The cell size of a Retina terminal, so the picture matches what people see.
const CELL: (usize, usize) = (17, 37);

fn type_text(app: &mut App, text: &str) {
    for c in text.chars() {
        app.handle(Event::Key(KeyEvent::plain(Key::Char(c))));
    }
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
    app.set_measure(std::rc::Rc::new(FontMeasure::new(CELL.0, CELL.1)));
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
        Some(view) if view.starts_with("search:") => {
            app.run(Command::FindInProject);
            type_text(&mut app, &view["search:".len()..]);
            std::thread::sleep(std::time::Duration::from_millis(800));
            app.poll();
        }
        Some(file) => app.open(&root.join(file)).unwrap(),
        None => {}
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
