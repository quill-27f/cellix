//! Runs the real binary in a PTY, types and clicks like a terminal would, and reads
//! the screen back through a terminal emulator.

use std::io::{Read, Write};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use portable_pty::{CommandBuilder, PtySize, native_pty_system};

type Input = Arc<Mutex<Box<dyn Write + Send>>>;
/// The start of what Cellix wrote, to show when a test fails.
type Raw = Arc<Mutex<Vec<u8>>>;
/// The background colour the "terminal" says it has when asked, as `rrrr/gggg/bbbb`.
type Colour = Arc<Mutex<Option<String>>>;
/// What else the "terminal" answers: when Cellix writes the first, it's sent the second.
type Answers = Arc<Mutex<Vec<(Vec<u8>, Vec<u8>)>>>;

/// How long a test waits for something to show before calling it lost. A test goes on the
/// moment it shows, so this is only the patience: a minute, since on a machine at a load of 50
/// the real binary took longer than ten seconds to answer, and Windows twice that, where a
/// debug build can take many seconds to start while it's scanned, with several starting at once.
const WAIT: Duration = Duration::from_secs(if cfg!(windows) { 120 } else { 60 });

/// Feeds what Cellix draws to `parser`, answering a cursor position query as a terminal would.
/// Windows' ConPTY asks one before it passes anything on, and waits for the answer. An answer
/// that comes too soon can be lost, so it's given again until something else arrives.
fn pump(
    mut reader: Box<dyn Read + Send>,
    parser: Arc<Mutex<vt100::Parser>>,
    (input, raw): (Input, Raw),
    (colour, answers): (Colour, Answers),
    all: Raw,
) {
    let started = Instant::now();
    let seen = Arc::new(Mutex::new(0usize));
    std::thread::spawn(move || {
        let mut buf = [0; 8192];
        while let Ok(n) = reader.read(&mut buf) {
            if n == 0 {
                break;
            }
            let asked = buf[..n].windows(4).any(|w| w == b"\x1b[6n");
            {
                let mut seen = seen.lock().unwrap();
                let mut raw = raw.lock().unwrap();
                if !asked && *seen == 0 {
                    let note = format!("<output at {} ms>", started.elapsed().as_millis());
                    raw.extend_from_slice(note.as_bytes());
                }
                *seen += usize::from(!asked);
                let room = 2000usize.saturating_sub(raw.len()).min(n);
                raw.extend_from_slice(&buf[..room]);
            }
            if asked {
                let (row, column) = parser.lock().unwrap().screen().cursor_position();
                let answer = format!("\x1b[{};{}R", row + 1, column + 1);
                let (input, raw, seen) = (input.clone(), raw.clone(), seen.clone());
                let before = *seen.lock().unwrap();
                std::thread::spawn(move || {
                    for attempt in 0..5 {
                        if *seen.lock().unwrap() > before {
                            return;
                        }
                        let mut input = input.lock().unwrap();
                        let sent = input
                            .write_all(answer.as_bytes())
                            .and_then(|()| input.flush());
                        let note = format!("<answer {attempt}: {}>", sent.is_ok());
                        raw.lock().unwrap().extend_from_slice(note.as_bytes());
                        drop(input);
                        std::thread::sleep(Duration::from_secs(2));
                    }
                });
            }
            // Asked its colour, it answers as a terminal does, and then device attributes.
            let said = colour.lock().unwrap().clone();
            if let Some(said) = said.filter(|_| buf[..n].windows(5).any(|w| w == b"]11;?")) {
                let answer = format!("\x1b]11;rgb:{said}\x1b\\\x1b[?62c");
                let mut input = input.lock().unwrap();
                let _ = input
                    .write_all(answer.as_bytes())
                    .and_then(|()| input.flush());
            }
            for (when, with) in answers.lock().unwrap().iter() {
                if buf[..n].windows(when.len()).any(|w| w == when.as_slice()) {
                    let mut input = input.lock().unwrap();
                    let _ = input.write_all(with).and_then(|()| input.flush());
                }
            }
            all.lock().unwrap().extend_from_slice(&buf[..n]);
            parser.lock().unwrap().process(&buf[..n]);
        }
    });
}

struct Screen {
    parser: Arc<Mutex<vt100::Parser>>,
    input: Input,
    raw: Raw,
    /// Everything Cellix wrote, for what never shows on screen, such as a notification.
    #[cfg_attr(not(unix), allow(dead_code))]
    all: Raw,
    child: Box<dyn portable_pty::Child + Send + Sync>,
    /// Kept for as long as the screen: on Windows, dropping the master closes the console.
    pty: Option<portable_pty::PtyPair>,
}

impl Screen {
    fn start(dir: &Path, args: &[&str]) -> Self {
        Self::start_with(dir, args, &[])
    }

    /// Starts Cellix with environment variables of its own, as `(name, value)`.
    fn start_with(dir: &Path, args: &[&str], env: &[(&str, &Path)]) -> Self {
        Self::start_answering(dir, args, env, Colour::default())
    }

    /// Starts Cellix in a terminal that answers what colour it is with `colour`.
    fn start_answering(dir: &Path, args: &[&str], env: &[(&str, &Path)], colour: Colour) -> Self {
        Self::start_full(dir, args, env, (colour, Answers::default()))
    }

    /// Starts Cellix in a terminal that answers what it's asked as `answers` say.
    #[cfg(unix)]
    fn start_with_answers(dir: &Path, args: &[&str], answers: Vec<(Vec<u8>, Vec<u8>)>) -> Self {
        let answers = Arc::new(Mutex::new(answers));
        Self::start_full(dir, args, &[], (Colour::default(), answers))
    }

    fn start_full(
        dir: &Path,
        args: &[&str],
        env: &[(&str, &Path)],
        (colour, answers): (Colour, Answers),
    ) -> Self {
        let pty = native_pty_system()
            .openpty(PtySize {
                rows: 30,
                cols: 100,
                pixel_width: 0,
                pixel_height: 0,
            })
            .unwrap();
        let mut command = CommandBuilder::new(env!("CARGO_BIN_EXE_cellix"));
        command.cwd(dir);
        command.args(args);
        for (name, value) in env {
            // An empty value takes the variable away, as tmux's must be for a test inside it.
            if value.as_os_str().is_empty() {
                command.env_remove(name);
            } else {
                command.env(name, value);
            }
        }
        // Never the real cache: what one Cellix leaves there, such as the workspaces it had
        // open, would reach every other test's Cellix and the person's own. One cache per
        // project, so a Cellix started again in it finds what the last one left. Short, since a
        // socket's path has to fit in about a hundred bytes.
        if !env.iter().any(|(name, _)| *name == "XDG_CACHE_HOME") {
            use std::hash::{Hash, Hasher};
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            dir.hash(&mut hasher);
            // Under the run's own cache when harness/x gave it one, so what's left there goes
            // when the run ends.
            let base = std::env::var("XDG_CACHE_HOME").unwrap_or_else(|_| "/tmp".to_string());
            command.env(
                "XDG_CACHE_HOME",
                format!("{base}/cxt-{:x}", hasher.finish()),
            );
        }
        let child = pty.slave.spawn_command(command).unwrap();
        let reader = pty.master.try_clone_reader().unwrap();
        let input: Input = Arc::new(Mutex::new(pty.master.take_writer().unwrap()));
        let parser = Arc::new(Mutex::new(vt100::Parser::new(30, 100, 0)));
        let raw = Raw::default();
        let all = Raw::default();
        pump(
            reader,
            parser.clone(),
            (input.clone(), raw.clone()),
            (colour, answers),
            all.clone(),
        );
        Self {
            parser,
            input,
            raw,
            all,
            child,
            pty: Some(pty),
        }
    }

    /// Closes the terminal under Cellix, as closing its window does. On Windows that closes
    /// the pseudoconsole. On Unix the reader and writer still hold the master open, so the
    /// hang-up a closing terminal sends is sent here.
    fn close(&mut self) {
        #[cfg(unix)]
        {
            let pid = self.child.process_id().unwrap() as i32;
            // SAFETY: kill only sends a signal to the child this test started.
            assert_eq!(unsafe { libc::kill(pid, libc::SIGHUP) }, 0);
        }
        self.pty.take();
    }

    /// What Cellix wrote first and last, and whether it's still running, for a failure to
    /// show. A panic's message and backtrace are the last it writes, after it has left its
    /// screen, so they're shown whole from where they start.
    fn story(&mut self) -> String {
        let raw = self.raw.lock().unwrap();
        let status = self.child.try_wait();
        let last = last_written(&self.all.lock().unwrap());
        format!(
            "exited: {status:?}, wrote: {:?}\nwrote last: {last}",
            String::from_utf8_lossy(&raw)
        )
    }

    fn contents(&self) -> String {
        self.parser.lock().unwrap().screen().contents()
    }

    fn wait_for(&mut self, text: &str) {
        let mut deadline = Instant::now() + WAIT;
        let mut ended = false;
        while Instant::now() < deadline {
            if self.contents().contains(text) {
                return;
            }
            // Ended, it shows nothing more once what it wrote last is read: a little longer
            // for that, and no more, so its story is told at once.
            if !ended && self.child.try_wait().is_ok_and(|status| status.is_some()) {
                ended = true;
                deadline = deadline.min(Instant::now() + Duration::from_secs(2));
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let story = self.story();
        panic!("{text:?} never appeared:\n{}\n{story}", self.contents());
    }

    /// Waits for `text` to leave the screen.
    fn wait_gone(&mut self, text: &str) {
        let deadline = Instant::now() + WAIT;
        while self.contents().contains(text) {
            assert!(
                Instant::now() < deadline,
                "{text:?} stayed:\n{}",
                self.contents()
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn send(&mut self, bytes: &[u8]) {
        let mut input = self.input.lock().unwrap();
        input.write_all(bytes).unwrap();
        input.flush().unwrap();
    }

    /// Sends more than a terminal's input buffer holds, so the write waits on Cellix reading
    /// it: a Cellix that stops reading fails the test, where `send` would wait for ever.
    #[cfg(unix)]
    fn send_much(&mut self, bytes: Vec<u8>) {
        let input = Arc::clone(&self.input);
        let (done, sent) = std::sync::mpsc::channel();
        let written = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let progress = Arc::clone(&written);
        let total = bytes.len();
        std::thread::spawn(move || {
            let mut input = input.lock().unwrap();
            let mut ok = true;
            for chunk in bytes.chunks(1024) {
                ok = input.write_all(chunk).and_then(|()| input.flush()).is_ok();
                if !ok {
                    break;
                }
                progress.fetch_add(chunk.len(), std::sync::atomic::Ordering::SeqCst);
            }
            let _ = done.send(ok);
        });
        match sent.recv_timeout(WAIT * 3) {
            Ok(true) => {}
            Ok(false) => panic!("the write failed: {}", self.story()),
            Err(_) => {
                let at = written.load(std::sync::atomic::Ordering::SeqCst);
                let pid = self.child.process_id().unwrap_or(0);
                let mut state = String::new();
                for task in std::fs::read_dir(format!("/proc/{pid}/task"))
                    .into_iter()
                    .flatten()
                    .flatten()
                {
                    let at = task.path();
                    let read = |f: &str| std::fs::read_to_string(at.join(f)).unwrap_or_default();
                    state += &format!(
                        "\n[{}] {} syscall {} stack {}",
                        read("comm").trim(),
                        read("stat").split(' ').nth(2).unwrap_or(""),
                        read("syscall").trim(),
                        read("stack").replace('\n', " | ")
                    );
                }
                let wchan = "";
                panic!(
                    "Cellix stopped reading after {at} of {total} bytes; stat {state} wchan {wchan}\n{}\n{}",
                    self.contents(),
                    self.story()
                )
            }
        }
    }

    /// Waits for Cellix to end, however it does, and gives how. A test that ends it some way
    /// other than a quit says which way it expects; `wait_exit` expects a quit's 0.
    fn wait_end(&mut self) -> portable_pty::ExitStatus {
        let deadline = Instant::now() + WAIT;
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                return status;
            }
            if Instant::now() > deadline {
                let story = self.story();
                panic!("cellix did not exit:\n{}\n{story}", self.contents());
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// Waits for Cellix to end, and requires it to end well, with 0. A panic ends it with
    /// 101, and every quit once did, unseen: the message went to the screen it was leaving.
    fn wait_exit(&mut self) {
        let status = self.wait_end();
        if !status.success() {
            let story = self.story();
            panic!(
                "cellix ended with {status:?}:\n{}\n{story}",
                self.contents()
            );
        }
    }
}

/// What Cellix wrote last, for a failure to show: from a panic's message on where there is
/// one, its backtrace with it, or else the end of what it wrote, its escapes made visible.
fn last_written(all: &[u8]) -> String {
    match all.windows(11).position(|w| w == b"panicked at") {
        Some(at) => String::from_utf8_lossy(&all[at.saturating_sub(200)..]).into_owned(),
        None => format!(
            "{:?}",
            String::from_utf8_lossy(&all[all.len().saturating_sub(1500)..])
        ),
    }
}

/// Waits up to `patience` for a Cellix started outside a `Screen` to end, and requires it to
/// end with 0, as `Screen::wait_exit` does, saying what it wrote last where it doesn't.
#[cfg_attr(not(unix), allow(dead_code))]
fn exits_well(
    child: &mut Box<dyn portable_pty::Child + Send + Sync>,
    wrote: &Mutex<Vec<u8>>,
    patience: Duration,
) {
    let deadline = Instant::now() + patience;
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        let late = Instant::now() > deadline;
        assert!(
            !late,
            "cellix did not exit: {}",
            last_written(&wrote.lock().unwrap())
        );
        std::thread::sleep(Duration::from_millis(20));
    };
    let last = last_written(&wrote.lock().unwrap());
    assert!(status.success(), "cellix ended with {status:?}: {last}");
}

impl Drop for Screen {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

/// What a terminal with paste events (Kitty's clipboard protocol, as Ghostty 1.4 has it)
/// sends when the person pastes: the kinds the clipboard holds, with a one-time password.
#[cfg(unix)]
fn paste_offer(types: &str, password: &str) -> Vec<u8> {
    use base64::Engine;
    let b64 = |s: &[u8]| base64::engine::general_purpose::STANDARD.encode(s);
    let pw = b64(password.as_bytes());
    format!(
        "\x1b]5522;type=read:status=OK:pw={pw}\x1b\\\x1b]5522;type=read:status=DATA:mime=Lg==:pw={pw};{}\x1b\\\x1b]5522;type=read:status=DONE:pw={pw}\x1b\\",
        b64(format!("{types}\n").as_bytes())
    )
    .into_bytes()
}

/// What the terminal answers to a read: the data, in chunks of 4096 bytes before encoding.
#[cfg(unix)]
fn paste_data(mime: &str, data: &[u8]) -> Vec<u8> {
    use base64::Engine;
    let b64 = |s: &[u8]| base64::engine::general_purpose::STANDARD.encode(s);
    let mut out = b"\x1b]5522;type=read:status=OK\x1b\\".to_vec();
    for chunk in data.chunks(4096) {
        out.extend(
            format!(
                "\x1b]5522;type=read:status=DATA:mime={};{}\x1b\\",
                b64(mime.as_bytes()),
                b64(chunk)
            )
            .bytes(),
        );
    }
    out.extend(b"\x1b]5522;type=read:status=DONE\x1b\\");
    out
}

/// Waits for Cellix to have written `what` to the terminal.
#[cfg(unix)]
fn wait_written(screen: &mut Screen, what: &[u8]) {
    let deadline = Instant::now() + WAIT;
    while !screen
        .all
        .lock()
        .unwrap()
        .windows(what.len())
        .any(|w| w == what)
    {
        assert!(
            Instant::now() < deadline,
            "never wrote {:?}",
            String::from_utf8_lossy(what)
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Unix only: Windows asks the terminal nothing, so it has no paste events.
#[test]
#[cfg(unix)]
fn a_picture_pasted_from_the_terminals_clipboard_lands_in_the_project() {
    let dir = project();
    // A terminal with paste events, as Ghostty 1.4 and kitty are.
    let has_them = (b"\x1b[?5522$p".to_vec(), b"\x1b[?5522;2$y".to_vec());
    let mut screen = Screen::start_with_answers(dir.path(), &["hello.txt"], vec![has_them]);
    screen.wait_for("hello from a file");
    wait_written(&mut screen, b"\x1b[?5522h");

    // Text: the terminal lists what the clipboard holds, Cellix asks for the text with the
    // paste's password, and the text lands as any paste does.
    screen.send(&paste_offer("text/plain text/html", "first"));
    wait_written(
        &mut screen,
        b"\x1b]5522;type=read:pw=Zmlyc3Q=:name=UGFzdGUgZXZlbnQ=;dGV4dC9wbGFpbg==\x1b\\",
    );
    screen.send(&paste_data("text/plain", b"PASTED WORDS "));
    screen.wait_for("PASTED WORDS hello from a file");

    // A picture, as a Mac screenshot is: PNG asked for, kept in the project, its path pasted.
    // Any bytes do, since PNG is kept as it comes: enough for the terminal's write to be split
    // mid-message, which once lost a chunk.
    let mut picture = b"\x89PNG\r\n\x1a\n".to_vec();
    picture.extend((0..200_000u32).map(|i| (i * 7 % 251) as u8));
    screen.send(&paste_offer("image/tiff image/png", "second"));
    wait_written(
        &mut screen,
        b"\x1b]5522;type=read:pw=c2Vjb25k:name=UGFzdGUgZXZlbnQ=;aW1hZ2UvcG5n\x1b\\",
    );
    screen.send_much(paste_data("image/png", &picture));
    screen.wait_for("Pasted a picture, kept as .cellix/pasted/pasted-");
    let folder = dir.path().join(".cellix/pasted");
    let kept: Vec<_> = std::fs::read_dir(&folder)
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "png"))
        .collect();
    assert_eq!(kept.len(), 1, "{kept:?}");
    let got = std::fs::read(&kept[0]).unwrap();
    let first = got.iter().zip(&picture).position(|(a, b)| a != b);
    assert!(
        got == picture,
        "kept {} bytes of {}, first differing at {first:?}",
        got.len(),
        picture.len()
    );
}

/// Where `text` starts on screen, as the column and row, counting from one, that a terminal
/// reports a press at: on the last row that says it, a wide letter taking two columns.
#[cfg(unix)]
fn cell_of(screen: &Screen, text: &str) -> Option<(u16, u16)> {
    let parser = screen.parser.lock().unwrap();
    let shown = parser.screen();
    let (rows, columns) = shown.size();
    (0..rows).rev().find_map(|row| {
        let mut line = String::new();
        let mut starts = Vec::new();
        for column in 0..columns {
            let cell = shown.cell(row, column)?;
            if cell.is_wide_continuation() {
                continue;
            }
            let letters = if cell.has_contents() {
                cell.contents()
            } else {
                " "
            };
            starts.extend(letters.chars().map(|_| column));
            line.push_str(letters);
        }
        let at = line.find(text)?;
        Some((starts[line[..at].chars().count()] + 1, row + 1))
    })
}

/// A right click on a URL that a terminal in the side pane printed has Copy Link, which goes
/// to the terminal Cellix runs in as OSC 52, as every copy does, so the address lands on the
/// person's own clipboard, over SSH too. The real binary, driven by keys and the mouse.
#[test]
#[cfg(unix)]
fn a_links_copy_reaches_the_terminals_clipboard() {
    use base64::Engine;
    let dir = project();
    let shell = Path::new("/bin/sh");
    let mut screen = Screen::start_with(dir.path(), &["hello.txt"], &[("SHELL", shell)]);
    screen.wait_for("hello from a file");
    screen.send(b"\x0a"); // Ctrl+J, a terminal in the side pane
    // Worked out by the shell, so the line typed doesn't hold the address it prints.
    let address = "https://example.com/pty/42";
    screen.send(b"echo \"https://example.com/pty/$((6*7)).\"\r");
    screen.wait_for(address);
    let (column, row) = cell_of(&screen, address).unwrap();
    let column = column + 3;
    // SGR: a press and a release of the right button.
    screen.send(format!("\x1b[<2;{column};{row}M\x1b[<2;{column};{row}m").as_bytes());
    screen.wait_for("Copy Link");
    let (column, row) = cell_of(&screen, "Copy Link").unwrap();
    let column = column + 1;
    screen.send(format!("\x1b[<0;{column};{row}M\x1b[<0;{column};{row}m").as_bytes());
    let encoded = base64::engine::general_purpose::STANDARD.encode(address);
    wait_written(&mut screen, format!("\x1b]52;c;{encoded}\x07").as_bytes());
    screen.wait_for("Copied the link");
    // Quitting asks first while the shell counts as busy, which it may still do just after
    // the echo: the host looks whether it's back at its prompt every half second. Asked, it's
    // stopped with Cellix; at its prompt, Cellix quits at once. Either way it ends, and well.
    screen.send(b"\x11"); // Ctrl+Q
    let deadline = Instant::now() + WAIT;
    let mut stopped = false;
    while screen.child.try_wait().unwrap().is_none() {
        if Instant::now() > deadline {
            let story = screen.story();
            panic!("still running:\n{}\n{story}", screen.contents());
        }
        if !stopped && let Some((column, row)) = cell_of(&screen, "Stop and Quit") {
            let column = column + 1;
            screen.send(format!("\x1b[<0;{column};{row}M\x1b[<0;{column};{row}m").as_bytes());
            stopped = true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    screen.wait_exit();
}

fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("hello.txt"), "hello from a file\n").unwrap();
    std::fs::write(dir.path().join("other.txt"), "other contents\n").unwrap();
    dir
}

#[test]
fn pty_shows_file_contents() {
    let dir = project();
    let mut screen = Screen::start(dir.path(), &["hello.txt"]);
    screen.wait_for("hello from a file");
    screen.wait_for("other.txt");
    screen.send(b"\x11"); // Ctrl+Q
    screen.wait_exit();
}

/// Ignored on Windows by default: the runner's built-in console passes no clicks on, though
/// keys arrive. CI's "PTY tests through Windows Terminal's ConPTY" step runs it through the
/// ConPTY Windows Terminal ships, where the click arrives.
#[test]
#[cfg_attr(
    windows,
    ignore = "the built-in console passes no clicks on; CI runs it through Windows Terminal's ConPTY"
)]
fn pty_mouse_click_opens_file_and_typing_saves() {
    let dir = project();
    let mut screen = Screen::start(dir.path(), &[]);
    screen.wait_for("other.txt");
    let row = screen
        .contents()
        .lines()
        .position(|l| l.contains("other.txt"))
        .unwrap();
    // SGR mouse press and release, which is what terminals send with mouse capture on.
    screen.send(format!("\x1b[<0;6;{}M\x1b[<0;6;{}m", row + 1, row + 1).as_bytes());
    screen.wait_for("other contents");
    screen.send(b"new ");
    screen.wait_for("new other contents");
    screen.send(b"\x13"); // Ctrl+S
    screen.wait_for("Saved other.txt");
    screen.send(b"\x11");
    screen.wait_exit();
    let saved = std::fs::read_to_string(dir.path().join("other.txt")).unwrap();
    assert_eq!(saved, "new other contents\n");
}

#[test]
fn pty_bracketed_paste_inserts_verbatim() {
    let dir = project();
    let mut screen = Screen::start(dir.path(), &["hello.txt"]);
    screen.wait_for("hello from a file");
    // Typed, the bracket would close itself and the new lines indent. Windows Terminal sends a
    // pasted line break as a carriage return.
    let newline = if cfg!(windows) { "\r" } else { "\n" };
    let pasted = format!("pasted({newline}\ttext{newline}more{newline}");
    screen.send(format!("\x1b[200~{pasted}\x1b[201~").as_bytes());
    screen.wait_for("more");
    screen.send(b"\x13"); // Ctrl+S
    screen.wait_for("Saved hello.txt");
    screen.send(b"\x11");
    screen.wait_exit();
    let saved = std::fs::read_to_string(dir.path().join("hello.txt")).unwrap();
    assert_eq!(saved, "pasted(\n\ttext\nmore\nhello from a file\n");
}

/// Windows has no Kitty keyboard protocol: keys arrive as console events.
#[test]
#[cfg(unix)]
fn pty_ctrl_shift_p_and_cmd_p_work_as_in_vs_code() {
    let dir = project();
    let mut screen = Screen::start(dir.path(), &[]);
    screen.wait_for("other.txt");
    screen.send(b"\x1b[112;6u"); // Ctrl+Shift+P, Kitty keyboard protocol
    // The palette's own hint, and a command under its category.
    screen.wait_for("Type to search");
    screen.wait_for("Agent: List Agents");
    screen.send(b"\x1b");
    std::thread::sleep(Duration::from_millis(200));
    screen.send(b"\x1b[112;9u"); // Cmd+P
    screen.wait_for("Search files");
    screen.send(b"\x1b");
    std::thread::sleep(Duration::from_millis(200));
    screen.send(b"\x11");
    screen.wait_exit();
}

/// Windows has no Cmd key, nor the Kitty keyboard protocol that carries it.
#[test]
#[cfg(unix)]
fn pty_cmd_w_closes_the_tab_and_cmd_q_quits() {
    let dir = project();
    let mut screen = Screen::start(dir.path(), &["hello.txt"]);
    screen.wait_for("hello from a file");
    screen.send(b"\x1b[119;9u"); // Cmd+W, Kitty keyboard protocol
    let deadline = Instant::now() + WAIT;
    while screen.contents().contains("hello from a file") {
        assert!(Instant::now() < deadline, "the tab stayed open");
        std::thread::sleep(Duration::from_millis(20));
    }
    screen.send(b"\x1b[113;9u"); // Cmd+Q
    screen.wait_exit();
}

#[test]
fn pty_restores_the_terminal_on_exit() {
    let dir = project();
    let mut screen = Screen::start(dir.path(), &["hello.txt"]);
    screen.wait_for("hello from a file");
    assert!(screen.parser.lock().unwrap().screen().alternate_screen());
    screen.send(b"\x11");
    screen.wait_exit();
    let deadline = Instant::now() + WAIT;
    while screen.parser.lock().unwrap().screen().alternate_screen() {
        assert!(Instant::now() < deadline, "still on the alternate screen");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Where `words` are on the screen, as the terminal's column and row.
#[cfg(unix)]
fn place_of(screen: &Screen, words: &str) -> (usize, usize) {
    let contents = screen.contents();
    contents
        .lines()
        .enumerate()
        .find_map(|(row, line)| Some((line.find(words)?, row)))
        .unwrap_or_else(|| panic!("{words:?} isn't on screen:\n{contents}"))
}

/// Ghostty and xterm keep a Shift+click for their own selection unless the program asks for
/// it with XTSHIFTESCAPE, so Cellix asks as it starts and hands it back as it ends; a
/// Shift+click on a page then selects from the click before it, with the buttons for a
/// comment beside it.
#[test]
#[cfg(unix)]
fn pty_a_shift_click_on_a_page_selects_from_the_click_before() {
    let dir = project();
    let page = "# Scan\n\nThe first words.\n\n- an item\n\n> a quote\n\nThe last words.\n";
    std::fs::write(dir.path().join("scan.md"), page).unwrap();
    let mut screen = Screen::start(dir.path(), &["scan.md"]);
    screen.wait_for("The last words.");
    assert!(written(&screen).contains("\x1b[>1s"), "asks for Shift");
    let (x, y) = place_of(&screen, "first words");
    screen.send(format!("\x1b[<0;{};{}M\x1b[<0;{};{}m", x + 1, y + 1, x + 1, y + 1).as_bytes());
    let (x, y) = place_of(&screen, "last words.");
    let x = x + "last words".len();
    // The mouse's report with Shift held: 4 in its button.
    screen.send(format!("\x1b[<4;{};{}M\x1b[<4;{};{}m", x + 1, y + 1, x + 1, y + 1).as_bytes());
    screen.wait_for("Ask Agent");
    screen.send(b"\x11"); // Ctrl+Q
    screen.wait_exit();
    let deadline = Instant::now() + WAIT;
    while !written(&screen).contains("\x1b[>0s") {
        assert!(Instant::now() < deadline, "Shift was never handed back");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// A fake terminal that says it speaks Kitty graphics, so the binary takes the pixel path
/// with real fonts. Returns once it has sent a tile with this size marker. Windows asks the
/// terminal nothing, so it never takes this path.
#[cfg(unix)]
fn expect_tiles(pixels: (u16, u16), reply: &[u8], tile_marker: &[u8]) {
    let dir = project();
    let pty = native_pty_system()
        .openpty(PtySize {
            rows: 12,
            cols: 40,
            pixel_width: pixels.0,
            pixel_height: pixels.1,
        })
        .unwrap();
    let mut command = CommandBuilder::new(env!("CARGO_BIN_EXE_cellix"));
    command.cwd(dir.path());
    command.arg("hello.txt");
    let mut child = pty.slave.spawn_command(command).unwrap();
    let mut reader = pty.master.try_clone_reader().unwrap();
    let mut input = pty.master.take_writer().unwrap();
    let raw = Arc::new(Mutex::new(Vec::new()));
    let sink = raw.clone();
    std::thread::spawn(move || {
        let mut buf = [0; 65536];
        while let Ok(n) = reader.read(&mut buf) {
            if n == 0 {
                break;
            }
            sink.lock().unwrap().extend_from_slice(&buf[..n]);
        }
    });
    let contains = |needle: &[u8]| {
        let raw = raw.lock().unwrap();
        raw.windows(needle.len()).any(|w| w == needle)
    };
    let deadline = Instant::now() + WAIT;
    while !contains(b"a=q") {
        assert!(Instant::now() < deadline, "no graphics query");
        std::thread::sleep(Duration::from_millis(5));
    }
    input.write_all(reply).unwrap();
    input.flush().unwrap();
    let deadline = Instant::now() + WAIT;
    while !(contains(b"a=t,f=24") && contains(tile_marker)) {
        assert!(Instant::now() < deadline, "no tiles sent");
        std::thread::sleep(Duration::from_millis(20));
    }
    input.write_all(b"\x11").unwrap();
    input.flush().unwrap();
    exits_well(&mut child, &raw, WAIT);
}

#[test]
#[cfg(unix)]
fn pty_pixel_tier_sends_tiles_when_the_terminal_supports_graphics() {
    expect_tiles(
        (40 * 10, 12 * 20),
        b"\x1b_Gi=31;OK\x1b\\\x1b[?62c",
        b"s=400,v=20",
    );
}

/// Zellij can report no pixels in the window size but answers `CSI 16t`.
#[test]
#[cfg(unix)]
fn pty_pixel_tier_uses_the_reported_cell_size_without_window_pixels() {
    expect_tiles(
        (0, 0),
        b"\x1b[6;37;17t\x1b_Gi=31;OK\x1b\\\x1b[?62c",
        b"s=680,v=37",
    );
}

/// A press reported in pixels lands on the line under the pointer after the window has
/// changed size, in a terminal whose window is taller than its rows: one that counts its
/// padding or a tab bar in the height it reports. The cell's height was taken at each resize
/// as the window's pixels over its rows, two or three pixels too tall here, so low in the
/// window a press landed lines above the pointer until some later resize happened to guess
/// right. The terminal says its cell size exactly, and is asked again once the size settles.
#[test]
#[cfg(unix)]
fn pty_a_press_in_pixels_lands_on_its_line_after_a_resize() {
    let dir = tempfile::tempdir().unwrap();
    let text: String = (1..=40).map(|n| format!("line {n}\n")).collect();
    let file = dir.path().join("lines.txt");
    std::fs::write(&file, text).unwrap();
    // Cells of 10 by 20, and 90 pixels of the window that aren't rows.
    let size = |rows: u16| PtySize {
        rows,
        cols: 100,
        pixel_width: 1000,
        pixel_height: rows * 20 + 90,
    };
    let pty = native_pty_system().openpty(size(30)).unwrap();
    let mut command = CommandBuilder::new(env!("CARGO_BIN_EXE_cellix"));
    command.cwd(dir.path());
    command.arg("lines.txt");
    let mut child = pty.slave.spawn_command(command).unwrap();
    let mut reader = pty.master.try_clone_reader().unwrap();
    let input: Input = Arc::new(Mutex::new(pty.master.take_writer().unwrap()));
    let raw = Arc::new(Mutex::new(Vec::new()));
    let sink = raw.clone();
    std::thread::spawn(move || {
        let mut buf = [0; 65536];
        while let Ok(n) = reader.read(&mut buf) {
            if n == 0 {
                break;
            }
            sink.lock().unwrap().extend_from_slice(&buf[..n]);
        }
    });
    // The terminal: every time it's asked what it is, it says its cell size first, and the
    // first time that it draws pictures and reports the mouse in pixels.
    let answered = Arc::new(Mutex::new(0usize));
    {
        let (raw, input, answered) = (raw.clone(), input.clone(), answered.clone());
        std::thread::spawn(move || {
            loop {
                let asked = {
                    let raw = raw.lock().unwrap();
                    raw.windows(3).filter(|w| *w == b"\x1b[c").count()
                };
                let mut done = answered.lock().unwrap();
                while *done < asked {
                    let mut reply = b"\x1b[6;20;10t".to_vec();
                    if *done == 0 {
                        reply.extend_from_slice(b"\x1b[?1016;2$y\x1b_Gi=31;OK\x1b\\");
                    }
                    reply.extend_from_slice(b"\x1b[?62c");
                    let mut input = input.lock().unwrap();
                    if input.write_all(&reply).is_err() {
                        return;
                    }
                    let _ = input.flush();
                    *done += 1;
                }
                drop(done);
                std::thread::sleep(Duration::from_millis(5));
            }
        });
    }
    let send = |bytes: &[u8]| {
        let mut input = input.lock().unwrap();
        input.write_all(bytes).unwrap();
        input.flush().unwrap();
    };
    let wait = |what: &str, met: &dyn Fn() -> bool| {
        let deadline = Instant::now() + Duration::from_secs(30);
        while !met() {
            assert!(Instant::now() < deadline, "{what}");
            std::thread::sleep(Duration::from_millis(20));
        }
    };
    let sent = |needle: &[u8]| {
        let raw = raw.lock().unwrap();
        raw.windows(needle.len()).any(|w| w == needle)
    };
    wait("no pictures were sent", &|| sent(b"a=t,f=24"));
    // A press past the end of a line low in the window, a letter, and a save: the line the
    // letter is on is the line the press landed on.
    let press = b"\x1b[<0;701;431M\x1b[<0;701;431m";
    let marked = |letter: char| {
        let saved = std::fs::read_to_string(&file).unwrap_or_default();
        saved
            .lines()
            .find(|line| line.contains(letter))
            .map(str::to_string)
    };
    std::thread::sleep(Duration::from_millis(500));
    send(press);
    send(b"A\x13");
    wait("the first letter was never saved", &|| {
        marked('A').is_some()
    });
    let line = marked('A').unwrap();
    assert!(line.starts_with("line "), "{line}");

    // A row more, with the same pixels that aren't rows.
    let before = *answered.lock().unwrap();
    pty.master.resize(size(31)).unwrap();
    wait("the cell's size wasn't asked again", &|| {
        *answered.lock().unwrap() > before
    });
    std::thread::sleep(Duration::from_millis(700));
    send(press);
    send(b"Z\x13");
    wait("the second letter was never saved", &|| {
        marked('Z').is_some()
    });
    assert_eq!(
        marked('Z'),
        Some(format!("{line}Z")),
        "the same place in the window is the same line"
    );
    send(b"\x11");
    exits_well(&mut child, &raw, Duration::from_secs(30));
}

#[test]
fn pty_follows_a_resize() {
    let dir = project();
    let pty = native_pty_system()
        .openpty(PtySize {
            rows: 10,
            cols: 30,
            pixel_width: 0,
            pixel_height: 0,
        })
        .unwrap();
    let mut command = CommandBuilder::new(env!("CARGO_BIN_EXE_cellix"));
    command.cwd(dir.path());
    let mut child = pty.slave.spawn_command(command).unwrap();
    let reader = pty.master.try_clone_reader().unwrap();
    let input: Input = Arc::new(Mutex::new(pty.master.take_writer().unwrap()));
    let parser = Arc::new(Mutex::new(vt100::Parser::new(10, 120, 0)));
    let raw = Raw::default();
    pump(
        reader,
        parser.clone(),
        (input, raw.clone()),
        (Colour::default(), Answers::default()),
        Raw::default(),
    );
    let wait = |text: &str| {
        let deadline = Instant::now() + WAIT;
        while !parser.lock().unwrap().screen().contents().contains(text) {
            let wrote = String::from_utf8_lossy(&raw.lock().unwrap()).into_owned();
            assert!(
                Instant::now() < deadline,
                "{text:?} never appeared; wrote: {wrote:?}"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    };
    wait("Ctrl+P open");
    assert!(
        !parser
            .lock()
            .unwrap()
            .screen()
            .contents()
            .contains("Ctrl+Q quit")
    );
    pty.master
        .resize(PtySize {
            rows: 10,
            cols: 120,
            pixel_width: 0,
            pixel_height: 0,
        })
        .unwrap();
    wait("Ctrl+Q quit");
    let _ = child.kill();
}

#[test]
fn wait_blocks_until_closed() {
    let dir = project();
    let plan = dir.path().join("plan.md");
    std::fs::write(&plan, "# Plan\nstep\n").unwrap();
    let mut screen = Screen::start(dir.path(), &["--wait", "plan.md"]);
    screen.wait_for("step");
    assert!(screen.child.try_wait().unwrap().is_none());
    screen.send(b"\x1bc"); // Alt+C on line 1
    screen.wait_for("Comment on line 1");
    screen.send(b"nice\r");
    screen.wait_for("\u{25c6}");
    assert!(screen.child.try_wait().unwrap().is_none());
    screen.send(b"\x11");
    screen.wait_exit();
    let text = std::fs::read_to_string(plan).unwrap();
    assert!(text.ends_with("- Line 1: nice\n"), "{text}");
}

/// Windows has no hang-up signal.
#[test]
#[cfg(unix)]
fn a_hang_up_leaves_nothing_behind() {
    let dir = project();
    let mut screen = Screen::start(dir.path(), &["hello.txt"]);
    screen.wait_for("hello from a file");
    let socket_exists = || {
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_cellix"))
            .arg("comments")
            .current_dir(dir.path())
            .output()
            .unwrap();
        output.status.success()
    };
    assert!(socket_exists(), "the workspace socket should answer");
    let pid = screen.child.process_id().unwrap() as i32;
    // SAFETY: kill only sends a signal to the child this test started.
    assert_eq!(unsafe { libc::kill(pid, libc::SIGHUP) }, 0);
    screen.wait_exit();
    let status = screen.child.wait().unwrap();
    assert!(status.success(), "{status:?}");
    assert!(!socket_exists(), "the socket outlived Cellix");
}

/// Stops Cellix the way a closing terminal does: a stop, and the terminal gone straight
/// after. Gives how it exited, as `waitpid` says, or nothing where it ran on. It's tried a
/// few times, since the stop has to land while it waits for input. It used to read the
/// closed terminal forever.
#[cfg(unix)]
fn stopped_and_closed(dir: &Path) -> Option<i32> {
    use std::ffi::CString;
    let binary = CString::new(env!("CARGO_BIN_EXE_cellix")).unwrap();
    let folder = CString::new(dir.as_os_str().as_encoded_bytes()).unwrap();
    let argv = [binary.as_ptr(), std::ptr::null()];
    let mut master = 0;
    let mut size = libc::winsize {
        ws_row: 30,
        ws_col: 100,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    // SAFETY: the child only changes folder and execs, with everything it needs made before
    // the fork.
    let pid = unsafe {
        libc::forkpty(
            &mut master,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            // Linux takes the size as const and macOS as mut.
            (&raw mut size) as _,
        )
    };
    if pid == 0 {
        unsafe {
            libc::chdir(folder.as_ptr());
            libc::execv(binary.as_ptr(), argv.as_ptr());
            libc::_exit(127);
        }
    }
    assert!(pid > 0, "forkpty failed");
    // Read what it draws, so it never waits on a full terminal, until it's settled.
    let started = Instant::now();
    let mut buffer = [0u8; 65536];
    while started.elapsed() < Duration::from_millis(2500) {
        let mut ready = libc::pollfd {
            fd: master,
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: one pollfd, and a buffer as long as the read asks for.
        unsafe {
            if libc::poll(&mut ready, 1, 50) > 0 {
                libc::read(master, buffer.as_mut_ptr().cast(), buffer.len());
            }
        }
    }
    // SAFETY: the child this test started, and the terminal it opened for it.
    unsafe {
        libc::kill(pid, libc::SIGTERM);
        libc::close(master);
    }
    let deadline = Instant::now() + Duration::from_secs(6);
    while Instant::now() < deadline {
        let mut status = 0;
        // SAFETY: waits only on that child.
        if unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) } == pid {
            return Some(status);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    // SAFETY: as above; it's left running only if the test has already failed.
    unsafe {
        libc::kill(pid, libc::SIGKILL);
        libc::waitpid(pid, std::ptr::null_mut(), 0);
    }
    None
}

/// Windows has no stop signal to send, nor forkpty to send it through.
#[test]
#[cfg(unix)]
fn a_stop_as_the_terminal_closes_still_ends_cellix() {
    let dir = project();
    for attempt in 1..=3 {
        let status = stopped_and_closed(dir.path());
        let status = status.unwrap_or_else(|| panic!("attempt {attempt}: still running"));
        // Ended well, as a stop's end is, and not by a panic's 101 or a signal.
        let well = libc::WIFEXITED(status) && libc::WEXITSTATUS(status) == 0;
        assert!(
            well,
            "attempt {attempt}: ended with wait status {status:#x}"
        );
    }
}

/// The harness's terminal is sent pictures, so Cellix also says what's on screen in a file
/// when asked to. The tour waits on it rather than on a clock.
#[test]
fn the_screen_is_written_as_text_for_the_harness() {
    let dir = project();
    let told = tempfile::tempdir().unwrap();
    let file = told.path().join("screen.txt");
    let mut screen =
        Screen::start_with(dir.path(), &["hello.txt"], &[("CELLIX_SCREEN_TEXT", &file)]);
    screen.wait_for("hello from a file");
    let deadline = Instant::now() + WAIT;
    let said = || std::fs::read_to_string(&file).unwrap_or_default();
    while !said().contains("hello from a file") {
        assert!(Instant::now() < deadline, "the file says: {}", said());
        std::thread::sleep(Duration::from_millis(50));
    }
    // It follows the screen: the other file's words come once it's opened.
    let row = screen
        .contents()
        .lines()
        .position(|l| l.contains("other.txt"))
        .unwrap();
    screen.send(format!("\x1b[<0;6;{}M\x1b[<0;6;{}m", row + 1, row + 1).as_bytes());
    screen.wait_for("other contents");
    while !said().contains("other contents") {
        assert!(Instant::now() < deadline, "the file says: {}", said());
        std::thread::sleep(Duration::from_millis(50));
    }
    screen.send(b"\x11");
    screen.wait_exit();
}

/// `CELLIX_PROF` asks for a line a second of what the loop did: the first frames, drawn for
/// the window's size, what a key brings, and the bytes the terminal was sent.
#[test]
fn the_counters_say_what_the_loop_did() {
    let dir = project();
    let told = tempfile::tempdir().unwrap();
    let file = told.path().join("prof.log");
    let mut screen = Screen::start_with(dir.path(), &["hello.txt"], &[("CELLIX_PROF", &file)]);
    screen.wait_for("hello from a file");
    let sum = |key: &str| -> u64 {
        let text = std::fs::read_to_string(&file).unwrap_or_default();
        let words = text.lines().flat_map(|line| line.split(' '));
        let values = words.filter_map(|word| word.strip_prefix(&format!("{key}=")));
        values.filter_map(|value| value.parse::<u64>().ok()).sum()
    };
    let deadline = Instant::now() + WAIT;
    while sum("draws") == 0 || sum("bytes") == 0 {
        assert!(
            Instant::now() < deadline,
            "no counts: {:?}",
            std::fs::read_to_string(&file)
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(sum("wakes") >= sum("draws"), "a frame takes a wake");
    assert!(sum("resize") >= 1, "the first frame is the window's");
    assert!(sum("presents") >= 1);
    // A key that moves the caret is a frame drawn for input, and it's counted.
    screen.send(b"\x1b[B");
    while sum("input") == 0 {
        assert!(
            Instant::now() < deadline,
            "no input: {:?}",
            std::fs::read_to_string(&file)
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    screen.send(b"\x11");
    screen.wait_exit();
}

/// A window where nothing happens draws nothing and sends the terminal nothing: the tick
/// goes on looking at the disk, and draws only what it found changed.
#[test]
fn an_idle_window_draws_and_sends_nothing() {
    let dir = project();
    let told = tempfile::tempdir().unwrap();
    let file = told.path().join("prof.log");
    let mut screen = Screen::start_with(dir.path(), &["hello.txt"], &[("CELLIX_PROF", &file)]);
    screen.wait_for("hello from a file");
    let lines = || -> Vec<String> {
        let text = std::fs::read_to_string(&file).unwrap_or_default();
        text.lines().map(str::to_string).collect()
    };
    let field = |line: &str, key: &str| -> u64 {
        let word = line
            .split(' ')
            .find_map(|w| w.strip_prefix(&format!("{key}=")));
        word.and_then(|w| w.parse().ok()).unwrap_or(u64::MAX)
    };
    // The start, and the first looks at the disk, are past by the fifth second.
    let deadline = Instant::now() + WAIT;
    while lines().len() < 7 {
        assert!(Instant::now() < deadline, "too few lines: {:?}", lines());
        std::thread::sleep(Duration::from_millis(100));
    }
    for line in &lines()[4..7] {
        assert!(field(line, "wakes") > 0, "{line}");
        assert_eq!(field(line, "bytes"), 0, "{line}");
        assert_eq!(field(line, "sent"), 0, "{line}");
        assert!(field(line, "draws") <= 1, "only the heartbeat: {line}");
    }
    screen.send(b"\x11");
    screen.wait_exit();
}

/// A window where nothing happens wakes only when something is due: with nothing turning,
/// the tick, twice a second. It used to look every 50 ms in case anything had come, about
/// twenty wakes a second with nothing to do; now whatever comes wakes it.
#[test]
fn an_idle_window_wakes_only_when_something_is_due() {
    let dir = project();
    let told = tempfile::tempdir().unwrap();
    let file = told.path().join("prof.log");
    let mut screen = Screen::start_with(dir.path(), &["hello.txt"], &[("CELLIX_PROF", &file)]);
    screen.wait_for("hello from a file");
    let lines = || -> Vec<String> {
        let text = std::fs::read_to_string(&file).unwrap_or_default();
        text.lines().map(str::to_string).collect()
    };
    let field = |line: &str, key: &str| -> u64 {
        let word = line
            .split(' ')
            .find_map(|w| w.strip_prefix(&format!("{key}=")));
        word.and_then(|w| w.parse().ok()).unwrap_or(u64::MAX)
    };
    // The start, and the first looks at the disk, are past by the fifth line.
    let deadline = Instant::now() + WAIT * 2;
    while lines().len() < 9 {
        assert!(Instant::now() < deadline, "too few lines: {:?}", lines());
        std::thread::sleep(Duration::from_millis(100));
    }
    let idle = &lines()[4..9];
    let wakes: u64 = idle.iter().map(|line| field(line, "wakes")).sum();
    let ms: u64 = idle.iter().map(|line| field(line, "ms")).sum();
    // Each line is from one wake to the first a second or more after it.
    let rate = wakes as f64 * 1000.0 / ms as f64;
    assert!(rate <= 2.01, "{rate:.2} wakes a second: {idle:#?}");
    // What comes from outside still wakes it: an agent's request opens a file between ticks.
    let opened = std::process::Command::new(env!("CARGO_BIN_EXE_cellix"))
        .args(["open", "other.txt"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert!(opened.status.success(), "{opened:?}");
    screen.wait_for("other contents");
    screen.send(b"\x11");
    screen.wait_exit();
}

/// The tick looks at the disk on time however much input comes. With every pointer move
/// reported, a hand on the mouse held it off, and a file an agent wrote wasn't read again
/// until the hand was still.
#[test]
fn a_pointer_moving_all_the_while_doesnt_hold_off_the_tick() {
    let dir = project();
    let mut screen = Screen::start(dir.path(), &["hello.txt"]);
    screen.wait_for("hello from a file");
    std::thread::sleep(Duration::from_millis(50));
    std::fs::write(
        dir.path().join("hello.txt"),
        "written while the pointer moved\n",
    )
    .unwrap();
    let deadline = Instant::now() + WAIT;
    let mut column = 10;
    while !screen
        .contents()
        .contains("written while the pointer moved")
    {
        assert!(
            Instant::now() < deadline,
            "not read again while the pointer moved:\n{}",
            screen.contents()
        );
        column = if column == 10 { 11 } else { 10 };
        screen.send(format!("\x1b[<35;{column};5M").as_bytes());
        std::thread::sleep(Duration::from_millis(10));
    }
    screen.send(b"\x11");
    screen.wait_exit();
}

/// Closing the window is how a session often ends. Cellix finishes as it does on Quit: an
/// unsent comment is kept for next time, and the socket goes. On Windows that's a console
/// handler; elsewhere the hang-up.
#[test]
fn closing_the_terminal_keeps_unsent_comments() {
    let dir = project();
    let cellix = |args: &[&str]| {
        std::process::Command::new(env!("CARGO_BIN_EXE_cellix"))
            .args(args)
            .current_dir(dir.path())
            .output()
            .unwrap()
    };
    let mut screen = Screen::start(dir.path(), &["hello.txt"]);
    screen.wait_for("hello from a file");
    screen.send(b"\x1bc");
    screen.wait_for("Enter sends");
    screen.send(b"kept for later\r");
    // The comment itself, in the pane: the comments icon is a ◆ too, there from the start.
    screen.wait_for("kept for later");
    assert!(cellix(&["comments"]).status.success());
    screen.close();
    screen.wait_exit();
    let deadline = Instant::now() + WAIT;
    while cellix(&["comments"]).status.success() {
        assert!(Instant::now() < deadline, "the socket outlived Cellix");
        std::thread::sleep(Duration::from_millis(100));
    }
    drop(screen);

    let mut again = Screen::start(dir.path(), &["hello.txt"]);
    again.wait_for("from last time");
    let json = String::from_utf8(cellix(&["comments"]).stdout).unwrap();
    assert!(json.contains("kept for later"), "{json}");
    again.send(b"\x11");
    again.wait_exit();
}

/// A terminal can go with no hang-up reaching Cellix, when Cellix runs in a session of its
/// own. Its input then reads as ended for ever, and crossterm reads that for ever: one was
/// found a week later still using a whole core. Cellix sees its terminal is gone and ends.
///
/// Run only on macOS. On Linux, reading a terminal that has gone is an error, which ends the
/// loop, and Cellix doesn't start there without a terminal of its own. It's built and
/// listed there all the same, as ignored, so the ledger's check finds it on any Unix.
#[test]
#[cfg(unix)]
#[cfg_attr(
    not(target_os = "macos"),
    ignore = "the read that never ends is macOS's"
)]
fn a_terminal_that_goes_without_a_hang_up_still_ends_cellix() {
    use std::os::fd::{FromRawFd, OwnedFd};
    use std::os::unix::process::CommandExt;
    let dir = project();
    let cache = tempfile::tempdir_in("/tmp").unwrap();
    let (mut master, mut slave) = (0, 0);
    let mut size = libc::winsize {
        ws_row: 30,
        ws_col: 100,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    let (name, modes) = (std::ptr::null_mut(), std::ptr::null_mut());
    // A pointer, since macOS takes the size as one it may write through and Linux as one it
    // only reads: a pointer passes for both, where a `&mut` is one too many for Linux.
    let size: *mut libc::winsize = &mut size;
    // SAFETY: openpty fills in the two descriptors, which this test then owns and closes.
    assert_eq!(
        unsafe { libc::openpty(&mut master, &mut slave, name, modes, size) },
        0
    );
    // Cellix mustn't inherit the terminal's own end, or closing it here wouldn't close it.
    // SAFETY: sets a flag on the test's own descriptor.
    unsafe { libc::fcntl(master, libc::F_SETFD, libc::FD_CLOEXEC) };
    // SAFETY: each is a fresh copy of the terminal's end, owned by the command it's given to.
    let end = || std::process::Stdio::from(unsafe { OwnedFd::from_raw_fd(libc::dup(slave)) });
    let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_cellix"));
    command
        .arg("hello.txt")
        .current_dir(dir.path())
        .env("XDG_CACHE_HOME", cache.path())
        .stdin(end())
        .stdout(end())
        .stderr(end());
    // A session of its own: the terminal isn't its controlling terminal, so closing it sends
    // Cellix nothing.
    // SAFETY: setsid is safe between fork and exec.
    unsafe {
        command.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    let mut child = command.spawn().unwrap();
    // SAFETY: the test's own descriptors, used only here.
    unsafe {
        libc::close(slave);
        libc::fcntl(master, libc::F_SETFL, libc::O_NONBLOCK);
    }
    // What it draws is read, as a terminal would, until the file is up and a little longer:
    // by then it's waiting for keys.
    let started = Instant::now();
    let mut parser = vt100::Parser::new(30, 100, 0);
    let mut up = None;
    let mut buffer = [0u8; 65536];
    while up.is_none_or(|at: Instant| at.elapsed() < Duration::from_secs(1)) {
        if started.elapsed() > WAIT * 3 {
            let _ = child.kill();
            panic!("the file never showed: {}", parser.screen().contents());
        }
        // SAFETY: reads into a buffer of that length from the test's own descriptor.
        let read = unsafe { libc::read(master, buffer.as_mut_ptr().cast(), buffer.len()) };
        if read > 0 {
            parser.process(&buffer[..read as usize]);
            if up.is_none() && parser.screen().contents().contains("hello from a file") {
                up = Some(Instant::now());
            }
        } else {
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    assert!(child.try_wait().unwrap().is_none(), "Cellix is running");
    // SAFETY: the terminal goes, as when its window is closed.
    unsafe { libc::close(master) };
    let deadline = Instant::now() + WAIT;
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            panic!("Cellix ran on without its terminal");
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    // And it ends well, as a hang-up's does, rather than by a panic's 101.
    assert!(status.success(), "Cellix ended with {status:?}");
}

/// A dropped SSH connection hangs up on Cellix. The next Cellix in the folder has the same
/// tabs, with what was typed and never saved.
#[test]
#[cfg(unix)]
fn a_dropped_connection_loses_no_unsaved_edits() {
    let dir = project();
    let cache = tempfile::tempdir().unwrap();
    let env = [("XDG_CACHE_HOME", cache.path())];
    let mut screen = Screen::start_with(dir.path(), &["hello.txt"], &env);
    screen.wait_for("hello from a file");
    screen.send(b"KEPT ");
    screen.wait_for("KEPT hello");
    screen.close();
    screen.wait_exit();
    drop(screen);
    let on_disk = std::fs::read_to_string(dir.path().join("hello.txt")).unwrap();
    assert_eq!(
        on_disk, "hello from a file\n",
        "nothing was saved to the file"
    );

    let mut again = Screen::start_with(dir.path(), &[], &env);
    again.wait_for("KEPT hello from a file");
    again.wait_for("are back as you left them");
    again.send(b"\x13"); // Ctrl+S
    let deadline = Instant::now() + WAIT;
    while !std::fs::read_to_string(dir.path().join("hello.txt"))
        .unwrap()
        .starts_with("KEPT ")
    {
        assert!(
            Instant::now() < deadline,
            "the edits should save as any edits do"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    again.send(b"\x11");
    again.wait_exit();
}

#[cfg(unix)]
fn walk(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut found = Vec::new();
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() {
            found.extend(walk(&path));
        } else {
            found.push(path);
        }
    }
    found
}

/// Every workspace open when Cellix quits opens again next time, and `cellix ls` lists it,
/// even with nothing running in it: only those with something running used to come back.
/// Closing one from its tile lets it go.
#[test]
#[cfg(unix)]
fn every_open_workspace_comes_back_next_time() {
    let parent = tempfile::tempdir_in("/tmp").unwrap();
    let roots: Vec<_> = ["alpha", "beta", "gamma"]
        .iter()
        .map(|n| parent.path().join(n))
        .collect();
    for root in &roots {
        std::fs::create_dir(root).unwrap();
        std::fs::write(root.join("notes.txt"), "notes\n").unwrap();
    }
    // Short, since a socket's path has to fit in about a hundred bytes.
    let cache = tempfile::tempdir_in("/tmp").unwrap();
    let shell = Path::new("/bin/sh");
    let env = [("XDG_CACHE_HOME", cache.path()), ("SHELL", shell)];
    let cellix_ls = || {
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_cellix"))
            .arg("ls")
            .env("XDG_CACHE_HOME", cache.path())
            .output()
            .unwrap();
        String::from_utf8_lossy(&output.stdout).into_owned()
    };
    let bar = |screen: &Screen| screen.contents().lines().last().unwrap_or("").to_string();
    // Alpha has a terminal; beta and gamma, opened beside it, only their files.
    let mut screen = Screen::start_with(&roots[0], &[], &env);
    screen.wait_for("notes.txt");
    screen.send(b"\x0a"); // Ctrl+J, a terminal in the side pane
    screen.send(b"echo in-alpha-$((40+2))\r");
    screen.wait_for("in-alpha-42");
    for (index, name) in ["beta", "gamma"].iter().enumerate() {
        // The bar is drawn with the rest of the frame, which a busy machine can be late with.
        let deadline = Instant::now() + WAIT;
        // The button that opens a workspace: its mark, with its words after it where the
        // bar has a row of its own and alone where it leads the status bar's.
        while !bar(&screen).contains(" + ") {
            assert!(Instant::now() < deadline, "no bar: {}", screen.contents());
            std::thread::sleep(Duration::from_millis(50));
        }
        let line = bar(&screen);
        let column = line[..line.find(" + ").expect("the bar")].chars().count() + 2;
        let row = screen.contents().lines().count();
        screen.send(format!("\x1b[<0;{column};{row}M\x1b[<0;{column};{row}m").as_bytes());
        screen.wait_for("Folder to open");
        screen.send(roots[index + 1].display().to_string().as_bytes());
        std::thread::sleep(Duration::from_millis(500));
        screen.send(b"\r");
        let deadline = Instant::now() + WAIT;
        while !bar(&screen).contains(name) {
            assert!(
                Instant::now() < deadline,
                "no tile for {name}: {}",
                bar(&screen)
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }
    screen.send(b"\x11"); // Ctrl+Q, with nothing busy to ask about
    screen.wait_exit();
    drop(screen);
    let listed = cellix_ls();
    for name in ["alpha", "beta", "gamma"] {
        assert!(listed.contains(name), "{name} missing: {listed}");
    }
    assert!(listed.contains("open, nothing running"), "{listed}");

    // The next Cellix has all three, and closing one from its tile lets it go.
    let mut again = Screen::start_with(&roots[0], &[], &env);
    let deadline = Instant::now() + WAIT;
    while !(bar(&again).contains("beta") && bar(&again).contains("gamma")) {
        assert!(Instant::now() < deadline, "not all back: {}", bar(&again));
        std::thread::sleep(Duration::from_millis(50));
    }
    // Its close button shows once it's the workspace in front. The tiles settle as what each
    // is doing comes in, so the click is placed from the bar as it is, and made again until
    // it lands.
    let row = again.contents().lines().count();
    let deadline = Instant::now() + WAIT;
    let mut last_click = Instant::now() - Duration::from_secs(1);
    let column = loop {
        let line = bar(&again);
        let at = line.find("gamma").unwrap();
        if let Some(close) = line[at..].find('×') {
            break line[..at + close].chars().count() + 1;
        }
        assert!(
            Instant::now() < deadline,
            "no close on gamma: {line}\n{}",
            again.contents()
        );
        if last_click.elapsed() >= Duration::from_secs(1) {
            if again.contents().contains("Folder to open") {
                again.send(b"\x1b");
                std::thread::sleep(Duration::from_millis(300));
            }
            let line = bar(&again);
            let on = line[..line.find("gamma").unwrap()].chars().count() + 2;
            again.send(format!("\x1b[<0;{on};{row}M\x1b[<0;{on};{row}m").as_bytes());
            last_click = Instant::now();
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    again.send(format!("\x1b[<0;{column};{row}M\x1b[<0;{column};{row}m").as_bytes());
    let deadline = Instant::now() + WAIT;
    while bar(&again).contains("gamma") {
        assert!(
            Instant::now() < deadline,
            "gamma still open: {}",
            bar(&again)
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    again.send(b"\x11");
    again.wait_exit();
    let _ = std::process::Command::new("pkill")
        .args(["-f", &cache.path().display().to_string()])
        .status();
    let listed = cellix_ls();
    assert!(!listed.contains("gamma"), "{listed}");
    assert!(listed.contains("beta"), "{listed}");
}

/// The screen's last row, where the workspaces are.
#[cfg(unix)]
fn foot(screen: &Screen) -> String {
    screen.contents().lines().last().unwrap_or("").to_string()
}

/// Opens `root` beside the workspaces a Cellix has, from the button in its bar, and waits for
/// its tile.
#[cfg(unix)]
fn open_beside(screen: &mut Screen, root: &Path) {
    let name = root.file_name().unwrap().to_string_lossy().into_owned();
    // The bar is drawn with the rest of the frame, which a busy machine can be late with.
    let deadline = Instant::now() + WAIT;
    while !foot(screen).contains(" + ") {
        assert!(Instant::now() < deadline, "no bar: {}", screen.contents());
        std::thread::sleep(Duration::from_millis(50));
    }
    let line = foot(screen);
    let column = line[..line.find(" + ").expect("the bar")].chars().count() + 2;
    let row = screen.contents().lines().count();
    screen.send(format!("\x1b[<0;{column};{row}M\x1b[<0;{column};{row}m").as_bytes());
    screen.wait_for("Folder to open");
    screen.send(root.display().to_string().as_bytes());
    std::thread::sleep(Duration::from_millis(500));
    screen.send(b"\r");
    let deadline = Instant::now() + WAIT;
    while !foot(screen).contains(&name) {
        assert!(
            Instant::now() < deadline,
            "no tile for {name}: {}",
            foot(screen)
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// A Cellix that's killed, or cut off with its connection, never gets to say what it had
/// open. A workspace is noted as open while it's open, so the next Cellix has every one of
/// them and `cellix ls` lists them. Only what had something running used to come back: after
/// a dropped SSH connection the first screen offered one workspace of several.
#[test]
#[cfg(unix)]
fn workspaces_open_when_cellix_dies_come_back() {
    let parent = tempfile::tempdir_in("/tmp").unwrap();
    let roots: Vec<_> = ["alpha", "beta", "gamma"]
        .iter()
        .map(|n| parent.path().join(n))
        .collect();
    for root in &roots {
        std::fs::create_dir(root).unwrap();
        std::fs::write(root.join("notes.txt"), "notes\n").unwrap();
    }
    // Short, since a socket's path has to fit in about a hundred bytes.
    let cache = tempfile::tempdir_in("/tmp").unwrap();
    let home = tempfile::tempdir_in("/tmp").unwrap();
    let env = [
        ("XDG_CACHE_HOME", cache.path()),
        ("SHELL", Path::new("/bin/sh")),
        ("HOME", home.path()),
    ];
    let cellix_ls = || {
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_cellix"))
            .arg("ls")
            .env("XDG_CACHE_HOME", cache.path())
            .output()
            .unwrap();
        String::from_utf8_lossy(&output.stdout).into_owned()
    };
    // Nothing runs in any of the three: they're only open.
    let mut screen = Screen::start_with(&roots[0], &[], &env);
    screen.wait_for("notes.txt");
    for root in &roots[1..] {
        open_beside(&mut screen, root);
    }
    let pid = screen.child.process_id().unwrap() as i32;
    // SAFETY: kill only sends a signal to the child this test started.
    assert_eq!(unsafe { libc::kill(pid, libc::SIGKILL) }, 0);
    // Ended by the kill, which nothing outlives, and not by anything of its own.
    let status = screen.wait_end();
    let killed = status.signal().is_some_and(|s| s.starts_with("Killed"));
    assert!(killed, "it ended with {status:?}, not by the kill");
    drop(screen);
    let listed = cellix_ls();
    for name in ["alpha", "beta", "gamma"] {
        assert!(listed.contains(name), "{name} missing: {listed}");
    }

    // Started at home, the next Cellix offers all three to pick from.
    let mut again = Screen::start_with(home.path(), &[], &env);
    again.wait_for("3 workspaces are open");
    for name in ["alpha", "beta", "gamma"] {
        again.wait_for(name);
    }
    again.send(b"\x11");
    again.wait_exit();
}

/// Work left running in one project is a tile in the next Cellix's bar, as a tmux window
/// would be, and `cellix ls` lists it.
#[test]
#[cfg(unix)]
fn work_left_running_is_in_the_next_cellixs_bar() {
    let parent = tempfile::tempdir().unwrap();
    let (alpha, beta) = (parent.path().join("alpha"), parent.path().join("beta"));
    for root in [&alpha, &beta] {
        std::fs::create_dir(root).unwrap();
        std::fs::write(root.join("notes.txt"), "notes\n").unwrap();
    }
    // Short, since a socket's path has to fit in about a hundred bytes.
    let cache = tempfile::tempdir_in("/tmp").unwrap();
    let shell = Path::new("/bin/sh");
    let env = [("XDG_CACHE_HOME", cache.path()), ("SHELL", shell)];
    let cellix_ls = || {
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_cellix"))
            .arg("ls")
            .env("XDG_CACHE_HOME", cache.path())
            .output()
            .unwrap();
        String::from_utf8_lossy(&output.stdout).into_owned()
    };
    assert!(cellix_ls().contains("Nothing is running"));

    let mut screen = Screen::start_with(&alpha, &[], &env);
    screen.wait_for("notes.txt");
    screen.send(b"\x0a"); // Ctrl+J, a terminal in the side pane
    screen.send(b"echo marker-$((40+2))\r");
    screen.wait_for("marker-42");
    screen.send(b"sleep 600\r");
    std::thread::sleep(Duration::from_millis(700));
    screen.close();
    screen.wait_exit();
    drop(screen);
    let listed = cellix_ls();
    let files = walk(cache.path());
    assert!(
        listed.contains("alpha") && listed.contains("Terminal"),
        "{listed} {files:#?}"
    );

    let mut other = Screen::start_with(&beta, &[], &env);
    other.wait_for("notes.txt");
    let bar = |screen: &Screen| screen.contents().lines().last().unwrap_or("").to_string();
    let deadline = Instant::now() + WAIT;
    while !bar(&other).contains("alpha working") {
        assert!(
            Instant::now() < deadline,
            "no tile for alpha: {:?}",
            bar(&other)
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(bar(&other).contains("beta"), "{}", bar(&other));
    let line = bar(&other);
    let column = line[..line.find("alpha").unwrap()].chars().count() + 1;
    other.send(format!("\x1b[<0;{column};30M\x1b[<0;{column};30m").as_bytes());
    other.wait_for("marker-42");

    // The shell is still the one left running: it never stopped.
    let _ = std::process::Command::new("pkill")
        .args(["-f", &cache.path().display().to_string()])
        .status();
    other.send(b"\x11");
}

/// The terminal turns dark while Cellix is away, as a Mac does at sunset, and Cellix follows
/// when it comes back into focus.
#[test]
#[cfg(unix)]
fn coming_back_to_a_terminal_that_turned_dark_turns_cellix_dark() {
    let dir = project();
    let colour = Colour::default();
    *colour.lock().unwrap() = Some("ffff/ffff/ffff".to_string());
    let mut screen = Screen::start_answering(dir.path(), &["hello.txt"], &[], colour.clone());
    screen.wait_for("hello from a file");
    // The text can show before the rest of the frame has arrived, so it waits for the cell's
    // colour rather than taking it as it first is.
    let brightness = |screen: &Screen| {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let cell = {
                let parser = screen.parser.lock().unwrap();
                parser.screen().cell(10, 60).map(|c| c.bgcolor())
            };
            match cell {
                Some(vt100::Color::Rgb(r, g, b)) => {
                    return u32::from(r) + u32::from(g) + u32::from(b);
                }
                other => assert!(Instant::now() < deadline, "no colour: {other:?}"),
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    };
    assert!(
        brightness(&screen) > 600,
        "light at first: {}",
        brightness(&screen)
    );

    *colour.lock().unwrap() = Some("1e1e/1e1e/2e2e".to_string());
    screen.send(b"\x1b[I"); // focus comes back
    let deadline = Instant::now() + WAIT;
    while brightness(&screen) > 300 {
        assert!(
            Instant::now() < deadline,
            "still light: {}",
            brightness(&screen)
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    // Typing still works after the question.
    screen.send(b"x");
    screen.wait_for("xhello from a file");
    screen.send(b"\x11");
}

/// Waits until the status bar's column moves off where it is at the start of the line, and
/// gives it: the place the caret has once what came before the key that moved it is read.
#[cfg(unix)]
fn column_moved(screen: &mut Screen) -> usize {
    let deadline = Instant::now() + WAIT;
    loop {
        let contents = screen.contents();
        let column = contents.split("Ln 1, Col ").nth(1).and_then(|rest| {
            let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
            digits.parse().ok()
        });
        if let Some(column) = column.filter(|&column| column != 1) {
            return column;
        }
        if Instant::now() > deadline {
            let story = screen.story();
            panic!("the caret never moved:\n{contents}\n{story}");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// An answer the terminal sends after Cellix has stopped waiting for it is the terminal's and
/// never keys. On a loaded machine the background's answer came late and was typed into the
/// open file as `11;rgb:1010/1111/1414`, and its tab showed it changed. Here the terminal
/// starts its answer about the colour in time and ends it late, then answers late in full,
/// as a slow link does, and the same again once Cellix asks on coming back into focus.
/// Nothing is typed, nothing changes, and a key typed right after the answers arrives.
#[test]
#[cfg(unix)]
fn pty_an_answer_that_comes_late_is_never_typed() {
    let dir = project();
    // Asked its colour, the terminal gets as far as this before the wait runs out.
    let started = (b"]11;?".to_vec(), b"\x1b]11;rgb:10".to_vec());
    let mut screen = Screen::start_with_answers(dir.path(), &["hello.txt"], vec![started]);
    screen.wait_for("hello from a file");
    screen.wait_for("Ln 1, Col 1");
    assert!(!screen.contents().contains('●'), "{}", screen.contents());
    // The rest of that answer, and of the others asked with it.
    screen.send(b"10/1111/1414\x1b\\\x1b[6;37;17t\x1b[?1016;2$y\x1b[?62;22c");
    // Answers in full, as to questions asked again, ended both ways an OSC can end.
    screen.send(b"\x1b]11;rgb:1010/1111/1414\x1b\\\x1b[6;37;17t\x1b[?62;22c");
    screen.send(b"\x1b]11;rgb:1010/1111/1414\x07\x1b[?62;22c");
    // The graphics answers, where the wait had read as far as the mode's report before them.
    screen.send(b"\x1b_Gi=31;OK\x1b\\\x1b_Gi=32;EINVAL:refused\x1b\\\x1b[?5522;2$y\x1b[?62;22c");
    // End: the caret goes to the end of the line, as far as the line is long.
    screen.send(b"\x1b[F");
    let column = column_moved(&mut screen);
    let contents = screen.contents();
    assert_eq!(column, 18, "the line is as it was:\n{contents}");
    assert!(!contents.contains("rgb"), "{contents}");
    assert!(
        !contents.contains('●'),
        "the tab shows no change:\n{contents}"
    );

    // Back in focus, Cellix asks the colour again, and the answer starts and ends late again.
    let asked = |screen: &Screen| {
        let all = screen.all.lock().unwrap();
        let at = all.windows(5).rposition(|w| w == b"]11;?");
        let count = all.windows(5).filter(|w| *w == b"]11;?").count();
        // The pointer's shape is said in the frame drawn after it, once the wait has run out.
        let drawn = at.is_some_and(|at| all[at..].windows(4).any(|w| w == b"]22;"));
        count >= 2 && drawn
    };
    screen.send(b"\x1b[I");
    let deadline = Instant::now() + WAIT;
    while !asked(&screen) {
        assert!(
            Instant::now() < deadline,
            "never asked again: {}",
            screen.story()
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    // The rest of it, and a key right after.
    screen.send(b"10/1111/1414\x1b\\\x1b[?62;22cx");
    screen.wait_for("hello from a filex");
    screen.send(b"\x13"); // Ctrl+S
    screen.wait_for("Saved hello.txt");
    let contents = screen.contents();
    assert!(!contents.contains("rgb"), "{contents}");
    screen.send(b"\x11");
    screen.wait_exit();
    let saved = std::fs::read_to_string(dir.path().join("hello.txt")).unwrap();
    assert_eq!(saved, "hello from a filex\n");
}

/// How bright the editor's background is drawn, red, green and blue added up, once it's drawn.
#[cfg(unix)]
fn editor_brightness(screen: &Screen) -> u32 {
    let deadline = Instant::now() + WAIT;
    loop {
        let cell = {
            let parser = screen.parser.lock().unwrap();
            parser.screen().cell(10, 60).map(|c| c.bgcolor())
        };
        match cell {
            Some(vt100::Color::Rgb(r, g, b)) => return u32::from(r) + u32::from(g) + u32::from(b),
            other => assert!(Instant::now() < deadline, "no colour: {other:?}"),
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// What a late answer says of the colour still counts: started in a terminal that doesn't
/// answer in time, Cellix keeps its own dark colours, and turns light when the answer comes.
#[test]
#[cfg(unix)]
fn pty_a_late_answer_about_the_colour_still_counts() {
    let dir = project();
    let mut screen = Screen::start(dir.path(), &["hello.txt"]);
    screen.wait_for("hello from a file");
    let dark = editor_brightness(&screen);
    assert!(dark < 300, "dark at first: {dark}");
    screen.send(b"\x1b]11;rgb:ffff/ffff/ffff\x1b\\\x1b[?62;22c");
    let deadline = Instant::now() + WAIT;
    while editor_brightness(&screen) < 600 {
        assert!(
            Instant::now() < deadline,
            "still dark: {}",
            screen.contents()
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    screen.send(b"x");
    screen.wait_for("xhello from a file");
    assert!(!screen.contents().contains("rgb"), "{}", screen.contents());
    screen.send(b"\x11");
}

#[test]
fn cli_reads_comments_over_socket() {
    let dir = project();
    let mut screen = Screen::start(dir.path(), &["hello.txt"]);
    screen.wait_for("hello from a file");
    screen.send(b"\x1bc");
    screen.wait_for("Enter sends");
    screen.send(b"from a human\r");
    // The box closing is the comment kept. A ◆ isn't: on the cell tier the Comments icon at
    // the top right is one, there from the start, so the CLI could ask before Enter landed.
    screen.wait_gone("Enter sends");
    let cli = |args: &[&str]| {
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_cellix"))
            .args(args)
            .current_dir(dir.path())
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        String::from_utf8(output.stdout).unwrap()
    };
    let json = cli(&["comments"]);
    assert!(json.contains(r#""file":"hello.txt""#), "{json}");
    assert!(json.contains(r#""text":"from a human""#), "{json}");
    assert!(cli(&["comments", "--take"]).contains("from a human"));
    assert_eq!(cli(&["comments"]).trim(), "[]");
    cli(&["open", "other.txt"]);
    screen.wait_for("other contents");
    screen.send(b"\x11");
    screen.wait_exit();
}

/// Everything Cellix has written, as text.
#[cfg_attr(not(unix), allow(dead_code))]
fn written(screen: &Screen) -> String {
    String::from_utf8_lossy(&screen.all.lock().unwrap()).into_owned()
}

/// Has the Cellix in `dir` hear from an agent through its hook, as Claude Code's would.
#[cfg_attr(not(unix), allow(dead_code))]
fn hook(dir: &Path, cache: &Path, kind: &str, input: &str) {
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_cellix"))
        .args(["hook", kind])
        .current_dir(dir)
        .env("XDG_CACHE_HOME", cache)
        .stdin(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    assert!(child.wait().unwrap().success());
}

#[test]
#[cfg(unix)]
fn an_agent_asking_while_cellix_is_behind_becomes_a_terminal_notification() {
    use std::os::unix::fs::PermissionsExt;
    let dir = project();
    let cache = tempfile::Builder::new()
        .prefix("cxn")
        .tempdir_in("/tmp")
        .unwrap();
    let root = dir.path().canonicalize().unwrap();
    // The agent has a tab here: one running outside Cellix, with nothing here to answer it
    // in, isn't listed, counted or rung for. A stand-in for Claude Code that waits.
    let bin = tempfile::tempdir().unwrap();
    let claude = bin.path().join("claude");
    std::fs::write(
        &claude,
        "#!/bin/sh\necho 'stand-in claude'\nexec sleep 60\n",
    )
    .unwrap();
    std::fs::set_permissions(&claude, std::fs::Permissions::from_mode(0o755)).unwrap();
    let cellix = Path::new(env!("CARGO_BIN_EXE_cellix")).parent().unwrap();
    let path = std::env::join_paths([bin.path(), cellix, Path::new("/usr/bin"), Path::new("/bin")])
        .unwrap();
    let path = std::path::PathBuf::from(path);
    let mut screen = Screen::start_with(
        dir.path(),
        &["hello.txt"],
        &[
            ("XDG_CACHE_HOME", cache.path()),
            ("TERM_PROGRAM", Path::new("ghostty")),
            ("TMUX", Path::new("")),
            ("PATH", path.as_path()),
        ],
    );
    screen.wait_for("hello from a file");
    screen.send(b"\x1b[112;6u");
    screen.wait_for("Type to search");
    screen.send(b"Open Claude Code");
    screen.wait_for("Agent: Open Claude Code");
    std::thread::sleep(Duration::from_millis(200));
    screen.send(b"\r");
    screen.wait_for("stand-in claude");
    let asking = format!(
        r#"{{"message":"Claude needs your permission to use Bash","cwd":{:?},"session_id":"s1"}}"#,
        root.to_str().unwrap()
    );
    let expected = "\x1b]777;notify;Cellix";
    // In front, the person sees it in Cellix, and the terminal isn't asked to say it.
    hook(dir.path(), cache.path(), "notify", &asking);
    screen.wait_for("needs you");
    std::thread::sleep(Duration::from_millis(500));
    assert!(!written(&screen).contains(expected), "told while in front");

    // Behind another window, it's the terminal's own notification, through OSC 777 for
    // Ghostty, saying where and what.
    screen.send(b"\x1b[O");
    let finished = format!(
        r#"{{"cwd":{:?},"session_id":"s1"}}"#,
        root.to_str().unwrap()
    );
    hook(dir.path(), cache.path(), "stop", &finished);
    let name = root.file_name().unwrap().to_string_lossy().into_owned();
    let wanted = format!("\x1b]777;notify;Cellix · {name};Claude Code finished its turn\x1b\\");
    let deadline = Instant::now() + WAIT;
    while !written(&screen).contains(&wanted) {
        assert!(
            Instant::now() < deadline,
            "no notification: {:?}",
            written(&screen).rsplit('\n').next()
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(
        written(&screen).matches(expected).count(),
        1,
        "one per event"
    );
    // Quit from the file, stopping the agent's stand-in.
    let press = |screen: &mut Screen, words: &str| {
        let contents = screen.contents();
        let (row, column) = contents
            .lines()
            .enumerate()
            .find_map(|(row, line)| Some((row, line[..line.find(words)?].chars().count())))
            .unwrap_or_else(|| panic!("no {words}: {contents}"));
        let (x, y) = (column + 2, row + 1);
        screen.send(format!("\x1b[<0;{x};{y}M\x1b[<0;{x};{y}m").as_bytes());
    };
    press(&mut screen, "hello from a file");
    screen.send(b"\x11");
    screen.wait_for("Stop and Quit");
    press(&mut screen, "Stop and Quit");
    screen.wait_exit();
}

/// Copilot CLI and Cursor's agent aren't installed here, so stand-ins for them are: each runs
/// the hooks `cellix setup` installed for it, as its own agent's docs say it runs them, with
/// what they say it sends, and prints that it ran. Started from the Agents list as the real
/// one would be, under its session host, what the hooks told Cellix shows in the list, and
/// the session's id is left beside the host for Resume.
#[test]
#[cfg(unix)]
fn stand_ins_for_copilot_and_cursor_report_through_the_hooks_setup_installed() {
    use std::os::unix::fs::PermissionsExt;
    let dir = project();
    let home = tempfile::tempdir().unwrap();
    let bin = home.path().join("bin");
    std::fs::create_dir(&bin).unwrap();
    for agent in ["copilot", "cursor"] {
        let setup = std::process::Command::new(env!("CARGO_BIN_EXE_cellix"))
            .args(["setup", agent])
            .env("HOME", home.path())
            .env_remove("COPILOT_HOME")
            .output()
            .unwrap();
        assert!(setup.status.success(), "{setup:?}");
    }
    // Each stand-in finds its hook's command in the file setup wrote, by the event's name, and
    // runs it in a shell with the event's input, as the agent does.
    let stand_in = |name: &str, hooks: &str, key: &str, events: &[(&str, String)]| {
        let mut script = format!(
            "#!/bin/sh\nhooks=\"$HOME/{hooks}\"\n\
             run() {{ cmd=$(sed -n \"/\\\"$1\\\"/,/\\\"{key}\\\"/p\" \"$hooks\" | \
             sed -n 's/.*\"{key}\": \"\\(.*\\)\",$/\\1/p' | head -1 | sed 's/\\\\\"/\"/g'); \
             printf '%s' \"$2\" | sh -c \"$cmd\"; }}\n\
             echo 'stand-in {name} ran'\n"
        );
        for (event, input) in events {
            script.push_str(&format!("run {event} '{input}'\n"));
        }
        script.push_str("echo 'stand-in hooks ran'\nsleep 60\n");
        let path = bin.join(name);
        std::fs::write(&path, script).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    };
    let root = dir.path().canonicalize().unwrap().display().to_string();
    stand_in(
        "copilot",
        ".copilot/hooks/cellix.json",
        "bash",
        &[
            (
                "SessionStart",
                format!(r#"{{"session_id":"cp-1","cwd":"{root}","source":"startup"}}"#),
            ),
            (
                "UserPromptSubmit",
                format!(r#"{{"session_id":"cp-1","cwd":"{root}","prompt":"Fix it"}}"#),
            ),
            (
                "Notification",
                format!(
                    r#"{{"session_id":"cp-1","cwd":"{root}","notification_type":"permission_prompt","message":"Allow cargo publish?"}}"#
                ),
            ),
        ],
    );
    stand_in(
        "cursor-agent",
        ".cursor/hooks.json",
        "command",
        &[
            (
                "sessionStart",
                format!(
                    r#"{{"session_id":"cu-1","conversation_id":"cu-1","workspace_roots":["{root}"]}}"#
                ),
            ),
            (
                "beforeSubmitPrompt",
                format!(
                    r#"{{"conversation_id":"cu-1","workspace_roots":["{root}"],"prompt":"Fix it"}}"#
                ),
            ),
            (
                "stop",
                format!(
                    r#"{{"conversation_id":"cu-1","workspace_roots":["{root}"],"status":"completed"}}"#
                ),
            ),
        ],
    );
    let cellix = Path::new(env!("CARGO_BIN_EXE_cellix")).parent().unwrap();
    let path = std::env::join_paths([
        bin.as_path(),
        cellix,
        Path::new("/usr/bin"),
        Path::new("/bin"),
    ])
    .unwrap();
    let path = std::path::PathBuf::from(path);
    // A cache of the test's own, short enough for a socket's path, to find the hosts in.
    let cache = tempfile::tempdir_in("/tmp").unwrap();
    let env = [
        ("HOME", home.path()),
        ("PATH", path.as_path()),
        ("XDG_CACHE_HOME", cache.path()),
    ];
    let mut screen = Screen::start_with(dir.path(), &[], &env);
    screen.wait_for("other.txt");
    // The Agents list, through the palette, waited for until the palette has gone.
    let list_agents = |screen: &mut Screen| {
        screen.send(b"\x1b[112;6u");
        screen.wait_for("Type to search");
        screen.send(b"List Agents");
        screen.wait_for("Agent: List Agents");
        std::thread::sleep(Duration::from_millis(200));
        screen.send(b"\r");
        screen.wait_gone("Type to search");
    };
    // The list stays in front after each agent's news, as List Agents again would close it.
    // It stands agents by urgency: Copilot CLI asking, with what it asks under it, and Cursor
    // CLI, its turn finished with its tab in front, idle, no longer under Working.
    list_agents(&mut screen);
    for (name, says) in [
        ("Copilot CLI", "Allow cargo publish?"),
        ("Cursor CLI", "IDLE 1"),
    ] {
        screen.wait_for(name);
        let contents = screen.contents();
        let (row, column) = contents
            .lines()
            .enumerate()
            .filter_map(|(row, line)| Some((row, line[..line.find(name)?].chars().count())))
            .last()
            .unwrap_or_else(|| panic!("no {name} to start: {contents}"));
        let (x, y) = (column + 2, row + 1);
        screen.send(format!("\x1b[<0;{x};{y}M\x1b[<0;{x};{y}m").as_bytes());
        screen.wait_for("stand-in hooks ran");
        list_agents(&mut screen);
        screen.wait_for(says);
        screen.wait_gone("WORKING");
    }
    // Each one's session is left beside its host, which is what Resume reads.
    let left: Vec<String> = walk(cache.path())
        .into_iter()
        .filter(|p| p.extension().is_some_and(|e| e == "agent"))
        .map(|p| std::fs::read_to_string(p).unwrap())
        .collect();
    for (agent, session) in [("Copilot", "cp-1"), ("Cursor", "cu-1")] {
        let word = format!(r#""agent":"{agent}","session":"{session}""#);
        assert!(left.iter().any(|l| l.contains(&word)), "{word} in {left:?}");
    }
    screen.send(b"\x11");
}

/// A stand-in agent for `cellix agent`: it says it's up and answers each line it's given, and
/// told `ask` it says through Cellix's real hook, as Codex's would, that it's asking the person
/// leave to run something. It runs the hook through a shell, as agents do.
#[cfg(unix)]
const FAKE_AGENT: &str = r#"#!/bin/sh
printf 'fake agent up\n'
while IFS= read -r line; do
  printf 'got: %s\n' "$line"
  if [ "$line" = ask ]; then
    sh -c 'printf "{\"session_id\":\"f1\",\"cwd\":\"%s\",\"notification_type\":\"permission_prompt\",\"message\":\"Allow Bash?\"}" "$PWD" | cellix hook --agent Codex notify'
    printf 'asked\n'
  fi
done
"#;

/// `cellix agent` with the real binary on both ends: it finds the Cellix that has the folder
/// open, the person says yes in a card first, what's sent and read crosses the socket, and a
/// program's real hook saying it's asking keeps a message from answering for the person.
#[test]
#[cfg(unix)]
fn pty_agents_start_and_message_each_other_through_the_socket() {
    use std::os::unix::fs::PermissionsExt;
    use std::process::{Command, Stdio};
    let dir = project();
    let root = dir.path().canonicalize().unwrap();
    let fake = root.join("fake-agent");
    std::fs::write(&fake, FAKE_AGENT).unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    // Short, since a socket's path has to fit in about a hundred bytes.
    let cache = tempfile::tempdir_in("/tmp").unwrap();
    let agent = |args: &[&str], folder: &Path| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_cellix"));
        command
            .arg("agent")
            .args(args)
            .current_dir(folder)
            .env("XDG_CACHE_HOME", cache.path())
            .stdin(Stdio::null());
        // Run as an agent outside Cellix would be: no terminal, no session of its own.
        for name in ["CELLIX_NAME", "CELLIX_HELD", "CLAUDECODE", "GEMINI_CLI"] {
            command.env_remove(name);
        }
        command.output().unwrap()
    };
    let text = |out: &std::process::Output| {
        let said = String::from_utf8_lossy(&out.stdout).into_owned();
        said + &String::from_utf8_lossy(&out.stderr)
    };

    // With no Cellix here, it says so and what to do.
    let nobody = agent(&["ls"], &root);
    assert_eq!(nobody.status.code(), Some(1));
    assert!(
        text(&nobody).contains("no Cellix has this folder open"),
        "{}",
        text(&nobody)
    );

    let mut screen = Screen::start_with(&root, &[], &[("XDG_CACHE_HOME", cache.path())]);
    screen.wait_for("hello.txt");
    let fake_path = fake.display().to_string();
    let args = [
        "run",
        "--name",
        "helper",
        "--task",
        "hi",
        fake_path.as_str(),
    ];
    let started = std::thread::scope(|scope| {
        let starting = scope.spawn(|| agent(&args, &root));
        // Asked first, in a card.
        screen.wait_for("An agent wants to start fake-agent");
        // Allow is the last of the card's buttons, after Always Allow Here.
        screen.wait_for("Always Allow Here");
        let contents = screen.contents();
        let row = contents
            .lines()
            .position(|l| l.contains("Always Allow Here"))
            .unwrap();
        let line = contents.lines().nth(row).unwrap();
        let column = line[..line.rfind("Allow").unwrap()].chars().count() + 1;
        let (row, column) = (row + 1, column + 1);
        screen.send(format!("\x1b[<0;{column};{row}M\x1b[<0;{column};{row}m").as_bytes());
        screen.wait_gone("wants to start");
        starting.join().unwrap()
    });
    assert_eq!(started.status.code(), Some(0), "{}", text(&started));
    let json: serde_json::Value = serde_json::from_slice(&started.stdout).unwrap();
    assert_eq!(json["name"], "helper");
    assert_eq!(json["task_sent"], true);

    // What it was given reached it, and what it said comes back.
    let deadline = Instant::now() + WAIT;
    loop {
        let read = agent(&["read", "helper"], &root);
        if text(&read).contains("got: hi") {
            break;
        }
        assert!(Instant::now() < deadline, "never read: {}", text(&read));
        std::thread::sleep(Duration::from_millis(100));
    }
    let listed = agent(&["ls"], &root);
    let list: serde_json::Value = serde_json::from_slice(&listed.stdout).unwrap();
    assert_eq!(list[0]["name"], "helper", "{list}");

    // Told to ask, it reports through the real hook that it's asking the person, and then a
    // message to it is refused with nothing typed.
    let sent = agent(&["send", "helper", "ask"], &root);
    assert_eq!(sent.status.code(), Some(0), "{}", text(&sent));
    let deadline = Instant::now() + WAIT;
    loop {
        let listed = agent(&["ls"], &root);
        if text(&listed).contains("\"asking\"") {
            break;
        }
        assert!(Instant::now() < deadline, "never asking: {}", text(&listed));
        std::thread::sleep(Duration::from_millis(100));
    }
    let refused = agent(&["send", "helper", "rm everything"], &root);
    assert_eq!(refused.status.code(), Some(4), "{}", text(&refused));
    assert!(text(&refused).contains("Allow Bash?"), "{}", text(&refused));
    let read = agent(&["read", "helper"], &root);
    assert!(!text(&read).contains("rm everything"), "{}", text(&read));

    let stopped = agent(&["stop", "helper"], &root);
    assert_eq!(stopped.status.code(), Some(0), "{}", text(&stopped));
    drop(screen);
    let _ = Command::new("pkill")
        .args(["-f", &cache.path().display().to_string()])
        .status();
}

/// What `harness/live/term.py`, a headless terminal that takes Kitty graphics as Ghostty does,
/// printed while it ran the real binary on `hello.txt` in `dir` through `steps`, each a JSON
/// list. Nothing in the environment says there's a connection, a multiplexer or Claude Code
/// between, as for a person's Ghostty, whatever runs the tests; `env` comes on top.
#[cfg(unix)]
fn headless(dir: &Path, steps: &[String], env: &[(&str, &Path)]) -> String {
    let scratch = tempfile::tempdir().unwrap();
    let script = scratch.path().join("script.json");
    std::fs::write(&script, format!("[{}]", steps.join(", "))).unwrap();
    let term = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../harness/live/term.py");
    let mut command = std::process::Command::new("python3");
    command
        .arg("-I")
        .arg(term)
        .arg(&script)
        .arg(env!("CARGO_BIN_EXE_cellix"))
        .arg("hello.txt")
        .current_dir(dir);
    for name in [
        "SSH_CONNECTION",
        "SSH_CLIENT",
        "SSH_TTY",
        "TMUX",
        "STY",
        "ZELLIJ",
        "CLAUDECODE",
        "CELLIX_MOD",
        "CELLIX_TILE_FILES",
        "CELLIX_TIER",
    ] {
        command.env_remove(name);
    }
    command
        .env("CELLIX_MACHINE", "dev-box")
        .env("SHELL", "/bin/sh")
        .env("TERM_SIZE", "100x26")
        .env("TERM_ROW", "50")
        .env("CELLIX_ROW_FIT", "0");
    for (name, value) in env {
        command.env(name, value);
    }
    let output = command
        .output()
        .expect("python3 runs the headless terminal");
    let printed = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(
        output.status.success(),
        "{printed}{}",
        String::from_utf8_lossy(&output.stderr)
    );
    printed
}

/// What a `["bytes", label]` step said: (bytes written, files taken, the files' bytes).
#[cfg(unix)]
fn counted(printed: &str, label: &str) -> (usize, usize, usize) {
    let line = printed
        .lines()
        .find_map(|line| line.strip_prefix(&format!("{label}: ")))
        .unwrap_or_else(|| panic!("no {label:?} in {printed}"));
    let numbers: Vec<usize> = line
        .split(' ')
        .filter_map(|word| word.parse().ok())
        .collect();
    (numbers[0], numbers[1], numbers[2])
}

/// In Ghostty on this machine, tiles are temporary files the terminal reads and deletes, so a
/// frame of new tiles writes their names rather than their pixels. The headless terminal takes
/// them as Ghostty does: the first paint and a resize look exactly as they do inline, for a
/// fraction of the bytes, and nothing is left in the temp folder once Cellix has gone.
#[test]
#[cfg(unix)]
fn tiles_go_as_files_to_ghostty_here_and_look_the_same_for_fewer_bytes() {
    use std::os::unix::fs::PermissionsExt;
    let dir = project();
    let temp = tempfile::Builder::new()
        .prefix("cxf")
        .tempdir_in("/tmp")
        .unwrap();
    // The user's own temp folder, as macOS gives each user one: private.
    let own = temp.path().join("tmp");
    std::fs::create_dir(&own).unwrap();
    std::fs::set_permissions(&own, std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut seen = Vec::new();
    for (run, switch) in [("files", None), ("inline", Some("0"))] {
        let shot = |name: &str| temp.path().join(format!("{run}-{name}.png"));
        let steps = [
            r#"["until", "hello from a file"]"#.to_string(),
            format!(r#"["shot", {:?}]"#, shot("first")),
            r#"["bytes", "first paint"]"#.to_string(),
            r#"["resize", 80, 22]"#.to_string(),
            r#"["until", "hello from a file"]"#.to_string(),
            format!(r#"["shot", {:?}]"#, shot("resized")),
            r#"["bytes", "resize"]"#.to_string(),
        ];
        let cache = temp.path().join(format!("{run}-cache"));
        let text = temp.path().join(format!("{run}-screen.txt"));
        let mut env = vec![
            ("TMPDIR", own.as_path()),
            ("XDG_CACHE_HOME", cache.as_path()),
            ("CELLIX_SCREEN_TEXT", text.as_path()),
        ];
        if let Some(switch) = switch {
            env.push(("CELLIX_TILE_FILES", Path::new(switch)));
        }
        let printed = headless(dir.path(), &steps, &env);
        let pictures = [
            std::fs::read(shot("first")).unwrap(),
            std::fs::read(shot("resized")).unwrap(),
        ];
        seen.push((
            counted(&printed, "first paint"),
            counted(&printed, "resize"),
            pictures,
        ));
    }
    let (filed, inline) = (&seen[0], &seen[1]);
    eprintln!(
        "first paint: {} bytes written with files ({} files of {} bytes), {} inline; \
         resize: {} with files ({} files of {} bytes), {} inline",
        filed.0.0, filed.0.1, filed.0.2, inline.0.0, filed.1.0, filed.1.1, filed.1.2, inline.1.0
    );
    assert!(filed.0.1 > 0 && filed.1.1 > 0, "tiles went as files");
    assert_eq!(
        (inline.0.1, inline.1.1),
        (0, 0),
        "and not with the switch off"
    );
    assert!(filed.0.0 * 3 < inline.0.0, "first paint");
    assert!(filed.1.0 * 3 < inline.1.0, "resize");
    assert!(filed.2 == inline.2, "the same pictures either way");
    // The terminal took every file, and Cellix cleared its own folder as it ended: what's
    // left is the user's folder, empty.
    let ours: Vec<_> = std::fs::read_dir(&own)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.to_string_lossy().contains("cellix-tiles-"))
        .collect();
    assert_eq!(ours.len(), 1, "{ours:?}");
    let left: Vec<_> = std::fs::read_dir(&ours[0]).unwrap().collect();
    assert!(left.is_empty(), "left in {:?}: {left:?}", ours[0]);
}

/// `cellix reply <id> --done "…"`, as an agent runs it, with the real binary on both ends: the
/// running Cellix keeps the comment marked addressed by that agent, still open for the
/// reviewer to settle.
#[test]
#[cfg(unix)]
fn pty_an_agent_says_it_addressed_a_comment() {
    use std::process::{Command, Stdio};
    let dir = project();
    let root = dir.path().canonicalize().unwrap();
    std::fs::create_dir_all(root.join(".cellix")).unwrap();
    let thread =
        r#"[{"id":1,"file":"hello.txt","line":1,"quote":"hello","text":"Say goodbye too."}]"#;
    std::fs::write(root.join(".cellix/threads.json"), thread).unwrap();
    // Short, since a socket's path has to fit in about a hundred bytes.
    let cache = tempfile::tempdir_in("/tmp").unwrap();
    let mut screen = Screen::start_with(&root, &[], &[("XDG_CACHE_HOME", cache.path())]);
    screen.wait_for("hello.txt");
    let done = Command::new(env!("CARGO_BIN_EXE_cellix"))
        .args(["reply", "1", "--done", "added", "a", "goodbye"])
        .current_dir(&root)
        .env("XDG_CACHE_HOME", cache.path())
        .env("CELLIX_AGENT", "Codex")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    let said = String::from_utf8_lossy(&done.stderr).into_owned();
    assert!(done.status.success(), "{said}");
    let deadline = Instant::now() + WAIT;
    loop {
        let kept = std::fs::read_to_string(root.join(".cellix/threads.json")).unwrap_or_default();
        if kept.contains("Addressed: added a goodbye") {
            assert!(kept.contains("\"author\": \"Codex\""), "{kept}");
            assert!(kept.contains("\"resolved\": false"), "{kept}");
            break;
        }
        assert!(Instant::now() < deadline, "never kept: {kept}");
        std::thread::sleep(Duration::from_millis(100));
    }
    drop(screen);
    let _ = Command::new("pkill")
        .args(["-f", &cache.path().display().to_string()])
        .status();
}

/// `cellix comments --waiting`, as the Claude Code mod runs it, against the real binary: a
/// page's comment no agent has had is listed, as `cellix comments` (the review's on code
/// alone) never listed it, and one already told isn't; asking takes nothing.
#[test]
#[cfg(unix)]
fn pty_comments_waiting_lists_a_pages_comments() {
    use std::process::{Command, Stdio};
    let dir = project();
    let root = dir.path().canonicalize().unwrap();
    std::fs::write(root.join("notes.md"), "# Notes\n\nThe cache goes first.\n").unwrap();
    std::fs::create_dir_all(root.join(".cellix")).unwrap();
    let threads = r#"[
      {"id":1,"file":"notes.md","line":3,"quote":"cache","text":"Why first?"},
      {"id":2,"file":"notes.md","line":3,"quote":"first","text":"Told already.","told":0}
    ]"#;
    std::fs::write(root.join(".cellix/threads.json"), threads).unwrap();
    // Short, since a socket's path has to fit in about a hundred bytes.
    let cache = tempfile::tempdir_in("/tmp").unwrap();
    let mut screen = Screen::start_with(&root, &[], &[("XDG_CACHE_HOME", cache.path())]);
    screen.wait_for("hello.txt");
    let comments = |args: &[&str]| {
        let out = Command::new(env!("CARGO_BIN_EXE_cellix"))
            .arg("comments")
            .args(args)
            .current_dir(&root)
            .env("XDG_CACHE_HOME", cache.path())
            .stdin(Stdio::null())
            .output()
            .unwrap();
        let said = String::from_utf8_lossy(&out.stderr).into_owned();
        assert!(out.status.success(), "{said}");
        String::from_utf8_lossy(&out.stdout).into_owned()
    };
    assert_eq!(comments(&[]).trim(), "[]", "the review's alone");
    let waiting = comments(&["--waiting"]);
    assert!(waiting.contains("Why first?"), "{waiting}");
    assert!(!waiting.contains("Told already."), "{waiting}");
    assert_eq!(comments(&["--waiting"]), waiting, "read, not taken");
    drop(screen);
    let _ = Command::new("pkill")
        .args(["-f", &cache.path().display().to_string()])
        .status();
}
