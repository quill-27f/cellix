//! The host, on Unix: each program runs under a small process of its own, which owns the PTY
//! and a Unix socket and remembers recent output.

use std::collections::VecDeque;
use std::io::{self, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use portable_pty::{CommandBuilder, MasterPty, PtySize, native_pty_system};

use super::{Job, Stopped};
use crate::clock::Clock;
use crate::outbox::Outbox;

/// How much output is kept for the next Cellix to show.
const KEPT: usize = 4 * 1024 * 1024;
/// How often the host looks at whether a shell is back at its prompt.
const LOOK: Duration = Duration::from_millis(500);
/// How long Cellix waits for a new host's socket.
const START_WAIT: Duration = Duration::from_secs(5);
/// A viewer this far behind holds the program until it catches up, as a terminal holds a
/// program printing faster than it shows: what the program prints isn't let pile up.
const BEHIND: usize = 4 * 1024 * 1024;
/// A viewer that reads nothing of that for this long has stopped, as a laptop asleep with
/// Cellix open, and is let go, so the program and the other viewers carry on.
const STALLED: Duration = Duration::from_secs(2);
/// How long a host that's ending waits for its viewers to have what they were last sent.
const LAST_WORDS: Duration = Duration::from_secs(2);

const OUTPUT: u8 = b'o';
const EXITED: u8 = b'x';
const AT_PROMPT: u8 = b'p';
/// The name of the program in front of the one held, as a shell's command: empty when the
/// held program is in front again. Sent when it changes; a Cellix from before ignores it.
const FOREGROUND: u8 = b'f';
const INPUT: u8 = b'i';
const RESIZE: u8 = b'r';
const KILL: u8 = b'k';
/// What asks a terminal to say when its window gains and loses focus (DECSET 1004).
const FOCUS_ASKED: &[u8] = b"\x1b[?1004h";
/// The longest unfinished private mode sequence carried from one output to the next.
const LONGEST_MODES: usize = 64;

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
const BESIDE: [&str; 4] = ["json", "log", "agent", "pid"];

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

/// A connection to a host, from Cellix's side. Everything sent to it goes through one queue,
/// in order and off the loop: keys, sizes, the word to stop.
pub struct Held {
    stream: UnixStream,
    pub socket: PathBuf,
    at_prompt: Arc<Mutex<bool>>,
    foreground: Arc<Mutex<String>>,
    out: Outbox,
}

/// Writes keys to a host, for the session to use like any writer.
pub struct Keys(Outbox);

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
        let stream = UnixStream::connect(socket)?;
        let out = Outbox::new(Box::new(stream.try_clone()?));
        Ok(Self {
            stream,
            socket: socket.to_path_buf(),
            at_prompt: Arc::new(Mutex::new(true)),
            foreground: Arc::default(),
            out,
        })
    }

    pub fn keys(&self) -> io::Result<Keys> {
        Ok(Keys(self.out.clone()))
    }

    /// Passes what the program writes to `output`, and says on `done` when it's over.
    /// Passes what the program prints to `output`, and calls `ended` once it has ended or
    /// its host has gone.
    pub fn listen(
        &self,
        output: impl super::Sink,
        ended: impl FnOnce() + Send + 'static,
    ) -> io::Result<()> {
        let mut stream = self.stream.try_clone()?;
        let at_prompt = self.at_prompt.clone();
        let foreground = self.foreground.clone();
        std::thread::spawn(move || {
            while let Ok((kind, payload)) = read_frame(&mut stream) {
                match kind {
                    OUTPUT => {
                        if !output.put(payload) {
                            return;
                        }
                    }
                    EXITED => break,
                    AT_PROMPT => {
                        if let Ok(mut flag) = at_prompt.lock() {
                            *flag = payload == [1];
                        }
                    }
                    FOREGROUND => {
                        if let Ok(mut name) = foreground.lock() {
                            *name = String::from_utf8_lossy(&payload).into_owned();
                        }
                    }
                    _ => {}
                }
            }
            // It ended, or the host went away and took the program with it.
            ended();
        });
        Ok(())
    }

    pub fn at_prompt(&self) -> bool {
        self.at_prompt.lock().is_ok_and(|flag| *flag)
    }

    /// The program in front of the one held, by name, as a command run at a shell's prompt:
    /// empty while the held program is in front, or from a host that doesn't say.
    pub fn foreground(&self) -> String {
        self.foreground
            .lock()
            .map(|name| name.clone())
            .unwrap_or_default()
    }

    pub fn resize(&self, size: (u16, u16), pixels: (u16, u16)) {
        let mut payload = Vec::with_capacity(8);
        for number in [size.0, size.1, pixels.0, pixels.1] {
            payload.extend(number.to_be_bytes());
        }
        let _ = send_frame(&mut self.out.clone(), RESIZE, &payload);
    }

    pub fn kill(&self) {
        let _ = send_frame(&mut self.out.clone(), KILL, &[]);
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
    /// The name of the program in front of the held one, empty while that's in front.
    foreground: Vec<u8>,
    /// What it printed, on disk too, so it can be shown again after the machine restarts.
    log: Option<Log>,
    /// The size the program has, as `RESIZE` sends it.
    size: Vec<u8>,
    /// The time a viewer's reading nothing is measured by: the wall's, or a test's.
    clock: Clock,
    /// Whether the program asked to hear of focus, which a viewer joining is told again.
    focus: Focus,
}

/// Follows whether the program has asked to hear of focus (focus reporting, DECSET 1004)
/// through output that may split a sequence anywhere. A viewer that joins after the asking has
/// gone from what's kept is told of it again, ahead of what's kept: otherwise its program never
/// hears where the keyboard is, and Claude Code keeps its prompt's cursor as an earlier Cellix
/// last told it, hidden while the keys are in it.
#[derive(Default)]
struct Focus {
    asked: bool,
    /// The start of a sequence the last output ended inside.
    carry: Vec<u8>,
}

impl Focus {
    fn follow(&mut self, bytes: &[u8]) {
        let mut text = std::mem::take(&mut self.carry);
        text.extend_from_slice(bytes);
        let mut at = 0;
        while let Some(found) = text[at..].iter().position(|b| *b == 0x1b) {
            let start = at + found;
            let rest = &text[start..];
            match (rest.get(1), rest.get(2)) {
                (None, _) | (Some(b'['), None) => {
                    self.carry = rest.to_vec();
                    return;
                }
                // A full reset forgets it.
                (Some(b'c'), _) => {
                    self.asked = false;
                    at = start + 2;
                    continue;
                }
                (Some(b'['), Some(b'?')) => {}
                _ => {
                    at = start + 1;
                    continue;
                }
            }
            let body = &rest[3..];
            let Some(end) = body.iter().position(|b| !b.is_ascii_digit() && *b != b';') else {
                if rest.len() <= LONGEST_MODES {
                    self.carry = rest.to_vec();
                }
                return;
            };
            let set = match body[end] {
                b'h' => Some(true),
                b'l' => Some(false),
                _ => None,
            };
            if let Some(set) = set
                && body[..end]
                    .split(|b| *b == b';')
                    .any(|mode| mode == b"1004")
            {
                self.asked = set;
            }
            at = start + 3 + end + 1;
        }
    }
}

struct Client {
    visit: u64,
    /// What it's sent waits here, so a viewer that stops reading holds up nothing else.
    out: Outbox,
    /// To end it with, when it has left too much unread.
    stream: UnixStream,
    /// The size this client wants, which it gets once it types.
    size: Option<Vec<u8>>,
}

impl Shared {
    /// Tells every client, forgetting any that have gone. Nothing here waits on a client:
    /// it runs under the lock every thread shares, on the thread that reads the program, and
    /// writing to each in turn here let one viewer not reading stop the program and every
    /// other viewer (`take_output` holds the program back, outside the lock, for one that's
    /// behind). One whose queue is refused is let go; it can join again, and is sent what's
    /// kept.
    fn tell(&mut self, kind: u8, payload: &[u8]) {
        self.clients.retain_mut(|client| {
            let told = send_frame(&mut client.out, kind, payload).is_ok();
            if !told {
                let _ = client.stream.shutdown(std::net::Shutdown::Both);
            }
            told
        });
    }

    /// The viewers far behind, by visit, with their queues.
    fn behind(&self) -> Vec<(u64, Outbox)> {
        let far = self.clients.iter().filter(|c| c.out.waiting() > BEHIND);
        far.map(|c| (c.visit, c.out.clone())).collect()
    }

    /// Whether anyone heard what was last told: each viewer's queue is waited on, and those
    /// it couldn't reach are let go. The host is about to end, which would take with it
    /// what its viewers' queues hold.
    fn heard(&mut self) -> bool {
        self.clients
            .retain(|client| client.out.delivered(LAST_WORDS));
        !self.clients.is_empty()
    }

    /// Lets a viewer go: what it hasn't read is dropped, and its connection closed.
    fn let_go(&mut self, visit: u64) {
        self.clients.retain(|client| {
            let keep = client.visit != visit;
            if !keep {
                let _ = client.stream.shutdown(std::net::Shutdown::Both);
            }
            keep
        });
    }

    fn output(&mut self, bytes: &[u8]) {
        self.focus.follow(bytes);
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

/// What the program printed, kept and sent to every viewer. A viewer far behind holds the
/// program here, outside the lock, until it catches up; one that has stopped reading is let
/// go after `STALLED`, and the program carries on.
fn take_output(shared: &Arc<Mutex<Shared>>, bytes: &[u8]) {
    let (behind, clock) = match shared.lock() {
        Ok(mut shared) => {
            shared.output(bytes);
            (shared.behind(), shared.clock.clone())
        }
        Err(_) => return,
    };
    for (visit, out) in behind {
        if !catches_up(&out, &clock)
            && let Ok(mut shared) = shared.lock()
        {
            shared.let_go(visit);
        }
    }
}

/// Waits while a viewer's queue is far behind. Says whether it caught up, or false once it
/// has read nothing for `STALLED` by `clock`.
fn catches_up(out: &Outbox, clock: &Clock) -> bool {
    let mut left = out.waiting();
    let mut moved = clock.now();
    while left > BEHIND {
        std::thread::sleep(Duration::from_millis(2));
        let now = out.waiting();
        if now < left {
            moved = clock.now();
        } else if clock.now().duration_since(moved) >= STALLED {
            return false;
        }
        left = now;
    }
    true
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
    writer: &Outbox,
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
                // Queued, so a program not reading its input can't stop this reading what
                // comes next, such as the word to stop.
                let _ = writer.clone().write_all(&payload);
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
    // Which process is the program held here, so a hook can tell it from an agent that
    // program started, which inherits where it's held.
    if let Some(pid) = pid {
        let _ = std::fs::write(socket.with_extension("pid"), pid.to_string());
    }
    let mut reader = pair.master.try_clone_reader().map_err(io::Error::other)?;
    let writer = Outbox::new(pair.master.take_writer().map_err(io::Error::other)?);
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
            take_output(&from_program, &buffer[..count]);
        }
    });

    let looking = shared.clone();
    let front = master.clone();
    let reached_at = socket.to_path_buf();
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(LOOK);
            // With its socket gone, nobody can reach this host again: the folder that held it
            // was cleared, or a test's was deleted. Its program ends with it rather than
            // running on unseen. Leaving closes the terminal the program runs in.
            if !reached_at.exists() {
                finish(&reached_at);
            }
            let leader = front.lock().ok().and_then(|m| m.process_group_leader());
            let at_prompt = leader.is_some() && leader.map(|p| p as u32) == pid;
            // Read on each look while a command runs, not once per group: a group is in front
            // from the moment the shell forks its leader, which is the shell's own name until
            // it has started the command. One read is a system call, and clients are told
            // only when the name changes.
            let running = leader
                .filter(|_| !at_prompt)
                .and_then(|leader| super::program_name(leader as u32))
                .unwrap_or_default()
                .into_bytes();
            if let Ok(mut shared) = looking.lock() {
                if shared.at_prompt != at_prompt {
                    shared.at_prompt = at_prompt;
                    shared.tell(AT_PROMPT, &[u8::from(at_prompt)]);
                }
                if shared.foreground != running {
                    shared.tell(FOREGROUND, &running);
                    shared.foreground = running;
                }
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
            if shared.heard() {
                finish(&place);
            }
        }
    });

    for stream in listener.incoming().flatten() {
        let (Ok(outgoing), Ok(ending)) = (stream.try_clone(), stream.try_clone()) else {
            continue;
        };
        let mut visit = 0;
        if let Ok(mut shared) = shared.lock() {
            let mut out = Outbox::new(Box::new(outgoing));
            // Asked before what's kept begins, it's asked again ahead of it; asked or taken
            // back since, what's kept says so after.
            let asked = if shared.focus.asked { FOCUS_ASKED } else { &[] };
            let (front, back) = shared.kept.as_slices();
            let sent = send_frame(&mut out, OUTPUT, asked)
                .and_then(|()| send_frame(&mut out, OUTPUT, front))
                .and_then(|()| send_frame(&mut out, OUTPUT, back))
                .and_then(|()| send_frame(&mut out, AT_PROMPT, &[u8::from(shared.at_prompt)]))
                .and_then(|()| send_frame(&mut out, FOREGROUND, &shared.foreground));
            if sent.is_err() {
                continue;
            }
            shared.visit += 1;
            visit = shared.visit;
            shared.clients.push(Client {
                visit,
                out,
                stream: ending,
                size: None,
            });
            if shared.exited {
                shared.tell(EXITED, &[]);
                if shared.heard() {
                    finish(socket);
                }
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    /// Whether the program asked to hear of focus is followed through its output however
    /// it's cut: a request split between two reads, one among other modes, one taken back, a
    /// full reset, and other sequences that only look alike.
    #[test]
    fn the_host_follows_whether_its_program_asked_to_hear_of_focus() {
        let followed = |pieces: &[&[u8]]| {
            let mut focus = Focus::default();
            for piece in pieces {
                focus.follow(piece);
            }
            focus.asked
        };
        assert!(followed(&[b"ready\x1b[?1004h".as_slice()]));
        for cut in 1..FOCUS_ASKED.len() {
            let (a, b) = FOCUS_ASKED.split_at(cut);
            assert!(
                followed(&[b"text".as_slice(), a, b, b"more"]),
                "cut after {cut}"
            );
        }
        assert!(followed(&[b"\x1b[?1000;1004;1006h".as_slice()]));
        let taken_back: [&[u8]; 2] = [b"\x1b[?1004h", b"\x1b[?2004;1004l"];
        assert!(!followed(&taken_back));
        let reset: [&[u8]; 2] = [b"\x1b[?1004h", b"x\x1bc"];
        assert!(!followed(&reset));
        let alike: [&[u8]; 2] = [b"\x1b[?1004h", b"\x1b[?10045l\x1b[1004l\x1b[?1004$p"];
        assert!(followed(&alike), "only 1004 itself, set or reset, counts");
        assert!(!followed(&[b"\x1b[?10041h".as_slice()]));
    }

    fn client(visit: u64, stream: UnixStream) -> Client {
        Client {
            visit,
            out: Outbox::new(Box::new(stream.try_clone().unwrap())),
            stream,
            size: None,
        }
    }

    /// One viewer stops reading, as a laptop asleep with Cellix open: the program's output
    /// still goes to the other at once, the program isn't held up, and the one not reading
    /// is let go once it has left too much unread. How long it has read nothing is measured
    /// by a clock the test moves on: by the wall's, sending 32 MB on a loaded machine took
    /// longer than the program may wait, and the one awake could be let go for reading late.
    #[test]
    fn a_viewer_that_stops_reading_holds_up_neither_the_program_nor_the_others() {
        let (asleep, _never_read) = UnixStream::pair().unwrap();
        let (awake, mut reader) = UnixStream::pair().unwrap();
        let mut shared = Shared::default();
        let clock = Clock::stopped();
        shared.clock = clock.clone();
        let sleeper = client(1, asleep);
        let unread = sleeper.out.clone();
        shared.clients.push(sleeper);
        shared.clients.push(client(2, awake));
        let shared = Arc::new(Mutex::new(shared));
        let total = 32 * 1024 * 1024;
        let heard = std::thread::spawn(move || {
            let mut got = 0;
            while got < total {
                let Ok((kind, payload)) = read_frame(&mut reader) else {
                    break;
                };
                if kind == OUTPUT {
                    got += payload.len();
                }
            }
            got
        });
        let program = {
            let shared = shared.clone();
            std::thread::spawn(move || {
                let chunk = vec![b'x'; 8192];
                for _ in 0..total / chunk.len() {
                    take_output(&shared, &chunk);
                }
            })
        };
        // The one asleep falls far behind, and the program looks at the clock while it waits:
        // it's the first behind, and nothing else is. The deadline only keeps a broken host
        // from hanging the test; the clock isn't the wall's.
        let deadline = Instant::now() + Duration::from_secs(60);
        let until = |done: &dyn Fn() -> bool, what: &str| {
            while !done() {
                assert!(Instant::now() < deadline, "{what}");
                std::thread::sleep(Duration::from_millis(1));
            }
        };
        until(
            &|| unread.waiting() > BEHIND,
            "the one asleep never fell behind",
        );
        let reads = clock.reads();
        until(&|| clock.reads() > reads, "the program never waited");
        // Held while the one asleep might still read, however long that takes...
        std::thread::sleep(Duration::from_millis(50));
        assert!(!program.is_finished(), "the program went on before STALLED");
        assert_eq!(shared.lock().unwrap().clients.len(), 2, "let go too soon");
        // ...and not for ever: once it has read nothing for STALLED, it's let go.
        clock.advance(STALLED);
        program.join().unwrap();
        assert_eq!(
            heard.join().unwrap(),
            total,
            "the one awake heard all of it"
        );
        let shared = shared.lock().unwrap();
        let left: Vec<u64> = shared.clients.iter().map(|c| c.visit).collect();
        assert_eq!(left, [2], "the one asleep was let go");
    }
}
