use std::io::{self, Write};

use cellix_render::{Grid, Style};

use crate::Backend;

/// Styled text cells. Works everywhere, so every feature must work here.
#[derive(Default)]
pub struct CellBackend {
    shown: Option<Grid>,
}

fn sgr(style: Style, out: &mut Vec<u8>) {
    let (f, b) = (style.fg, style.bg);
    let _ = write!(
        out,
        "\x1b[0;38;2;{};{};{};48;2;{};{};{}",
        f.0, f.1, f.2, b.0, b.1, b.2
    );
    if style.bold {
        out.extend_from_slice(b";1");
    }
    if style.italic {
        out.extend_from_slice(b";3");
    }
    if style.underline {
        out.extend_from_slice(b";4");
    }
    out.push(b'm');
}

impl Backend for CellBackend {
    fn present(&mut self, grid: &Grid, out: &mut dyn Write) -> io::Result<()> {
        let mut bytes = Vec::new();
        // Synchronized output stops terminals showing half a frame.
        bytes.extend_from_slice(b"\x1b[?2026h");
        let runs = match &self.shown {
            Some(shown) => grid.diff(shown),
            None => grid.diff(&Grid::new(0, 0, Style::default())),
        };
        let mut last = None;
        for run in runs {
            let _ = write!(bytes, "\x1b[{};{}H", run.y + 1, run.x + 1);
            for x in run.x..run.x + run.len {
                let cell = grid.cell(x, run.y);
                if last != Some(cell.style) {
                    sgr(cell.style, &mut bytes);
                    last = Some(cell.style);
                }
                bytes.extend_from_slice(cell.symbol.as_bytes());
            }
        }
        match grid.cursor {
            Some((x, y)) => {
                let _ = write!(bytes, "\x1b[{};{}H\x1b[?25h", y + 1, x + 1);
            }
            None => bytes.extend_from_slice(b"\x1b[?25l"),
        }
        bytes.extend_from_slice(b"\x1b[?2026l");
        out.write_all(&bytes)?;
        out.flush()?;
        self.shown = Some(grid.clone());
        Ok(())
    }

    fn invalidate(&mut self) {
        self.shown = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn second_present_sends_only_the_change() {
        let mut backend = CellBackend::default();
        let mut grid = Grid::new(20, 5, Style::default());
        let mut first = Vec::new();
        backend.present(&grid, &mut first).unwrap();
        grid.put_str(3, 2, "x", Style::default(), 20);
        let mut second = Vec::new();
        backend.present(&grid, &mut second).unwrap();
        assert!(second.len() < 80, "sent {} bytes", second.len());
        assert!(second.windows(6).any(|w| w == b"\x1b[3;4H"));
    }
}
