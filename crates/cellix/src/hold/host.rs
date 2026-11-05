//! The host, on Unix: each program runs under a small process of its own, which owns the PTY
//! and a Unix socket and remembers recent output.

use std::collections::VecDeque;
use std::io::{self, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use portable_pty::{CommandBuilder, MasterPty, PtySize, native_pty_system};

use super::{Job, Stopped};

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
    // The program learns where it's held, so an agent's hooks can leave word of how to
    // resume it beside its socket.
    let mut job = job.clone();
    job.env
        .push(format!("{}={}", super::HELD_VAR, socket.display()));
    std::fs::write(&notes, serde_json::to_vec(&job)?)?;
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
            // Its host is gone without finishing, as when the machine restarted, so what it
            // held is put aside to be offered again. A job that can't be read is forgotten.
            let _ = std::fs::remove_file(&path);
            put_aside(&path);
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

/// The files a host keeps beside its socket, by extension.
const BESIDE: [&str; 3] = ["json", "log", "agent"];

/// Moves what a host that died left behind into the stopped folder, if its job can be read.
fn put_aside(socket: &Path) {
    let notes = std::fs::read(socket.with_extension("json"));
    let readable = notes.is_ok_and(|n| serde_json::from_slice::<Job>(&n).is_ok());
    let Some(folder) = socket.parent() else {
        return;
    };
    let aside = folder.join(super::STOPPED);
    if readable && ensure_private(&aside).is_ok() {
        for extension in BESIDE {
            let from = socket.with_extension(extension);
            if let Some(name) = from.file_name() {
                let _ = std::fs::rename(&from, aside.join(name));
            }
        }
    }
    for extension in BESIDE {
        let _ = std::fs::remove_file(socket.with_extension(extension));
    }
}

/// What stopped without finishing, as when the machine restarted, oldest first. Each has the
/// job it ran, what it last showed, and how to resume it if an agent left word.
pub fn stopped(folder: &Path) -> Vec<Stopped> {
    let aside = folder.join(super::STOPPED);
    // Anything still in the folder whose host is gone is put aside first.
    left_running(folder);
    let mut found = Vec::new();
    for entry in std::fs::read_dir(&aside).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "json") {
            continue;
        }
        let Some(job) = std::fs::read(&path)
            .ok()
            .and_then(|n| serde_json::from_slice::<Job>(&n).ok())
        else {
            let _ = std::fs::remove_file(&path);
            continue;
        };
        let when = entry.metadata().and_then(|m| m.modified()).ok();
        let log = path.with_extension("log");
        let ended = std::fs::metadata(&log)
            .and_then(|m| m.modified())
            .ok()
            .or(when);
        let output = std::fs::read(&log).unwrap_or_default();
        let agent = std::fs::read(path.with_extension("agent"))
            .ok()
            .and_then(|a| serde_json::from_slice(&a).ok());
        found.push((
            when,
            Stopped {
                job,
                output,
                agent,
                ended,
                notes: path,
            },
        ));
    }
    found.sort_by_key(|(when, _)| *when);
    found.into_iter().map(|(_, stopped)| stopped).collect()
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
    /// Every Cellix showing it, as on two machines at once, by the number of its visit.
    clients: Vec<Client>,
    /// Counts clients, so each can be told from the next.
    visit: u64,
    exited: bool,
    at_prompt: bool,
    /// What it printed, on disk too, so it can be shown again after the machine restarts.
    log: Option<Log>,
    /// The size the program has, as `RESIZE` sends it.
    size: Vec<u8>,
}

struct Client {
    visit: u64,
    stream: UnixStream,
    /// The size this client wants, which it gets once it types.
    size: Option<Vec<u8>>,
}

impl Shared {
    /// Tells every client, forgetting any that have gone.
    fn tell(&mut self, kind: u8, payload: &[u8]) {
        self.clients
            .retain_mut(|client| send_frame(&mut client.stream, kind, payload).is_ok());
    }

    fn output(&mut self, bytes: &[u8]) {
        self.kept.extend(bytes);
        let extra = self.kept.len().saturating_sub(KEPT);
        self.kept.drain(..extra);
        if let Some(log) = &mut self.log {
            log.add(bytes, &self.kept);
        }
        self.tell(OUTPUT, bytes);
    }

    /// The size a client asks for. The first, or one that is typing, gets it at once; with
    /// several watching, whoever types last decides, as in tmux.
    fn wants_size(&mut self, visit: u64, size: Vec<u8>) -> Option<Vec<u8>> {
        let alone = self.clients.len() <= 1;
        if let Some(client) = self.clients.iter_mut().find(|c| c.visit == visit) {
            client.size = Some(size.clone());
        }
        (alone || self.size.is_empty()).then(|| self.take_size(size))
    }

    /// Whether a client's own size should be taken because it's typing.
    fn typing(&mut self, visit: u64) -> Option<Vec<u8>> {
        let size = self
            .clients
            .iter()
            .find(|c| c.visit == visit)
            .and_then(|c| c.size.clone())?;
        (size != self.size).then(|| self.take_size(size))
    }

    fn take_size(&mut self, size: Vec<u8>) -> Vec<u8> {
        self.size = size.clone();
        size
    }
}

/// The output on disk, kept to about `KEPT` by starting again from what's in memory when it
/// grows to twice that.
struct Log {
    file: std::fs::File,
    path: PathBuf,
    written: usize,
}

impl Log {
    fn open(path: &Path) -> Option<Self> {
        use std::os::unix::fs::OpenOptionsExt;
        let file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)
            .ok()?;
        Some(Self {
            file,
            path: path.to_path_buf(),
            written: 0,
        })
    }

    fn add(&mut self, bytes: &[u8], kept: &VecDeque<u8>) {
        if self.written + bytes.len() > 2 * KEPT {
            if let Some(fresh) = Self::open(&self.path) {
                *self = fresh;
            }
            let (front, back) = kept.as_slices();
            let _ = self
                .file
                .write_all(front)
                .and_then(|()| self.file.write_all(back));
            self.written = kept.len();
            return;
        }
        if self.file.write_all(bytes).is_ok() {
            self.written += bytes.len();
        }
    }
}

fn resize(master: &Arc<Mutex<Box<dyn MasterPty + Send>>>, payload: &[u8]) {
    if payload.len() != 8 {
        return;
    }
    let number = |at: usize| u16::from_be_bytes([payload[at], payload[at + 1]]);
    if let Ok(master) = master.lock() {
        let size = pty_size((number(0), number(2)), (number(4), number(6)));
        let _ = master.resize(size);
    }
}

fn serve_client(
    mut stream: UnixStream,
    (shared, visit): (&Arc<Mutex<Shared>>, u64),
    writer: &Arc<Mutex<Box<dyn Write + Send>>>,
    master: &Arc<Mutex<Box<dyn MasterPty + Send>>>,
    pid: Option<u32>,
) {
    while let Ok((kind, payload)) = read_frame(&mut stream) {
        match kind {
            INPUT => {
                let size = shared.lock().ok().and_then(|mut s| s.typing(visit));
                if let Some(size) = size {
                    resize(master, &size);
                }
                if let Ok(mut writer) = writer.lock() {
                    let _ = writer.write_all(&payload);
                    let _ = writer.flush();
                }
            }
            RESIZE if payload.len() == 8 => {
                let size = shared
                    .lock()
                    .ok()
                    .and_then(|mut s| s.wants_size(visit, payload));
                if let Some(size) = size {
                    resize(master, &size);
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
    let shared = Arc::new(Mutex::new(Shared {
        log: Log::open(&socket.with_extension("log")),
        ..Shared::default()
    }));

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
            if !shared.clients.is_empty() {
                finish(&place);
            }
        }
    });

    for stream in listener.incoming().flatten() {
        let Ok(mut outgoing) = stream.try_clone() else {
            continue;
        };
        let mut visit = 0;
        if let Ok(mut shared) = shared.lock() {
            let (front, back) = shared.kept.as_slices();
            let sent = send_frame(&mut outgoing, OUTPUT, front)
                .and_then(|()| send_frame(&mut outgoing, OUTPUT, back))
                .and_then(|()| send_frame(&mut outgoing, AT_PROMPT, &[u8::from(shared.at_prompt)]));
            if sent.is_err() {
                continue;
            }
            shared.visit += 1;
            visit = shared.visit;
            shared.clients.push(Client {
                visit,
                stream: outgoing,
                size: None,
            });
            if shared.exited {
                shared.tell(EXITED, &[]);
                finish(socket);
            }
        }
        let (writer, master, leaving) = (writer.clone(), master.clone(), shared.clone());
        std::thread::spawn(move || {
            serve_client(stream, (&leaving, visit), &writer, &master, pid);
            if let Ok(mut shared) = leaving.lock() {
                shared.clients.retain(|c| c.visit != visit);
            }
        });
    }
    Ok(())
}

/// The program is over and someone knows: nothing is left to hold.
fn finish(socket: &Path) -> ! {
    let _ = std::fs::remove_file(socket);
    for extension in BESIDE {
        let _ = std::fs::remove_file(socket.with_extension(extension));
    }
    std::process::exit(0);
}
