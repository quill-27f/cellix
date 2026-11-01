//! Times what a keystroke costs, so slowness gets measured rather than guessed at.
//! Usage: frames <workspace> <file> [cols] [rows] [cell width] [cell height]

use std::io;
use std::path::PathBuf;
use std::time::Instant;

use cellix::App;
use cellix_input::{Command, Event, Key, KeyEvent};
use cellix_render::{FontMeasure, Grid, SoftwareRasterizer};
use cellix_term::{Backend, DEFAULT_QUOTA, PixelBackend, Placement};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let root = PathBuf::from(&args[0]).canonicalize().unwrap();
    let number =
        |i: usize, default: usize| args.get(i).and_then(|v| v.parse().ok()).unwrap_or(default);
    let size = (number(2, 160) as u16, number(3, 46) as u16);
    let cell = (number(4, 16), number(5, 36));
    let mut app = App::new(root.clone(), size);
    app.set_measure(std::rc::Rc::new(FontMeasure::new(cell.0, cell.1)));
    app.open(&root.join(&args[1])).unwrap();
    let raster = SoftwareRasterizer::new(cell.0, cell.1).unwrap();
    let mut backend = PixelBackend::new(raster, Placement::Placeholder, DEFAULT_QUOTA);
    let mut frame = |app: &App, label: &str| {
        let mut sink = Vec::new();
        let start = Instant::now();
        let mut grid = Grid::new(size.0, size.1, app.background());
        app.draw(&mut grid);
        let drawn = start.elapsed();
        backend.present(&grid, &mut sink).unwrap();
        println!(
            "{label:12} draw {:6.1}ms  present {:6.1}ms  {:7} KB",
            drawn.as_secs_f64() * 1000.0,
            (start.elapsed() - drawn).as_secs_f64() * 1000.0,
            sink.len() / 1024,
        );
        let _ = io::Write::write_all(&mut io::sink(), &sink);
    };
    frame(&app, "first");
    app.run(Command::CommandPalette);
    frame(&app, "F1");
    for c in "open claude".chars() {
        app.handle(Event::Key(KeyEvent::plain(Key::Char(c))));
        frame(&app, &format!("type {c:?}"));
    }
    app.handle(Event::Key(KeyEvent::plain(Key::Esc)));
    frame(&app, "close");
    for c in "resource".chars() {
        let start = Instant::now();
        app.handle(Event::Key(KeyEvent::plain(Key::Char(c))));
        let handled = start.elapsed().as_secs_f64() * 1000.0;
        frame(&app, &format!("edit {c:?} {handled:.1}ms"));
    }
    for c in "resource".chars() {
        let start = Instant::now();
        app.handle(Event::Key(KeyEvent::plain(Key::Char(c))));
        let handled = start.elapsed().as_secs_f64() * 1000.0;
        frame(&app, &format!("edit {c:?} {handled:.1}ms"));
    }
}
