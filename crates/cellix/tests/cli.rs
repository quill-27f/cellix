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
