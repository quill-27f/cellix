use std::io::{self, Write};

use cellix_render::{Grid, Piece, Rect, Style};

use crate::Backend;

/// Styled text cells. Works everywhere, so every feature must work here.
#[derive(Default)]
pub struct CellBackend {
    shown: Option<Grid>,
    /// A scroll opened a synchronized update and moved the cursor, for this present to end.
    scrolled: bool,
    /// What it has done since the counters last asked: rows written count as rows.
    work: crate::Work,
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
        // Synchronized output stops terminals showing half a frame. One that doesn't know it
        // shows each run as it comes, so the cursor is hidden while they're written, or it
        // runs over the screen with them, and shown again where it belongs at the end.
        bytes.extend_from_slice(b"\x1b[?2026h\x1b[?25l");
        let runs = match &self.shown {
            Some(shown) => grid.diff(shown),
            None => grid.diff(&Grid::new(0, 0, Style::default())),
        };
        // Nothing different, and the cursor where it was: nothing to send.
        let cursor_kept = self.shown.as_ref().is_some_and(|s| s.cursor == grid.cursor);
        if runs.is_empty() && cursor_kept && !std::mem::take(&mut self.scrolled) {
            return Ok(());
        }
        self.scrolled = false;
        let mut last = None;
        let mut rows = runs.iter().map(|run| run.y).collect::<Vec<_>>();
        rows.dedup();
        self.work.rows += rows.len() as u64;
        for run in runs {
            let _ = write!(bytes, "\x1b[{};{}H", run.y + 1, run.x + 1);
            for x in run.x..run.x + run.len {
                let cell = grid.cell(x, run.y);
                if last != Some(cell.style) {
                    sgr(cell.style, &mut bytes);
                    last = Some(cell.style);
                }
                match Piece::decode(&cell.symbol) {
                    Some(piece) => bytes.extend_from_slice(piece.fallback().as_bytes()),
                    None => bytes.extend_from_slice(cell.symbol.as_bytes()),
                }
            }
        }
        if let Some((x, y)) = grid.cursor {
            let _ = write!(bytes, "\x1b[{};{}H\x1b[?25h", y + 1, x + 1);
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

    fn take_work(&mut self) -> crate::Work {
        std::mem::take(&mut self.work)
    }

    /// Has the terminal shift the band of rows the scrolled area spans, as vim and Claude
    /// Code do, so only the lines that come into view and the columns that stay put (rail,
    /// sidebar, minimap) are sent after it. Rewriting every row instead is what shows as a
    /// scan over a slow link, in tmux especially. A whole-width region works in every
    /// terminal, which left and right margins don't. The present that follows closes the
    /// synchronized update this opens, so the shift and the repair land as one frame.
    fn scroll(
        &mut self,
        _from: &Grid,
        to: &Grid,
        area: Rect,
        rows: i32,
        out: &mut dyn Write,
    ) -> io::Result<()> {
        let Some(shown) = &mut self.shown else {
            return Ok(());
        };
        let fits = (shown.width(), shown.height()) == (to.width(), to.height());
        if !fits || rows == 0 || rows.unsigned_abs() >= u32::from(area.h) || area.h == 0 {
            return Ok(());
        }
        let (top, bottom) = (area.y, area.bottom() - 1);
        let mut bytes = b"\x1b[?2026h\x1b[?25l".to_vec();
        let _ = write!(bytes, "\x1b[{};{}r", top + 1, bottom + 1);
        let n = rows.unsigned_abs();
        let _ = if rows > 0 {
            write!(bytes, "\x1b[{n}S")
        } else {
            write!(bytes, "\x1b[{n}T")
        };
        bytes.extend_from_slice(b"\x1b[r");
        out.write_all(&bytes)?;
        shown.scroll_rows(top, bottom, rows);
        self.scrolled = true;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A grid with a sidebar column that stays and lines of text that move.
    fn page(first: usize) -> Grid {
        let mut grid = Grid::new(40, 12, Style::default());
        for y in 0..12u16 {
            grid.put_str(0, y, "|side|", Style::default(), 40);
            let letter = (b'a' + ((first + y as usize) % 26) as u8) as char;
            let line = format!("{} {}", first + y as usize, letter.to_string().repeat(28));
            grid.put_str(8, y, &line, Style::default(), 40);
        }
        grid
    }

    #[test]
    fn a_scroll_shifts_the_band_and_sends_only_what_it_must() {
        let mut backend = CellBackend::default();
        let mut sink = Vec::new();
        let before = page(0);
        backend.present(&before, &mut sink).unwrap();
        let after = page(1);
        let mut full = CellBackend::default();
        let mut redrawn = Vec::new();
        full.present(&before, &mut Vec::new()).unwrap();
        full.present(&after, &mut redrawn).unwrap();
        let mut moved = Vec::new();
        let area = Rect::new(8, 0, 32, 12);
        backend
            .scroll(&before, &after, area, 1, &mut moved)
            .unwrap();
        backend.present(&after, &mut moved).unwrap();
        let text = String::from_utf8_lossy(&moved);
        assert!(
            text.contains("\x1b[1;12r\x1b[1S\x1b[r"),
            "the terminal shifts the band"
        );
        assert!(
            text.contains("12 mmmm"),
            "the line coming into view is sent"
        );
        assert!(
            moved.len() * 2 < redrawn.len(),
            "{} bytes where a redraw takes {}",
            moved.len(),
            redrawn.len()
        );
        // The sidebar the shift moved is put back.
        assert_eq!(backend.shown.as_ref().unwrap().cell(0, 11).symbol, "|");
    }

    /// Nothing different and the cursor where it was: nothing is written. A moved cursor
    /// is, and so is what follows a scroll, which leaves an update open for it to end.
    #[test]
    fn an_unchanged_frame_writes_nothing() {
        let mut backend = CellBackend::default();
        let mut grid = Grid::new(20, 5, Style::default());
        grid.cursor = Some((2, 2));
        backend.present(&grid, &mut Vec::new()).unwrap();
        let mut again = Vec::new();
        backend.present(&grid, &mut again).unwrap();
        assert!(again.is_empty(), "{:?}", String::from_utf8_lossy(&again));
        grid.cursor = Some((3, 2));
        let mut moved = Vec::new();
        backend.present(&grid, &mut moved).unwrap();
        assert!(moved.windows(6).any(|w| w == b"\x1b[3;4H"));
        let area = Rect::new(0, 0, 20, 5);
        backend
            .scroll(&grid, &grid, area, 1, &mut Vec::new())
            .unwrap();
        let mut after = Vec::new();
        backend.present(&grid, &mut after).unwrap();
        assert!(
            after.ends_with(b"\x1b[?2026l"),
            "the scroll's update is ended"
        );
    }

    /// The cursor is hidden before anything is written and shown only once the frame is
    /// whole, where it belongs: a terminal without synchronized output shows every run as it
    /// comes, and the cursor ran over the screen with them.
    #[test]
    fn the_cursor_is_hidden_while_a_frame_is_written() {
        let mut backend = CellBackend::default();
        let mut grid = Grid::new(20, 5, Style::default());
        grid.put_str(0, 0, "first", Style::default(), 20);
        grid.put_str(0, 4, "last", Style::default(), 20);
        grid.cursor = Some((2, 1));
        let mut out = Vec::new();
        backend.present(&grid, &mut out).unwrap();
        let text = String::from_utf8_lossy(&out).into_owned();
        assert!(text.starts_with("\x1b[?2026h\x1b[?25l"), "{text:?}");
        let shown = text.find("\x1b[?25h").expect("the cursor comes back");
        assert!(shown > text.find("last").unwrap(), "after the last run");
        assert!(text[shown..].starts_with("\x1b[?25h\x1b[?2026l"));
        assert!(text.contains("\x1b[2;3H\x1b[?25h"), "where it belongs");
        // Without a cursor, it stays hidden.
        grid.cursor = None;
        grid.put_str(0, 2, "more", Style::default(), 20);
        let mut out = Vec::new();
        backend.present(&grid, &mut out).unwrap();
        assert!(!String::from_utf8_lossy(&out).contains("\x1b[?25h"));
    }

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
