//! Counts what the main loop does, a second at a time, when `CELLIX_PROF` asks: how often it
//! wakes, what each frame was drawn for, what reached the terminal, and where the time went.
//! `slow-frames.log` sees only frames slow enough to feel; this sees the many cheap ones that
//! add up to a busy core. Off, which it is unless asked, it's an `Option` that's `None`.
//!
//! `CELLIX_PROF=1` writes `prof.log` beside `slow-frames.log`; any other value is the path to
//! write. One line a second, as `key=value` words, so `harness/x perf` can read it.

use std::fs::File;
use std::io::{self, Write};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use cellix_term::Work;

/// Why a frame was drawn. A frame can have several reasons, and counts under each.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cause {
    /// Keys, the mouse, a paste, focus.
    Input,
    /// A session on screen printed.
    Output,
    /// The half-second tick found something changed.
    Tick,
    /// Something else came back: a language server, a thread, an agent's request.
    Poll,
    /// The window, the cell or the rows changed size, or another workspace came in front.
    Resize,
    /// A scroll's glide arrived and the page underneath takes over.
    Scroll,
    /// Sessions not on screen printed, and what's drawn from them is brought up to date.
    Hidden,
}

const CAUSES: [(Cause, &str); 7] = [
    (Cause::Input, "input"),
    (Cause::Output, "output"),
    (Cause::Tick, "tick"),
    (Cause::Poll, "poll"),
    (Cause::Resize, "resize"),
    (Cause::Scroll, "scroll"),
    (Cause::Hidden, "hidden"),
];

/// The reasons gathered for the next frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Causes(u8);

impl Causes {
    pub fn add(&mut self, cause: Cause) {
        self.0 |= 1 << cause as u8;
    }

    pub fn has(self, cause: Cause) -> bool {
        self.0 & (1 << cause as u8) != 0
    }
}

/// Where a trip round the loop spent its time, as the phases `slow.rs` names.
#[derive(Clone, Copy, Debug)]
pub enum Phase {
    Events,
    Poll,
    Draw,
    Present,
}

const PHASES: [(Phase, &str); 4] = [
    (Phase::Events, "events"),
    (Phase::Poll, "poll"),
    (Phase::Draw, "draw"),
    (Phase::Present, "present"),
];

#[derive(Clone, Copy, Default)]
struct Timing {
    count: u32,
    total: Duration,
    most: Duration,
}

/// What a second held.
#[derive(Clone, Copy, Default)]
struct Window {
    wakes: u64,
    draws: u64,
    causes: [u64; CAUSES.len()],
    presents: u64,
    /// Presents that sent anything at all.
    sent: u64,
    tiles: u64,
    rows: u64,
    bytes: u64,
    timings: [Timing; PHASES.len()],
}

/// The counters of a run.
pub struct Prof {
    out: Box<dyn Write>,
    began: Instant,
    since: Instant,
    now: Window,
    /// Bytes written to the terminal as last told.
    total: u64,
}

/// Where `CELLIX_PROF` says to write, if it says anything.
fn asked(value: Option<String>, cache: Option<PathBuf>) -> Option<PathBuf> {
    let value = value?;
    match value.trim() {
        "" | "0" | "off" => None,
        "1" | "on" => Some(cache?.join("cellix").join("prof.log")),
        path => Some(PathBuf::from(path)),
    }
}

fn cache_folder() -> Option<PathBuf> {
    let var = |name: &str| std::env::var_os(name).map(PathBuf::from);
    var("XDG_CACHE_HOME").or_else(|| var("HOME").map(|home| home.join(".cache")))
}

impl Prof {
    /// The counters, if `CELLIX_PROF` asks for them and the file can be made.
    pub fn start() -> Option<Self> {
        let path = asked(std::env::var("CELLIX_PROF").ok(), cache_folder())?;
        if let Some(folder) = path.parent() {
            std::fs::create_dir_all(folder).ok()?;
        }
        let file = File::create(path).ok()?;
        Some(Self::to(Box::new(file), Instant::now()))
    }

    fn to(out: Box<dyn Write>, now: Instant) -> Self {
        Self {
            out,
            began: now,
            since: now,
            now: Window::default(),
            total: 0,
        }
    }

    pub fn wake(&mut self) {
        self.now.wakes += 1;
    }

    pub fn drew(&mut self, causes: Causes) {
        self.now.draws += 1;
        for (index, (cause, _)) in CAUSES.iter().enumerate() {
            if causes.has(*cause) {
                self.now.causes[index] += 1;
            }
        }
    }

    /// A present, and how many bytes it wrote.
    pub fn presented(&mut self, bytes: u64) {
        self.now.presents += 1;
        self.now.sent += u64::from(bytes > 0);
    }

    pub fn worked(&mut self, work: Work) {
        self.now.tiles += work.tiles;
        self.now.rows += work.rows;
    }

    /// All the bytes written to the terminal so far, of which the window counts what's new.
    pub fn written(&mut self, total: u64) {
        self.now.bytes += total.saturating_sub(self.total);
        self.total = total;
    }

    pub fn took(&mut self, phase: Phase, time: Duration) {
        let timing = &mut self.now.timings[phase as usize];
        timing.count += 1;
        timing.total += time;
        timing.most = timing.most.max(time);
    }

    /// Writes the second just ended, once one has.
    pub fn flush(&mut self, now: Instant) {
        let length = now.duration_since(self.since);
        if length < Duration::from_secs(1) {
            return;
        }
        let line = self.line(now, length);
        // One write, so a reader never finds half a line.
        let _ = self.out.write_all(format!("{line}\n").as_bytes());
        let _ = self.out.flush();
        self.since = now;
        self.now = Window::default();
    }

    fn line(&self, now: Instant, length: Duration) -> String {
        let w = &self.now;
        let mut line = format!(
            "t={:.1} ms={} wakes={} draws={}",
            now.duration_since(self.began).as_secs_f32(),
            length.as_millis(),
            w.wakes,
            w.draws
        );
        for (index, (_, name)) in CAUSES.iter().enumerate() {
            line.push_str(&format!(" {name}={}", w.causes[index]));
        }
        line.push_str(&format!(
            " presents={} sent={} tiles={} rows={} bytes={}",
            w.presents, w.sent, w.tiles, w.rows, w.bytes
        ));
        let ms = |time: Duration| time.as_secs_f64() * 1000.0;
        for (index, (_, name)) in PHASES.iter().enumerate() {
            let timing = w.timings[index];
            let average = if timing.count == 0 {
                0.0
            } else {
                ms(timing.total) / f64::from(timing.count)
            };
            line.push_str(&format!(" {name}_ms={average:.2}/{:.2}", ms(timing.most)));
        }
        line
    }
}

/// The terminal's output, counting the bytes that go through, which costs an addition a
/// write whether or not anyone reads the count.
pub struct Counted<W> {
    inner: W,
    total: u64,
}

impl<W> Counted<W> {
    pub fn new(inner: W) -> Self {
        Self { inner, total: 0 }
    }

    /// Bytes written so far.
    pub fn total(&self) -> u64 {
        self.total
    }
}

impl<W: Write> Write for Counted<W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let written = self.inner.write(bytes)?;
        self.total += written as u64;
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    /// Lines written, shared with the test.
    #[derive(Clone, Default)]
    struct Lines(Arc<Mutex<Vec<u8>>>);

    impl Write for Lines {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn field<'a>(line: &'a str, key: &str) -> &'a str {
        let found = line
            .split(' ')
            .find_map(|word| word.strip_prefix(&format!("{key}=")));
        found.unwrap_or_else(|| panic!("no {key} in {line}"))
    }

    /// A second's counts come out as one line of `key=value` words, frames under each of
    /// their reasons, and the counts start again for the next second.
    #[test]
    fn the_counters_write_a_line_a_second_of_what_the_loop_did() {
        let lines = Lines::default();
        let start = Instant::now();
        let mut prof = Prof::to(Box::new(lines.clone()), start);
        for _ in 0..25 {
            prof.wake();
        }
        let mut both = Causes::default();
        both.add(Cause::Input);
        both.add(Cause::Output);
        prof.drew(both);
        let mut tick = Causes::default();
        tick.add(Cause::Tick);
        prof.drew(tick);
        prof.presented(120);
        prof.presented(0);
        prof.worked(Work { tiles: 3, rows: 2 });
        prof.written(100);
        prof.written(120);
        prof.took(Phase::Draw, Duration::from_millis(2));
        prof.took(Phase::Draw, Duration::from_millis(4));
        // Not yet a second: nothing is written.
        prof.flush(start + Duration::from_millis(400));
        assert!(lines.0.lock().unwrap().is_empty());
        prof.flush(start + Duration::from_millis(1000));
        let text = String::from_utf8(lines.0.lock().unwrap().clone()).unwrap();
        let line = text.lines().next().unwrap();
        assert_eq!(field(line, "wakes"), "25");
        assert_eq!(field(line, "draws"), "2");
        assert_eq!((field(line, "input"), field(line, "output")), ("1", "1"));
        assert_eq!((field(line, "tick"), field(line, "poll")), ("1", "0"));
        assert_eq!((field(line, "presents"), field(line, "sent")), ("2", "1"));
        assert_eq!((field(line, "tiles"), field(line, "rows")), ("3", "2"));
        assert_eq!(field(line, "bytes"), "120");
        assert_eq!(field(line, "draw_ms"), "3.00/4.00", "average and most");
        assert_eq!(field(line, "present_ms"), "0.00/0.00");
        // The next second starts from nothing.
        prof.wake();
        prof.flush(start + Duration::from_millis(2100));
        let text = String::from_utf8(lines.0.lock().unwrap().clone()).unwrap();
        let second = text.lines().nth(1).unwrap();
        assert_eq!(field(second, "wakes"), "1");
        assert_eq!(field(second, "draws"), "0");
        assert_eq!(field(second, "ms"), "1100");
    }

    /// Off unless asked; `1` is the file beside the slow frames' log, and anything else is
    /// where to write.
    #[test]
    fn the_counters_are_off_unless_asked() {
        let cache = Some(PathBuf::from("/c"));
        assert_eq!(asked(None, cache.clone()), None);
        assert_eq!(asked(Some("0".into()), cache.clone()), None);
        assert_eq!(asked(Some(String::new()), cache.clone()), None);
        assert_eq!(
            asked(Some("1".into()), cache.clone()),
            Some(PathBuf::from("/c/cellix/prof.log"))
        );
        assert_eq!(
            asked(Some("/tmp/p.log".into()), cache),
            Some(PathBuf::from("/tmp/p.log"))
        );
    }

    #[test]
    fn the_terminals_output_is_counted_as_it_goes() {
        let mut out = Counted::new(Vec::new());
        out.write_all(b"\x1b[?2026h").unwrap();
        out.write_all(b"abc").unwrap();
        assert_eq!(out.total(), 11);
    }
}
