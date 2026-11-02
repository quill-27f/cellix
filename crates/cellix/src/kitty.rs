//! Pictures from programs in the side pane. The Kitty graphics protocol travels in the output
//! like any escape sequence, so it's lifted out before the terminal emulator sees it, and what
//! it carries becomes a bitmap the pane draws. This is what lets a real browser, or a plot from
//! a training run, show beside the file.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use base64::Engine;
use cellix_render::{Bitmap, Pixels};

/// The most a single picture may take, so a broken program can't fill memory.
const MAX_BYTES: usize = 256 * 1024 * 1024;
/// Bitmap ids, far from the ones hashed from content.
static NEXT_ID: AtomicU64 = AtomicU64::new(1 << 60);

/// Part of a program's output.
#[derive(Debug, PartialEq, Eq)]
pub enum Piece {
    /// Goes to the terminal emulator.
    Text(Vec<u8>),
    /// A graphics command, without its `ESC _ G` and terminator.
    Graphics(Vec<u8>),
}

#[derive(Clone, Copy, Default)]
enum State {
    #[default]
    Text,
    Escape,
    Underscore,
    Graphics,
    GraphicsEscape,
}

/// Finds graphics commands in a stream that can split anywhere.
#[derive(Default)]
pub struct Scanner {
    state: State,
    command: Vec<u8>,
}

impl Scanner {
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<Piece> {
        let mut pieces = Vec::new();
        let mut text = Vec::with_capacity(bytes.len());
        for &byte in bytes {
            match self.state {
                State::Text if byte == 0x1b => self.state = State::Escape,
                State::Text => text.push(byte),
                State::Escape if byte == b'_' => self.state = State::Underscore,
                State::Escape => {
                    text.extend([0x1b, byte]);
                    self.state = State::Text;
                }
                State::Underscore if byte == b'G' => {
                    if !text.is_empty() {
                        pieces.push(Piece::Text(std::mem::take(&mut text)));
                    }
                    self.state = State::Graphics;
                }
                State::Underscore => {
                    text.extend([0x1b, b'_', byte]);
                    self.state = State::Text;
                }
                State::Graphics if byte == 0x1b => self.state = State::GraphicsEscape,
                State::Graphics => {
                    if self.command.len() < MAX_BYTES {
                        self.command.push(byte);
                    }
                }
                State::GraphicsEscape if byte == b'\\' => {
                    pieces.push(Piece::Graphics(std::mem::take(&mut self.command)));
                    self.state = State::Text;
                }
                State::GraphicsEscape => {
                    self.command.extend([0x1b, byte]);
                    self.state = State::Graphics;
                }
            }
        }
        if !text.is_empty() {
            pieces.push(Piece::Text(text));
        }
        pieces
    }
}

/// A picture a program wants shown, at the cursor.
#[derive(Clone, Debug)]
pub struct Shown {
    pub image: u32,
    pub placement: u32,
    pub bitmap: Bitmap,
    /// The size asked for, in cells.
    pub columns: Option<u16>,
    pub rows: Option<u16>,
    /// Whether the cursor stays where it was.
    pub cursor_stays: bool,
}

/// What a command comes to.
#[derive(Debug, Default)]
pub struct Outcome {
    pub show: Option<Shown>,
    /// Pictures to take away: all of them, or those of one image.
    pub delete: Option<Option<u32>>,
    /// What to write back to the program.
    pub reply: Option<String>,
}

/// The keys of a command, by their one-letter names.
#[derive(Clone, Debug, Default)]
struct Header(HashMap<char, String>);

impl Header {
    fn parse(text: &str) -> Self {
        let pair = |item: &str| {
            let (key, value) = item.split_once('=')?;
            Some((key.chars().next()?, value.to_string()))
        };
        Self(text.split(',').filter_map(pair).collect())
    }

    fn number(&self, key: char) -> Option<u32> {
        self.0.get(&key)?.parse().ok()
    }

    fn letter(&self, key: char, default: char) -> char {
        let value = self.0.get(&key).and_then(|v| v.chars().next());
        value.unwrap_or(default)
    }
}

/// A picture that hasn't all arrived.
struct Partial {
    header: Header,
    data: Vec<u8>,
}

/// The state of the protocol for one program.
#[derive(Default)]
pub struct Protocol {
    partial: Option<Partial>,
    stored: HashMap<u32, Bitmap>,
}

impl Protocol {
    pub fn command(&mut self, raw: &[u8]) -> Outcome {
        let split = raw.iter().position(|b| *b == b';').unwrap_or(raw.len());
        let header = Header::parse(&String::from_utf8_lossy(&raw[..split]));
        let payload = raw.get(split + 1..).unwrap_or_default();
        let more = header.number('m') == Some(1);
        let (header, data) = match self.partial.take() {
            Some(mut partial) => {
                partial.data.extend_from_slice(payload);
                (partial.header, partial.data)
            }
            None => (header, payload.to_vec()),
        };
        if more {
            self.partial = Some(Partial { header, data });
            return Outcome::default();
        }
        let quiet = header.number('q').unwrap_or(0);
        let image = header.number('i').unwrap_or(0);
        let answer = |result: &str| {
            let failed = result != "OK";
            let wanted = match quiet {
                0 => true,
                1 => failed,
                _ => false,
            };
            wanted.then(|| format!("\x1b_Gi={image};{result}\x1b\\"))
        };
        match header.letter('a', 't') {
            'q' => Outcome {
                // A query is always answered: it's how a program learns what works here.
                reply: Some(match decode(&header, &data) {
                    Ok(_) => format!("\x1b_Gi={image};OK\x1b\\"),
                    Err(error) => format!("\x1b_Gi={image};{error}\x1b\\"),
                }),
                ..Outcome::default()
            },
            action @ ('t' | 'T') => match decode(&header, &data) {
                Ok(bitmap) => {
                    self.stored.insert(image, bitmap.clone());
                    Outcome {
                        show: (action == 'T').then(|| shown(&header, image, bitmap)),
                        reply: answer("OK"),
                        ..Outcome::default()
                    }
                }
                Err(error) => Outcome {
                    reply: answer(error),
                    ..Outcome::default()
                },
            },
            'p' => match self.stored.get(&image) {
                Some(bitmap) => Outcome {
                    show: Some(shown(&header, image, bitmap.clone())),
                    reply: answer("OK"),
                    ..Outcome::default()
                },
                None => Outcome {
                    reply: answer("ENOENT:no such image"),
                    ..Outcome::default()
                },
            },
            'd' => {
                let one = matches!(header.letter('d', 'a'), 'i' | 'I').then_some(image);
                match one {
                    Some(id) => {
                        self.stored.remove(&id);
                    }
                    None => self.stored.clear(),
                }
                Outcome {
                    delete: Some(one),
                    ..Outcome::default()
                }
            }
            _ => Outcome::default(),
        }
    }
}

fn shown(header: &Header, image: u32, bitmap: Bitmap) -> Shown {
    let cells = |key| header.number(key).map(|n| n.min(u16::MAX as u32) as u16);
    Shown {
        image,
        placement: header.number('p').unwrap_or(0),
        bitmap,
        columns: cells('c').filter(|n| *n > 0),
        rows: cells('r').filter(|n| *n > 0),
        cursor_stays: header.number('C') == Some(1),
    }
}

/// The bytes of a picture, from wherever the command says they are.
fn load(header: &Header, data: &[u8]) -> Result<Vec<u8>, &'static str> {
    let text: Vec<u8> = data.iter().copied().filter(|b| *b != b'\n').collect();
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(&text)
        .or_else(|_| base64::engine::general_purpose::STANDARD_NO_PAD.decode(&text))
        .map_err(|_| "EINVAL:bad base64")?;
    let name = || String::from_utf8_lossy(&decoded).into_owned();
    match header.letter('t', 'd') {
        'd' => Ok(decoded),
        's' => read_shared(&name()),
        'f' => std::fs::read(name()).map_err(|_| "EBADF:cannot read file"),
        't' => {
            // A temporary file is the terminal's to remove, once it's been read.
            let path = name();
            if !path.contains("tty-graphics-protocol") {
                return Err("EPERM:not a temporary file");
            }
            let bytes = std::fs::read(&path).map_err(|_| "EBADF:cannot read file")?;
            let _ = std::fs::remove_file(&path);
            Ok(bytes)
        }
        _ => Err("EINVAL:unknown medium"),
    }
}

fn inflate(bytes: Vec<u8>, header: &Header) -> Result<Vec<u8>, &'static str> {
    if header.letter('o', ' ') != 'z' {
        return Ok(bytes);
    }
    use std::io::Read;
    let mut out = Vec::new();
    let reader = flate2::read::ZlibDecoder::new(bytes.as_slice());
    reader
        .take(MAX_BYTES as u64)
        .read_to_end(&mut out)
        .map_err(|_| "EINVAL:bad compression")?;
    Ok(out)
}

fn decode(header: &Header, data: &[u8]) -> Result<Bitmap, &'static str> {
    let bytes = inflate(load(header, data)?, header)?;
    let (mut width, mut height) = (
        header.number('s').unwrap_or(0),
        header.number('v').unwrap_or(0),
    );
    let mut pixels = match header.number('f').unwrap_or(32) {
        32 => bytes,
        24 => {
            let mut out = Vec::with_capacity(bytes.len() / 3 * 4);
            for rgb in bytes.as_chunks::<3>().0 {
                out.extend_from_slice(rgb);
                out.push(255);
            }
            out
        }
        100 => {
            let picture = image::load_from_memory(&bytes).map_err(|_| "EBADPNG:bad image")?;
            let picture = picture.into_rgba8();
            (width, height) = picture.dimensions();
            picture.into_raw()
        }
        _ => return Err("EINVAL:unknown format"),
    };
    if width == 0 || height == 0 || pixels.len() < width as usize * height as usize * 4 {
        return Err("ENODATA:too little data");
    }
    // Shared memory comes in whole pages, so there can be more here than the frame.
    pixels.truncate(width as usize * height as usize * 4);
    Ok(Bitmap {
        id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
        width,
        height,
        kind: Pixels::Rgba,
        pixels: Arc::new(pixels),
    })
}

/// Reads a POSIX shared memory object, which the protocol says the terminal removes.
#[cfg(unix)]
fn read_shared(name: &str) -> Result<Vec<u8>, &'static str> {
    let name = std::ffi::CString::new(name).map_err(|_| "EINVAL:bad name")?;
    // SAFETY: the name is a valid C string, the mapping is the object's whole length and is
    // only read, and everything opened here is closed before returning.
    unsafe {
        let fd = libc::shm_open(name.as_ptr(), libc::O_RDONLY, 0);
        if fd < 0 {
            return Err("ENOENT:no such object");
        }
        let mut info: libc::stat = std::mem::zeroed();
        let size = if libc::fstat(fd, &raw mut info) == 0 {
            info.st_size as usize
        } else {
            0
        };
        let mut out = None;
        if size > 0 && size <= MAX_BYTES {
            let map = libc::mmap(
                std::ptr::null_mut(),
                size,
                libc::PROT_READ,
                libc::MAP_SHARED,
                fd,
                0,
            );
            if map != libc::MAP_FAILED {
                out = Some(std::slice::from_raw_parts(map as *const u8, size).to_vec());
                libc::munmap(map, size);
            }
        }
        libc::close(fd);
        libc::shm_unlink(name.as_ptr());
        out.ok_or("EBADF:cannot map object")
    }
}

#[cfg(not(unix))]
fn read_shared(_name: &str) -> Result<Vec<u8>, &'static str> {
    Err("ENOTSUP:no shared memory")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b64(bytes: &[u8]) -> String {
        base64::engine::general_purpose::STANDARD.encode(bytes)
    }

    fn command(header: &str, data: &[u8]) -> Vec<u8> {
        format!("{header};{}", b64(data)).into_bytes()
    }

    #[test]
    fn graphics_come_out_of_the_stream_wherever_it_splits() {
        let stream = b"one\x1b[1m\x1b_Ga=q,i=1;QUJD\x1b\\two";
        for split in 0..stream.len() {
            let mut scanner = Scanner::default();
            let mut pieces = scanner.feed(&stream[..split]);
            pieces.extend(scanner.feed(&stream[split..]));
            let text: Vec<u8> = pieces
                .iter()
                .flat_map(|p| match p {
                    Piece::Text(t) => t.clone(),
                    Piece::Graphics(_) => Vec::new(),
                })
                .collect();
            assert_eq!(text, b"one\x1b[1mtwo", "split at {split}");
            assert!(
                pieces.contains(&Piece::Graphics(b"a=q,i=1;QUJD".to_vec())),
                "split at {split}"
            );
        }
    }

    #[test]
    fn other_escapes_pass_through_untouched() {
        let mut scanner = Scanner::default();
        let pieces = scanner.feed(b"\x1b_Xother\x1b\\\x1b]0;t\x07");
        assert_eq!(
            pieces,
            [Piece::Text(b"\x1b_Xother\x1b\\\x1b]0;t\x07".to_vec())]
        );
    }

    #[test]
    fn a_query_is_answered_even_when_quiet() {
        let mut protocol = Protocol::default();
        let raw = command("a=q,i=7,t=d,f=24,s=1,v=1,q=2", &[1, 2, 3]);
        assert_eq!(
            protocol.command(&raw).reply.as_deref(),
            Some("\x1b_Gi=7;OK\x1b\\")
        );
    }

    #[test]
    fn a_frame_replaces_itself_and_can_be_compressed_and_chunked() {
        use std::io::Write;
        let mut zipped = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
        zipped.write_all(&[9u8; 16]).unwrap();
        let data = b64(&zipped.finish().unwrap());
        let (first, second) = data.split_at(4);
        let mut protocol = Protocol::default();
        let start = format!("a=T,f=32,s=2,v=2,i=3,p=1,o=z,m=1,C=1,c=5,r=6;{first}");
        assert!(protocol.command(start.as_bytes()).show.is_none());
        let done = protocol.command(format!("m=0;{second}").as_bytes());
        let shown = done.show.unwrap();
        assert_eq!((shown.bitmap.width, shown.bitmap.height), (2, 2));
        assert_eq!(*shown.bitmap.pixels, vec![9u8; 16]);
        assert_eq!((shown.columns, shown.rows), (Some(5), Some(6)));
        assert!(shown.cursor_stays);
        assert_eq!(done.reply.as_deref(), Some("\x1b_Gi=3;OK\x1b\\"));
    }

    #[test]
    fn rgb_gains_its_alpha_and_short_data_is_refused() {
        let mut protocol = Protocol::default();
        let raw = command("a=T,f=24,s=1,v=1,i=1", &[1, 2, 3]);
        let bitmap = protocol.command(&raw).show.unwrap().bitmap;
        assert_eq!(*bitmap.pixels, [1, 2, 3, 255]);
        let raw = command("a=T,f=32,s=4,v=4,i=2", &[1, 2, 3]);
        let outcome = protocol.command(&raw);
        assert!(outcome.show.is_none());
        assert!(outcome.reply.unwrap().contains("ENODATA"));
    }

    #[test]
    fn stored_images_show_on_request_and_can_be_deleted() {
        let mut protocol = Protocol::default();
        protocol.command(&command("a=t,f=24,s=1,v=1,i=4,q=2", &[1, 2, 3]));
        assert!(protocol.command(b"a=p,i=4,q=2").show.is_some());
        let outcome = protocol.command(b"a=d,d=i,i=4");
        assert_eq!(outcome.delete, Some(Some(4)));
        assert!(protocol.command(b"a=p,i=4,q=2").show.is_none());
    }

    #[test]
    #[cfg(unix)]
    fn a_frame_arrives_through_shared_memory_and_the_object_is_removed() {
        let name = format!("/cx{}", std::process::id());
        let c_name = std::ffi::CString::new(name.clone()).unwrap();
        let frame = [7u8; 8];
        // SAFETY: a fresh object of the frame's size, written once and closed.
        unsafe {
            let flags = libc::O_CREAT | libc::O_RDWR;
            let fd = libc::shm_open(c_name.as_ptr(), flags, 0o600 as libc::c_uint);
            assert!(fd >= 0);
            assert_eq!(libc::ftruncate(fd, 8), 0);
            let map = libc::mmap(
                std::ptr::null_mut(),
                8,
                libc::PROT_WRITE,
                libc::MAP_SHARED,
                fd,
                0,
            );
            std::ptr::copy_nonoverlapping(frame.as_ptr(), map as *mut u8, 8);
            libc::munmap(map, 8);
            libc::close(fd);
        }
        let mut protocol = Protocol::default();
        let raw = command("a=T,f=32,s=2,v=1,t=s,i=1,q=2", name.as_bytes());
        let shown = protocol.command(&raw).show.unwrap();
        assert_eq!(*shown.bitmap.pixels, frame);
        // SAFETY: only asks whether the name still exists.
        let again = unsafe { libc::shm_open(c_name.as_ptr(), libc::O_RDONLY, 0) };
        assert!(again < 0, "the terminal removes the object");
    }
}
