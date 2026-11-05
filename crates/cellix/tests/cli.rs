use std::process::Command;

#[test]
fn prints_version() {
    let output = Command::new(env!("CARGO_BIN_EXE_cellix"))
        .arg("--version")
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert_eq!(
        stdout.trim(),
        format!("cellix {}", env!("CARGO_PKG_VERSION"))
    );
}

fn git(dir: &std::path::Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .unwrap();
    assert!(output.status.success(), "git {args:?}");
    String::from_utf8(output.stdout).unwrap()
}

/// The prompt hook checkpoints in a process of its own and comes back at once, so an agent's
/// time limit on hooks can't drop the comments it carries. A large untracked file, like a
/// project's data, stays out of the checkpoint and out of `.git`.
#[test]
fn the_prompt_hook_checkpoints_without_holding_the_prompt() {
    use std::io::Write;
    use std::time::{Duration, Instant};

    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    git(root, &["init", "-q"]);
    std::fs::write(root.join("kept.txt"), "kept\n").unwrap();
    git(root, &["add", "kept.txt"]);
    git(
        root,
        &[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "commit",
            "-qm",
            "init",
        ],
    );
    std::fs::write(root.join("notes.txt"), "small\n").unwrap();
    std::fs::write(root.join("dump.bin"), vec![b'x'; 12 * 1024 * 1024]).unwrap();

    let input = format!(
        r#"{{"prompt":"Fix the bug","cwd":{:?},"session_id":"s"}}"#,
        root.to_str().unwrap()
    );
    let started = Instant::now();
    let mut child = Command::new(env!("CARGO_BIN_EXE_cellix"))
        .args(["hook", "prompt"])
        .current_dir(root)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    assert!(child.wait().unwrap().success());
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "{:?}",
        started.elapsed()
    );

    // The checkpoint lands, from the process the hook left it to.
    let deadline = Instant::now() + Duration::from_secs(30);
    let turns = loop {
        let turns = git(
            root,
            &[
                "for-each-ref",
                "--format=%(objectname)",
                "refs/cellix/turns/",
            ],
        );
        if !turns.trim().is_empty() || Instant::now() > deadline {
            break turns;
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    let commit = turns
        .lines()
        .next()
        .expect("the prompt hook made no checkpoint");
    let files = git(root, &["ls-tree", "-r", "--name-only", commit]);
    assert!(files.lines().any(|f| f == "notes.txt"), "{files}");
    assert!(!files.lines().any(|f| f == "dump.bin"), "{files}");
}

/// One frame of cells: its columns, rows and each cell's character.
type CellFrame = (u16, u16, Vec<char>);

/// Ends a process however the test ends.
struct KillOnDrop(std::process::Child);

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// One frame of cells as `cellix pane` writes it.
fn decode_cells(line: &str) -> CellFrame {
    use base64::Engine;
    let mut parts = line.split(' ');
    assert_eq!(parts.next(), Some("cells"));
    let columns: u16 = parts.next().unwrap().parse().unwrap();
    let rows: u16 = parts.next().unwrap().parse().unwrap();
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(parts.next().unwrap())
        .unwrap();
    assert_eq!(bytes.len(), columns as usize * rows as usize * 12);
    let chars = bytes
        .chunks(12)
        .map(|cell| {
            let code = u32::from_le_bytes(cell[..4].try_into().unwrap());
            // What Claude Code's Raster takes, or it refuses the whole frame.
            assert!(code <= 0xFFFF, "outside the BMP: {code:#x}");
            let c = char::from_u32(code).unwrap();
            assert!(!c.is_control(), "a control character: {code:#x}");
            c
        })
        .collect();
    (columns, rows, chars)
}

/// Over SSH the pane inside Claude Code sends its frames as cells, where it sent a picture
/// of the whole pane each time the pointer moved, which blinked and held up clicks. This
/// runs the real pane as the mod does: input goes in batches through `pane-send`.
#[test]
fn over_ssh_the_pane_sends_cells_and_takes_input_in_batches() {
    use std::io::BufRead;
    use std::process::Stdio;
    use std::time::{Duration, Instant};

    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("work");
    let home = dir.path().join("home");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(root.join("plan.md"), "Cells cross the wire\n").unwrap();
    let cellix = || {
        let mut command = Command::new(env!("CARGO_BIN_EXE_cellix"));
        command
            .current_dir(&root)
            .env("HOME", &home)
            .env("XDG_CONFIG_HOME", home.join(".config"))
            .env("XDG_CACHE_HOME", home.join(".cache"))
            .env("XDG_DATA_HOME", home.join(".local/share"))
            .env("CELLIX_MOD", cellix::pane::PROTOCOL)
            .env("SSH_CONNECTION", "10.0.0.1 50000 10.0.0.2 22");
        command
    };
    let mut pane = KillOnDrop(
        cellix()
            .arg("pane")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let stdout = pane.0.stdout.take().unwrap();
    let (sender, lines) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for line in std::io::BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            if sender.send(line).is_err() {
                break;
            }
        }
    });
    let next_frame = |what: &dyn Fn(&CellFrame) -> bool| {
        let deadline = Instant::now() + Duration::from_secs(30);
        while Instant::now() < deadline {
            let Ok(line) = lines.recv_timeout(Duration::from_millis(200)) else {
                continue;
            };
            assert!(
                !line.starts_with("frame ") && !line.starts_with("png "),
                "a picture"
            );
            if line.starts_with("cells ") {
                let frame = decode_cells(&line);
                if what(&frame) {
                    return Some(frame);
                }
            }
        }
        None
    };
    let first = next_frame(&|_| true).expect("a first frame of cells");
    assert_eq!((first.0, first.1), (100, 40));
    let batch = format!(
        r#"[{{"type":"resize","columns":72,"rows":24}},{{"type":"pointer","kind":"move","x":3,"y":3}},{{"type":"pointer","kind":"move","x":4,"y":3}},{{"type":"open","path":{:?}}}]"#,
        root.join("plan.md").to_str().unwrap()
    );
    // The socket may still be coming up, as the mod finds.
    let deadline = Instant::now() + Duration::from_secs(10);
    while !cellix()
        .args(["pane-send", &batch])
        .status()
        .unwrap()
        .success()
    {
        assert!(Instant::now() < deadline, "pane-send never got through");
        std::thread::sleep(Duration::from_millis(100));
    }
    let shown = next_frame(&|(columns, rows, chars)| {
        let text: String = chars.iter().collect();
        (*columns, *rows) == (72, 24) && text.contains("Cells cross the wire")
    });
    assert!(
        shown.is_some(),
        "the batch was taken and the file shown, as cells"
    );
}

/// An upgrade replaces the binary but used to leave the plugin it wrote, so the old mod waited
/// on "Starting Cellix…" for lines it couldn't read. Now the pane writes the new plugin and
/// says to restart Claude Code, which the old mod shows.
#[test]
fn a_mod_left_by_an_upgrade_is_replaced_and_told_to_restart() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let done = Command::new(env!("CARGO_BIN_EXE_cellix"))
        .arg("pane")
        .current_dir(dir.path())
        .env("HOME", dir.path())
        .env("XDG_DATA_HOME", &data)
        .env_remove("CELLIX_MOD")
        .stdin(std::process::Stdio::null())
        .output()
        .unwrap();
    assert!(!done.status.success());
    let said = String::from_utf8_lossy(&done.stderr);
    assert!(said.contains("Restart Claude Code"), "{said}");
    assert!(done.stdout.is_empty(), "no lines an old mod could misread");
    let written =
        std::fs::read_to_string(data.join("cellix/claude-code/hooks/register.tsx")).unwrap();
    assert!(written.contains(&format!("const PROTOCOL = '{}'", cellix::pane::PROTOCOL)));
}
