//! The real binary inside real multiplexers. Run with `harness/x mux`.

use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

struct Tmux {
    socket: String,
}

impl Tmux {
    fn start(dir: &Path, name: &str) -> Self {
        Self::start_with(dir, name, &[])
    }

    /// Starts with server options set first, as `set -g` lines in a tmux.conf would.
    fn start_with(dir: &Path, name: &str, options: &[&[&str]]) -> Self {
        let tmux = Self {
            socket: format!("cellix-test-{}-{name}", std::process::id()),
        };
        let cellix = env!("CARGO_BIN_EXE_cellix");
        let mut args = vec!["start-server"];
        for option in options {
            args.push(";");
            args.extend_from_slice(option);
        }
        args.push(";");
        tmux.run(
            &[
                &args[..],
                &[
                    "new-session",
                    "-d",
                    "-x",
                    "100",
                    "-y",
                    "30",
                    "-c",
                    dir.to_str().unwrap(),
                    &format!("{cellix} hello.txt"),
                ],
            ]
            .concat(),
        );
        tmux
    }

    fn run(&self, args: &[&str]) -> String {
        let output = Command::new("tmux")
            // Your own tmux.conf can renumber windows and panes.
            .args(["-f", "/dev/null", "-L", &self.socket])
            .args(args)
            .output()
            .expect("tmux must be installed");
        assert!(output.status.success(), "tmux {args:?}: {output:?}");
        String::from_utf8_lossy(&output.stdout).into_owned()
    }

    fn pane(&self, target: &str) -> String {
        self.run(&["capture-pane", "-p", "-t", target])
    }

    fn wait(&self, target: &str, text: &str) {
        let deadline = Instant::now() + Duration::from_secs(15);
        while !self.pane(target).contains(text) {
            assert!(
                Instant::now() < deadline,
                "{text:?} not in pane:\n{}",
                self.pane(target)
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

impl Drop for Tmux {
    fn drop(&mut self) {
        let _ = Command::new("tmux")
            .args(["-L", &self.socket, "kill-server"])
            .output();
    }
}

fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("hello.txt"), "hello from a file\n").unwrap();
    dir
}

#[test]
#[ignore = "needs tmux"]
fn tmux_typing_and_saving_work() {
    let dir = project();
    let tmux = Tmux::start(dir.path(), "edit");
    tmux.wait("0", "hello from a file");
    tmux.run(&["send-keys", "-t", "0", "abc "]);
    tmux.wait("0", "abc hello from a file");
    tmux.run(&["send-keys", "-t", "0", "C-s"]);
    tmux.wait("0", "Saved hello.txt");
    let saved = std::fs::read_to_string(dir.path().join("hello.txt")).unwrap();
    assert_eq!(saved, "abc hello from a file\n");
}

#[test]
#[ignore = "needs tmux"]
fn ctrl_keys_arrive_with_tmux_extended_keys_on() {
    // As the README sets it up, with tmux's own default, the xterm form, for the keys.
    let dir = project();
    let options: &[&[&str]] = &[&["set", "-g", "extended-keys", "on"]];
    let tmux = Tmux::start_with(dir.path(), "extended", options);
    tmux.wait("0", "hello from a file");
    tmux.run(&["send-keys", "-t", "0", "abc "]);
    tmux.wait("0", "abc hello from a file");
    tmux.run(&["send-keys", "-t", "0", "C-s"]);
    tmux.wait("0", "Saved hello.txt");
}

#[test]
#[ignore = "needs tmux"]
fn tmux_split_resize_and_pane_switch_keep_the_screen() {
    let dir = project();
    let tmux = Tmux::start(dir.path(), "split");
    tmux.wait("0", "hello from a file");
    tmux.run(&["split-window", "-h", "-t", "0", "cat"]);
    tmux.wait("0:0.0", "hello.txt");
    tmux.run(&["select-pane", "-t", "0:0.1"]);
    tmux.run(&["select-pane", "-t", "0:0.0"]);
    tmux.run(&["resize-window", "-x", "140", "-y", "40"]);
    tmux.wait("0:0.0", "hello.txt");
    tmux.run(&["kill-pane", "-t", "0:0.1"]);
    // The status line's far end shows only once the whole width is drawn again.
    tmux.wait("0", "Ln 1, Col 1");
    tmux.run(&["send-keys", "-t", "0", "x"]);
    tmux.wait("0", "xhello from a file");
}

#[test]
#[ignore = "needs tmux"]
fn tmux_survives_a_second_window_and_coming_back() {
    let dir = project();
    let tmux = Tmux::start(dir.path(), "window");
    tmux.wait("0", "hello from a file");
    tmux.run(&["new-window", "-d", "cat"]);
    tmux.run(&["select-window", "-t", "1"]);
    tmux.run(&["select-window", "-t", "0"]);
    tmux.wait("0", "hello from a file");
    tmux.run(&["send-keys", "-t", "0", "y"]);
    tmux.wait("0", "yhello from a file");
}
