//! The workspace socket where there are no Unix sockets, as on Windows: a port on 127.0.0.1,
//! and a file where the socket would be that names the port, two tokens and the process. The
//! file sits in a folder only this user can read, and each side proves itself with a token
//! before anything else is said: the client first, then Cellix. Anything else on the machine
//! that finds the port is turned away, and anything that takes the port once its Cellix has
//! gone can't pass for it.
//!
//! The requests and answers are the socket's own lines, after the tokens'.

use std::fs;
use std::io::{self, BufRead, BufReader, ErrorKind, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::Path;
use std::time::Duration;

/// How long a connection has to give its token, so a stranger can't hold a thread open.
const INTRODUCTION: Duration = Duration::from_secs(5);
/// How long a check on an older Cellix waits.
const CHECK: Duration = Duration::from_secs(1);

/// The port, and the tokens each side gives.
pub struct Listener {
    listener: TcpListener,
    pub(crate) note: Note,
}

/// What the file at the socket's place says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Note {
    port: u16,
    /// What a client gives, so Cellix hears only this user.
    client: String,
    /// What Cellix answers, so a client tells only Cellix what it wants.
    server: String,
    pid: u32,
}

impl Note {
    fn read(path: &Path) -> io::Result<Self> {
        let text = fs::read_to_string(path)?;
        let invalid = || io::Error::new(ErrorKind::InvalidData, "not a Cellix endpoint");
        let mut parts = text.split_whitespace();
        let mut next = || parts.next().ok_or_else(invalid);
        let note = Self {
            port: next()?.parse().map_err(|_| invalid())?,
            client: next()?.to_string(),
            server: next()?.to_string(),
            pid: next()?.parse().map_err(|_| invalid())?,
        };
        Ok(note)
    }

    fn line(&self) -> String {
        format!(
            "{} {} {} {}\n",
            self.port, self.client, self.server, self.pid
        )
    }

    /// Whether `path` still holds this note, and not one a newer Cellix wrote.
    pub(crate) fn is_at(&self, path: &Path) -> bool {
        Self::read(path).is_ok_and(|note| note == *self)
    }

    /// Removes the note at `path` if it's still this one.
    fn forget(&self, path: &Path) {
        if self.is_at(path) {
            let _ = fs::remove_file(path);
        }
    }
}

fn write_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    options.open(path)?.write_all(bytes)
}

/// Listens on a new port, and notes it at `path` with new tokens.
pub fn bind(path: &Path) -> io::Result<Listener> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
    let note = Note {
        port: listener.local_addr()?.port(),
        client: crate::new_token()?,
        server: crate::new_token()?,
        pid: std::process::id(),
    };
    // Written whole beside it and then moved in, so a client never reads half of it. Named
    // for this process, so two starting at once don't write into one file.
    let partial = path.with_extension(format!("{}.partial", note.pid));
    write_private(&partial, note.line().as_bytes())?;
    fs::rename(&partial, path)?;
    Ok(Listener { listener, note })
}

fn same(given: &str, token: &str) -> bool {
    // Compared in full whatever differs, so the time taken says nothing about the token.
    given.len() == token.len()
        && given
            .bytes()
            .zip(token.bytes())
            .fold(0, |differ, (a, b)| differ | (a ^ b))
            == 0
}

/// Reads one short line, so a stranger can't fill memory with a line that never ends.
fn short_line(reader: &mut impl BufRead) -> io::Result<String> {
    let mut line = String::new();
    reader.take(256).read_line(&mut line)?;
    Ok(line.trim_end().to_string())
}

impl Listener {
    /// Every connection, before it has said who it is.
    pub fn incoming(&self) -> impl Iterator<Item = TcpStream> + '_ {
        self.listener.incoming().flatten()
    }

    /// Hears a connection out only if it gives the client's token first, and then answers
    /// with Cellix's. Returns the rest of what it sends, and where to answer it.
    pub fn admit(&self, mut stream: TcpStream) -> io::Result<(BufReader<TcpStream>, TcpStream)> {
        stream.set_read_timeout(Some(INTRODUCTION))?;
        let mut reader = BufReader::new(stream.try_clone()?);
        if !same(&short_line(&mut reader)?, &self.note.client) {
            return Err(io::Error::new(ErrorKind::PermissionDenied, "wrong token"));
        }
        writeln!(stream, "{}", self.note.server)?;
        stream.set_read_timeout(None)?;
        Ok((reader, stream))
    }
}

/// Whether the process `pid` is still running. A note outlives its Cellix when Windows closes
/// the window under it.
fn running(pid: u32) -> bool {
    #[cfg(unix)]
    {
        let Ok(pid) = libc::pid_t::try_from(pid) else {
            return false;
        };
        // SAFETY: signal 0 only asks whether the process exists; nothing is sent.
        let found = unsafe { libc::kill(pid, 0) } == 0;
        // Someone else's process is running too, though it can't be signalled.
        found || io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::{CloseHandle, ERROR_ACCESS_DENIED, GetLastError};
        use windows_sys::Win32::System::Threading::{
            GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
        };
        /// What GetExitCodeProcess gives for a process that hasn't ended.
        const STILL_ACTIVE: u32 = 259;
        // SAFETY: the handle is checked before use and closed after, and the exit code is
        // written to a local.
        unsafe {
            let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if process.is_null() {
                return GetLastError() == ERROR_ACCESS_DENIED;
            }
            let mut code = 0;
            let ok = GetExitCodeProcess(process, &mut code) != 0;
            CloseHandle(process);
            !ok || code == STILL_ACTIVE
        }
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = pid;
        true
    }
}

/// Connects to the Cellix noted at `path`, and has each side prove itself, ready for a
/// request. Gives up after `wait` at each step. A note whose Cellix has gone is removed, so
/// the next hook doesn't wait on it again.
pub fn connect(path: &Path, wait: Duration) -> io::Result<TcpStream> {
    let note = Note::read(path)?;
    let gone = |why: &str| {
        note.forget(path);
        io::Error::new(ErrorKind::NotFound, format!("no Cellix there: {why}"))
    };
    if !running(note.pid) {
        return Err(gone("its process has ended"));
    }
    let address = SocketAddr::from((Ipv4Addr::LOCALHOST, note.port));
    let mut stream = match TcpStream::connect_timeout(&address, wait) {
        Err(e) if e.kind() == ErrorKind::ConnectionRefused => return Err(gone("refused")),
        connected => connected?,
    };
    stream.set_read_timeout(Some(wait))?;
    writeln!(stream, "{}", note.client)?;
    // Nothing is asked of what answers until it gives Cellix's own token.
    let answer = short_line(&mut BufReader::new(stream.try_clone()?));
    match answer {
        Ok(answer) if same(&answer, &note.server) => Ok(stream),
        Ok(_) => Err(gone("something else holds its port")),
        Err(e) => Err(e),
    }
}

/// Whether a Cellix still answers at `path`, as only the one that wrote it can.
pub fn alive(path: &Path) -> bool {
    connect(path, CHECK).is_ok()
}
