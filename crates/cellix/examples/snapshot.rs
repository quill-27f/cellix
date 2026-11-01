//! Renders the real app through the real rasterizer to a PNG, with no terminal or window.
//!
//!   snapshot <workspace> <out.png> [view] [columns] [rows]
//!
//! `view` is `review`, or a file to open. Agents use this to see the interface.

use std::fs::File;
use std::io::BufWriter;
use std::path::PathBuf;

use cellix::App;
use cellix_input::Command;
use cellix_render::{Grid, Rasterizer, SoftwareRasterizer};

const DEFAULT_SIZE: (u16, u16) = (120, 36);
// The cell size of a Retina terminal, so the picture matches what people see.
const CELL: (usize, usize) = (17, 37);

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [root, out, rest @ ..] = args.as_slice() else {
        panic!("usage: snapshot <workspace> <out.png> [review|file]");
    };
    let root = PathBuf::from(root).canonicalize().unwrap();
    let number = |i: usize, default| rest.get(i).and_then(|v| v.parse().ok()).unwrap_or(default);
    let size = (number(1, DEFAULT_SIZE.0), number(2, DEFAULT_SIZE.1));
    let mut app = App::new(root.clone(), size);
    match rest.first().map(String::as_str) {
        Some("review") => app.run(Command::OpenDiff),
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
