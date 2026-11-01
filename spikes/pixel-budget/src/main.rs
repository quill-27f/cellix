//! Measures whether a pixel-rendered editor UI is viable through the Kitty
//! graphics protocol, comparing full frames, dirty rectangles and line tiles.
//!
//!   pixel-budget measure   print raster/encode time and wire bytes
//!   pixel-budget demo      scroll line tiles in a real terminal (Kitty protocol)
//!   pixel-budget show <direct|placeholder> [--wrap]
//!                          hold a static page of tiles; --wrap uses tmux passthrough

use std::collections::HashMap;
use std::io::Write;
use std::time::{Duration, Instant};

use base64::Engine;
use cosmic_text::{Attrs, Buffer, Color, Family, FontSystem, Metrics, Shaping, SwashCache};
use flate2::{write::ZlibEncoder, Compression};

const SOURCE: &str = include_str!("main.rs");
const BG: [u8; 3] = [24, 26, 31];
const SIDEBAR: [u8; 3] = [19, 21, 25];
const KEYWORDS: &[&str] = &[
    "fn", "let", "mut", "use", "const", "for", "in", "if", "else", "struct", "impl", "match",
    "return", "pub", "self",
];

struct Painter {
    fonts: FontSystem,
    swash: SwashCache,
    metrics: Metrics,
}

/// A tightly packed RGB image, matching the protocol's `f=24`.
struct Image {
    w: usize,
    h: usize,
    rgb: Vec<u8>,
}

impl Image {
    fn new(w: usize, h: usize, bg: [u8; 3]) -> Self {
        Self { w, h, rgb: bg.iter().copied().cycle().take(w * h * 3).collect() }
    }

    fn blit(&mut self, src: &Image, x: usize, y: usize) {
        for row in 0..src.h.min(self.h.saturating_sub(y)) {
            let n = src.w.min(self.w.saturating_sub(x)) * 3;
            let to = (y + row) * self.w * 3 + x * 3;
            self.rgb[to..to + n].copy_from_slice(&src.rgb[row * src.w * 3..row * src.w * 3 + n]);
        }
    }

    fn crop(&self, y: usize, h: usize) -> Vec<u8> {
        self.rgb[y * self.w * 3..(y + h) * self.w * 3].to_vec()
    }
}

fn spans(line: &str) -> Vec<(String, Color)> {
    let text = Color::rgb(201, 209, 217);
    let trimmed = line.trim_start();
    if trimmed.starts_with("//") {
        return vec![(line.to_string(), Color::rgb(110, 118, 129))];
    }
    let mut out: Vec<(String, Color)> = Vec::new();
    let mut word = String::new();
    let mut in_string = false;
    let flush = |word: &mut String, out: &mut Vec<(String, Color)>, string: bool| {
        if word.is_empty() {
            return;
        }
        let color = if string {
            Color::rgb(165, 214, 255)
        } else if KEYWORDS.contains(&word.as_str()) {
            Color::rgb(255, 123, 114)
        } else if word.chars().next().is_some_and(char::is_uppercase) {
            Color::rgb(121, 192, 255)
        } else {
            text
        };
        out.push((std::mem::take(word), color));
    };
    for c in line.chars() {
        if c == '"' {
            flush(&mut word, &mut out, in_string);
            in_string = !in_string;
            out.push(("\"".into(), Color::rgb(165, 214, 255)));
        } else if in_string || c.is_alphanumeric() || c == '_' {
            word.push(c);
        } else {
            flush(&mut word, &mut out, false);
            out.push((c.to_string(), text));
        }
    }
    flush(&mut word, &mut out, in_string);
    out
}

impl Painter {
    fn new(cell_h: f32) -> Self {
        Self {
            fonts: FontSystem::new(),
            swash: SwashCache::new(),
            metrics: Metrics::new(cell_h * 0.68, cell_h),
        }
    }

    fn line(&mut self, text: &str, w: usize, h: usize, bg: [u8; 3]) -> Image {
        let mut image = Image::new(w, h, bg);
        let mut buffer = Buffer::new(&mut self.fonts, self.metrics);
        buffer.set_size(Some(w as f32), Some(h as f32));
        let attrs = Attrs::new().family(Family::Monospace);
        let owned = spans(text);
        let rich = owned.iter().map(|(s, c)| (s.as_str(), attrs.clone().color(*c)));
        buffer.set_rich_text(rich, &attrs, Shaping::Advanced, None);
        buffer.shape_until_scroll(&mut self.fonts, false);
        let default = Color::rgb(201, 209, 217);
        buffer.draw(&mut self.fonts, &mut self.swash, default, |x, y, gw, gh, color| {
            let a = color.a() as u32;
            if a == 0 {
                return;
            }
            let (r, g, b) = (color.r() as u32, color.g() as u32, color.b() as u32);
            for py in y..y + gh as i32 {
                for px in x..x + gw as i32 {
                    if px < 0 || py < 0 || px as usize >= w || py as usize >= h {
                        continue;
                    }
                    let i = (py as usize * w + px as usize) * 3;
                    for (c, v) in [r, g, b].into_iter().enumerate() {
                        let old = image.rgb[i + c] as u32;
                        image.rgb[i + c] = ((v * a + old * (255 - a)) / 255) as u8;
                    }
                }
            }
        });
        image
    }
}

fn zlib(data: &[u8]) -> Vec<u8> {
    let mut z = ZlibEncoder::new(Vec::with_capacity(data.len() / 8), Compression::fast());
    z.write_all(data).unwrap();
    z.finish().unwrap()
}

/// Escape-framed size of a compressed `d`-medium payload: base64 in 4096-byte chunks.
fn wire_bytes(compressed: usize) -> usize {
    let b64 = compressed.div_ceil(3) * 4;
    b64 + b64.div_ceil(4096) * 12 + 48
}

struct Scene {
    w: usize,
    h: usize,
    sidebar: usize,
    cell_h: usize,
    lines: Vec<String>,
}

impl Scene {
    fn new(w: usize, h: usize, cell_h: usize) -> Self {
        let lines: Vec<String> = SOURCE.lines().map(str::to_string).collect();
        Self { w, h, sidebar: w / 6, cell_h, lines }
    }

    fn rows(&self) -> usize {
        self.h / self.cell_h
    }

    fn text(&self, row: usize) -> &str {
        &self.lines[row % self.lines.len()]
    }

    fn frame(&self, p: &mut Painter, top: usize, edit: Option<(usize, &str)>) -> Image {
        let mut frame = Image::new(self.w, self.h, BG);
        for row in 0..self.rows() {
            let label = format!("  file_{:02}.rs", row % 40);
            let side = p.line(&label, self.sidebar, self.cell_h, SIDEBAR);
            frame.blit(&side, 0, row * self.cell_h);
            let text = match edit {
                Some((r, t)) if r == row => t.to_string(),
                _ => self.text(top + row).to_string(),
            };
            let line = p.line(&text, self.w - self.sidebar, self.cell_h, BG);
            frame.blit(&line, self.sidebar, row * self.cell_h);
        }
        frame
    }
}

fn mb(bytes: usize) -> f64 {
    bytes as f64 / 1_000_000.0
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

fn measure(w: usize, h: usize, cell_h: usize) {
    let scene = Scene::new(w, h, cell_h);
    let mut p = Painter::new(cell_h as f32);
    println!("\n== {w}x{h}, {} rows, CPU only ==", scene.rows());

    p.line(scene.text(0), scene.w, cell_h, BG); // warm font discovery out of the timing
    let t = Instant::now();
    let a = scene.frame(&mut p, 0, None);
    let raster_full = t.elapsed();
    let b = scene.frame(&mut p, 1, None);

    let t = Instant::now();
    let full = zlib(&a.rgb);
    let encode_full = t.elapsed();
    let scroll_full = zlib(&b.rgb);
    println!(
        "raw {:.1} MB | raster {:.1} ms | zlib {:.1} ms",
        mb(a.rgb.len()),
        ms(raster_full),
        ms(encode_full)
    );
    println!("full frame            {:>8.3} MB wire", mb(wire_bytes(full.len())));
    println!("scroll, full frame    {:>8.3} MB wire", mb(wire_bytes(scroll_full.len())));

    let row = scene.rows() / 2;
    let typed = format!("{}x", scene.text(row));
    let t = Instant::now();
    let tile = p.line(&typed, scene.w - scene.sidebar, cell_h, BG);
    let raster_line = t.elapsed();
    let t = Instant::now();
    let dirty = zlib(&tile.crop(0, cell_h));
    let encode_line = t.elapsed();
    println!(
        "keystroke, dirty line {:>8.3} MB wire | raster {:.2} ms | zlib {:.2} ms",
        mb(wire_bytes(dirty.len())),
        ms(raster_line),
        ms(encode_line)
    );

    let mut sizes: HashMap<usize, usize> = HashMap::new();
    for i in 0..scene.lines.len() {
        let image = p.line(scene.text(i), scene.w - scene.sidebar, cell_h, BG);
        sizes.insert(i, wire_bytes(zlib(&image.rgb).len()));
    }
    let avg = sizes.values().sum::<usize>() / sizes.len();
    let placements = scene.rows() * "\x1b_Ga=p,i=1234,p=1,C=1,q=2\x1b\\\x1b[120;40H".len();
    let scroll_tile = avg + placements;
    println!("scroll, line tiles    {:>8.3} MB wire (1 new line + {} re-placements)", mb(scroll_tile), scene.rows());
    println!("first paint, tiles    {:>8.3} MB wire", mb(avg * scene.rows()));
    for (name, per) in [("full frames", scroll_full.len()), ("line tiles", scroll_tile)] {
        let wire = if name == "full frames" { wire_bytes(per) } else { per };
        println!("60 fps scroll, {name:<11} {:>8.1} MB/s ({:.0} Mbit/s)", mb(wire * 60), mb(wire * 60) * 8.0);
    }
    // A cell TUI rewrites at most every visible cell with colour changes on a scroll.
    let cols = scene.w / (cell_h / 2);
    println!("(cell fallback scroll estimate {:.3} MB wire)", mb(scene.rows() * cols * 6));
}

/// Writes one APC, inside tmux's DCS passthrough when `wrap` is set.
fn apc(out: &mut impl Write, wrap: bool, control: &str, payload: &[u8]) {
    if wrap {
        out.write_all(b"\x1bPtmux;\x1b\x1b_G").unwrap();
    } else {
        out.write_all(b"\x1b_G").unwrap();
    }
    out.write_all(control.as_bytes()).unwrap();
    if !payload.is_empty() {
        out.write_all(b";").unwrap();
        out.write_all(payload).unwrap();
    }
    out.write_all(if wrap { b"\x1b\x1b\\\x1b\\" as &[u8] } else { b"\x1b\\" }).unwrap();
}

fn send_chunks(out: &mut impl Write, wrap: bool, id: u32, image: &Image) {
    let b64 = base64::engine::general_purpose::STANDARD.encode(zlib(&image.rgb));
    let chunks: Vec<&[u8]> = b64.as_bytes().chunks(4096).collect();
    for (i, chunk) in chunks.iter().enumerate() {
        let more = u8::from(i + 1 < chunks.len());
        let control = if i == 0 {
            format!("a=t,f=24,o=z,t=d,q=2,i={id},s={},v={},m={more}", image.w, image.h)
        } else {
            format!("m={more},q=2")
        };
        apc(out, wrap, &control, chunk);
    }
}

/// Holds one static page so a screenshot can show what actually rendered.
fn show(placeholder: bool, wrap: bool) {
    let (cols, rows, cell_w, cell_h) = (90usize, 20usize, 16usize, 34usize);
    let scene = Scene::new(cols * cell_w, rows * cell_h, cell_h);
    let mut p = Painter::new(cell_h as f32);
    let mut out = std::io::BufWriter::with_capacity(1 << 20, std::io::stdout().lock());
    write!(out, "\x1b[2J\x1b[H").unwrap();
    for row in 0..rows {
        let id = row as u32 + 1;
        send_chunks(&mut out, wrap, id, &p.line(scene.text(row), scene.w, cell_h, BG));
        write!(out, "\x1b[{};1H", row + 1).unwrap();
        if placeholder {
            apc(&mut out, wrap, &format!("a=p,U=1,i={id},c={cols},r=1,q=2"), b"");
            // Row and column 0 are both U+0305; later cells infer their column.
            write!(out, "\x1b[38;5;{id}m\u{10EEEE}\u{305}\u{305}").unwrap();
            write!(out, "{}\x1b[39m", "\u{10EEEE}".repeat(cols - 1)).unwrap();
        } else {
            apc(&mut out, wrap, &format!("a=p,i={id},p=1,c={cols},r=1,C=1,q=2"), b"");
        }
    }
    out.flush().unwrap();
    std::thread::sleep(Duration::from_secs(12));
}

fn send(out: &mut impl Write, id: u32, image: &Image) {
    let z = zlib(&image.rgb);
    let b64 = base64::engine::general_purpose::STANDARD.encode(z);
    let chunks: Vec<&[u8]> = b64.as_bytes().chunks(4096).collect();
    for (i, chunk) in chunks.iter().enumerate() {
        let more = u8::from(i + 1 < chunks.len());
        if i == 0 {
            write!(out, "\x1b_Ga=t,f=24,o=z,t=d,q=2,i={id},s={},v={},m={more};", image.w, image.h).unwrap();
        } else {
            write!(out, "\x1b_Gm={more},q=2;").unwrap();
        }
        out.write_all(chunk).unwrap();
        out.write_all(b"\x1b\\").unwrap();
    }
}

/// Scrolls cached tiles and reports the rate; needs a Kitty-protocol terminal.
fn demo() {
    let (cols, rows, cell_w, cell_h) = (100usize, 40usize, 16usize, 34usize);
    let scene = Scene::new(cols * cell_w, rows * cell_h, cell_h);
    let mut p = Painter::new(cell_h as f32);
    let mut out = std::io::BufWriter::with_capacity(1 << 20, std::io::stdout().lock());
    write!(out, "\x1b[?1049h\x1b[?25l\x1b[2J").unwrap();
    for (i, text) in scene.lines.iter().enumerate() {
        send(&mut out, i as u32 + 1, &p.line(text, scene.w, cell_h, BG));
    }
    out.flush().unwrap();
    let n = scene.lines.len();
    let frames = 1200;
    let start = Instant::now();
    for top in 0..frames {
        // Reusing p=1 moves a tile; the line that scrolled off loses its placement.
        let gone = (top + n - 1) % n + 1;
        write!(out, "\x1b_Ga=d,d=i,i={gone},q=2\x1b\\").unwrap();
        for row in 0..rows {
            let id = (top + row) % n + 1;
            write!(out, "\x1b[{};1H\x1b_Ga=p,i={id},p=1,C=1,q=2\x1b\\", row + 1).unwrap();
        }
        out.flush().unwrap();
    }
    let elapsed = start.elapsed();
    write!(out, "\x1b_Ga=d,d=A,q=2\x1b\\\x1b[?25h\x1b[?1049l").unwrap();
    out.flush().unwrap();
    eprintln!("{frames} frames in {:.2}s = {:.0} fps written", elapsed.as_secs_f64(), frames as f64 / elapsed.as_secs_f64());
}

fn main() {
    match std::env::args().nth(1).as_deref() {
        Some("demo") => demo(),
        Some("show") => {
            let args: Vec<String> = std::env::args().collect();
            show(args.iter().any(|a| a == "placeholder"), args.iter().any(|a| a == "--wrap"));
        }
        _ => {
            measure(1920, 1080, 22);
            measure(3840, 2160, 44);
        }
    }
}
