//! The server Claude Code connects to when it treats Cellix as its IDE: MCP over a local
//! WebSocket, announced by a lock file. Claude Code hasn't published this protocol, so it
//! follows what its VS Code extension does.

// tungstenite's handshake callback has to return its large error type.
#![allow(clippy::result_large_err)]

use std::fs;
use std::io::{self, ErrorKind};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use serde_json::{Value, json};
use tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tungstenite::http::{HeaderValue, StatusCode};
use tungstenite::{Message, WebSocket};

use crate::new_token;

const AUTH_HEADER: &str = "x-claude-code-ide-authorization";
const SUBPROTOCOL: &str = "mcp";
/// How long a connection waits for input before checking for notifications to send.
const POLL: Duration = Duration::from_millis(50);

/// Tools answered by the app, because they need its state. `openDiff` holds its reply
/// until the reviewer decides.
const TOOLS: [(&str, &str); 10] = [
    ("openFile", "Open a file in Cellix"),
    (
        "openDiff",
        "Show a proposed change and wait for the reviewer",
    ),
    ("getCurrentSelection", "The selection in the active editor"),
    ("getLatestSelection", "The most recent selection"),
    ("getOpenEditors", "The open tabs"),
    ("getDiagnostics", "Language server errors and warnings"),
    ("checkDocumentDirty", "Whether a file has unsaved edits"),
    ("saveDocument", "Save a file"),
    ("close_tab", "Close a proposed change"),
    ("closeAllDiffTabs", "Close every proposed change"),
];

/// A tool call for the app. `reply` takes the text of each content item.
#[derive(Debug)]
pub struct IdeCall {
    pub tool: String,
    pub args: Value,
    pub reply: Sender<Vec<String>>,
}

type Clients = Arc<Mutex<Vec<Sender<String>>>>;

/// The running server. Dropping it removes the lock file, which is what hides this
/// Cellix from `/ide`.
pub struct IdeServer {
    pub port: u16,
    lock: PathBuf,
    clients: Clients,
    pub calls: Receiver<IdeCall>,
}

impl IdeServer {
    /// The lock file that tells Claude Code this Cellix is here.
    pub fn lock(&self) -> &Path {
        &self.lock
    }
}

impl Drop for IdeServer {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.lock);
    }
}

fn config_dir() -> Option<PathBuf> {
    let configured = std::env::var_os("CLAUDE_CONFIG_DIR").map(PathBuf::from);
    configured.or_else(|| Some(PathBuf::from(std::env::var_os("HOME")?).join(".claude")))
}

/// A path as a file URL. A Windows path starts with its drive, `file:///C:/x`.
pub fn file_url(path: &Path) -> String {
    let text = path.display().to_string();
    if cfg!(windows) {
        format!("file:///{}", text.replace('\\', "/"))
    } else {
        format!("file://{text}")
    }
}

/// The path a file URL names, or the text as it is if it isn't one.
pub fn url_path(text: &str) -> &str {
    let Some(path) = text.strip_prefix("file://") else {
        return text;
    };
    // `file:///C:/x` is `C:/x`, which Windows takes.
    let drive = path
        .strip_prefix('/')
        .filter(|rest| rest.get(1..2) == Some(":"));
    match drive {
        Some(rest) if cfg!(windows) => rest,
        _ => path,
    }
}

/// A folder only this user can enter. On Windows, the user's profile already is one.
fn private_folder(folder: &Path) -> io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    builder.mode(0o700);
    builder.create(folder)
}

/// Writes a file only this user can read.
fn write_private(file: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    options.mode(0o600);
    io::Write::write_all(&mut options.open(file)?, bytes)
}

impl IdeServer {
    pub fn start(root: &Path) -> io::Result<Self> {
        let config = config_dir().ok_or_else(|| io::Error::other("no home folder"))?;
        Self::start_in(root, &config, None)
    }

    /// Starts on the port and token noted in `kept`, if it can, and notes what it used. An
    /// agent that outlived the last Cellix was started pointing at that port, so this is the
    /// only way it can find the next one.
    pub fn start_kept(root: &Path, kept: &Path) -> io::Result<Self> {
        let config = config_dir().ok_or_else(|| io::Error::other("no home folder"))?;
        Self::start_in(root, &config, Some(kept))
    }

    fn start_in(root: &Path, config: &Path, kept: Option<&Path>) -> io::Result<Self> {
        let noted: Option<Value> = kept
            .and_then(|file| fs::read(file).ok())
            .and_then(|bytes| serde_json::from_slice(&bytes).ok());
        let old_port = noted.as_ref().and_then(|n| n["port"].as_u64());
        let old_token = noted.as_ref().and_then(|n| n["token"].as_str());
        let again = old_port
            .and_then(|port| u16::try_from(port).ok())
            .and_then(|port| TcpListener::bind((Ipv4Addr::LOCALHOST, port)).ok());
        let reused = again.is_some();
        let listener = match again {
            Some(listener) => listener,
            None => TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?,
        };
        let port = listener.local_addr()?.port();
        let token = match old_token.filter(|_| reused) {
            Some(token) => token.to_string(),
            None => new_token()?,
        };
        if let Some(file) = kept {
            if let Some(parent) = file.parent() {
                private_folder(parent)?;
            }
            let noted = json!({"port": port, "token": token}).to_string();
            write_private(file, noted.as_bytes())?;
        }
        let folder = config.join("ide");
        private_folder(&folder)?;
        let lock = folder.join(format!("{port}.lock"));
        let contents = json!({
            "pid": std::process::id(),
            "workspaceFolders": [root],
            "ideName": "Cellix",
            "transport": "ws",
            "authToken": token,
        });
        write_private(&lock, contents.to_string().as_bytes())?;
        let (sender, calls) = channel();
        let clients = Clients::default();
        let shared = clients.clone();
        let root = root.to_path_buf();
        thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let context = (token.clone(), root.clone(), sender.clone(), shared.clone());
                thread::spawn(move || connection(stream, context));
            }
        });
        Ok(Self {
            port,
            lock,
            clients,
            calls,
        })
    }

    /// Tells every connected client something changed.
    pub fn notify(&self, method: &str, params: Value) {
        let message = json!({"jsonrpc": "2.0", "method": method, "params": params}).to_string();
        if let Ok(mut clients) = self.clients.lock() {
            clients.retain(|client| client.send(message.clone()).is_ok());
        }
    }

    pub fn connected(&self) -> bool {
        self.clients.lock().is_ok_and(|c| !c.is_empty())
    }
}

/// Only a client holding the token gets in. Browsers can't set this header, and refusing
/// any Origin also keeps web pages off a port on localhost.
fn authorize(
    token: &str,
    request: &Request,
    mut response: Response,
) -> Result<Response, ErrorResponse> {
    let headers = request.headers();
    let allowed = headers
        .get(AUTH_HEADER)
        .is_some_and(|value| value.as_bytes() == token.as_bytes());
    if !allowed || headers.contains_key("origin") {
        let mut refused = ErrorResponse::new(None);
        *refused.status_mut() = StatusCode::UNAUTHORIZED;
        return Err(refused);
    }
    let asked = headers
        .get("sec-websocket-protocol")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.split(',').any(|p| p.trim() == SUBPROTOCOL));
    if asked {
        response.headers_mut().insert(
            "sec-websocket-protocol",
            HeaderValue::from_static(SUBPROTOCOL),
        );
    }
    Ok(response)
}

fn connection(stream: TcpStream, context: (String, PathBuf, Sender<IdeCall>, Clients)) {
    let (token, root, calls, clients) = context;
    let _ = stream.set_nodelay(true);
    let check = |request: &Request, response: Response| authorize(&token, request, response);
    let Ok(mut socket) = tungstenite::accept_hdr(stream, check) else {
        return;
    };
    let (outgoing, queue) = channel();
    if let Ok(mut clients) = clients.lock() {
        clients.push(outgoing.clone());
    }
    if socket.get_ref().set_read_timeout(Some(POLL)).is_ok() {
        let _ = pump(&mut socket, &queue, &outgoing, &root, &calls);
    }
    // Sends fail once `queue` drops, and the next notification clears this client out.
    drop(queue);
    if let Ok(mut clients) = clients.lock() {
        clients.retain(|client| client.send(String::new()).is_ok());
    }
}

fn pump(
    socket: &mut WebSocket<TcpStream>,
    queue: &Receiver<String>,
    outgoing: &Sender<String>,
    root: &Path,
    calls: &Sender<IdeCall>,
) -> tungstenite::Result<()> {
    loop {
        match socket.read() {
            Ok(Message::Text(text)) => {
                if let Some(reply) = answer(text.as_str(), root, calls, outgoing) {
                    socket.send(Message::text(reply))?;
                }
            }
            Ok(Message::Close(_)) => return Ok(()),
            Ok(_) => {}
            Err(tungstenite::Error::Io(e))
                if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
            Err(e) => return Err(e),
        }
        while let Ok(message) = queue.try_recv() {
            if !message.is_empty() {
                socket.send(Message::text(message))?;
            }
        }
    }
}

fn result(id: &Value, result: Value) -> String {
    json!({"jsonrpc": "2.0", "id": id, "result": result}).to_string()
}

fn tool_list() -> Value {
    let open = json!({"type": "object", "additionalProperties": true});
    let mut tools: Vec<Value> = TOOLS
        .iter()
        .map(|(name, description)| {
            json!({"name": name, "description": description, "inputSchema": open})
        })
        .collect();
    let folders = ("getWorkspaceFolders", "The folders Cellix has open");
    tools.push(json!({"name": folders.0, "description": folders.1, "inputSchema": open}));
    json!({"tools": tools})
}

fn content(texts: Vec<String>) -> Value {
    let items: Vec<Value> = texts
        .into_iter()
        .map(|text| json!({"type": "text", "text": text}))
        .collect();
    json!({"content": items})
}

fn workspace_folders(root: &Path) -> String {
    let name = root.file_name().unwrap_or_default().to_string_lossy();
    json!({
        "success": true,
        "folders": [{"name": name, "uri": file_url(root), "path": root}],
        "rootPath": root,
    })
    .to_string()
}

/// Answers one message, or returns None for notifications and for calls that finish later.
fn answer(
    text: &str,
    root: &Path,
    calls: &Sender<IdeCall>,
    outgoing: &Sender<String>,
) -> Option<String> {
    let message: Value = serde_json::from_str(text).ok()?;
    let id = message.get("id")?;
    let params = &message["params"];
    Some(match message["method"].as_str()? {
        "initialize" => {
            let version = params["protocolVersion"].as_str().unwrap_or("2025-03-26");
            result(
                id,
                json!({
                    "protocolVersion": version,
                    "capabilities": {"tools": {"listChanged": false}},
                    "serverInfo": {"name": "cellix", "version": env!("CARGO_PKG_VERSION")},
                }),
            )
        }
        "ping" => result(id, json!({})),
        "tools/list" => result(id, tool_list()),
        "prompts/list" => result(id, json!({"prompts": []})),
        "resources/list" => result(id, json!({"resources": []})),
        "tools/call" => {
            let tool = params["name"].as_str().unwrap_or_default().to_string();
            if tool == "getWorkspaceFolders" {
                return Some(result(id, content(vec![workspace_folders(root)])));
            }
            let args = params["arguments"].clone();
            let (reply, done) = channel();
            let fallback = tool_fallback(&tool, &args);
            let _ = calls.send(IdeCall { tool, args, reply });
            // On its own thread, because a diff can wait for as long as the reviewer takes.
            let (id, outgoing) = (id.clone(), outgoing.clone());
            thread::spawn(move || {
                let texts = done.recv().unwrap_or(fallback);
                let _ = outgoing.send(result(&id, content(texts)));
            });
            return None;
        }
        method => json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": {"code": -32601, "message": format!("Unknown method {method}")},
        })
        .to_string(),
    })
}

/// What a call answers if the app goes away first. Nothing was approved, so nothing is.
fn tool_fallback(tool: &str, args: &Value) -> Vec<String> {
    if tool == "openDiff" {
        let tab = args["tab_name"].as_str().unwrap_or_default().to_string();
        vec!["DIFF_REJECTED".to_string(), tab]
    } else {
        vec!["Cellix closed before answering".to_string()]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_next_cellix_takes_the_same_port_and_token() {
        // Between freeing a port and taking it again, another test or program on a busy
        // machine can take it, which is the case `a_port_someone_else_took_is_given_up` covers.
        // Only that is tried again; anything else fails as before.
        for _ in 0..5 {
            let dir = tempfile::tempdir().unwrap();
            let root = dir.path().join("project");
            let kept = dir.path().join("held/ide.json");
            // The port a Cellix that has since quit was on.
            let free = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
            let port = free.local_addr().unwrap().port();
            drop(free);
            fs::create_dir_all(kept.parent().unwrap()).unwrap();
            let noted = json!({"port": port, "token": "kept-token"}).to_string();
            fs::write(&kept, noted).unwrap();
            let next = IdeServer::start_in(&root, dir.path(), Some(&kept)).unwrap();
            let taken =
                next.port != port && TcpListener::bind((Ipv4Addr::LOCALHOST, port)).is_err();
            if taken {
                continue;
            }
            assert_eq!(next.port, port);
            assert_eq!(lock_of(dir.path(), port)["authToken"], "kept-token");
            return;
        }
        panic!("something else took the freed port five times running");
    }

    #[test]
    fn a_port_someone_else_took_is_given_up() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("project");
        let kept = dir.path().join("ide.json");
        let first = IdeServer::start_in(&root, dir.path(), Some(&kept)).unwrap();
        // Still running, so the port's taken: a second window here gets its own.
        let second = IdeServer::start_in(&root, dir.path(), Some(&kept)).unwrap();
        assert_ne!(first.port, second.port);
        let (first_token, second_token) = (
            lock_of(dir.path(), first.port)["authToken"].clone(),
            lock_of(dir.path(), second.port)["authToken"].clone(),
        );
        assert_ne!(first_token, second_token, "a token goes with its port");
    }
    use tungstenite::client::IntoClientRequest;
    use tungstenite::stream::MaybeTlsStream;

    type Client = WebSocket<MaybeTlsStream<TcpStream>>;

    fn lock_of(config: &Path, port: u16) -> Value {
        let lock = config.join("ide").join(format!("{port}.lock"));
        serde_json::from_str(&fs::read_to_string(lock).unwrap()).unwrap()
    }

    fn connect(port: u16, token: &str) -> tungstenite::Result<Client> {
        let mut request = format!("ws://127.0.0.1:{port}")
            .into_client_request()
            .unwrap();
        let headers = request.headers_mut();
        headers.insert(AUTH_HEADER, token.parse().unwrap());
        headers.insert("sec-websocket-protocol", "mcp".parse().unwrap());
        let (client, response) = tungstenite::connect(request)?;
        assert_eq!(response.headers()["sec-websocket-protocol"], SUBPROTOCOL);
        Ok(client)
    }

    fn start() -> (tempfile::TempDir, IdeServer, Client) {
        let dir = tempfile::tempdir().unwrap();
        let server = IdeServer::start_in(dir.path(), dir.path(), None).unwrap();
        let token = lock_of(dir.path(), server.port)["authToken"]
            .as_str()
            .unwrap()
            .to_string();
        let client = connect(server.port, &token).unwrap();
        (dir, server, client)
    }

    fn call(client: &mut Client, id: u32, method: &str, params: Value) -> Value {
        let message = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        client.send(Message::text(message.to_string())).unwrap();
        read(client)
    }

    fn read(client: &mut Client) -> Value {
        let message = client.read().unwrap();
        serde_json::from_str(message.to_text().unwrap()).unwrap()
    }

    #[test]
    fn claude_code_can_connect_and_list_tools() {
        let (dir, server, mut client) = start();
        let lock = lock_of(dir.path(), server.port);
        assert_eq!(lock["ideName"], "Cellix");
        assert_eq!(lock["transport"], "ws");
        assert_eq!(lock["authToken"].as_str().unwrap().len(), 32);
        let init = call(
            &mut client,
            1,
            "initialize",
            json!({"protocolVersion": "2025-06-18"}),
        );
        assert_eq!(init["result"]["protocolVersion"], "2025-06-18");
        let tools = call(&mut client, 2, "tools/list", json!({}));
        let names: Vec<&str> = tools["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        for name in [
            "openDiff",
            "openFile",
            "getDiagnostics",
            "getWorkspaceFolders",
        ] {
            assert!(names.contains(&name), "{name}");
        }
        let folders = call(
            &mut client,
            3,
            "tools/call",
            json!({"name": "getWorkspaceFolders", "arguments": {}}),
        );
        let text = folders["result"]["content"][0]["text"].as_str().unwrap();
        let reply: Value = serde_json::from_str(text).unwrap();
        let folder = &reply["folders"][0];
        assert_eq!(folder["path"], json!(dir.path()), "{text}");
        assert_eq!(folder["uri"], file_url(dir.path()), "{text}");
        let unknown = call(&mut client, 4, "nonsense", json!({}));
        assert_eq!(unknown["error"]["code"], -32601);
    }

    #[test]
    fn a_wrong_token_or_a_web_page_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let server = IdeServer::start_in(dir.path(), dir.path(), None).unwrap();
        assert!(connect(server.port, "wrong").is_err());
        let token = lock_of(dir.path(), server.port)["authToken"]
            .as_str()
            .unwrap()
            .to_string();
        let mut request = format!("ws://127.0.0.1:{}", server.port)
            .into_client_request()
            .unwrap();
        request
            .headers_mut()
            .insert(AUTH_HEADER, token.parse().unwrap());
        request
            .headers_mut()
            .insert("origin", "https://evil.example".parse().unwrap());
        assert!(tungstenite::connect(request).is_err());
    }

    #[test]
    fn file_urls_go_both_ways() {
        let path = std::env::temp_dir().join("notes").join("a.md");
        assert_eq!(Path::new(url_path(&file_url(&path))), path);
        assert_eq!(url_path("docs/a.md"), "docs/a.md");
        #[cfg(windows)]
        assert_eq!(file_url(Path::new(r"C:\x\y.md")), "file:///C:/x/y.md");
        #[cfg(unix)]
        assert_eq!(file_url(Path::new("/x/y.md")), "file:///x/y.md");
    }

    #[test]
    fn the_lock_file_is_private_and_goes_away() {
        let dir = tempfile::tempdir().unwrap();
        let server = IdeServer::start_in(dir.path(), dir.path(), None).unwrap();
        let path = dir.path().join("ide").join(format!("{}.lock", server.port));
        // Windows has no modes: the lock is private because the user's profile is.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        assert!(path.exists());
        drop(server);
        assert!(!path.exists());
    }

    #[test]
    fn open_diff_waits_for_the_reviewer() {
        let (_dir, server, mut client) = start();
        let params = json!({"name": "openDiff", "arguments": {"tab_name": "t"}});
        let message = json!({"jsonrpc": "2.0", "id": 9, "method": "tools/call", "params": params});
        client.send(Message::text(message.to_string())).unwrap();
        let waiting = server.calls.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(waiting.tool, "openDiff");
        // A selection still gets through while the diff is open.
        server.notify("selection_changed", json!({"text": "x"}));
        assert_eq!(read(&mut client)["method"], "selection_changed");
        waiting
            .reply
            .send(vec!["FILE_SAVED".into(), "new".into()])
            .unwrap();
        let done = read(&mut client);
        assert_eq!(done["id"], 9);
        assert_eq!(done["result"]["content"][1]["text"], "new");
    }

    #[test]
    fn a_diff_abandoned_by_the_app_is_rejected() {
        let (_dir, server, mut client) = start();
        let params = json!({"name": "openDiff", "arguments": {"tab_name": "t"}});
        let message = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": params});
        client.send(Message::text(message.to_string())).unwrap();
        drop(server.calls.recv_timeout(Duration::from_secs(5)).unwrap());
        let done = read(&mut client);
        assert_eq!(done["result"]["content"][0]["text"], "DIFF_REJECTED");
    }
}
