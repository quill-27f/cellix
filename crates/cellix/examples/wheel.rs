//! Times the wheel over the Comments pane, so what a long list costs is measured: a notch
//! and the frame after it, and a trackpad's flood of notches taken before one frame.
//! Usage: wheel <workspace> <file|-> [notches] [cols] [rows] [cell width] [cell height]
//! `harness/many_comments.py` makes a workspace with as many comments as asked for.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use cellix::App;
use cellix_input::{Command, Event, Mods, MouseEvent, MouseKind};
use cellix_render::{FontMeasure, Grid, SoftwareRasterizer};
use cellix_term::{Backend, DEFAULT_QUOTA, PixelBackend, Placement};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let root = PathBuf::from(&args[0]).canonicalize().unwrap();
    let number =
        |i: usize, default: usize| args.get(i).and_then(|v| v.parse().ok()).unwrap_or(default);
    let notches = number(2, 200);
    let size = (number(3, 160) as u16, number(4, 46) as u16);
    let cell = (number(5, 16), number(6, 36));
    let mut app = App::new(root.clone(), size);
    app.set_measure(std::rc::Rc::new(FontMeasure::new(cell.0, cell.1)));
    if args[1] != "-" {
        app.open(&root.join(&args[1])).unwrap();
    }
    app.run(Command::ShowComments);
    let raster = SoftwareRasterizer::new(cell.0, cell.1).unwrap();
    let mut backend = PixelBackend::new(raster, Placement::Placeholder, DEFAULT_QUOTA);
    let mut frame = |app: &App| {
        let mut sink = Vec::new();
        let start = Instant::now();
        let mut grid = Grid::new(size.0, size.1, app.background());
        app.draw(&mut grid);
        let drawn = start.elapsed();
        backend.present(&grid, &mut sink).unwrap();
        (drawn, start.elapsed() - drawn)
    };
    let ms = |time: Duration| time.as_secs_f64() * 1000.0;
    let (draw, present) = frame(&app);
    println!(
        "first frame: draw {:.1} ms, present {:.1} ms",
        ms(draw),
        ms(present)
    );
    let wheel = |kind| {
        Event::Mouse(MouseEvent {
            kind,
            x: size.0 - 12,
            y: size.1 / 2,
            fine_x: f32::from(size.0 - 12) + 0.5,
            fine_y: f32::from(size.1 / 2) + 0.5,
            mods: Mods::default(),
        })
    };
    for (name, kind) in [("down", MouseKind::ScrollDown), ("up", MouseKind::ScrollUp)] {
        let (mut handled, mut drawn, mut shown) = (Vec::new(), Vec::new(), Vec::new());
        for _ in 0..notches {
            let start = Instant::now();
            app.handle(wheel(kind));
            handled.push(start.elapsed());
            let (draw, present) = frame(&app);
            drawn.push(draw);
            shown.push(present);
        }
        let stats = |times: &[Duration]| {
            let total: Duration = times.iter().sum();
            let most = times.iter().max().copied().unwrap_or_default();
            format!(
                "mean {:.1} ms, most {:.1} ms",
                ms(total) / times.len() as f64,
                ms(most)
            )
        };
        println!("{notches} notches {name}, a frame after each:");
        println!("  handle  {}", stats(&handled));
        println!("  draw    {}", stats(&drawn));
        println!("  present {}", stats(&shown));
    }
    // A trackpad's flood: every notch waiting is handled before the next frame.
    for (name, kind) in [("down", MouseKind::ScrollDown), ("up", MouseKind::ScrollUp)] {
        let start = Instant::now();
        for _ in 0..notches {
            app.handle(wheel(kind));
        }
        let handled = start.elapsed();
        let (draw, present) = frame(&app);
        println!(
            "{notches} notches {name} at once: handle {:.0} ms, then draw {:.1} ms, present {:.1} ms",
            ms(handled),
            ms(draw),
            ms(present)
        );
    }
}
