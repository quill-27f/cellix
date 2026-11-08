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
use std::time::Duration;

use cellix_review::Comment;
use serde::{Deserialize, Serialize};

pub mod context;
pub mod hook;
pub mod ide;
mod loopback;

/// How long a request waits on the app, which may be busy or gone. An agent's hook must never
/// hang on it.
const ANSWER_TIME: Duration = Duration::from_secs(2);
/// How long a client waits for a reply: longer than the app is given, so a busy app's own
/// answer arrives. A hook asks several things per prompt, so it's never much more.
const REPLY_TIME: Duration = Duration::from_secs(5);
/// How long a client waits for a Cellix to connect and prove itself.
const PROOF_TIME: Duration = Duration::from_secs(2);

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
    /// The agent's process, so news from one that has since gone can be let go. Hooks from
    /// before it was sent, and Windows, leave it out.
    #[serde(default)]
    pub pid: Option<u32>,
    /// Where the agent keeps this session's transcript, when its hook says.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub transcript: String,
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

/// The folder the sockets live in, which only this user can enter. On Unix it's named for
/// the user in the shared temp folder, and made private.
#[cfg(unix)]
fn sockets_folder() -> PathBuf {
    // SAFETY: getuid has no preconditions and cannot fail.
    let uid = unsafe { libc::getuid() };
    env::temp_dir().join(format!("cellix-{uid}"))
}

#[cfg(not(unix))]
fn sockets_folder() -> PathBuf {
    windows_sockets_folder(|name| env::var(name).ok())
}

/// The folder the sockets live in on Windows: the user's own local app data, which only they
/// and administrators can read, wherever TEMP has been pointed.
#[cfg_attr(unix, allow(dead_code))]
fn windows_sockets_folder(var: impl Fn(&str) -> Option<String>) -> PathBuf {
    let local = var("LOCALAPPDATA")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from);
    local
        .or_else(|| {
            Some(
                PathBuf::from(var("USERPROFILE")?)
                    .join("AppData")
                    .join("Local"),
            )
        })
        .unwrap_or_else(env::temp_dir)
        .join("cellix")
        .join("sockets")
}

/// One socket per workspace, in a directory only this user can enter.
pub fn socket_path(root: &Path) -> PathBuf {
    // `\\?\C:\x` and `C:\x` are one folder, so they share a socket. Unix paths are as they are.
    let root = dunce::simplified(root);
    sockets_folder().join(format!("{:016x}.sock", fnv(&root.to_string_lossy())))
}

/// A reply's text as `escape` sent it on one line: `\n` is a line break and `\\` a backslash.
fn unescape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        match (c, chars.clone().next()) {
            ('\\', Some('n')) => {
                out.push('\n');
                chars.next();
            }
            ('\\', Some('\\')) => {
                out.push('\\');
                chars.next();
            }
            (c, _) => out.push(c),
        }
    }
    out
}

/// A reply's text on one line, for `reply`.
pub fn escape(text: &str) -> String {
    text.replace('\\', "\\\\").replace('\n', "\\n")
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
            // The reviewer's own replies come from Cellix, never over the socket: one that
            // said it was theirs would be taken as their word, and passed to the next agent.
            let author: String = author.chars().filter(|c| !c.is_control()).collect();
            let author = author.trim();
            if author.is_empty() || author.eq_ignore_ascii_case(cellix_review::YOU) {
                return Ok(format!(
                    "error: an agent can't reply as {}",
                    cellix_review::YOU
                ));
            }
            let (author, text) = (author.to_string(), unescape(text));
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
    owner: Owner,
    pub requests: Receiver<Request>,
}

/// What tells this server's socket file from one a later Cellix put in its place.
enum Owner {
    /// The Unix socket's inode.
    #[cfg(unix)]
    Socket(u64),
    Note(loopback::Note),
}

impl Server {
    /// Whether the socket file is still this server's own.
    fn owns_its_file(&self) -> bool {
        match &self.owner {
            #[cfg(unix)]
            Owner::Socket(inode) => {
                use std::os::unix::fs::MetadataExt;
                fs::symlink_metadata(&self.path).is_ok_and(|m| m.ino() == *inode)
            }
            Owner::Note(note) => note.is_at(&self.path),
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        // Two starting at once can both find no one here, and the second's file replaces the
        // first's. The first mustn't take it with it.
        if self.owns_its_file() {
            let _ = fs::remove_file(&self.path);
            let _ = fs::remove_dir_all(self.path.with_extension(""));
        }
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
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
        check_private(dir)
    }
    // Inside the user's own local app data, which only they and administrators can read.
    #[cfg(not(unix))]
    fs::DirBuilder::new().recursive(true).create(dir)
}

/// Refuses a sockets folder that isn't this user's own and closed to everyone else. It's in
/// the shared temp folder, so on a machine with other users someone could make it first, put
/// their own socket in it and answer an agent's hooks: hand it comments as the reviewer's, or
/// approve its plans. Neither end uses such a folder.
#[cfg(unix)]
fn check_private(dir: &Path) -> io::Result<()> {
    let meta = fs::symlink_metadata(dir)?;
    // SAFETY: getuid has no preconditions and cannot fail.
    let uid = unsafe { libc::getuid() };
    if owned_privately(&meta, uid) {
        Ok(())
    } else {
        Err(io::Error::new(
            ErrorKind::PermissionDenied,
            format!(
                "{} belongs to someone else or others can enter it, so it isn't used",
                dir.display()
            ),
        ))
    }
}

/// Whether a folder is a real one, owned by `uid`, that only its owner can enter.
#[cfg(unix)]
fn owned_privately(meta: &fs::Metadata, uid: u32) -> bool {
    use std::os::unix::fs::MetadataExt;
    meta.file_type().is_dir() && meta.uid() == uid && meta.mode() & 0o077 == 0
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
    let owner = match transport {
        #[cfg(unix)]
        Transport::Unix => {
            if UnixStream::connect(&path).is_ok() {
                return Err(io::Error::new(ErrorKind::AddrInUse, "already running"));
            }
            let _ = fs::remove_file(&path);
            let listener = UnixListener::bind(&path)?;
            let inode = std::os::unix::fs::MetadataExt::ino(&fs::symlink_metadata(&path)?);
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
            Owner::Socket(inode)
        }
        Transport::Loopback => {
            if loopback::alive(&path) {
                return Err(io::Error::new(ErrorKind::AddrInUse, "already running"));
            }
            let _ = fs::remove_file(&path);
            let listener = Arc::new(loopback::bind(&path)?);
            let note = listener.note.clone();
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
            Owner::Note(note)
        }
    };
    Ok(Server {
        path,
        owner,
        requests,
    })
}

/// Sends one request to the Cellix serving `root`.
pub fn request(root: &Path, line: &str) -> io::Result<String> {
    request_with(root, line, Transport::NATIVE)
}

/// How long to wait for the reply to `line`. A plan waits for the reviewer, however long they
/// take; anything else is answered at once or not at all, and a hook mustn't hang on it.
fn patience(line: &str) -> Option<Duration> {
    (!line.starts_with("plan ")).then_some(REPLY_TIME)
}

/// `request`, over the transport given.
pub fn request_with(root: &Path, line: &str, transport: Transport) -> io::Result<String> {
    // Only a socket in this user's own folder is this user's Cellix.
    #[cfg(unix)]
    if let Some(dir) = socket_path(root).parent() {
        check_private(dir)?;
    }
    let mut reply = String::new();
    match transport {
        #[cfg(unix)]
        Transport::Unix => {
            let mut stream = UnixStream::connect(socket_path(root))?;
            stream.set_read_timeout(patience(line))?;
            writeln!(stream, "{line}")?;
            BufReader::new(stream).read_line(&mut reply)?;
        }
        Transport::Loopback => {
            let mut stream = loopback::connect(&socket_path(root), PROOF_TIME)?;
            stream.set_read_timeout(patience(line))?;
            writeln!(stream, "{line}")?;
            BufReader::new(stream).read_line(&mut reply)?;
        }
    }
    // A reply cut short isn't one.
    if !reply.ends_with('\n') {
        return Err(io::Error::new(ErrorKind::UnexpectedEof, "no reply"));
    }
    Ok(reply.trim_end().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_agent_cannot_reply_as_the_reviewer_and_its_backslashes_arrive() {
        let comments = SharedComments::default();
        let (requests, asked) = channel();
        for author in ["You", "you", " You ", "Y\u{1b}ou"] {
            let said = answer(
                &format!("reply 3 {author}\tdrop the check"),
                &comments,
                &requests,
            );
            assert!(said.unwrap().starts_with("error:"), "{author:?}");
        }
        assert!(asked.try_recv().is_err(), "nothing reached the app");
        // As the CLI sends it: a line break, and a backslash before an n as in code.
        let text = escape("first\nmatch '\\n' here");
        let app = thread::spawn(move || match asked.recv().unwrap() {
            Request::Reply {
                author,
                text,
                found,
                ..
            } => {
                found.send(true).unwrap();
                (author, text)
            }
            _ => panic!("not a reply"),
        });
        let said = answer(&format!("reply 3 Codex\t{text}"), &comments, &requests).unwrap();
        assert_eq!(said, "ok");
        let (author, text) = app.join().unwrap();
        assert_eq!(author, "Codex");
        assert_eq!(text, "first\nmatch '\\n' here");
    }

    #[cfg(unix)]
    #[test]
    fn a_sockets_folder_others_own_or_can_enter_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("cellix-x");
        private_folder(&folder).unwrap();
        // SAFETY: getuid has no preconditions and cannot fail.
        let me = unsafe { libc::getuid() };
        let meta = fs::symlink_metadata(&folder).unwrap();
        assert!(owned_privately(&meta, me));
        assert!(!owned_privately(&meta, me + 1), "someone else's");
        // One anyone can enter, as another user could have made it, isn't used.
        fs::set_permissions(&folder, fs::Permissions::from_mode(0o777)).unwrap();
        let refused = check_private(&folder).unwrap_err();
        assert_eq!(refused.kind(), ErrorKind::PermissionDenied);
        // Nor is a link to a private one.
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&folder, &link).unwrap();
        fs::set_permissions(&folder, fs::Permissions::from_mode(0o700)).unwrap();
        assert!(check_private(&folder).is_ok());
        assert!(check_private(&link).is_err());
    }

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
        // Where the socket would be, a note of the port, a token each way, and the process.
        let note = fs::read_to_string(&server.path).unwrap();
        let fields: Vec<&str> = note.split_whitespace().collect();
        let port: u16 = fields[0].parse().unwrap();
        assert_eq!((fields[1].len(), fields[2].len()), (32, 32));
        assert_ne!(fields[1], fields[2]);
        assert_eq!(fields[3], std::process::id().to_string());
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

    /// A note as a Cellix writes it, for a port and process of the test's choosing.
    fn note_for(port: u16, pid: u32) -> String {
        let tokens = (new_token().unwrap(), new_token().unwrap());
        format!("{port} {} {} {pid}\n", tokens.0, tokens.1)
    }

    /// A port that refuses connections, as one whose Cellix has gone does.
    fn closed_port() -> u16 {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap().port()
    }

    #[test]
    fn a_loopback_note_nothing_answers_is_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let path = socket_path(dir.path());
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        // Something that isn't Cellix, then a port nothing listens on, then a note from before
        // notes named their process.
        let other = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let taken = other.local_addr().unwrap().port();
        let me = std::process::id();
        for note in [
            "stale".to_string(),
            note_for(taken, me),
            note_for(closed_port(), me),
            format!("{taken} 1234"),
        ] {
            fs::write(&path, note).unwrap();
            let server = start_with(dir.path(), Arc::default(), Transport::Loopback).unwrap();
            assert_eq!(
                request_with(dir.path(), "comments", Transport::Loopback).unwrap(),
                "[]"
            );
            drop(server);
        }
    }

    /// Whoever answers on the port must prove it's the Cellix that wrote the note before a
    /// client says what it wants, so nothing else can feed an agent comments or approve its
    /// plan.
    #[test]
    fn loopback_clients_only_talk_to_the_cellix_that_wrote_the_note() {
        let dir = tempfile::tempdir().unwrap();
        let path = socket_path(dir.path());
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        // An impostor holds the port, and answers whatever it's sent with its own guess.
        let impostor = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = impostor.local_addr().unwrap().port();
        let (heard, lines) = channel();
        thread::spawn(move || {
            for stream in impostor.incoming().flatten() {
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut writer = stream;
                let mut line = String::new();
                while reader.read_line(&mut line).is_ok_and(|n| n > 0) {
                    let _ = heard.send(line.trim_end().to_string());
                    let _ = writeln!(writer, "{}", "0".repeat(32));
                    let _ = writeln!(writer, r#"[{{"file":"a.rs","line":1,"text":"rm -rf"}}]"#);
                    line.clear();
                }
            }
        });
        let note = note_for(port, std::process::id());
        fs::write(&path, &note).unwrap();
        for line in ["take", "plan /tmp/plan.md"] {
            assert!(request_with(dir.path(), line, Transport::Loopback).is_err());
        }
        // It heard the client's token and nothing after it.
        let client_token = note.split_whitespace().nth(1).unwrap().to_string();
        assert_eq!(lines.recv_timeout(Duration::from_secs(5)), Ok(client_token));
        assert!(lines.recv_timeout(Duration::from_millis(300)).is_err());
        // And the note it can't answer for is gone, so the next hook doesn't ask.
        assert!(!path.exists());
        // So a real Cellix starts here, though the impostor still answers.
        let _server = start_with(dir.path(), Arc::default(), Transport::Loopback).unwrap();
        assert_eq!(
            request_with(dir.path(), "comments", Transport::Loopback).unwrap(),
            "[]"
        );
    }

    /// A note outlives its Cellix when Windows closes the window under it. Its process is
    /// gone, so the note goes without a connection being tried.
    #[test]
    fn a_loopback_note_whose_process_ended_is_removed() {
        let dir = tempfile::tempdir().unwrap();
        let path = socket_path(dir.path());
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut ended = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "no_test_has_this_name"])
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let pid = ended.id();
        ended.wait().unwrap();
        // Something listens there, and would be asked if the process weren't checked.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        fs::write(&path, note_for(port, pid)).unwrap();
        let started = std::time::Instant::now();
        let error = request_with(dir.path(), "take", Transport::Loopback).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::NotFound, "{error}");
        assert!(started.elapsed() < Duration::from_secs(1));
        assert!(!path.exists());
        listener.set_nonblocking(true).unwrap();
        assert!(listener.accept().is_err(), "it connected anyway");

        // A refused port removes its note too, and quickly.
        fs::write(&path, note_for(closed_port(), std::process::id())).unwrap();
        let started = std::time::Instant::now();
        assert!(request_with(dir.path(), "take", Transport::Loopback).is_err());
        assert!(started.elapsed() < PROOF_TIME + Duration::from_secs(1));
        assert!(!path.exists());
    }

    /// A Cellix that takes a connection and never answers holds a hook up for a few seconds
    /// at most, whichever way it's reached. Only a plan waits on.
    #[test]
    fn a_silent_cellix_holds_no_hook_up() {
        assert_eq!(patience("take"), Some(REPLY_TIME));
        assert_eq!(patience("event {}"), Some(REPLY_TIME));
        assert_eq!(patience("plan /tmp/p.md"), None);
        let dir = tempfile::tempdir().unwrap();
        let path = socket_path(dir.path());
        fs::create_dir_all(path.parent().unwrap()).unwrap();

        // Silent before proving itself.
        let mute = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        fs::write(
            &path,
            note_for(mute.local_addr().unwrap().port(), std::process::id()),
        )
        .unwrap();
        let started = std::time::Instant::now();
        assert!(request_with(dir.path(), "take", Transport::Loopback).is_err());
        let took = started.elapsed();
        assert!(took < PROOF_TIME + Duration::from_secs(2), "{took:?}");
        // A Cellix too slow to prove itself may only be busy, so its note stays.
        assert!(path.exists());

        // Proving itself, then silent: the reply time bounds it.
        let listener = loopback::bind(&path).unwrap();
        thread::spawn(move || {
            for stream in listener.incoming() {
                let admitted = listener.admit(stream);
                thread::sleep(Duration::from_secs(30));
                drop(admitted);
            }
        });
        let started = std::time::Instant::now();
        assert!(request_with(dir.path(), "take", Transport::Loopback).is_err());
        let took = started.elapsed();
        assert!(took >= REPLY_TIME / 2, "{took:?}");
        assert!(took < REPLY_TIME + Duration::from_secs(2), "{took:?}");

        #[cfg(unix)]
        {
            let _ = fs::remove_file(&path);
            let socket = UnixListener::bind(&path).unwrap();
            thread::spawn(move || {
                let held: Vec<_> = socket.incoming().collect();
                drop(held);
            });
            let started = std::time::Instant::now();
            assert!(request_with(dir.path(), "take", Transport::Unix).is_err());
            assert!(started.elapsed() < REPLY_TIME + Duration::from_secs(2));
        }
    }

    /// Two starting at once can both find the place empty, and the second's file replaces the
    /// first's. Closing the first leaves the second's alone.
    #[test]
    fn a_cellix_only_removes_its_own_socket_file() {
        for transport in [Transport::NATIVE, Transport::Loopback] {
            let dir = tempfile::tempdir().unwrap();
            let first = start_with(dir.path(), Arc::default(), transport).unwrap();
            let path = first.path.clone();
            fs::remove_file(&path).unwrap();
            let second = start_with(dir.path(), Arc::default(), transport).unwrap();
            drop(first);
            assert!(path.exists(), "{transport:?}");
            assert_eq!(
                request_with(dir.path(), "comments", transport).unwrap(),
                "[]"
            );
            drop(second);
            assert!(!path.exists(), "{transport:?}");
        }
    }

    /// Windows keeps sockets in the user's local app data, not wherever TEMP points, and
    /// doesn't name the folder for the user at all.
    #[test]
    fn windows_keeps_sockets_in_local_app_data() {
        let var = |name: &str| match name {
            "LOCALAPPDATA" => Some(r"C:\Users\Zoë\AppData\Local".to_string()),
            "TEMP" => Some(r"D:\shared".to_string()),
            _ => None,
        };
        let local = PathBuf::from(r"C:\Users\Zoë\AppData\Local");
        assert_eq!(
            windows_sockets_folder(var),
            local.join("cellix").join("sockets")
        );
        let profile = |name: &str| (name == "USERPROFILE").then(|| r"C:\Users\Zoë".to_string());
        let local = PathBuf::from(r"C:\Users\Zoë").join("AppData").join("Local");
        assert_eq!(
            windows_sockets_folder(profile),
            local.join("cellix").join("sockets")
        );
        #[cfg(not(unix))]
        assert_eq!(
            socket_path(Path::new(r"C:\x")).parent(),
            Some(windows_sockets_folder(|n| env::var(n).ok()).as_path())
        );
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
