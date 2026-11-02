//! Keeps what runs in the side pane alive without Cellix. Each program runs under a small
//! process of its own, which owns the PTY and a Unix socket and remembers recent output, so
//! Cellix can quit, or an SSH connection drop, and the next Cellix finds it still running.

use std::collections::VecDeque;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::io::{self, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use portable_pty::{CommandBuilder, MasterPty, PtySize, native_pty_system};
use serde::{Deserialize, Serialize};

/// How much output is kept for the next Cellix to show.
const KEPT: usize = 4 * 1024 * 1024;
/// How often the host looks at whether a shell is back at its prompt.
const LOOK: Duration = Duration::from_millis(500);
/// How long Cellix waits for a new host's socket.
const START_WAIT: Duration = Duration::from_secs(5);

const OUTPUT: u8 = b'o';
const EXITED: u8 = b'x';
const AT_PROMPT: u8 = b'p';
const INPUT: u8 = b'i';
const RESIZE: u8 = b'r';
const KILL: u8 = b'k';

/// What a host is told to run, kept beside its socket.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Job {
    pub name: String,
    pub argv: Vec<String>,
    pub env: Vec<String>,
    pub folder: PathBuf,
    pub size: (u16, u16),
    pub pixels: (u16, u16),
}

fn send_frame(stream: &mut impl Write, kind: u8, payload: &[u8]) -> io::Result<()> {
    let mut frame = Vec::with_capacity(payload.len() + 5);
    frame.push(kind);
    frame.extend((payload.len() as u32).to_be_bytes());
    frame.extend(payload);
    stream.write_all(&frame)
}

fn read_frame(stream: &mut impl Read) -> io::Result<(u8, Vec<u8>)> {
    let mut head = [0; 5];
    stream.read_exact(&mut head)?;
    let length = u32::from_be_bytes([head[1], head[2], head[3], head[4]]) as usize;
    let mut payload = vec![0; length];
    stream.read_exact(&mut payload)?;
    Ok((head[0], payload))
}

fn pty_size(size: (u16, u16), pixels: (u16, u16)) -> PtySize {
    PtySize {
        rows: size.1,
        cols: size.0,
        pixel_width: pixels.0,
        pixel_height: pixels.1,
    }
}

/// Where a project's sessions are kept.
pub fn folder_of(project: &Path) -> PathBuf {
    let mut hasher = DefaultHasher::new();
    project.hash(&mut hasher);
    let var = |name: &str| std::env::var_os(name).map(PathBuf::from);
    let cache = var("XDG_CACHE_HOME").or_else(|| var("HOME").map(|home| home.join(".cache")));
    cache
        .unwrap_or_else(std::env::temp_dir)
        .join("cellix")
        .join("sessions")
        .join(format!("{:016x}", hasher.finish()))
}

fn ensure_private(folder: &Path) -> io::Result<()> {
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(folder)?;
    std::fs::set_permissions(folder, std::fs::Permissions::from_mode(0o700))
}

/// Starts a host for `job`, and connects to it. `program` is Cellix itself.
pub fn start(program: &Path, folder: &Path, job: &Job) -> io::Result<Held> {
    ensure_private(folder)?;
    let id = std::process::id();
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    let socket = folder.join(format!("{id}-{stamp}.sock"));
    let notes = socket.with_extension("json");
    std::fs::write(&notes, serde_json::to_vec(job)?)?;
    let mut command = Command::new(program);
    command
        .arg("session-host")
        .arg(&socket)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // SAFETY: setsid is async-signal-safe, and this runs in the child before it execs.
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() < 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
    command.spawn()?;
    let deadline = std::time::Instant::now() + START_WAIT;
    loop {
        match Held::connect(&socket) {
            Ok(held) => return Ok(held),
            Err(e) if std::time::Instant::now() > deadline => return Err(e),
            Err(_) => std::thread::sleep(Duration::from_millis(10)),
        }
    }
}

/// The sessions left running for a project, with what each is called.
pub fn left_running(folder: &Path) -> Vec<(String, PathBuf)> {
    let mut found = Vec::new();
    for entry in std::fs::read_dir(folder).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "sock") {
            continue;
        }
        if UnixStream::connect(&path).is_err() {
            // Its host is gone, and so is what it held.
            let _ = std::fs::remove_file(&path);
            let _ = std::fs::remove_file(path.with_extension("json"));
            continue;
        }
        let notes = std::fs::read(path.with_extension("json"));
        let job: Option<Job> = notes.ok().and_then(|n| serde_json::from_slice(&n).ok());
        if let Some(job) = job {
            found.push((job.name, path));
        }
    }
    found.sort();
    found
}

/// A connection to a host, from Cellix's side.
pub struct Held {
    stream: UnixStream,
    pub socket: PathBuf,
    at_prompt: Arc<Mutex<bool>>,
}

/// Writes keys to a host, for the session to use like any writer.
pub struct Keys(UnixStream);

impl Write for Keys {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        send_frame(&mut self.0, INPUT, bytes)?;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Held {
    pub fn connect(socket: &Path) -> io::Result<Self> {
        Ok(Self {
            stream: UnixStream::connect(socket)?,
            socket: socket.to_path_buf(),
            at_prompt: Arc::new(Mutex::new(true)),
        })
    }

    pub fn keys(&self) -> io::Result<Keys> {
        Ok(Keys(self.stream.try_clone()?))
    }

    /// Passes what the program writes to `output`, and says on `done` when it's over.
    pub fn listen(&self, output: Sender<Vec<u8>>, done: Sender<()>) -> io::Result<()> {
        let mut stream = self.stream.try_clone()?;
        let at_prompt = self.at_prompt.clone();
        std::thread::spawn(move || {
            while let Ok((kind, payload)) = read_frame(&mut stream) {
                match kind {
                    OUTPUT => {
                        if output.send(payload).is_err() {
                            return;
                        }
                    }
                    EXITED => {
                        let _ = done.send(());
                        return;
                    }
                    AT_PROMPT => {
                        if let Ok(mut flag) = at_prompt.lock() {
                            *flag = payload == [1];
                        }
                    }
                    _ => {}
                }
            }
            // The host went away, and took the program with it.
            let _ = done.send(());
        });
        Ok(())
    }

    pub fn at_prompt(&self) -> bool {
        self.at_prompt.lock().is_ok_and(|flag| *flag)
    }

    pub fn resize(&self, size: (u16, u16), pixels: (u16, u16)) {
        let mut payload = Vec::with_capacity(8);
        for number in [size.0, size.1, pixels.0, pixels.1] {
            payload.extend(number.to_be_bytes());
        }
        let _ = send_frame(&mut &self.stream, RESIZE, &payload);
    }

    pub fn kill(&self) {
        let _ = send_frame(&mut &self.stream, KILL, &[]);
    }
}

/// What the host shares between its threads.
#[derive(Default)]
struct Shared {
    kept: VecDeque<u8>,
    client: Option<UnixStream>,
    /// Counts clients, so one leaving can't take the place of the next.
    visit: u64,
    exited: bool,
    at_prompt: bool,
}

impl Shared {
    fn tell(&mut self, kind: u8, payload: &[u8]) {
        if let Some(client) = &mut self.client
            && send_frame(client, kind, payload).is_err()
        {
            self.client = None;
        }
    }

    fn output(&mut self, bytes: &[u8]) {
        self.kept.extend(bytes);
        let extra = self.kept.len().saturating_sub(KEPT);
        self.kept.drain(..extra);
        self.tell(OUTPUT, bytes);
    }
}

fn serve_client(
    mut stream: UnixStream,
    writer: &Arc<Mutex<Box<dyn Write + Send>>>,
    master: &Arc<Mutex<Box<dyn MasterPty + Send>>>,
    pid: Option<u32>,
) {
    while let Ok((kind, payload)) = read_frame(&mut stream) {
        match kind {
            INPUT => {
                if let Ok(mut writer) = writer.lock() {
                    let _ = writer.write_all(&payload);
                    let _ = writer.flush();
                }
            }
            RESIZE if payload.len() == 8 => {
                let number = |at: usize| u16::from_be_bytes([payload[at], payload[at + 1]]);
                if let Ok(master) = master.lock() {
                    let size = pty_size((number(0), number(2)), (number(4), number(6)));
                    let _ = master.resize(size);
                }
            }
            KILL => {
                if let Some(pid) = pid {
                    // SAFETY: only signals the process this host started, and its group.
                    unsafe {
                        libc::kill(-(pid as i32), libc::SIGHUP);
                        libc::kill(pid as i32, libc::SIGHUP);
                    }
                }
            }
            _ => {}
        }
    }
}

/// Runs a job, serving its socket until the program has ended and someone has been told.
pub fn serve(socket: &Path) -> io::Result<()> {
    let notes = socket.with_extension("json");
    let job: Job = serde_json::from_slice(&std::fs::read(&notes)?)?;
    let pair = native_pty_system()
        .openpty(pty_size(job.size, job.pixels))
        .map_err(io::Error::other)?;
    let (program, args) = job.argv.split_first().ok_or(io::ErrorKind::InvalidInput)?;
    let mut command = CommandBuilder::new(program);
    command.args(args);
    command.cwd(&job.folder);
    command.env("TERM", "xterm-256color");
    command.env("COLORTERM", "truecolor");
    for pair in &job.env {
        if let Some((key, value)) = pair.split_once('=') {
            command.env(key, value);
        }
    }
    let mut child = pair
        .slave
        .spawn_command(command)
        .map_err(io::Error::other)?;
    drop(pair.slave);
    let pid = child.process_id();
    let mut reader = pair.master.try_clone_reader().map_err(io::Error::other)?;
    let writer = pair.master.take_writer().map_err(io::Error::other)?;
    let writer = Arc::new(Mutex::new(writer));
    let master = Arc::new(Mutex::new(pair.master));
    // With the job read, its environment needn't sit on disk.
    let quiet = Job {
        env: Vec::new(),
        ..job
    };
    std::fs::write(&notes, serde_json::to_vec(&quiet)?)?;
    let listener = UnixListener::bind(socket)?;
    let shared = Arc::new(Mutex::new(Shared::default()));

    let from_program = shared.clone();
    std::thread::spawn(move || {
        let mut buffer = [0; 8192];
        while let Ok(count) = reader.read(&mut buffer) {
            if count == 0 {
                break;
            }
            if let Ok(mut shared) = from_program.lock() {
                shared.output(&buffer[..count]);
            }
        }
    });

    let looking = shared.clone();
    let front = master.clone();
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(LOOK);
            let leader = front.lock().ok().and_then(|m| m.process_group_leader());
            let at_prompt = leader.is_some() && leader.map(|p| p as u32) == pid;
            if let Ok(mut shared) = looking.lock()
                && shared.at_prompt != at_prompt
            {
                shared.at_prompt = at_prompt;
                shared.tell(AT_PROMPT, &[u8::from(at_prompt)]);
            }
        }
    });

    let ended = shared.clone();
    let place = socket.to_path_buf();
    std::thread::spawn(move || {
        let _ = child.wait();
        // What it printed last may still be on its way.
        std::thread::sleep(Duration::from_millis(100));
        if let Ok(mut shared) = ended.lock() {
            shared.exited = true;
            // Only someone who really heard it lets the host go. Otherwise it waits to say so.
            shared.tell(EXITED, &[]);
            if shared.client.is_some() {
                finish(&place);
            }
        }
    });

    for stream in listener.incoming().flatten() {
        let Ok(mut outgoing) = stream.try_clone() else {
            continue;
        };
        if let Ok(mut shared) = shared.lock() {
            let (front, back) = shared.kept.as_slices();
            let sent = send_frame(&mut outgoing, OUTPUT, front)
                .and_then(|()| send_frame(&mut outgoing, OUTPUT, back))
                .and_then(|()| send_frame(&mut outgoing, AT_PROMPT, &[u8::from(shared.at_prompt)]));
            if sent.is_err() {
                continue;
            }
            shared.client = Some(outgoing);
            shared.visit += 1;
            if shared.exited {
                shared.tell(EXITED, &[]);
                finish(socket);
            }
        }
        let visit = shared.lock().map_or(0, |shared| shared.visit);
        let (writer, master, leaving) = (writer.clone(), master.clone(), shared.clone());
        std::thread::spawn(move || {
            serve_client(stream, &writer, &master, pid);
            if let Ok(mut shared) = leaving.lock()
                && shared.visit == visit
            {
                shared.client = None;
            }
        });
    }
    Ok(())
}

/// The program is over and someone knows: nothing is left to hold.
fn finish(socket: &Path) -> ! {
    let _ = std::fs::remove_file(socket);
    let _ = std::fs::remove_file(socket.with_extension("json"));
    std::process::exit(0);
}
