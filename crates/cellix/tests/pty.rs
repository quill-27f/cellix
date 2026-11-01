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
