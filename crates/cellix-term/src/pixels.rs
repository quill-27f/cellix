use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::io::{self, Write};
use std::time::Duration;

use base64::Engine;
use cellix_render::{Grid, Image, Rasterizer, Rect, UNIT};
use flate2::{Compression, write::ZlibEncoder};

use crate::Backend;

/// Decoded image memory Cellix keeps in the terminal. Kitty's default limit is 320 MB and
/// other terminals' are lower, so this stays well inside them.
pub const DEFAULT_QUOTA: usize = 64 * 1024 * 1024;

const CHUNK: usize = 4096;
const PLACEHOLDER: char = '\u{10EEEE}';
/// Row and column zero in the placeholder encoding.
const DIACRITIC_ZERO: char = '\u{305}';

/// Above the row tiles, so a scrolling region covers them and nothing else has to change.
const SCROLL_LAYER: u32 = 1;
/// Placement ids for a scrolling region, clear of the row placements.
const FIRST_STRIP: usize = 100_000;

/// How a region animates. Direct placement only, since only that can sit between cells.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Smooth {
    pub frames: u32,
    pub pause: Duration,
}

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
    smooth: Option<Smooth>,
    /// Whether region placements are waiting to be removed by the next present.
    scrolling: bool,
}

/// What a row looks like, as a key: rows with the same key draw the same pixels.
pub fn row_key(grid: &Grid, y: u16) -> u64 {
    let mut hasher = DefaultHasher::new();
    grid.row(y).hash(&mut hasher);
    grid.cursor.filter(|c| c.1 == y).hash(&mut hasher);
    // Only on its own row: hashed everywhere, moving the caret resent the whole screen.
    let (top, bottom) = (y as i32 * UNIT, (y as i32 + 1) * UNIT);
    grid.caret
        .filter(|c| c.y < bottom && c.y + c.h > top)
        .hash(&mut hasher);
    grid.paint_on_row(y).for_each(|p| p.hash(&mut hasher));
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
            smooth: None,
            scrolling: false,
        }
    }

    pub fn with_smooth(mut self, smooth: Smooth) -> Self {
        self.smooth = (self.placement == Placement::Direct).then_some(smooth);
        self
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
    fn tile(&mut self, out: &mut Vec<u8>, key: u64, grid: &Grid, y: u16) -> u32 {
        if let Some(tile) = self.tiles.get_mut(&key) {
            tile.last_used = self.frame;
            return tile.id;
        }
        let id = self.new_id();
        let image = self.rasterizer.row(grid, y);
        let cols = grid.width();
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

    /// The tile for part of a row, shared with any row that looks the same.
    fn region_tile(&mut self, out: &mut Vec<u8>, grid: &Grid, y: u16, area: Rect) -> (u32, Image) {
        let image = self.rasterizer.row(grid, y);
        let cell = image.width / grid.width().max(1) as usize;
        let (left, width) = (area.x as usize * cell, area.w as usize * cell);
        let mut rgb = Vec::with_capacity(width * image.height * 3);
        for line in image.rgb.chunks_exact(image.width * 3) {
            rgb.extend_from_slice(&line[left * 3..(left + width) * 3]);
        }
        let crop = Image {
            width,
            height: image.height,
            rgb,
        };
        let mut hasher = DefaultHasher::new();
        crop.rgb.hash(&mut hasher);
        let key = hasher.finish();
        if let Some(tile) = self.tiles.get_mut(&key) {
            tile.last_used = self.frame;
            return (tile.id, crop);
        }
        let id = self.new_id();
        self.transmit(out, id, &crop);
        self.used += crop.rgb.len();
        let bytes = crop.rgb.len();
        let last_used = self.frame;
        self.tiles.insert(
            key,
            Tile {
                id,
                bytes,
                last_used,
            },
        );
        (id, crop)
    }

    fn new_id(&mut self) -> u32 {
        let id = self.next_id;
        self.next_id = (self.next_id + 1) & 0xff_ffff;
        if self.next_id == 0 {
            self.next_id = 1;
        }
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
            let key = row_key(grid, y);
            let id = self.tile(&mut bytes, key, grid, y);
            on_screen.insert(id);
            if self.shown[y as usize] != Some(id) {
                // A placement belongs to its image, so a new image at this row doesn't
                // replace the old one. Left alone, stale rows pile up as you scroll.
                if let (Placement::Direct, Some(old)) = (self.placement, self.shown[y as usize]) {
                    self.apc(&mut bytes, &format!("a=d,d=i,i={old},p={},q=2", y + 1), b"");
                }
                self.place(&mut bytes, id, y, grid.width());
                self.shown[y as usize] = Some(id);
            }
        }
        if std::mem::take(&mut self.scrolling) {
            self.apc(&mut bytes, &format!("a=d,d=z,z={SCROLL_LAYER},q=2"), b"");
        }
        self.evict(&mut bytes, &on_screen);
        bytes.extend_from_slice(b"\x1b[?2026l");
        out.write_all(&bytes)?;
        out.flush()
    }

    fn scroll(
        &mut self,
        from: &Grid,
        to: &Grid,
        area: Rect,
        rows: i32,
        out: &mut dyn Write,
    ) -> io::Result<()> {
        let height = area.h as i32;
        let same_size = (from.width(), from.height()) == (to.width(), to.height());
        let Some(smooth) = self.smooth.filter(|_| {
            same_size && rows != 0 && rows.abs() < height && self.size == (to.width(), to.height())
        }) else {
            return Ok(());
        };
        let moved = rows.unsigned_abs() as u16;
        // The rows of both grids in one strip, top to bottom. Most appear in both.
        let lines = area.y..area.bottom();
        let strip: Vec<(&Grid, u16)> = if rows > 0 {
            let fresh = lines.clone().skip(area.h as usize - moved as usize);
            lines
                .map(|y| (from, y))
                .chain(fresh.map(|y| (to, y)))
                .collect()
        } else {
            let fresh = lines.clone().take(moved as usize);
            fresh
                .map(|y| (to, y))
                .chain(lines.map(|y| (from, y)))
                .collect()
        };
        let mut bytes = Vec::new();
        let tiles: Vec<(u32, Image)> = strip
            .iter()
            .map(|(grid, y)| self.region_tile(&mut bytes, grid, *y, area))
            .collect();
        let Some(cell_height) = tiles.first().map(|(_, image)| image.height as i32) else {
            return Ok(());
        };
        let travel = rows.abs() * cell_height;
        let window = height * cell_height;
        let mut placed = HashSet::new();
        self.scrolling = true;
        for frame in 1..=smooth.frames {
            // At a steady pace, so one notch running into the next doesn't pulse.
            let progress = frame as f32 / smooth.frames as f32;
            let moved_by = (travel as f32 * progress).round() as i32;
            let offset = if rows > 0 {
                moved_by
            } else {
                travel - moved_by
            };
            bytes.extend_from_slice(b"\x1b[?2026h");
            for (index, (id, image)) in tiles.iter().enumerate() {
                let top = index as i32 * cell_height - offset;
                let number = FIRST_STRIP + index;
                let (clip_top, clip_end) = ((-top).max(0), cell_height.min(window - top));
                if clip_top >= clip_end {
                    if placed.remove(&index) {
                        self.apc(&mut bytes, &format!("a=d,d=i,i={id},p={number},q=2"), b"");
                    }
                    continue;
                }
                let pixel = top.max(0);
                let row = area.y as i32 + pixel / cell_height;
                let _ = write!(bytes, "\x1b[{};{}H", row + 1, area.x + 1);
                let control = format!(
                    "a=p,i={id},p={number},x=0,y={clip_top},w={},h={},Y={},z={SCROLL_LAYER},C=1,q=2",
                    image.width,
                    clip_end - clip_top,
                    pixel % cell_height,
                );
                self.apc(&mut bytes, &control, b"");
                placed.insert(index);
            }
            bytes.extend_from_slice(b"\x1b[?2026l");
            out.write_all(&bytes)?;
            out.flush()?;
            bytes.clear();
            if frame < smooth.frames {
                std::thread::sleep(smooth.pause);
            }
        }
        Ok(())
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
        fn row(&mut self, grid: &Grid, _y: u16) -> Image {
            Image {
                width: grid.width() as usize,
                height: 1,
                rgb: vec![9; grid.width() as usize * 3],
            }
        }
    }

    /// Eight by sixteen, with a colour taken from the row's first letter.
    struct Sized;

    impl Rasterizer for Sized {
        fn row(&mut self, grid: &Grid, y: u16) -> Image {
            let tone = grid.row(y)[9].symbol.bytes().next().unwrap_or(0);
            let (width, height) = (grid.width() as usize * 8, 16);
            Image {
                width,
                height,
                rgb: vec![tone; width * height * 3],
            }
        }
    }

    /// A sidebar in the first eight columns that stays still, and a pane that moves.
    fn pane(top: usize) -> Grid {
        let mut grid = Grid::new(20, 8, Style::default());
        for y in 0..8 {
            grid.put_str(0, y, "side", Style::default(), 8);
            let letter = char::from(b'a' + (top + y as usize) as u8);
            grid.put_str(9, y, &letter.to_string(), Style::default(), 20);
        }
        grid
    }

    const PANE: Rect = Rect {
        x: 8,
        y: 1,
        w: 12,
        h: 6,
    };

    fn smooth(frames: u32) -> PixelBackend<Sized> {
        let pause = Duration::ZERO;
        PixelBackend::new(Sized, Placement::Direct, DEFAULT_QUOTA)
            .with_smooth(Smooth { frames, pause })
    }

    fn scroll(b: &mut PixelBackend<Sized>, from: &Grid, to: &Grid, rows: i32) -> String {
        let mut out = Vec::new();
        b.scroll(from, to, PANE, rows, &mut out).unwrap();
        String::from_utf8(out).unwrap()
    }

    fn present_sized(b: &mut PixelBackend<Sized>, grid: &Grid) -> String {
        let mut out = Vec::new();
        b.present(grid, &mut out).unwrap();
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn regions_scroll_by_pixels() {
        let mut b = smooth(4);
        let (from, to) = (pane(0), pane(3));
        present_sized(&mut b, &from);
        let moving = scroll(&mut b, &from, &to, 3);
        assert_eq!(count(&moving, "\x1b[?2026h"), 4, "every frame is atomic");
        assert!(count(&moving, ",Y=") > 0);
        let offsets: HashSet<String> = moving
            .split(",Y=")
            .skip(1)
            .map(|rest| rest.split(',').next().unwrap().to_string())
            .collect();
        assert!(
            offsets.iter().any(|y| y != "0"),
            "it sits between cells: {offsets:?}"
        );
        assert!(moving.contains(",z=1,"), "above the row tiles");
        assert!(
            !moving.contains("\x1b[1;9H"),
            "nothing lands on the row above the pane"
        );
        assert!(
            !moving.contains(";1H"),
            "the sidebar's column is never touched"
        );
        let settled = present_sized(&mut b, &to);
        assert_eq!(
            count(&settled, "a=d,d=z,z=1"),
            1,
            "the layer goes with the final page"
        );
        assert_eq!(count(&present_sized(&mut b, &to), "a=d,d=z"), 0);
    }

    #[test]
    fn a_region_clips_at_its_edges() {
        let mut b = smooth(2);
        let (from, to) = (pane(0), pane(1));
        present_sized(&mut b, &from);
        let moving = scroll(&mut b, &from, &to, 1);
        // Halfway through, the top row has lost its top half.
        assert!(moving.contains(",y=8,w=96,h=8,"), "{moving:?}");
    }

    #[test]
    fn scrolling_back_needs_nothing_new() {
        let mut b = smooth(3);
        let (from, to) = (pane(0), pane(2));
        present_sized(&mut b, &from);
        scroll(&mut b, &from, &to, 2);
        present_sized(&mut b, &to);
        let back = scroll(&mut b, &to, &from, -2);
        assert_eq!(
            count(&back, "a=t,"),
            0,
            "the tiles are in the terminal already"
        );
    }

    #[test]
    fn a_jump_past_the_region_or_a_tier_without_pixels_doesnt_animate() {
        let mut b = smooth(3);
        let (from, to) = (pane(0), pane(9));
        present_sized(&mut b, &from);
        assert_eq!(scroll(&mut b, &from, &to, 9), "");
        let tmux =
            PixelBackend::new(Sized, Placement::Placeholder, DEFAULT_QUOTA).with_smooth(Smooth {
                frames: 3,
                pause: Duration::ZERO,
            });
        assert_eq!(tmux.smooth, None);
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

    /// What a Kitty-protocol terminal keeps on screen: placements are keyed by image and
    /// placement id together, so a new image at a row doesn't replace the old one.
    #[derive(Default)]
    struct Screen {
        placements: HashMap<(u32, u32), u16>,
    }

    fn field(control: &str, key: &str) -> Option<String> {
        control
            .split(',')
            .find_map(|kv| kv.strip_prefix(&format!("{key}=")).map(str::to_string))
    }

    impl Screen {
        fn apply(&mut self, output: &str) {
            let mut rest = output;
            let mut row = 0;
            while let Some(at) = rest.find('\x1b') {
                rest = &rest[at..];
                if let Some(after) = rest.strip_prefix("\x1b_G") {
                    let end = after.find(['\x1b', ';']).unwrap_or(after.len());
                    let control = &after[..end];
                    let id = field(control, "i").and_then(|v| v.parse().ok());
                    let placement = field(control, "p").and_then(|v| v.parse().ok());
                    match (
                        field(control, "a").as_deref(),
                        field(control, "d").as_deref(),
                    ) {
                        (Some("p"), _) => {
                            self.placements
                                .insert((id.unwrap(), placement.unwrap()), row);
                        }
                        (Some("d"), Some("A")) => self.placements.clear(),
                        (Some("d"), Some("I" | "i")) => self.placements.retain(|k, _| {
                            k.0 != id.unwrap() || placement.is_some_and(|p| k.1 != p)
                        }),
                        _ => {}
                    }
                    rest = &rest[3..];
                } else {
                    let body = rest.strip_prefix("\x1b[").unwrap_or("");
                    if let Some(semi) = body.find(";1H")
                        && let Ok(y) = body[..semi].parse::<u16>()
                    {
                        row = y - 1;
                    }
                    rest = &rest[1..];
                }
            }
        }

        fn rows(&self) -> Vec<usize> {
            let mut counts = vec![0; 5];
            for row in self.placements.values() {
                counts[*row as usize] += 1;
            }
            counts
        }
    }

    #[test]
    fn scroll_leaves_exactly_one_tile_on_each_row() {
        let mut b = PixelBackend::new(Flat, Placement::Direct, DEFAULT_QUOTA);
        let mut screen = Screen::default();
        for top in [0, 1, 2, 3, 1, 0, 7, 2] {
            screen.apply(&present(&mut b, &page(top)));
            assert_eq!(screen.rows(), vec![1; 5], "after scrolling to {top}");
        }
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
    fn a_change_in_paint_makes_a_new_tile() {
        let mut b = PixelBackend::new(Flat, Placement::Direct, DEFAULT_QUOTA);
        let mut grid = page(0);
        present(&mut b, &grid);
        let bounds = cellix_render::Bounds::cells(0.0, 2.0, 4.0, 1.0);
        grid.panel(bounds, 0, Some(cellix_render::Rgb(1, 2, 3)), None);
        assert_eq!(
            count(&present(&mut b, &grid), "a=t,"),
            1,
            "only row 2 changed"
        );
    }

    #[test]
    fn moving_the_caret_sends_only_its_rows() {
        let mut b = PixelBackend::new(Flat, Placement::Direct, DEFAULT_QUOTA);
        let mut grid = page(0);
        grid.set_caret(1.0, 1, 0.6);
        present(&mut b, &grid);
        let mut moved = page(0);
        moved.set_caret(2.0, 1, 0.6);
        assert_eq!(count(&present(&mut b, &moved), "a=t,"), 1, "only row 1");
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
