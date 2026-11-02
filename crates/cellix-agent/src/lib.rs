//! The local socket any agent can use to reach a running Cellix. One line in, one line
//! out: `comments`, `take` (comments, then clear them), `threads`, `take-threads` (those an
//! agent hasn't seen), `reply <id> <author>` then a tab and the text, or `open <path>`.

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
use serde::{Deserialize, Serialize};

pub mod hook;
pub mod ide;

/// How long a request waits on the app, which may be busy or gone. An agent's hook must never
/// hang on it.
const ANSWER_TIME: std::time::Duration = std::time::Duration::from_secs(2);

/// Comments shared between the app and the socket thread.
pub type SharedComments = Arc<Mutex<Vec<Comment>>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgentState {
    Working,
    NeedsYou,
    Idle,
    Ended,
}

/// What an agent is doing, as its hooks report it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentEvent {
    pub session: String,
    pub agent: String,
    pub state: AgentState,
    pub detail: String,
    pub folder: String,
}

/// A test command an agent ran.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestRun {
    pub command: String,
    pub passed: bool,
    pub output: String,
}

/// The reviewer's answer to a plan.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Decision {
    /// With the plan's new text, if the reviewer edited it.
    Approve(Option<String>),
    SendBack(String),
}

/// Requests that need the app itself, drained on its next tick.
#[derive(Debug)]
pub enum Request {
    Open(PathBuf),
    /// The socket thread already emptied the shared list.
    CommentsTaken,
    Agent(AgentEvent),
    Test(TestRun),
    /// Input for a Cellix running as a pane, as the mod's JSON.
    Pane(String),
    /// An agent answering a comment.
    Reply {
        id: u32,
        author: String,
        text: String,
        /// Whether that comment exists, so an answer can't vanish with a success.
        found: Sender<bool>,
    },
    /// Wants the comments on the project, as JSON.
    Threads(Sender<String>),
    /// Wants the comments no agent has seen, as JSON, and marks them seen.
    TakeThreads(Sender<String>),
    /// A plan is waiting. The connection stays open until `reply` is answered or dropped.
    Plan {
        path: PathBuf,
        reply: Sender<Decision>,
    },
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
            // The hook takes on every prompt, and an empty take isn't news.
            if json != "[]" {
                let _ = requests.send(Request::CommentsTaken);
            }
            Ok(json)
        }
        (Some("open"), Some(path)) => {
            let _ = requests.send(Request::Open(PathBuf::from(path)));
            Ok("ok".into())
        }
        (Some("threads"), None) => {
            let (reply, threads) = channel();
            let _ = requests.send(Request::Threads(reply));
            Ok(threads
                .recv_timeout(ANSWER_TIME)
                .unwrap_or_else(|_| "[]".to_string()))
        }
        (Some("take-threads"), None) => {
            let (reply, threads) = channel();
            let _ = requests.send(Request::TakeThreads(reply));
            Ok(threads
                .recv_timeout(ANSWER_TIME)
                .unwrap_or_else(|_| "[]".to_string()))
        }
        (Some("reply"), Some(rest)) => {
            let Some((head, text)) = rest.split_once('\t') else {
                return Ok("error: reply needs an id, an author and text".into());
            };
            let (id, author) = head.split_once(' ').unwrap_or((head, "Agent"));
            let Ok(id) = id.parse() else {
                return Ok("error: reply needs a comment's number".into());
            };
            let (author, text) = (author.to_string(), text.replace("\\n", "\n"));
            let (found, answer) = channel();
            let _ = requests.send(Request::Reply {
                id,
                author,
                text,
                found,
            });
            Ok(match answer.recv_timeout(ANSWER_TIME) {
                Ok(true) => "ok".to_string(),
                _ => format!("error: there's no comment {id}"),
            })
        }
        (Some("pane"), Some(json)) => {
            let _ = requests.send(Request::Pane(json.to_string()));
            Ok("ok".into())
        }
        (Some("event"), Some(json)) => {
            let _ = requests.send(Request::Agent(serde_json::from_str(json)?));
            Ok("ok".into())
        }
        (Some("test"), Some(json)) => {
            let _ = requests.send(Request::Test(serde_json::from_str(json)?));
            Ok("ok".into())
        }
        (Some("plan"), Some(path)) => {
            let (reply, decision) = channel();
            let path = PathBuf::from(path);
            let _ = requests.send(Request::Plan { path, reply });
            // Quitting drops the sender, and the agent goes back to asking as usual.
            Ok(match decision.recv() {
                Ok(Decision::Approve(None)) => "approve".to_string(),
                Ok(Decision::Approve(Some(text))) => {
                    format!("approve {}", serde_json::to_string(&text)?)
                }
                Ok(Decision::SendBack(text)) => {
                    format!("send-back {}", serde_json::to_string(&text)?)
                }
                Err(_) => "pass".to_string(),
            })
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
            // One thread each, because a plan holds its connection open for as long as
            // the reviewer takes.
            let (comments, sender) = (comments.clone(), sender.clone());
            thread::spawn(move || {
                let _ = serve(stream, &comments, &sender);
            });
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
        assert!(matches!(server.requests.recv(), Ok(Request::CommentsTaken)));
    }

    #[test]
    fn open_reaches_the_app() {
        let dir = tempfile::tempdir().unwrap();
        let server = start(dir.path(), Arc::default()).unwrap();
        assert_eq!(request(dir.path(), "open /tmp/x.rs").unwrap(), "ok");
        assert!(matches!(
            server.requests.recv(),
            Ok(Request::Open(path)) if path == Path::new("/tmp/x.rs")
        ));
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
