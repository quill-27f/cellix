//! Sessions that outlive Cellix, through the real binary acting as their host. Unix only:
//! Windows has no host yet.
#![cfg(unix)]

use std::io::Write;
use std::path::Path;
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

use cellix::hold::{self, Held, Job};

fn job(name: &str, script: &str, folder: &Path) -> Job {
    Job {
        name: name.to_string(),
        argv: vec!["sh".to_string(), "-c".to_string(), script.to_string()],
        env: vec!["CELLIX_MARK=seen".to_string()],
        folder: folder.to_path_buf(),
        size: (80, 24),
        pixels: (0, 0),
    }
}

fn start(folder: &Path, job: &Job) -> Held {
    hold::start(Path::new(env!("CARGO_BIN_EXE_cellix")), folder, job).unwrap()
}

fn attach(held: &Held) -> (Receiver<Vec<u8>>, Receiver<()>) {
    let (output, seen) = channel();
    let (done, over) = channel();
    held.listen(output, done).unwrap();
    (seen, over)
}

fn read_until(seen: &Receiver<Vec<u8>>, wanted: &str) -> String {
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut text = String::new();
    while Instant::now() < deadline {
        if let Ok(bytes) = seen.recv_timeout(Duration::from_millis(100)) {
            text.push_str(&String::from_utf8_lossy(&bytes));
            if text.contains(wanted) {
                return text;
            }
        }
    }
    panic!("no {wanted} in: {text}");
}

#[test]
fn a_program_runs_on_with_nobody_attached_and_shows_what_it_did() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("held");
    let script = "echo first $CELLIX_MARK; read line; echo got $line; sleep 30";
    let held = start(&folder, &job("Shell", script, dir.path()));
    let (seen, _) = attach(&held);
    read_until(&seen, "first seen");
    held.keys().unwrap().write_all(b"hello\n").unwrap();
    read_until(&seen, "got hello");
    let socket = held.socket.clone();
    drop(seen);
    drop(held);

    // A new Cellix looks in the same folder, and finds it with what it wrote.
    let running = hold::left_running(&folder);
    assert_eq!(running.len(), 1);
    assert_eq!(running[0].0, "Shell");
    let again = Held::connect(&socket).unwrap();
    let (seen, _) = attach(&again);
    let text = read_until(&seen, "got hello");
    assert!(text.contains("first seen"), "{text}");
    again.kill();
}

#[test]
fn a_program_that_ends_while_nobody_is_attached_is_told_about_later() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("held");
    let held = start(&folder, &job("Job", "echo finished; exit 0", dir.path()));
    let socket = held.socket.clone();
    drop(held);
    std::thread::sleep(Duration::from_millis(500));
    let again = Held::connect(&socket).unwrap();
    let (seen, over) = attach(&again);
    read_until(&seen, "finished");
    over.recv_timeout(Duration::from_secs(5)).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !hold::left_running(&folder).is_empty() {
        assert!(Instant::now() < deadline, "the host should have gone");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn stopping_a_held_program_ends_it() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("held");
    let held = start(&folder, &job("Long", "echo up; sleep 60", dir.path()));
    let (seen, over) = attach(&held);
    read_until(&seen, "up");
    held.kill();
    over.recv_timeout(Duration::from_secs(5)).unwrap();
}

#[test]
fn a_new_size_reaches_the_program() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("held");
    let script = "trap 'stty size' WINCH; echo ready; sleep 5 & wait; sleep 5";
    let held = start(&folder, &job("Size", script, dir.path()));
    let (seen, _) = attach(&held);
    read_until(&seen, "ready");
    held.resize((100, 30), (1000, 600));
    read_until(&seen, "30 100");
    held.kill();
}

/// A host is reached through its socket. Once that's gone, as when the folder holding it is
/// cleared, nobody can attach again, so the host ends its program and itself rather than
/// run on unseen: dozens of these were found days after the tests that started them.
#[test]
fn a_host_whose_socket_is_gone_ends_its_program() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("held");
    let held = start(&folder, &job("Left", "echo up; sleep 600", dir.path()));
    let (seen, _over) = attach(&held);
    read_until(&seen, "up");
    let pid = std::fs::read_to_string(held.socket.with_extension("pid")).unwrap();
    let pid: i32 = pid.trim().parse().unwrap();
    // SAFETY: signal 0 only asks whether the process is there.
    let running = || unsafe { libc::kill(pid, 0) } == 0;
    assert!(running());
    std::fs::remove_dir_all(&folder).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while running() {
        assert!(Instant::now() < deadline, "the program ran on unreachable");
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn a_dead_host_leaves_nothing_behind() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("held");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("9-9.sock"), "").unwrap();
    std::fs::write(folder.join("9-9.json"), "{}").unwrap();
    assert!(hold::left_running(&folder).is_empty());
    assert_eq!(std::fs::read_dir(&folder).unwrap().count(), 0);
}

#[test]
fn a_host_that_dies_with_the_machine_leaves_what_resumes_it() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("held");
    // The program learns where it's held, as an agent's hooks do, and leaves word there.
    let script = r#"echo working on it; printf '{"agent":"Claude Code","session":"abc-123"}' > "${CELLIX_HELD%.sock}.agent"; echo noted; sleep 60"#;
    let held = start(&folder, &job("Claude Code", script, dir.path()));
    let (seen, _) = attach(&held);
    read_until(&seen, "noted");
    let socket = held.socket.clone();
    drop(seen);
    drop(held);

    // The machine goes down: the host dies without finishing, and its program with it.
    let killed = std::process::Command::new("pkill")
        .args(["-KILL", "-f", &socket.display().to_string()])
        .status()
        .unwrap();
    assert!(killed.success(), "the host should have been found");
    let deadline = Instant::now() + Duration::from_secs(5);
    while std::os::unix::net::UnixStream::connect(&socket).is_ok() {
        assert!(Instant::now() < deadline, "the host should have died");
        std::thread::sleep(Duration::from_millis(20));
    }

    assert!(hold::left_running(&folder).is_empty());
    let stopped = hold::stopped(&folder);
    assert_eq!(stopped.len(), 1, "{stopped:?}");
    let stopped = &stopped[0];
    assert_eq!(stopped.job.name, "Claude Code");
    assert_eq!(stopped.job.folder, dir.path());
    let output = String::from_utf8_lossy(&stopped.output);
    assert!(output.contains("working on it"), "{output}");
    let agent = stopped.agent.as_ref().expect("the agent's word");
    assert_eq!(agent.session, "abc-123");
    // Its environment stays off the disk, as for a running one.
    assert!(stopped.job.env.is_empty(), "{:?}", stopped.job.env);

    // Still there for the next Cellix, until it's resumed or closed.
    assert_eq!(hold::stopped(&folder).len(), 1);
    stopped.forget();
    assert!(hold::stopped(&folder).is_empty());
}

#[test]
fn a_program_that_finishes_leaves_nothing_to_resume() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("held");
    let held = start(&folder, &job("Job", "echo finished; exit 0", dir.path()));
    let (seen, over) = attach(&held);
    read_until(&seen, "finished");
    over.recv_timeout(Duration::from_secs(5)).unwrap();
    drop(held);
    let deadline = Instant::now() + Duration::from_secs(5);
    while std::fs::read_dir(&folder)
        .unwrap()
        .flatten()
        .any(|e| e.path().is_file())
    {
        assert!(Instant::now() < deadline, "the host should have cleared up");
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(hold::stopped(&folder).is_empty());
}

#[test]
fn two_cellix_windows_share_a_program_and_whoever_types_sets_its_size() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("held");
    let script = r#"echo ready; while read line; do echo "got $line at $(stty size)"; done"#;
    let held = start(&folder, &job("Shared", script, dir.path()));
    let (first, _) = attach(&held);
    read_until(&first, "ready");
    held.resize((100, 30), (1000, 600));

    // A second window, as on another machine: it sees what was shown, and what comes after.
    let other = Held::connect(&held.socket).unwrap();
    let (second, _) = attach(&other);
    read_until(&second, "ready");
    other.resize((80, 24), (800, 480));

    // Typing in the first keeps its size; the second watching changes nothing.
    held.keys().unwrap().write_all(b"one\n").unwrap();
    read_until(&first, "got one at 30 100");
    read_until(&second, "got one at 30 100");

    // Typing in the second gives the program its size.
    other.keys().unwrap().write_all(b"two\n").unwrap();
    read_until(&second, "got two at 24 80");
    read_until(&first, "got two at 24 80");

    // One leaving leaves the other with it.
    drop(second);
    drop(other);
    held.keys().unwrap().write_all(b"three\n").unwrap();
    read_until(&first, "got three at 30 100");
    held.kill();
}
