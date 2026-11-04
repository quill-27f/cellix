//! The workspace socket where there are no Unix sockets, as on Windows: a port on 127.0.0.1,
//! and a file where the socket would be that names the port and a token. The file sits in the
//! same private folder, so only this user can read the token, and a connection must give it
//! before it's heard. Anything else on the machine that finds the port is turned away.
//!
//! The requests and answers are the socket's own lines, after the token's.

use std::fs;
use std::io::{self, BufRead, BufReader, ErrorKind, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::Path;
use std::time::Duration;

/// How long a connection has to give the token, so a stranger can't hold a thread open.
const INTRODUCTION: Duration = Duration::from_secs(5);
/// How long a check on an older Cellix waits, as a Unix socket's connect would not.
const CHECK: Duration = Duration::from_secs(1);

/// The port, and the token that opens it.
pub struct Listener {
    listener: TcpListener,
    token: String,
}

fn write_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    options.open(path)?.write_all(bytes)
}

/// Listens on a new port, and notes it at `path` with a new token.
pub fn bind(path: &Path) -> io::Result<Listener> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
    let port = listener.local_addr()?.port();
    let token = crate::new_token()?;
    // Written whole beside it and then moved in, so a client never reads half of it.
    let partial = path.with_extension("partial");
    write_private(&partial, format!("{port} {token}\n").as_bytes())?;
    fs::rename(&partial, path)?;
    Ok(Listener { listener, token })
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

impl Listener {
    /// Every connection, before it has said who it is.
    pub fn incoming(&self) -> impl Iterator<Item = TcpStream> + '_ {
        self.listener.incoming().flatten()
    }

    /// Hears a connection out only if it gives the token first. Returns the rest of what it
    /// sends, and where to answer it.
    pub fn admit(&self, stream: TcpStream) -> io::Result<(BufReader<TcpStream>, TcpStream)> {
        stream.set_read_timeout(Some(INTRODUCTION))?;
        let mut reader = BufReader::new(stream.try_clone()?);
        let mut line = String::new();
        // A token is short, and a stranger mustn't fill memory with a line that never ends.
        (&mut reader).take(256).read_line(&mut line)?;
        if !same(line.trim_end(), &self.token) {
            return Err(io::Error::new(ErrorKind::PermissionDenied, "wrong token"));
        }
        stream.set_read_timeout(None)?;
        Ok((reader, stream))
    }
}

/// Connects to the Cellix noted at `path` and gives its token, ready for a request.
pub fn connect(path: &Path, wait: Option<Duration>) -> io::Result<TcpStream> {
    let note = fs::read_to_string(path)?;
    let invalid = || io::Error::new(ErrorKind::InvalidData, "not a Cellix endpoint");
    let (port, token) = note.trim().split_once(' ').ok_or_else(invalid)?;
    let port: u16 = port.parse().map_err(|_| invalid())?;
    let address = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let mut stream = match wait {
        Some(wait) => TcpStream::connect_timeout(&address, wait)?,
        None => TcpStream::connect(address)?,
    };
    writeln!(stream, "{token}")?;
    Ok(stream)
}

/// Whether a Cellix still answers at `path`. A port can be taken by something else once its
/// Cellix has gone, so this asks for the comments, which needs nothing of the app, and looks
/// for a list.
pub fn alive(path: &Path) -> bool {
    let Ok(mut stream) = connect(path, Some(CHECK)) else {
        return false;
    };
    let mut reply = String::new();
    stream.set_read_timeout(Some(CHECK)).is_ok()
        && writeln!(stream, "comments").is_ok()
        && BufReader::new(stream).read_line(&mut reply).is_ok()
        && reply.starts_with('[')
}
