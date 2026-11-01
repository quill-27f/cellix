//! A Language Server Protocol client. Requests never block the UI: replies arrive as
//! events on a channel, which the app drains between frames.

use std::collections::HashMap;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::thread;

use serde_json::{Value, json};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    /// 0-based, as the protocol counts.
    pub line: usize,
    pub column: usize,
    pub message: String,
    /// 1 is an error, 2 a warning.
    pub severity: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextEdit {
    pub start: (usize, usize),
    pub end: (usize, usize),
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LspEvent {
    Diagnostics(PathBuf, Vec<Diagnostic>),
    Definition(PathBuf, usize, usize),
    Completion(Vec<String>),
    Rename(Vec<(PathBuf, Vec<TextEdit>)>),
    /// The server refused a request, in its own words.
    Failed(String),
}

#[derive(Clone, Copy)]
enum Pending {
    Initialize,
    Definition,
    Completion,
    Rename,
}

/// Holds messages until the server has answered `initialize`, as the protocol requires.
struct Writer {
    stdin: ChildStdin,
    ready: bool,
    queue: Vec<Value>,
    /// Open documents, so their checks can run again once the server has loaded.
    opened: Vec<String>,
    /// Whether the server last said it was still loading.
    busy: bool,
}

impl Writer {
    fn write(&mut self, message: &Value) -> io::Result<()> {
        let body = message.to_string();
        write!(self.stdin, "Content-Length: {}\r\n\r\n{body}", body.len())?;
        self.stdin.flush()
    }

    fn send(&mut self, message: Value) {
        if self.ready {
            let _ = self.write(&message);
        } else {
            self.queue.push(message);
        }
    }
}

/// One running language server.
pub struct Client {
    child: Child,
    writer: Arc<Mutex<Writer>>,
    pending: Arc<Mutex<HashMap<u64, Pending>>>,
    next_id: u64,
    versions: HashMap<PathBuf, i64>,
    pub events: Receiver<LspEvent>,
}

impl Drop for Client {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn uri(path: &Path) -> String {
    format!("file://{}", path.display())
}

fn path_of(uri: &str) -> Option<PathBuf> {
    uri.strip_prefix("file://").map(PathBuf::from)
}

fn position(value: &Value) -> (usize, usize) {
    (
        value["line"].as_u64().unwrap_or(0) as usize,
        value["character"].as_u64().unwrap_or(0) as usize,
    )
}

fn text_edits(value: &Value) -> Vec<TextEdit> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .map(|edit| TextEdit {
            start: position(&edit["range"]["start"]),
            end: position(&edit["range"]["end"]),
            text: edit["newText"].as_str().unwrap_or("").to_string(),
        })
        .collect()
}

fn read_message(reader: &mut impl BufRead) -> Option<Value> {
    let mut length = None;
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header).ok()? == 0 {
            return None;
        }
        let header = header.trim();
        if header.is_empty() {
            break;
        }
        if let Some(value) = header.strip_prefix("Content-Length:") {
            length = value.trim().parse().ok();
        }
    }
    let mut body = vec![0; length?];
    reader.read_exact(&mut body).ok()?;
    serde_json::from_slice(&body).ok()
}

fn parse_location(result: &Value) -> Option<LspEvent> {
    let location = if result.is_array() {
        &result[0]
    } else {
        result
    };
    let uri = location["uri"]
        .as_str()
        .or_else(|| location["targetUri"].as_str())?;
    let range = if location["range"].is_null() {
        &location["targetSelectionRange"]
    } else {
        &location["range"]
    };
    let (line, column) = position(&range["start"]);
    Some(LspEvent::Definition(path_of(uri)?, line, column))
}

fn parse_completion(result: &Value) -> LspEvent {
    let items = if result.is_array() {
        result
    } else {
        &result["items"]
    };
    let labels = items
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| {
            item["textEdit"]["newText"]
                .as_str()
                .or_else(|| item["insertText"].as_str())
                .or_else(|| item["label"].as_str())
                .map(str::to_string)
        })
        .collect();
    LspEvent::Completion(labels)
}

fn parse_rename(result: &Value) -> LspEvent {
    let mut files: Vec<(PathBuf, Vec<TextEdit>)> = Vec::new();
    if let Some(changes) = result["changes"].as_object() {
        for (uri, edits) in changes {
            if let Some(path) = path_of(uri) {
                files.push((path, text_edits(edits)));
            }
        }
    }
    for change in result["documentChanges"].as_array().into_iter().flatten() {
        if let Some(path) = change["textDocument"]["uri"].as_str().and_then(path_of) {
            files.push((path, text_edits(&change["edits"])));
        }
    }
    LspEvent::Rename(files)
}

/// rust-analyzer ignores a save that arrives while it's loading, and then a file with
/// errors looks clean until the next save. So every open file gets its check when loading
/// ends.
fn recheck_when_loaded(writer: &mut Writer, quiet: bool) {
    let finished = writer.busy && quiet;
    writer.busy = !quiet;
    if finished {
        for uri in writer.opened.clone() {
            let params = json!({"textDocument": {"uri": uri}});
            let saved =
                json!({"jsonrpc": "2.0", "method": "textDocument/didSave", "params": params});
            let _ = writer.write(&saved);
        }
    }
}

fn read_loop(
    stdout: impl Read,
    writer: &Mutex<Writer>,
    pending: &Mutex<HashMap<u64, Pending>>,
    events: &Sender<LspEvent>,
) {
    let mut reader = BufReader::new(stdout);
    while let Some(message) = read_message(&mut reader) {
        if message["method"] == "textDocument/publishDiagnostics" {
            let params = &message["params"];
            let Some(path) = params["uri"].as_str().and_then(path_of) else {
                continue;
            };
            let items = params["diagnostics"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|d| {
                    let (line, column) = position(&d["range"]["start"]);
                    Diagnostic {
                        line,
                        column,
                        message: d["message"].as_str().unwrap_or("").to_string(),
                        severity: d["severity"].as_u64().unwrap_or(1),
                    }
                })
                .collect();
            let _ = events.send(LspEvent::Diagnostics(path, items));
            continue;
        }
        if message["method"] == "experimental/serverStatus" {
            let quiet = message["params"]["quiescent"].as_bool().unwrap_or(false);
            if let Ok(mut writer) = writer.lock() {
                recheck_when_loaded(&mut writer, quiet);
            }
            continue;
        }
        if !message["method"].is_null() {
            // A request from the server. Answering keeps servers that wait on it moving.
            if !message["id"].is_null()
                && let Ok(mut writer) = writer.lock()
            {
                let reply = json!({"jsonrpc": "2.0", "id": message["id"], "result": null});
                writer.send(reply);
            }
            continue;
        }
        let Some(id) = message["id"].as_u64() else {
            continue;
        };
        let kind = pending.lock().ok().and_then(|mut p| p.remove(&id));
        let result = &message["result"];
        if let Some(error) = message["error"]["message"].as_str() {
            let _ = events.send(LspEvent::Failed(error.to_string()));
            continue;
        }
        let event = match kind {
            Some(Pending::Initialize) => {
                if let Ok(mut writer) = writer.lock() {
                    writer.ready = true;
                    let _ = writer
                        .write(&json!({"jsonrpc": "2.0", "method": "initialized", "params": {}}));
                    for queued in std::mem::take(&mut writer.queue) {
                        let _ = writer.write(&queued);
                    }
                }
                None
            }
            Some(Pending::Definition) => parse_location(result),
            Some(Pending::Completion) => Some(parse_completion(result)),
            Some(Pending::Rename) => Some(parse_rename(result)),
            None => None,
        };
        if let Some(event) = event {
            let _ = events.send(event);
        }
    }
}

impl Client {
    /// Starts a server and begins the handshake. Fails if the command can't run, so the
    /// caller can leave the language without one.
    pub fn start(command: &[String], root: &Path) -> io::Result<Self> {
        let (program, args) = command
            .split_first()
            .ok_or_else(|| io::Error::other("empty server command"))?;
        let mut child = Command::new(program)
            .args(args)
            .current_dir(root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| io::Error::other("no stdin"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| io::Error::other("no stdout"))?;
        let writer = Arc::new(Mutex::new(Writer {
            stdin,
            ready: false,
            queue: Vec::new(),
            opened: Vec::new(),
            busy: true,
        }));
        let pending = Arc::new(Mutex::new(HashMap::new()));
        let (sender, events) = channel();
        let (thread_writer, thread_pending) = (writer.clone(), pending.clone());
        thread::spawn(move || read_loop(stdout, &thread_writer, &thread_pending, &sender));
        let mut client = Self {
            child,
            writer,
            pending,
            next_id: 1,
            versions: HashMap::new(),
            events,
        };
        client.initialize(root)?;
        Ok(client)
    }

    fn initialize(&mut self, root: &Path) -> io::Result<()> {
        let message = json!({
            "jsonrpc": "2.0", "id": 0, "method": "initialize",
            "params": {
                "processId": std::process::id(),
                "rootUri": uri(root),
                "capabilities": {
                    "general": {"positionEncodings": ["utf-32", "utf-16"]},
                    "experimental": {"serverStatusNotification": true},
                    "textDocument": {
                        "publishDiagnostics": {},
                        "definition": {},
                        "completion": {},
                        "rename": {},
                    },
                },
            },
        });
        if let Ok(mut pending) = self.pending.lock() {
            pending.insert(0, Pending::Initialize);
        }
        // Written directly: this is the one message that can't wait for `ready`.
        self.writer
            .lock()
            .map_err(|_| io::Error::other("poisoned"))?
            .write(&message)
    }

    fn notify(&self, method: &str, params: Value) {
        if let Ok(mut writer) = self.writer.lock() {
            writer.send(json!({"jsonrpc": "2.0", "method": method, "params": params}));
        }
    }

    fn request(&mut self, method: &str, params: Value, kind: Pending) {
        let id = self.next_id;
        self.next_id += 1;
        if let Ok(mut pending) = self.pending.lock() {
            pending.insert(id, kind);
        }
        if let Ok(mut writer) = self.writer.lock() {
            writer.send(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        }
    }

    pub fn open(&mut self, path: &Path, language: &str, text: &str) {
        self.versions.insert(path.to_path_buf(), 1);
        if let Ok(mut writer) = self.writer.lock() {
            writer.opened.push(uri(path));
        }
        self.notify(
            "textDocument/didOpen",
            json!({"textDocument": {
                "uri": uri(path),
                "languageId": language.to_lowercase(),
                "version": 1,
                "text": text,
            }}),
        );
    }

    pub fn change(&mut self, path: &Path, text: &str) {
        let version = self.versions.entry(path.to_path_buf()).or_insert(1);
        *version += 1;
        let version = *version;
        self.notify(
            "textDocument/didChange",
            json!({
                "textDocument": {"uri": uri(path), "version": version},
                "contentChanges": [{"text": text}],
            }),
        );
    }

    /// Servers such as rust-analyzer run their compiler check on this.
    pub fn saved(&self, path: &Path) {
        self.notify(
            "textDocument/didSave",
            json!({"textDocument": {"uri": uri(path)}}),
        );
    }

    fn at(path: &Path, line: usize, column: usize) -> Value {
        json!({
            "textDocument": {"uri": uri(path)},
            "position": {"line": line, "character": column},
        })
    }

    pub fn definition(&mut self, path: &Path, line: usize, column: usize) {
        let params = Self::at(path, line, column);
        self.request("textDocument/definition", params, Pending::Definition);
    }

    pub fn completion(&mut self, path: &Path, line: usize, column: usize) {
        let params = Self::at(path, line, column);
        self.request("textDocument/completion", params, Pending::Completion);
    }

    pub fn rename(&mut self, path: &Path, line: usize, column: usize, name: &str) {
        let mut params = Self::at(path, line, column);
        params["newName"] = json!(name);
        self.request("textDocument/rename", params, Pending::Rename);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn writer_over_cat() -> (Writer, Child) {
        let mut child = Command::new("cat")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let writer = Writer {
            stdin: child.stdin.take().unwrap(),
            ready: true,
            queue: Vec::new(),
            opened: vec!["file:///a.rs".to_string()],
            busy: true,
        };
        (writer, child)
    }

    #[test]
    fn open_files_get_their_check_once_loading_ends() {
        let (mut writer, child) = writer_over_cat();
        recheck_when_loaded(&mut writer, false);
        recheck_when_loaded(&mut writer, true);
        recheck_when_loaded(&mut writer, true);
        drop(writer);
        let sent = child.wait_with_output().unwrap().stdout;
        let sent = String::from_utf8(sent).unwrap();
        assert_eq!(sent.matches("textDocument/didSave").count(), 1, "{sent}");
        assert!(sent.contains("file:///a.rs"));
    }

    #[test]
    fn framing_round_trips() {
        let body = r#"{"id":1}"#;
        let raw = format!("Content-Length: {}\r\nX: y\r\n\r\n{body}", body.len());
        let value = read_message(&mut raw.as_bytes()).unwrap();
        assert_eq!(value["id"], 1);
        assert!(read_message(&mut "".as_bytes()).is_none());
    }

    #[test]
    fn locations_and_links_both_parse() {
        let plain = json!([{"uri": "file:///a.rs",
            "range": {"start": {"line": 3, "character": 4}, "end": {"line": 3, "character": 5}}}]);
        let link = json!([{"targetUri": "file:///a.rs",
            "targetSelectionRange": {"start": {"line": 3, "character": 4}}}]);
        let want = LspEvent::Definition(PathBuf::from("/a.rs"), 3, 4);
        assert_eq!(parse_location(&plain), Some(want.clone()));
        assert_eq!(parse_location(&link), Some(want));
        assert_eq!(parse_location(&json!(null)), None);
    }

    #[test]
    fn rename_reads_both_edit_shapes() {
        let edit = json!([{"range": {"start": {"line": 0, "character": 1},
            "end": {"line": 0, "character": 2}}, "newText": "x"}]);
        let changes = json!({"changes": {"file:///a.rs": edit}});
        let documents =
            json!({"documentChanges": [{"textDocument": {"uri": "file:///a.rs"}, "edits": edit}]});
        assert_eq!(parse_rename(&changes), parse_rename(&documents));
    }
}
