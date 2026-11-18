//! What a turning spinner costs on the wire, on each tier, so its frame rate is chosen from
//! numbers rather than guessed at.
//! Usage: spin [columns] [rows]
//!
//! The real app at the size asked, with the side pane open, is drawn through the real
//! rasterizer, with a spinner in three places it will stand: the pane's tab row, a row of the
//! pane's list, and the status bar. Each place is turned for two turns on its own, and what
//! each frame sends is counted: the direct tier with rows of Cellix's own, tmux's placeholders,
//! cells, and the pane inside Claude Code, by value (a PNG a row) and by file (a frame's RGB).

use std::path::PathBuf;

use cellix::App;
use cellix_input::Command;
use cellix_render::{
    CellMeasure, FontMeasure, Grid, Rasterizer, SoftwareRasterizer, Style, spinner,
};
use cellix_term::{Backend, CellBackend, DEFAULT_QUOTA, OwnRows, PixelBackend, Placement};

/// The user's terminal: a cell of 17 by 37 px, and Airy rows of about 50 px over it.
const CELL: (usize, usize) = (17, 37);
const ROW: usize = 50;
/// The pane's cell where Claude Code says none, as `pane.rs` takes it.
const PANE_CELL: (usize, usize) = (16, 36);

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let number =
        |i: usize, default: u16| args.get(i).and_then(|v| v.parse().ok()).unwrap_or(default);
    let size = (number(0, 150), number(1, 44));
    let dir = std::env::temp_dir().join(format!("cellix-spin-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("src/main.rs"),
        "fn main() {\n    println!(\"hi\");\n}\n".repeat(30),
    )
    .unwrap();
    let root = PathBuf::from(&dir).canonicalize().unwrap();
    let app = |measure: std::rc::Rc<dyn cellix_render::Measure>| {
        let mut app = App::new(root.clone(), size);
        app.set_measure(measure);
        app.open(&root.join("src/main.rs")).unwrap();
        app.run(Command::ShowAgents);
        app
    };
    let pixels = app(std::rc::Rc::new(
        FontMeasure::new(CELL.0, ROW).with_type_row(CELL.1),
    ));
    let cells = app(std::rc::Rc::new(CellMeasure));
    let accent = pixels.theme().accent;
    // Where each spinner stands: after the pane's tab's name, on the list's first row, and in
    // the status bar after its first words.
    let places = |app: &App| -> Vec<(&'static str, f32, u16)> {
        let mut grid = Grid::new(size.0, size.1, Style::default());
        app.draw(&mut grid);
        let after = |y: u16, word: &str| {
            let text = grid.row_text(y);
            let at = text
                .find(word)
                .map_or(2, |i| text[..i].chars().count() + word.len() + 1);
            at as f32
        };
        vec![
            ("pane's tab", after(0, "Agents"), 0),
            (
                "list's row",
                after(4, "HOW IT WORKS").min(f32::from(size.0) - 4.0),
                4,
            ),
            ("status bar", 30.0, size.1 - 1),
        ]
    };
    let frame = |app: &App, (x, y): (f32, u16), step: u8| {
        let mut grid = Grid::new(size.0, size.1, app.background());
        app.draw(&mut grid);
        grid.turn = Some(step);
        grid.spinner(x, y, 0.65, accent, app.background());
        grid
    };
    let steps = 2 * spinner::STEPS;
    let per_second = 1000 / spinner::STEP.as_millis() as usize;
    for (place, x, y) in places(&pixels) {
        println!("{place}, {} frames a second:", per_second);
        let mut tiers: Vec<(&str, Box<dyn Backend>)> = vec![
            (
                "direct, own rows",
                Box::new(
                    PixelBackend::new(
                        SoftwareRasterizer::new(CELL.0, ROW)
                            .unwrap()
                            .with_type_row(CELL.1),
                        Placement::Direct,
                        DEFAULT_QUOTA,
                    )
                    .with_own_rows(OwnRows {
                        row: ROW,
                        line: CELL.1,
                    }),
                ),
            ),
            (
                "tmux placeholders",
                Box::new(PixelBackend::new(
                    SoftwareRasterizer::new(CELL.0, CELL.1).unwrap(),
                    Placement::Placeholder,
                    DEFAULT_QUOTA,
                )),
            ),
        ];
        for (tier, backend) in &mut tiers {
            let mut sink = Vec::new();
            backend
                .present(&frame(&pixels, (x, y), 0), &mut sink)
                .unwrap();
            let (first, then) = turn(steps, |step| {
                let mut sink = Vec::new();
                backend
                    .present(&frame(&pixels, (x, y), step), &mut sink)
                    .unwrap();
                sink.len()
            });
            report(tier, first, then, per_second);
        }
        let cells_place = places(&cells).into_iter().find(|p| p.0 == place).unwrap();
        let mut backend = CellBackend::default();
        let mut sink = Vec::new();
        backend
            .present(&frame(&cells, (cells_place.1, cells_place.2), 0), &mut sink)
            .unwrap();
        let (first, then) = turn(steps, |step| {
            let mut sink = Vec::new();
            let grid = frame(&cells, (cells_place.1, cells_place.2), step);
            backend.present(&grid, &mut sink).unwrap();
            sink.len()
        });
        report("cells", first, then, per_second);
        // Inside Claude Code the mod holds only the pictures on screen, so each step is the
        // row's picture again; by file, the whole frame is written for the terminal to read.
        let mut raster = SoftwareRasterizer::new(PANE_CELL.0, PANE_CELL.1).unwrap();
        let pane = app(std::rc::Rc::new(FontMeasure::new(PANE_CELL.0, PANE_CELL.1)));
        let (first, then) = turn(steps, |step| {
            let grid = frame(&pane, (x, y), step);
            let image = raster.row(&grid, y);
            let png = cellix::pane::row_png(&image.rgb, image.width, image.height);
            // base64, and the names of every row, as the line to the mod has them.
            png.len() * 4 / 3 + usize::from(size.1) * 17
        });
        report("Claude Code, by value", first, then, per_second);
        let whole = usize::from(size.0) * PANE_CELL.0 * usize::from(size.1) * PANE_CELL.1 * 3;
        report("Claude Code, by file", whole, whole, per_second);
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// Bytes over the first turn, a frame on average, and over the second.
fn turn(steps: u8, mut frame: impl FnMut(u8) -> usize) -> (usize, usize) {
    let sizes: Vec<usize> = (1..=steps).map(|n| frame(n % spinner::STEPS)).collect();
    let half = sizes.len() / 2;
    let first = sizes[..half].iter().sum::<usize>() / half;
    let then = sizes[half..].iter().sum::<usize>() / (sizes.len() - half);
    (first, then)
}

fn report(tier: &str, first: usize, then: usize, per_second: usize) {
    println!(
        "  {tier:24} first turn {:>9} B a frame, then {:>9} B a frame, {:>9.1} KB/s",
        first,
        then,
        (then * per_second) as f64 / 1024.0
    );
}
