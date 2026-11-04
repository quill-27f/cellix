//! Runs notebook cells in a real Jupyter kernel. Jupyter's client library does the talking, so
//! a small Python program sits between it and Cellix and passes lines of JSON each way. Python
//! is already here if the notebook can run at all, and the protocol stays out of Rust.

use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};

use serde_json::{Value, json};

/// Starts a kernel, runs cells from its input, and reports what comes back.
const RUNNER: &str = r#"
import json, signal, sys

def say(**fields):
    print(json.dumps(fields), flush=True)

try:
    from jupyter_client.manager import start_new_kernel
except ImportError:
    say(missing=True)
    sys.exit(0)

try:
    manager, client = start_new_kernel(kernel_name=sys.argv[1])
except Exception as error:
    say(failed=str(error))
    sys.exit(0)

signal.signal(signal.SIGINT, lambda *_: manager.interrupt_kernel())
say(ready=True)

for line in sys.stdin:
    job = json.loads(line)
    sent = client.execute(job["code"], allow_stdin=False)
    count = None
    while True:
        try:
            message = client.get_iopub_msg(timeout=None)
        except InterruptedError:
            continue
        if message["parent_header"].get("msg_id") != sent:
            continue
        kind, content = message["msg_type"], message["content"]
        if kind == "execute_input":
            count = content.get("execution_count")
        elif kind == "stream":
            say(id=job["id"], output={"output_type": "stream", "name": content["name"], "text": content["text"]})
        elif kind in ("display_data", "execute_result"):
            output = {"output_type": kind, "data": content["data"], "metadata": content.get("metadata", {})}
            if kind == "execute_result":
                output["execution_count"] = content["execution_count"]
            say(id=job["id"], output=output)
        elif kind == "error":
            say(id=job["id"], output={"output_type": "error", "ename": content["ename"], "evalue": content["evalue"], "traceback": content["traceback"]})
        elif kind == "clear_output":
            say(id=job["id"], clear=True)
        elif kind == "status" and content.get("execution_state") == "idle":
            break
    say(id=job["id"], done=True, count=count)

client.stop_channels()
manager.shutdown_kernel(now=True)
"#;

/// What the kernel has to say.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    Ready,
    /// This Python has no Jupyter.
    Missing,
    /// It couldn't start, and why.
    Failed(String),
    Output(usize, Value),
    Clear(usize),
    Done(usize, Option<u64>),
}

fn event_of(line: &str) -> Option<Event> {
    let value: Value = serde_json::from_str(line).ok()?;
    if value["ready"] == true {
        return Some(Event::Ready);
    }
    if value["missing"] == true {
        return Some(Event::Missing);
    }
    if let Some(reason) = value["failed"].as_str() {
        return Some(Event::Failed(reason.to_string()));
    }
    let cell = value["id"].as_u64()? as usize;
    if value["done"] == true {
        return Some(Event::Done(cell, value["count"].as_u64()));
    }
    if value["clear"] == true {
        return Some(Event::Clear(cell));
    }
    Some(Event::Output(cell, value.get("output")?.clone()))
}

/// The Python a notebook means: the project's own if it has one.
pub fn python_for(folder: &Path, root: &Path) -> PathBuf {
    let var = std::env::var_os("VIRTUAL_ENV").map(PathBuf::from);
    let places = [Some(folder), Some(root)].into_iter().flatten();
    let local = places
        .flat_map(|place| [place.join(".venv"), place.join("venv")])
        .chain(var)
        .map(|venv| venv.join(VENV_PYTHON))
        .find(|python| python.is_file());
    local.unwrap_or_else(|| PathBuf::from(PYTHON))
}

/// Where a virtual environment keeps its Python, and the one on the path. Windows puts it in
/// `Scripts`, and its installer names it `python`.
#[cfg(unix)]
const VENV_PYTHON: &str = "bin/python";
#[cfg(unix)]
const PYTHON: &str = "python3";
#[cfg(not(unix))]
const VENV_PYTHON: &str = "Scripts/python.exe";
#[cfg(not(unix))]
const PYTHON: &str = "python";

pub struct Kernel {
    /// Only signalled on Unix, but held everywhere: it's this kernel's runner.
    #[cfg_attr(not(unix), allow(dead_code))]
    child: Child,
    input: Option<ChildStdin>,
    pub events: Receiver<Event>,
}

impl Kernel {
    /// Starts one for a folder. `command` stands in for the runner in tests.
    pub fn start(python: &Path, folder: &Path, name: &str) -> io::Result<Self> {
        let mut command = Command::new(python);
        command.arg("-c").arg(RUNNER).arg(name);
        Self::spawn(command, folder)
    }

    pub fn start_command(argv: &[String], folder: &Path) -> io::Result<Self> {
        let (program, args) = argv.split_first().ok_or(io::ErrorKind::InvalidInput)?;
        let mut command = Command::new(program);
        command.args(args);
        Self::spawn(command, folder)
    }

    fn spawn(mut command: Command, folder: &Path) -> io::Result<Self> {
        let mut child = command
            .current_dir(folder)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let output = child.stdout.take().ok_or(io::ErrorKind::BrokenPipe)?;
        let input = child.stdin.take();
        let (sender, events) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(output).lines().map_while(Result::ok) {
                if let Some(event) = event_of(&line)
                    && sender.send(event).is_err()
                {
                    return;
                }
            }
        });
        Ok(Self {
            child,
            input,
            events,
        })
    }

    /// Queues a cell. The kernel takes them one at a time, in order.
    pub fn run(&mut self, cell: usize, code: &str) -> io::Result<()> {
        let line = json!({"id": cell, "code": code}).to_string();
        let input = self.input.as_mut().ok_or(io::ErrorKind::BrokenPipe)?;
        writeln!(input, "{line}")?;
        input.flush()
    }

    /// Stops what's running, as Ctrl+C would.
    #[cfg(unix)]
    pub fn interrupt(&self) {
        // SAFETY: signals the runner this kernel started, which turns it into an interrupt.
        unsafe {
            libc::kill(self.child.id() as i32, libc::SIGINT);
        }
    }

    /// Windows has no SIGINT to send the runner, so a running cell can't be stopped short yet;
    /// it finishes, or ends with the kernel.
    #[cfg(not(unix))]
    pub fn interrupt(&self) {}
}

impl Drop for Kernel {
    fn drop(&mut self) {
        // With its input closed the runner shuts its kernel down and leaves. Killing it here
        // would leave the kernel behind.
        self.input.take();
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_the_runner_says_is_read() {
        assert_eq!(event_of(r#"{"ready": true}"#), Some(Event::Ready));
        assert_eq!(event_of(r#"{"missing": true}"#), Some(Event::Missing));
        assert_eq!(
            event_of(r#"{"id": 2, "done": true, "count": 7}"#),
            Some(Event::Done(2, Some(7)))
        );
        assert_eq!(
            event_of(r#"{"id": 2, "clear": true}"#),
            Some(Event::Clear(2))
        );
        let output = event_of(r#"{"id": 1, "output": {"output_type": "stream"}}"#);
        assert!(matches!(output, Some(Event::Output(1, _))));
        assert_eq!(event_of("not json"), None);
    }

    #[test]
    fn a_projects_own_python_comes_first() {
        let dir = tempfile::tempdir().unwrap();
        let python = dir.path().join(".venv/bin/python");
        std::fs::create_dir_all(python.parent().unwrap()).unwrap();
        std::fs::write(&python, "").unwrap();
        assert_eq!(python_for(dir.path(), dir.path()), python);
    }
}
