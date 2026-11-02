//! Runs the real binary in a PTY, types and clicks like a terminal would, and reads
//! the screen back through a terminal emulator.

use std::io::{Read, Write};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use portable_pty::{CommandBuilder, PtySize, native_pty_system};

struct Screen {
    parser: Arc<Mutex<vt100::Parser>>,
    input: Box<dyn Write + Send>,
    child: Box<dyn portable_pty::Child + Send + Sync>,
}

impl Screen {
    fn start(dir: &Path, args: &[&str]) -> Self {
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
        let child = pty.slave.spawn_command(command).unwrap();
        let mut reader = pty.master.try_clone_reader().unwrap();
        let input = pty.master.take_writer().unwrap();
        let parser = Arc::new(Mutex::new(vt100::Parser::new(30, 100, 0)));
        let sink = parser.clone();
        std::thread::spawn(move || {
            let mut buf = [0; 8192];
            while let Ok(n) = reader.read(&mut buf) {
                if n == 0 {
                    break;
                }
                sink.lock().unwrap().process(&buf[..n]);
            }
        });
        Self {
            parser,
            input,
            child,
        }
    }

    fn contents(&self) -> String {
        self.parser.lock().unwrap().screen().contents()
    }

    fn wait_for(&self, text: &str) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            if self.contents().contains(text) {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("{text:?} never appeared:\n{}", self.contents());
    }

    fn send(&mut self, bytes: &[u8]) {
        self.input.write_all(bytes).unwrap();
        self.input.flush().unwrap();
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

#[test]
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
    screen.send(b"\x1b[200~pasted\ttext\x1b[201~");
    screen.wait_for("pasted");
    screen.send(b"\x11");
    screen.wait_for("Save changes to");
    screen.send(b"n\r");
    screen.wait_exit();
}

#[test]
fn pty_ctrl_shift_p_and_cmd_p_work_as_in_vs_code() {
    let dir = project();
    let mut screen = Screen::start(dir.path(), &[]);
    screen.wait_for("other.txt");
    screen.send(b"\x1b[112;6u"); // Ctrl+Shift+P, Kitty keyboard protocol
    screen.wait_for("Toggle Sidebar");
    screen.send(b"\x1b");
    std::thread::sleep(Duration::from_millis(200));
    screen.send(b"\x1b[112;9u"); // Cmd+P
    screen.wait_for("Type to search");
    screen.send(b"\x1b");
    std::thread::sleep(Duration::from_millis(200));
    screen.send(b"\x11");
    screen.wait_exit();
}

#[test]
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
/// with real fonts. Returns once it has sent a tile with this size marker.
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
fn pty_pixel_tier_sends_tiles_when_the_terminal_supports_graphics() {
    expect_tiles(
        (40 * 10, 12 * 20),
        b"\x1b_Gi=31;OK\x1b\\\x1b[?62c",
        b"s=400,v=20",
    );
}

/// Zellij can report no pixels in the window size but answers `CSI 16t`.
#[test]
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
    let mut reader = pty.master.try_clone_reader().unwrap();
    let parser = Arc::new(Mutex::new(vt100::Parser::new(10, 120, 0)));
    let sink = parser.clone();
    std::thread::spawn(move || {
        let mut buf = [0; 8192];
        while let Ok(n) = reader.read(&mut buf) {
            if n == 0 {
                break;
            }
            sink.lock().unwrap().process(&buf[..n]);
        }
    });
    let wait = |text: &str| {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !parser.lock().unwrap().screen().contents().contains(text) {
            assert!(Instant::now() < deadline, "{text:?} never appeared");
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

#[test]
fn cli_reads_comments_over_socket() {
    let dir = project();
    let mut screen = Screen::start(dir.path(), &["hello.txt"]);
    screen.wait_for("hello from a file");
    screen.send(b"\x1bc");
    screen.wait_for("Enter sends");
    screen.send(b"from a human\r");
    screen.wait_for("\u{25c6}");
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
