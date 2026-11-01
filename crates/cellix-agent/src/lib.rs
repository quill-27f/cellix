//! The local socket any agent can use to reach a running Cellix. One line in, one line
//! out: `comments`, `take` (comments, then clear them) or `open <path>`.

use std::env;
use std::fs;
use std::io::{self, BufRead, BufReader, ErrorKind, Write};
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::thread;

use cellix_review::Comment;

/// Comments shared between the app and the socket thread.
pub type SharedComments = Arc<Mutex<Vec<Comment>>>;

/// Requests that need the app itself, drained on its next tick.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Request {
    Open(PathBuf),
    /// The socket thread already emptied the shared list.
    CommentsTaken,
}

fn fnv(text: &str) -> u64 {
    text.bytes().fold(0xcbf29ce484222325, |hash, byte| {
        (hash ^ byte as u64).wrapping_mul(0x100000001b3)
    })
}

/// One socket per workspace, in a directory only this user can enter.
pub fn socket_path(root: &Path) -> PathBuf {
    // SAFETY: getuid has no preconditions and cannot fail.
    let uid = unsafe { libc::getuid() };
    env::temp_dir()
        .join(format!("cellix-{uid}"))
        .join(format!("{:016x}.sock", fnv(&root.to_string_lossy())))
}

fn answer(line: &str, comments: &SharedComments, requests: &Sender<Request>) -> io::Result<String> {
    let mut parts = line.trim().splitn(2, ' ');
    let list = |clear: bool| -> io::Result<String> {
        let mut shared = comments.lock().map_err(|_| io::Error::other("poisoned"))?;
        let json = serde_json::to_string(&*shared)?;
        if clear {
            shared.clear();
        }
        Ok(json)
    };
    match (parts.next(), parts.next()) {
        (Some("comments"), None) => list(false),
        (Some("take"), None) => {
            let json = list(true)?;
            let _ = requests.send(Request::CommentsTaken);
            Ok(json)
        }
        (Some("open"), Some(path)) => {
            let _ = requests.send(Request::Open(PathBuf::from(path)));
            Ok("ok".into())
        }
        _ => Ok("error: unknown request".into()),
    }
}

fn serve(
    stream: UnixStream,
    comments: &SharedComments,
    requests: &Sender<Request>,
) -> io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let mut stream = stream;
    writeln!(stream, "{}", answer(&line, comments, requests)?)
}

/// The running server. Dropping it removes the socket file.
pub struct Server {
    path: PathBuf,
    pub requests: Receiver<Request>,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

/// Binds the workspace socket. Fails if another Cellix already serves this workspace.
pub fn start(root: &Path, comments: SharedComments) -> io::Result<Server> {
    let path = socket_path(root);
    let dir = path.parent().ok_or_else(|| io::Error::other("no parent"))?;
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)?;
    fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
    if UnixStream::connect(&path).is_ok() {
        return Err(io::Error::new(ErrorKind::AddrInUse, "already running"));
    }
    let _ = fs::remove_file(&path);
    let listener = UnixListener::bind(&path)?;
    let (sender, requests) = channel();
    thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let _ = serve(stream, &comments, &sender);
        }
    });
    Ok(Server { path, requests })
}

/// Sends one request to the Cellix serving `root`.
pub fn request(root: &Path, line: &str) -> io::Result<String> {
    let mut stream = UnixStream::connect(socket_path(root))?;
    writeln!(stream, "{line}")?;
    let mut reply = String::new();
    BufReader::new(stream).read_line(&mut reply)?;
    Ok(reply.trim_end().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn comment() -> Comment {
        Comment {
            file: "a.rs".into(),
            line: 3,
            text: "why?".into(),
        }
    }

    #[test]
    fn comments_can_be_read_and_taken() {
        let dir = tempfile::tempdir().unwrap();
        let shared: SharedComments = Arc::new(Mutex::new(vec![comment()]));
        let server = start(dir.path(), shared.clone()).unwrap();
        let json = request(dir.path(), "comments").unwrap();
        let parsed: Vec<Comment> = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, vec![comment()]);
        assert_eq!(shared.lock().unwrap().len(), 1);
        request(dir.path(), "take").unwrap();
        assert!(shared.lock().unwrap().is_empty());
        assert_eq!(server.requests.recv(), Ok(Request::CommentsTaken));
    }

    #[test]
    fn open_reaches_the_app() {
        let dir = tempfile::tempdir().unwrap();
        let server = start(dir.path(), Arc::default()).unwrap();
        assert_eq!(request(dir.path(), "open /tmp/x.rs").unwrap(), "ok");
        assert_eq!(
            server.requests.recv(),
            Ok(Request::Open(PathBuf::from("/tmp/x.rs")))
        );
    }

    #[test]
    fn a_second_server_is_refused_and_stale_sockets_are_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let server = start(dir.path(), Arc::default()).unwrap();
        assert!(start(dir.path(), Arc::default()).is_err());
        let path = server.path.clone();
        std::mem::forget(server);
        fs::remove_file(&path).unwrap();
        fs::write(&path, "stale").unwrap();
        assert!(start(dir.path(), Arc::default()).is_ok());
    }

    #[test]
    fn unknown_requests_get_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let _server = start(dir.path(), Arc::default()).unwrap();
        assert!(
            request(dir.path(), "nonsense")
                .unwrap()
                .starts_with("error")
        );
    }
}
