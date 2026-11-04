//! The local socket any agent can use to reach a running Cellix. One line in, one line
//! out: `comments`, `take` (comments, then clear them), `threads`, `take-threads` (those an
//! agent hasn't seen), `reply <id> <author>` then a tab and the text, or `open <path>`.
//!
//! It's a Unix socket where there are those, and elsewhere a port on 127.0.0.1 behind a token,
//! as `loopback` describes. Agents call `cellix`, so only this crate tells the two apart.

use std::env;
use std::fs;
use std::io::{self, BufRead, BufReader, ErrorKind, Write};
#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
#[cfg(unix)]
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::thread;

use cellix_review::Comment;
use serde::{Deserialize, Serialize};

pub mod hook;
pub mod ide;
mod loopback;

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
    /// An agent suggesting a comment the reviewer didn't make. Answers with its number.
    Suggest {
        file: String,
        line: usize,
        kind: String,
        quote: String,
        text: String,
        author: String,
        made: Sender<u32>,
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

/// How agents reach a Cellix.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transport {
    /// A Unix socket at `socket_path`.
    #[cfg(unix)]
    Unix,
    /// A port on 127.0.0.1, named with its token in a file at `socket_path`.
    Loopback,
}

impl Transport {
    /// What `start` and `request` use here.
    #[cfg(unix)]
    pub const NATIVE: Self = Self::Unix;
    #[cfg(not(unix))]
    pub const NATIVE: Self = Self::Loopback;
}

/// A token no one else can guess.
pub(crate) fn new_token() -> io::Result<String> {
    let mut bytes = [0u8; 16];
    #[cfg(unix)]
    io::Read::read_exact(&mut fs::File::open("/dev/urandom")?, &mut bytes)?;
    #[cfg(not(unix))]
    getrandom::fill(&mut bytes).map_err(io::Error::other)?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

/// Who this is, to keep each user's sockets apart.
#[cfg(unix)]
fn user() -> String {
    // SAFETY: getuid has no preconditions and cannot fail.
    unsafe { libc::getuid() }.to_string()
}

/// Who this is, to keep each user's sockets apart. Windows gives each user a temp folder of
/// their own as well.
#[cfg(not(unix))]
fn user() -> String {
    let name = env::var("USERNAME").unwrap_or_default();
    name.chars().filter(|c| c.is_ascii_alphanumeric()).collect()
}

/// One socket per workspace, in a directory only this user can enter.
pub fn socket_path(root: &Path) -> PathBuf {
    // `\\?\C:\x` and `C:\x` are one folder, so they share a socket. Unix paths are as they are.
    let root = dunce::simplified(root);
    let uid = user();
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
        (Some("suggest"), Some(json)) => {
            let value: serde_json::Value = serde_json::from_str(json)?;
            let field = |name: &str| value[name].as_str().unwrap_or_default().to_string();
            let (made, answer) = channel();
            let _ = requests.send(Request::Suggest {
                file: field("file"),
                line: value["line"].as_u64().unwrap_or(1) as usize,
                kind: field("kind"),
                quote: field("quote"),
                text: field("text"),
                author: field("author"),
                made,
            });
            Ok(match answer.recv_timeout(ANSWER_TIME) {
                Ok(id) => format!("ok {id}"),
                Err(_) => "error: Cellix didn't take the suggestion".to_string(),
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
    mut reader: impl BufRead,
    mut stream: impl Write,
    comments: &SharedComments,
    requests: &Sender<Request>,
) -> io::Result<()> {
    let mut line = String::new();
    reader.read_line(&mut line)?;
    writeln!(stream, "{}", answer(&line, comments, requests)?)
}

/// The running server. Dropping it removes the socket file, and the plans kept beside it.
pub struct Server {
    path: PathBuf,
    pub requests: Receiver<Request>,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
        let _ = fs::remove_dir_all(self.path.with_extension(""));
    }
}

/// Makes the folder the sockets live in, which only this user can enter.
fn private_folder(dir: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)?;
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700))
    }
    // Inside the user's own temp folder, which only they and administrators can read.
    #[cfg(not(unix))]
    fs::DirBuilder::new().recursive(true).create(dir)
}

/// Binds the workspace socket. Fails if another Cellix already serves this workspace.
pub fn start(root: &Path, comments: SharedComments) -> io::Result<Server> {
    start_with(root, comments, Transport::NATIVE)
}

/// `start`, over the transport given.
pub fn start_with(
    root: &Path,
    comments: SharedComments,
    transport: Transport,
) -> io::Result<Server> {
    let path = socket_path(root);
    let dir = path.parent().ok_or_else(|| io::Error::other("no parent"))?;
    private_folder(dir)?;
    let (sender, requests) = channel();
    match transport {
        #[cfg(unix)]
        Transport::Unix => {
            if UnixStream::connect(&path).is_ok() {
                return Err(io::Error::new(ErrorKind::AddrInUse, "already running"));
            }
            let _ = fs::remove_file(&path);
            let listener = UnixListener::bind(&path)?;
            thread::spawn(move || {
                for stream in listener.incoming().flatten() {
                    // One thread each, because a plan holds its connection open for as long as
                    // the reviewer takes.
                    let (comments, sender) = (comments.clone(), sender.clone());
                    thread::spawn(move || {
                        let reader = BufReader::new(stream.try_clone()?);
                        serve(reader, stream, &comments, &sender)
                    });
                }
            });
        }
        Transport::Loopback => {
            if loopback::alive(&path) {
                return Err(io::Error::new(ErrorKind::AddrInUse, "already running"));
            }
            let _ = fs::remove_file(&path);
            let listener = Arc::new(loopback::bind(&path)?);
            thread::spawn(move || {
                for stream in listener.incoming() {
                    let (comments, sender) = (comments.clone(), sender.clone());
                    let listener = listener.clone();
                    // Admitted on its own thread, so a stranger that never sends a token
                    // holds up no one else.
                    thread::spawn(move || {
                        let (reader, stream) = listener.admit(stream)?;
                        serve(reader, stream, &comments, &sender)
                    });
                }
            });
        }
    }
    Ok(Server { path, requests })
}

/// Sends one request to the Cellix serving `root`.
pub fn request(root: &Path, line: &str) -> io::Result<String> {
    request_with(root, line, Transport::NATIVE)
}

/// `request`, over the transport given.
pub fn request_with(root: &Path, line: &str, transport: Transport) -> io::Result<String> {
    let mut reply = String::new();
    match transport {
        #[cfg(unix)]
        Transport::Unix => {
            let mut stream = UnixStream::connect(socket_path(root))?;
            writeln!(stream, "{line}")?;
            BufReader::new(stream).read_line(&mut reply)?;
        }
        Transport::Loopback => {
            let mut stream = loopback::connect(&socket_path(root), None)?;
            writeln!(stream, "{line}")?;
            BufReader::new(stream).read_line(&mut reply)?;
        }
    }
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
            ..Comment::default()
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
    fn a_closed_server_takes_its_plans_with_it() {
        let dir = tempfile::tempdir().unwrap();
        let server = start(dir.path(), Arc::default()).unwrap();
        let plans = server.path.with_extension("").join("s1");
        fs::create_dir_all(&plans).unwrap();
        fs::write(plans.join("Plan.md"), "plan").unwrap();
        let path = server.path.clone();
        drop(server);
        assert!(!path.exists());
        assert!(!path.with_extension("").exists());
    }

    /// What Windows uses, proved on every platform.
    #[test]
    fn agents_reach_cellix_over_loopback_with_its_token() {
        let dir = tempfile::tempdir().unwrap();
        let shared: SharedComments = Arc::new(Mutex::new(vec![comment()]));
        let over = Transport::Loopback;
        let server = start_with(dir.path(), shared.clone(), over).unwrap();
        // Where the socket would be, a note of the port and a token.
        let note = fs::read_to_string(&server.path).unwrap();
        let (port, token) = note.trim().split_once(' ').unwrap();
        let port: u16 = port.parse().unwrap();
        assert_eq!(token.len(), 32);
        #[cfg(unix)]
        assert_eq!(
            fs::metadata(&server.path).unwrap().permissions().mode() & 0o777,
            0o600
        );

        let json = request_with(dir.path(), "comments", over).unwrap();
        let parsed: Vec<Comment> = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, vec![comment()]);
        assert_eq!(
            request_with(dir.path(), "open /tmp/x.rs", over).unwrap(),
            "ok"
        );
        assert!(matches!(
            server.requests.recv(),
            Ok(Request::Open(path)) if path == Path::new("/tmp/x.rs")
        ));
        request_with(dir.path(), "take", over).unwrap();
        assert!(shared.lock().unwrap().is_empty());
        assert!(matches!(server.requests.recv(), Ok(Request::CommentsTaken)));

        // Anything else that finds the port gets nothing without the token, even with a
        // guess as long as it.
        for guess in ["comments", &"0".repeat(32)] {
            let mut stranger = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
            writeln!(stranger, "{guess}\ncomments").unwrap();
            let mut reply = String::new();
            let _ = BufReader::new(stranger).read_line(&mut reply);
            assert_eq!(reply, "");
        }
        shared.lock().unwrap().push(comment());
        assert_ne!(request_with(dir.path(), "comments", over).unwrap(), "[]");

        // A second Cellix here is refused while this one answers.
        assert!(start_with(dir.path(), Arc::default(), over).is_err());
        let path = server.path.clone();
        drop(server);
        assert!(!path.exists());
    }

    #[test]
    fn a_loopback_note_nothing_answers_is_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let path = socket_path(dir.path());
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        // A port nothing listens on, then one held by something that isn't Cellix.
        let other = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let taken = other.local_addr().unwrap().port();
        for note in ["stale".to_string(), format!("{taken} 1234")] {
            fs::write(&path, note).unwrap();
            let server = start_with(dir.path(), Arc::default(), Transport::Loopback).unwrap();
            assert_eq!(
                request_with(dir.path(), "comments", Transport::Loopback).unwrap(),
                "[]"
            );
            drop(server);
        }
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
