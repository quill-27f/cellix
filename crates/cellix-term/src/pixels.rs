use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::io::{self, Write};

use base64::Engine;
use cellix_render::{Cell, Grid, Image, Rasterizer};
use flate2::{Compression, write::ZlibEncoder};

use crate::Backend;

/// Decoded image memory Cellix keeps in the terminal. Kitty's default limit is 320 MB and
/// other terminals' are lower, so this stays well inside them.
pub const DEFAULT_QUOTA: usize = 64 * 1024 * 1024;

const CHUNK: usize = 4096;
const PLACEHOLDER: char = '\u{10EEEE}';
/// Row and column zero in the placeholder encoding.
const DIACRITIC_ZERO: char = '\u{305}';

struct Tile {
    id: u32,
    bytes: usize,
    last_used: u64,
}

/// How tiles reach the screen. See the tier table in `docs/DESIGN.md`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Placement {
    /// `a=p` at the cursor. Fails under tmux, which can't track it.
    Direct,
    /// Unicode placeholder cells, which tmux stores like text. Every graphics command
    /// goes through tmux's DCS passthrough.
    Placeholder,
}

/// Renders each row once as a tile, keeps it in the terminal's image store, and moves it
/// by reference, so only new content crosses the wire.
pub struct PixelBackend<R> {
    rasterizer: R,
    placement: Placement,
    tiles: HashMap<u64, Tile>,
    shown: Vec<Option<u32>>,
    size: (u16, u16),
    next_id: u32,
    used: usize,
    quota: usize,
    frame: u64,
}

fn tile_key(cells: &[Cell], cursor: Option<u16>) -> u64 {
    let mut hasher = DefaultHasher::new();
    cells.hash(&mut hasher);
    cursor.hash(&mut hasher);
    hasher.finish()
}

fn zlib(data: &[u8]) -> Vec<u8> {
    let mut z = ZlibEncoder::new(Vec::with_capacity(data.len() / 8), Compression::fast());
    z.write_all(data).expect("writing to a Vec");
    z.finish().expect("writing to a Vec")
}

impl<R: Rasterizer> PixelBackend<R> {
    pub fn new(rasterizer: R, placement: Placement, quota: usize) -> Self {
        Self {
            rasterizer,
            placement,
            tiles: HashMap::new(),
            shown: Vec::new(),
            size: (0, 0),
            next_id: 1,
            used: 0,
            quota,
            frame: 0,
        }
    }

    /// Writes one graphics command, wrapped for tmux in placeholder mode.
    fn apc(&self, out: &mut Vec<u8>, control: &str, payload: &[u8]) {
        let wrap = self.placement == Placement::Placeholder;
        out.extend_from_slice(if wrap {
            b"\x1bPtmux;\x1b\x1b_G"
        } else {
            b"\x1b_G"
        });
        out.extend_from_slice(control.as_bytes());
        if !payload.is_empty() {
            out.push(b';');
            out.extend_from_slice(payload);
        }
        out.extend_from_slice(if wrap { b"\x1b\x1b\\\x1b\\" } else { b"\x1b\\" });
    }

    fn transmit(&self, out: &mut Vec<u8>, id: u32, image: &Image) {
        let b64 = base64::engine::general_purpose::STANDARD.encode(zlib(&image.rgb));
        let chunks: Vec<&[u8]> = b64.as_bytes().chunks(CHUNK).collect();
        for (i, chunk) in chunks.iter().enumerate() {
            let more = u8::from(i + 1 < chunks.len());
            let control = if i == 0 {
                format!(
                    "a=t,f=24,o=z,t=d,q=2,i={id},s={},v={},m={more}",
                    image.width, image.height
                )
            } else {
                format!("m={more},q=2")
            };
            self.apc(out, &control, chunk);
        }
    }

    fn place(&self, out: &mut Vec<u8>, id: u32, y: u16, cols: u16) {
        let _ = write!(out, "\x1b[{};1H", y + 1);
        match self.placement {
            Placement::Direct => {
                let control = format!("a=p,i={id},p={},c={cols},r=1,C=1,q=2", y + 1);
                self.apc(out, &control, b"");
            }
            Placement::Placeholder => {
                // The foreground colour carries the image id.
                let (r, g, b) = ((id >> 16) & 255, (id >> 8) & 255, id & 255);
                let _ = write!(out, "\x1b[38;2;{r};{g};{b}m");
                let _ = write!(out, "{PLACEHOLDER}{DIACRITIC_ZERO}{DIACRITIC_ZERO}");
                for _ in 1..cols {
                    let _ = write!(out, "{PLACEHOLDER}");
                }
                out.extend_from_slice(b"\x1b[39m");
            }
        }
    }

    /// Returns the tile for a row, rasterizing and sending it only if it's new.
    fn tile(
        &mut self,
        out: &mut Vec<u8>,
        key: u64,
        cells: &[Cell],
        cursor: Option<u16>,
        cols: u16,
    ) -> u32 {
        if let Some(tile) = self.tiles.get_mut(&key) {
            tile.last_used = self.frame;
            return tile.id;
        }
        let id = self.next_id;
        self.next_id = (self.next_id + 1) & 0xff_ffff;
        if self.next_id == 0 {
            self.next_id = 1;
        }
        let image = self.rasterizer.row(cells, cursor);
        self.transmit(out, id, &image);
        if self.placement == Placement::Placeholder {
            self.apc(out, &format!("a=p,U=1,i={id},c={cols},r=1,q=2"), b"");
        }
        self.used += image.rgb.len();
        self.tiles.insert(
            key,
            Tile {
                id,
                bytes: image.rgb.len(),
                last_used: self.frame,
            },
        );
        id
    }

    /// Frees the least recently used tiles that aren't on screen.
    fn evict(&mut self, out: &mut Vec<u8>, on_screen: &HashSet<u32>) {
        while self.used > self.quota {
            let oldest = self
                .tiles
                .iter()
                .filter(|(_, t)| !on_screen.contains(&t.id))
                .min_by_key(|(_, t)| t.last_used)
                .map(|(key, _)| *key);
            let Some(key) = oldest else { return };
            let tile = self.tiles.remove(&key).expect("key came from the map");
            self.used -= tile.bytes;
            self.apc(out, &format!("a=d,d=I,i={},q=2", tile.id), b"");
        }
    }

    pub fn tile_count(&self) -> usize {
        self.tiles.len()
    }

    pub fn used_bytes(&self) -> usize {
        self.used
    }
}

impl<R: Rasterizer> Backend for PixelBackend<R> {
    fn present(&mut self, grid: &Grid, out: &mut dyn Write) -> io::Result<()> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"\x1b[?2026h\x1b[?25l");
        let size = (grid.width(), grid.height());
        if size != self.size {
            // Tiles are as wide as the grid, so a new width makes all of them useless.
            self.apc(&mut bytes, "a=d,d=A,q=2", b"");
            bytes.extend_from_slice(b"\x1b[2J");
            self.tiles.clear();
            self.used = 0;
            self.shown = vec![None; grid.height() as usize];
            self.size = size;
        }
        self.frame += 1;
        let mut on_screen = HashSet::new();
        for y in 0..grid.height() {
            let cursor = grid.cursor.filter(|c| c.1 == y).map(|c| c.0);
            let cells = grid.row(y);
            let key = tile_key(cells, cursor);
            let id = self.tile(&mut bytes, key, cells, cursor, grid.width());
            on_screen.insert(id);
            if self.shown[y as usize] != Some(id) {
                self.place(&mut bytes, id, y, grid.width());
                self.shown[y as usize] = Some(id);
            }
        }
        self.evict(&mut bytes, &on_screen);
        bytes.extend_from_slice(b"\x1b[?2026l");
        out.write_all(&bytes)?;
        out.flush()
    }

    fn invalidate(&mut self) {
        self.size = (0, 0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cellix_render::Style;

    /// Solid tiles, so these tests measure the protocol and not fonts.
    struct Flat;

    impl Rasterizer for Flat {
        fn row(&mut self, cells: &[Cell], _cursor: Option<u16>) -> Image {
            Image {
                width: cells.len(),
                height: 1,
                rgb: vec![9; cells.len() * 3],
            }
        }
    }

    fn page(top: usize) -> Grid {
        let mut grid = Grid::new(20, 5, Style::default());
        for y in 0..5 {
            let text = format!("line {}", top + y as usize);
            grid.put_str(0, y, &text, Style::default(), 20);
        }
        grid
    }

    fn present(backend: &mut PixelBackend<Flat>, grid: &Grid) -> String {
        let mut out = Vec::new();
        backend.present(grid, &mut out).unwrap();
        String::from_utf8(out).unwrap()
    }

    fn count(haystack: &str, needle: &str) -> usize {
        haystack.matches(needle).count()
    }

    #[test]
    fn scroll_reuses_tiles_without_retransmit() {
        let mut b = PixelBackend::new(Flat, Placement::Direct, DEFAULT_QUOTA);
        assert_eq!(count(&present(&mut b, &page(0)), "a=t,"), 5);
        let scrolled = present(&mut b, &page(1));
        assert_eq!(count(&scrolled, "a=t,"), 1, "only the new line is sent");
        assert_eq!(count(&scrolled, "a=p,"), 5, "every row moves by reference");
        let back = present(&mut b, &page(0));
        assert_eq!(count(&back, "a=t,"), 0, "scrolling back sends nothing new");
    }

    #[test]
    fn an_unchanged_frame_sends_no_graphics() {
        let mut b = PixelBackend::new(Flat, Placement::Direct, DEFAULT_QUOTA);
        present(&mut b, &page(0));
        let again = present(&mut b, &page(0));
        assert_eq!(count(&again, "\x1b_G"), 0);
    }

    #[test]
    fn identical_rows_share_a_tile() {
        let mut b = PixelBackend::new(Flat, Placement::Direct, DEFAULT_QUOTA);
        let blank = Grid::new(20, 5, Style::default());
        let out = present(&mut b, &blank);
        assert_eq!(count(&out, "a=t,"), 1);
        assert_eq!(count(&out, "a=p,"), 5);
    }

    #[test]
    fn placeholder_cells_are_passthrough_wrapped() {
        let mut b = PixelBackend::new(Flat, Placement::Placeholder, DEFAULT_QUOTA);
        let out = present(&mut b, &page(0));
        assert_eq!(
            count(&out, "\x1bPtmux;\x1b\x1b_G"),
            count(&out, "\x1b\x1b\\\x1b\\")
        );
        assert_eq!(count(&out.replace("\x1bPtmux;\x1b\x1b_G", ""), "\x1b_G"), 0);
        assert_eq!(count(&out, "a=p,U=1,"), 5);
        assert_eq!(count(&out, &PLACEHOLDER.to_string()), 5 * 20);
        assert!(out.contains("\u{10EEEE}\u{305}\u{305}"));
    }

    #[test]
    fn placeholder_encodes_the_id_in_the_foreground() {
        let mut b = PixelBackend::new(Flat, Placement::Placeholder, DEFAULT_QUOTA);
        b.next_id = 0x01_02_03;
        let out = present(&mut b, &page(0));
        assert!(out.contains("\x1b[38;2;1;2;3m\u{10EEEE}"));
    }

    #[test]
    fn tile_cache_respects_quota() {
        // Each tile is 60 bytes and a page is five tiles.
        let mut b = PixelBackend::new(Flat, Placement::Direct, 60 * 7);
        for top in 0..40 {
            present(&mut b, &page(top * 5));
        }
        assert!(b.used_bytes() <= 60 * 7, "used {}", b.used_bytes());
        assert!(b.tile_count() >= 5, "the visible page must survive");
    }

    #[test]
    fn eviction_never_removes_a_visible_tile() {
        let mut b = PixelBackend::new(Flat, Placement::Direct, 60);
        let out = present(&mut b, &page(0));
        assert_eq!(count(&out, "a=d,d=I"), 0, "all five tiles are on screen");
        assert_eq!(b.tile_count(), 5);
    }

    #[test]
    fn evicted_tiles_are_freed_in_the_terminal() {
        let mut b = PixelBackend::new(Flat, Placement::Direct, 60 * 5);
        present(&mut b, &page(0));
        let out = present(&mut b, &page(5));
        assert_eq!(count(&out, "a=d,d=I"), 5);
    }

    #[test]
    fn resize_clears_the_terminal_store() {
        let mut b = PixelBackend::new(Flat, Placement::Direct, DEFAULT_QUOTA);
        present(&mut b, &page(0));
        let wider = Grid::new(30, 5, Style::default());
        let out = present(&mut b, &wider);
        assert!(out.contains("a=d,d=A"));
        assert_eq!(b.tile_count(), 1);
    }
}
