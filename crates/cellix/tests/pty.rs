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

/// How long a screen waits for text. Windows gets longer: a debug build there can take many
/// seconds to start while it's scanned, with several starting at once.
const WAIT: Duration = Duration::from_secs(if cfg!(windows) { 40 } else { 10 });

/// Feeds what Cellix draws to `parser`, answering a cursor position query as a terminal would.
/// Windows' ConPTY asks one before it passes anything on, and waits for the answer. An answer
/// that comes too soon can be lost, so it's given again until something else arrives.
fn pump(
    mut reader: Box<dyn Read + Send>,
    parser: Arc<Mutex<vt100::Parser>>,
    (input, raw): (Input, Raw),
    colour: Colour,
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
            command.env("XDG_CACHE_HOME", format!("/tmp/cxt-{:x}", hasher.finish()));
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
            colour,
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

    /// What Cellix wrote first, and whether it's still running, for a failure to show.
    fn story(&mut self) -> String {
        let raw = self.raw.lock().unwrap();
        let status = self.child.try_wait();
        format!(
            "exited: {status:?}, wrote: {:?}",
            String::from_utf8_lossy(&raw)
        )
    }

    fn contents(&self) -> String {
        self.parser.lock().unwrap().screen().contents()
    }

    fn wait_for(&mut self, text: &str) {
        let deadline = Instant::now() + WAIT;
        while Instant::now() < deadline {
            if self.contents().contains(text) {
                return;
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

    fn wait_exit(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            if self.child.try_wait().unwrap().is_some() {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("cellix did not exit");
    }
}

impl Drop for Screen {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
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
    screen.wait_for("Toggle Sidebar");
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
    let deadline = Instant::now() + Duration::from_secs(5);
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
    let deadline = Instant::now() + Duration::from_secs(5);
    while screen.parser.lock().unwrap().screen().alternate_screen() {
        assert!(Instant::now() < deadline, "still on the alternate screen");
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
    let deadline = Instant::now() + Duration::from_secs(10);
    while !contains(b"a=q") {
        assert!(Instant::now() < deadline, "no graphics query");
        std::thread::sleep(Duration::from_millis(5));
    }
    input.write_all(reply).unwrap();
    input.flush().unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    while !(contains(b"a=t,f=24") && contains(tile_marker)) {
        assert!(Instant::now() < deadline, "no tiles sent");
        std::thread::sleep(Duration::from_millis(20));
    }
    input.write_all(b"\x11").unwrap();
    input.flush().unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while child.try_wait().unwrap().is_none() {
        assert!(Instant::now() < deadline, "cellix did not exit");
        std::thread::sleep(Duration::from_millis(20));
    }
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
        Colour::default(),
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
/// after. Gives whether it exited, after a few tries, since the stop has to land while it
/// waits for input. It used to read the closed terminal forever.
#[cfg(unix)]
fn stopped_and_closed(dir: &Path) -> bool {
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
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    // SAFETY: as above; it's left running only if the test has already failed.
    unsafe {
        libc::kill(pid, libc::SIGKILL);
        libc::waitpid(pid, std::ptr::null_mut(), 0);
    }
    false
}

/// Windows has no stop signal to send, nor forkpty to send it through.
#[test]
#[cfg(unix)]
fn a_stop_as_the_terminal_closes_still_ends_cellix() {
    let dir = project();
    for attempt in 1..=3 {
        assert!(
            stopped_and_closed(dir.path()),
            "attempt {attempt}: still running"
        );
    }
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
    screen.wait_for("\u{25c6}");
    assert!(cellix(&["comments"]).status.success());
    screen.close();
    screen.wait_exit();
    let deadline = Instant::now() + Duration::from_secs(5);
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
        let line = bar(&screen);
        let column = line[..line.find("New Workspace").expect("the bar")]
            .chars()
            .count()
            + 2;
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
    let dir = project();
    let cache = tempfile::Builder::new()
        .prefix("cxn")
        .tempdir_in("/tmp")
        .unwrap();
    let root = dir.path().canonicalize().unwrap();
    let mut screen = Screen::start_with(
        dir.path(),
        &["hello.txt"],
        &[
            ("XDG_CACHE_HOME", cache.path()),
            ("TERM_PROGRAM", Path::new("ghostty")),
            ("TMUX", Path::new("")),
        ],
    );
    screen.wait_for("hello from a file");
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
    screen.send(b"\x11");
    screen.wait_exit();
}
