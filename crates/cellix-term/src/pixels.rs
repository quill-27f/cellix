use std::collections::btree_map::Entry;
use std::collections::hash_map::DefaultHasher;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::io::{self, Write};
use std::time::{Duration, Instant};

use base64::Engine;
use cellix_render::{Grid, Image, Rasterizer, Rect, Rgb, UNIT};
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
/// Rows of the page a glide can tell apart, which wrap round far out of sight.
const STRIP_ROWS: i32 = 1 << 20;
/// Pixels from the end at which a glide counts as there.
const ARRIVED: f32 = 0.5;
/// Screens of rows kept around the view, so turning round needs nothing sent.
const KEPT_SCREENS: i32 = 3;

/// How a region animates. Direct placement only, since only that can sit between cells.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Smooth {
    /// How often it moves.
    pub frame: Duration,
    /// How quickly it closes on where it's going: about two thirds of the way in this time.
    pub ease: Duration,
}

/// How far tiles reach past the text area. Ghostty doesn't clip images, so a tile can cover
/// its right and bottom padding, which no cell can. Left and top stay out of reach, and the
/// frame there is the terminal's own colour.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Bleed {
    /// Columns past the right edge.
    pub cols: u16,
    /// Rows past the bottom edge.
    pub rows: u16,
    /// Pixels the last row's content moves down, to centre it in the band it now fills.
    pub drop: usize,
}

struct Tile {
    id: u32,
    bytes: usize,
    last_used: u64,
}

/// A scroll under way. Its rows are tiles the terminal already has, shown through a crop and
/// a pixel offset, so moving costs a line of text per row and nothing has to be drawn. The
/// screen underneath is already at the end, and this stands over it until it gets there.
struct Glide {
    area: Rect,
    /// The rows of the page it's seen, counted from the top of the view when it started.
    rows: BTreeMap<i32, u32>,
    /// Where the view's heading, in rows from the start.
    target: i32,
    /// Where it is, in pixels from the start.
    at: f32,
    last: Instant,
    /// The rows with a placement to take away.
    placed: HashSet<i32>,
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
    bleed: Bleed,
    glide: Option<Glide>,
    /// A cell's size in pixels, from the tiles.
    cell: Option<(i32, i32)>,
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
    // Paint can stand in several rows, and each shows its own slice. Without the row, the
    // empty rows above and below a tall heading are the same tile, and one is blank.
    let mut paint = grid.paint_on_row(y).peekable();
    if paint.peek().is_some() {
        y.hash(&mut hasher);
    }
    paint.for_each(|p| p.hash(&mut hasher));
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
            bleed: Bleed::default(),
            glide: None,
            cell: None,
        }
    }

    /// Only direct placement can leave the text area: placeholders are cells.
    pub fn with_bleed(mut self, bleed: Bleed) -> Self {
        if self.placement == Placement::Direct {
            self.bleed = bleed;
        }
        self
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

    fn place(&self, out: &mut Vec<u8>, id: u32, y: u16, cols: u16, rows: u16) {
        let _ = write!(out, "\x1b[{};1H", y + 1);
        match self.placement {
            Placement::Direct => {
                let cols = cols + self.bleed.cols;
                let control = format!("a=p,i={id},p={},c={cols},r={rows},C=1,q=2", y + 1);
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

    /// A row's image, grown to cover the padding beyond it in the colours at the edge.
    fn bled(&self, image: Image, grid: &Grid, y: u16) -> Image {
        let Bleed { cols, rows, drop } = self.bleed;
        let cells = grid.row(y);
        let (Some(last), true) = (cells.last(), cols > 0 || rows > 0) else {
            return image;
        };
        let cell = image.width / cells.len();
        let width = image.width + cols as usize * cell;
        let paint = |line: &mut Vec<u8>, from: usize, to: usize, color: Rgb| {
            for _ in from..to {
                line.extend_from_slice(&[color.0, color.1, color.2]);
            }
        };
        let mut rgb = Vec::new();
        if y + 1 != grid.height() {
            for line in image.rgb.chunks_exact(image.width * 3) {
                rgb.extend_from_slice(line);
                paint(&mut rgb, image.width, width, last.style.bg);
            }
            return Image {
                width,
                rgb,
                ..image
            };
        }
        // The bar's own colours run on below it, column by column.
        let height = image.height * (1 + rows as usize);
        let drop = drop.min(height - image.height);
        let mut plain = Vec::with_capacity(width * 3);
        for c in cells {
            paint(&mut plain, 0, cell, c.style.bg);
        }
        paint(&mut plain, image.width, width, last.style.bg);
        for _ in 0..drop {
            rgb.extend_from_slice(&plain);
        }
        for line in image.rgb.chunks_exact(image.width * 3) {
            rgb.extend_from_slice(line);
            paint(&mut rgb, image.width, width, last.style.bg);
        }
        for _ in drop + image.height..height {
            rgb.extend_from_slice(&plain);
        }
        Image { width, height, rgb }
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
        self.cell = Some((
            (image.width / cols.max(1) as usize) as i32,
            image.height as i32,
        ));
        let image = self.bled(image, grid, y);
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

    /// A glide from what's on screen now, whose rows are in the terminal already.
    fn start_glide(&self, area: Rect) -> Option<Glide> {
        let shown = (area.y..area.bottom()).enumerate().map(|(index, y)| {
            let id = (*self.shown.get(y as usize)?)?;
            Some((index as i32, id))
        });
        Some(Glide {
            area,
            rows: shown.collect::<Option<_>>()?,
            target: 0,
            at: 0.0,
            last: Instant::now(),
            placed: HashSet::new(),
        })
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
            self.glide = None;
            self.used = 0;
            self.shown = vec![None; grid.height() as usize];
            self.size = size;
        }
        self.frame += 1;
        let mut on_screen = HashSet::new();
        for y in 0..grid.height() {
            let bottom = y + 1 == grid.height() && self.bleed.rows > 0;
            // The last row's tile is taller than the same row anywhere else.
            let key = row_key(grid, y).rotate_left(u32::from(bottom));
            let id = self.tile(&mut bytes, key, grid, y);
            on_screen.insert(id);
            if self.shown[y as usize] != Some(id) {
                // A placement belongs to its image, so a new image at this row doesn't
                // replace the old one. Left alone, stale rows pile up as you scroll.
                if let (Placement::Direct, Some(old)) = (self.placement, self.shown[y as usize]) {
                    self.apc(&mut bytes, &format!("a=d,d=i,i={old},p={},q=2", y + 1), b"");
                }
                let rows = 1 + if bottom { self.bleed.rows } else { 0 };
                self.place(&mut bytes, id, y, grid.width(), rows);
                self.shown[y as usize] = Some(id);
            }
        }
        if std::mem::take(&mut self.scrolling) {
            self.apc(&mut bytes, &format!("a=d,d=z,z={SCROLL_LAYER},q=2"), b"");
        }
        on_screen.extend(self.glide.iter().flat_map(|g| g.rows.values().copied()));
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
        let same_size = (from.width(), from.height()) == (to.width(), to.height());
        let can_glide = self.smooth.is_some()
            && self.cell.is_some()
            && same_size
            && self.size == (to.width(), to.height())
            && rows != 0
            && rows.abs() < area.h as i32;
        if !can_glide || self.glide.as_ref().is_some_and(|g| g.area != area) {
            self.settle();
            return Ok(());
        }
        let Some(mut glide) = self.glide.take().or_else(|| self.start_glide(area)) else {
            return Ok(());
        };
        let mut bytes = Vec::new();
        glide.target += rows;
        for (index, y) in (area.y..area.bottom()).enumerate() {
            let row = glide.target + index as i32;
            if let Entry::Vacant(place) = glide.rows.entry(row) {
                place.insert(self.tile(&mut bytes, row_key(to, y), to, y));
            }
        }
        self.glide = Some(glide);
        out.write_all(&bytes)?;
        out.flush()
    }

    fn gliding(&self) -> Option<Duration> {
        self.glide.as_ref()?;
        self.smooth.map(|smooth| smooth.frame)
    }

    fn glide(&mut self, now: Instant, out: &mut dyn Write) -> io::Result<()> {
        let (Some(mut glide), Some(smooth), Some((cell_w, cell_h))) =
            (self.glide.take(), self.smooth, self.cell)
        else {
            return Ok(());
        };
        let target = (glide.target * cell_h) as f32;
        let window = glide.area.h as i32 * cell_h;
        // A long way behind, as after a hard flick, it would only look like lag.
        glide.at = glide
            .at
            .clamp(target - window as f32, target + window as f32);
        let passed = now.duration_since(glide.last).as_secs_f32();
        glide.last = now;
        let left = (-passed / smooth.ease.as_secs_f32()).exp();
        glide.at = target + (glide.at - target) * left;
        if (glide.at - target).abs() < ARRIVED {
            glide.at = target;
        }
        let at = glide.at.round() as i32;
        let across = i32::from(glide.area.x) * cell_w;
        let wide = i32::from(glide.area.w) * cell_w;
        let mut bytes = b"\x1b[?2026h".to_vec();
        for (&row, &id) in &glide.rows {
            let top = row * cell_h - at;
            let (clip_top, clip_end) = ((-top).max(0), cell_h.min(window - top));
            let number = FIRST_STRIP + row.rem_euclid(STRIP_ROWS) as usize;
            if clip_top >= clip_end {
                if glide.placed.remove(&row) {
                    self.apc(&mut bytes, &format!("a=d,d=i,i={id},p={number},q=2"), b"");
                }
                continue;
            }
            let pixel = top.max(0);
            let y = i32::from(glide.area.y) + pixel / cell_h;
            let _ = write!(bytes, "\x1b[{};{}H", y + 1, glide.area.x + 1);
            let control = format!(
                "a=p,i={id},p={number},x={across},y={clip_top},w={wide},h={},Y={},z={SCROLL_LAYER},C=1,q=2",
                clip_end - clip_top,
                pixel % cell_h,
            );
            self.apc(&mut bytes, &control, b"");
            glide.placed.insert(row);
        }
        bytes.extend_from_slice(b"\x1b[?2026l");
        out.write_all(&bytes)?;
        out.flush()?;
        // What's far behind has no way back into view before it's drawn again.
        let reach = glide.area.h as i32 * KEPT_SCREENS;
        let view = at / cell_h;
        glide
            .rows
            .retain(|row, _| (row - view).abs() <= reach || glide.placed.contains(row));
        if glide.at == target {
            self.scrolling = true;
        } else {
            self.glide = Some(glide);
        }
        Ok(())
    }

    fn settle(&mut self) {
        self.scrolling |= self.glide.take().is_some();
    }

    fn invalidate(&mut self) {
        self.size = (0, 0);
        self.glide = None;
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

    const STEP: Duration = Duration::from_millis(8);
    const SMOOTH: Smooth = Smooth {
        frame: STEP,
        ease: Duration::from_millis(40),
    };

    fn smooth() -> PixelBackend<Sized> {
        PixelBackend::new(Sized, Placement::Direct, DEFAULT_QUOTA).with_smooth(SMOOTH)
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
    fn the_window_fills_to_its_edges() {
        let mut b = PixelBackend::new(Sized, Placement::Direct, DEFAULT_QUOTA).with_bleed(Bleed {
            cols: 2,
            rows: 1,
            drop: 3,
        });
        let grid = Grid::new(12, 2, Style::default());
        let output = present_sized(&mut b, &grid);
        let widths: Vec<String> = output
            .split("\x1b_G")
            .filter_map(|c| field(c, "s"))
            .collect();
        let tall: Vec<String> = output
            .split("\x1b_G")
            .filter_map(|c| field(c, "v"))
            .collect();
        assert!(
            widths.iter().all(|w| w == "112"),
            "two more cells: {widths:?}"
        );
        assert!(
            tall.contains(&"32".to_string()),
            "the last row takes two: {tall:?}"
        );
        assert!(tall.contains(&"16".to_string()));
        let placed: Vec<&str> = output
            .split("\x1b_G")
            .filter(|c| c.starts_with("a=p"))
            .collect();
        assert!(placed.iter().all(|c| c.contains(",c=14,")), "{placed:?}");
        assert!(placed[1].contains(",r=2,"), "{placed:?}");
        let mut placeholders = PixelBackend::new(Sized, Placement::Placeholder, DEFAULT_QUOTA)
            .with_bleed(Bleed {
                cols: 2,
                rows: 1,
                drop: 3,
            });
        assert_eq!(
            placeholders.bleed,
            Bleed::default(),
            "placeholders are cells"
        );
        let _ = present_sized(&mut placeholders, &grid);
    }

    #[test]
    fn a_tall_heading_keeps_its_lower_half() {
        let mut grid = Grid::new(20, 8, Style::default());
        let big = cellix_render::Font {
            size: 150,
            ..cellix_render::Font::default()
        };
        grid.text_run((1.0, 3.5), 20.0, "Workflow", (Rgb(9, 9, 9), false), big);
        assert_ne!(
            row_key(&grid, 2),
            row_key(&grid, 4),
            "the row above and the row below show different slices"
        );
        let blank = Grid::new(20, 8, Style::default());
        assert_eq!(
            row_key(&blank, 2),
            row_key(&blank, 4),
            "empty rows still share a tile, which scrolling depends on"
        );
    }

    /// Runs `count` frames of a glide, and gives back what they sent.
    fn frames(b: &mut PixelBackend<Sized>, count: u32) -> String {
        let mut out = Vec::new();
        for _ in 0..count {
            let now = b.glide.as_ref().map_or_else(Instant::now, |g| g.last) + STEP;
            b.glide(now, &mut out).unwrap();
        }
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn regions_scroll_by_pixels() {
        let mut b = smooth();
        let (from, to) = (pane(0), pane(3));
        present_sized(&mut b, &from);
        scroll(&mut b, &from, &to, 3);
        let moving = frames(&mut b, 4);
        assert_eq!(count(&moving, "\x1b[?2026h"), 4, "every frame is atomic");
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
        frames(&mut b, 200);
        assert_eq!(b.gliding(), None, "it gets there");
        let settled = present_sized(&mut b, &to);
        assert_eq!(
            count(&settled, "a=d,d=z,z=1"),
            1,
            "the layer goes with the final page"
        );
        assert_eq!(count(&present_sized(&mut b, &to), "a=d,d=z"), 0);
    }

    #[test]
    fn a_glide_moves_quickly_at_first_and_settles_gently() {
        let mut b = smooth();
        let (from, to) = (pane(0), pane(3));
        present_sized(&mut b, &from);
        scroll(&mut b, &from, &to, 3);
        let mut places = vec![0.0];
        while b.gliding().is_some() {
            frames(&mut b, 1);
            places.push(b.glide.as_ref().map_or(48.0, |g| g.at));
            assert!(places.len() < 80, "it takes too long: {places:?}");
        }
        let steps: Vec<f32> = places.windows(2).map(|p| p[1] - p[0]).collect();
        assert!(
            steps.iter().all(|step| *step >= 0.0),
            "never back: {steps:?}"
        );
        assert!(steps[0] > steps[steps.len() / 2], "it eases: {steps:?}");
    }

    #[test]
    fn a_new_notch_carries_on_from_where_it_is() {
        let mut b = smooth();
        let (first, second, third) = (pane(0), pane(2), pane(4));
        present_sized(&mut b, &first);
        scroll(&mut b, &first, &second, 2);
        frames(&mut b, 3);
        let at = b.glide.as_ref().unwrap().at;
        scroll(&mut b, &second, &third, 2);
        let glide = b.glide.as_ref().unwrap();
        assert_eq!((glide.at, glide.target), (at, 4), "no restart, no jump");
    }

    #[test]
    fn the_wrong_way_turns_round_smoothly() {
        let mut b = smooth();
        let (first, second) = (pane(0), pane(2));
        present_sized(&mut b, &first);
        scroll(&mut b, &first, &second, 2);
        frames(&mut b, 3);
        let at = b.glide.as_ref().unwrap().at;
        scroll(&mut b, &second, &first, -2);
        frames(&mut b, 1);
        let now = b.glide.as_ref().unwrap().at;
        assert!(now < at && now > 0.0, "{at} then {now}");
    }

    #[test]
    fn scrolling_only_sends_what_the_terminal_hasnt_seen() {
        let mut b = smooth();
        let (from, to) = (pane(0), pane(1));
        present_sized(&mut b, &from);
        let sent = count(&scroll(&mut b, &from, &to, 1), "a=t,");
        assert!(sent <= 1, "at most the one new row, not {sent}");
        assert_eq!(
            count(&frames(&mut b, 5), "a=t,"),
            0,
            "moving costs no pictures"
        );
    }

    #[test]
    fn a_region_clips_at_its_edges() {
        let mut b = smooth();
        let (from, to) = (pane(0), pane(1));
        present_sized(&mut b, &from);
        scroll(&mut b, &from, &to, 1);
        let moving = frames(&mut b, 3);
        assert!(
            moving.contains(",x=64,y="),
            "only the pane's columns: {moving:?}"
        );
        let clipped = moving
            .split(",y=")
            .skip(1)
            .any(|rest| !rest.starts_with("0,"));
        assert!(clipped, "the top row loses its top part: {moving:?}");
    }

    #[test]
    fn scrolling_back_needs_nothing_new() {
        let mut b = smooth();
        let (from, to) = (pane(0), pane(2));
        present_sized(&mut b, &from);
        scroll(&mut b, &from, &to, 2);
        frames(&mut b, 200);
        present_sized(&mut b, &to);
        let back = scroll(&mut b, &to, &from, -2);
        assert_eq!(
            count(&back, "a=t,"),
            0,
            "the tiles are in the terminal already"
        );
    }

    #[test]
    fn typing_ends_a_glide_at_once() {
        let mut b = smooth();
        let (from, to) = (pane(0), pane(3));
        present_sized(&mut b, &from);
        scroll(&mut b, &from, &to, 3);
        frames(&mut b, 1);
        b.settle();
        assert_eq!(b.gliding(), None);
        assert_eq!(count(&present_sized(&mut b, &to), "a=d,d=z,z=1"), 1);
    }

    #[test]
    fn tiles_a_glide_is_showing_arent_thrown_away() {
        let mut b = smooth();
        let (from, to) = (pane(0), pane(3));
        present_sized(&mut b, &from);
        scroll(&mut b, &from, &to, 3);
        b.quota = 0;
        let held: Vec<u32> = b.glide.as_ref().unwrap().rows.values().copied().collect();
        let sent = present_sized(&mut b, &to);
        for id in held {
            assert!(
                !sent.contains(&format!("a=d,d=I,i={id},")),
                "{id} is on show"
            );
        }
    }

    #[test]
    fn a_jump_past_the_region_or_a_tier_without_pixels_doesnt_animate() {
        let mut b = smooth();
        let (from, to) = (pane(0), pane(9));
        present_sized(&mut b, &from);
        assert_eq!(scroll(&mut b, &from, &to, 9), "");
        assert_eq!(b.gliding(), None);
        let tmux =
            PixelBackend::new(Sized, Placement::Placeholder, DEFAULT_QUOTA).with_smooth(SMOOTH);
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
