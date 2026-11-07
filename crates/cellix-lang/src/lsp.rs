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

/// A fix the language server offers: its title, and its edits by file.
pub type Fix = (String, Vec<(PathBuf, Vec<TextEdit>)>);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextEdit {
    pub start: (usize, usize),
    pub end: (usize, usize),
    pub text: String,
}

/// A symbol a server knows, in one file or anywhere in the workspace.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Symbol {
    pub name: String,
    /// The protocol's SymbolKind: 5 a class, 6 a method, 12 a function, 23 a struct.
    pub kind: u64,
    /// How deeply it's nested, 0 at the top of the file.
    pub depth: usize,
    /// 0-based, as the protocol counts.
    pub line: usize,
    pub column: usize,
    /// What it sits in, as the server names it: a parent symbol or `impl Queue<T>`.
    pub container: String,
    /// The file, for a workspace symbol.
    pub path: Option<PathBuf>,
}

/// The call being typed, as the server describes it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Signature {
    pub label: String,
    /// The parameter being typed, as a range of chars in `label`.
    pub active: Option<(usize, usize)>,
    /// What the server says about that parameter.
    pub parameter_doc: String,
    /// What it says about the whole signature.
    pub documentation: String,
    /// Which of the overloads this is, and how many there are.
    pub index: usize,
    pub count: usize,
}

/// A note the server would show inside a line, such as a variable's type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InlayHint {
    /// 0-based, as the protocol counts. The hint is drawn before the char at `column`.
    pub line: usize,
    pub column: usize,
    /// With the space the server asks for on either side.
    pub label: String,
    /// 1 a type, 2 a parameter's name.
    pub kind: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LspEvent {
    Diagnostics(PathBuf, Vec<Diagnostic>),
    Definition(PathBuf, usize, usize),
    Completion(Vec<String>),
    Rename(Vec<(PathBuf, Vec<TextEdit>)>),
    /// Everywhere a symbol is used, as (file, line, column).
    References(Vec<(PathBuf, usize, usize)>),
    /// The edits that format a file.
    Formatted(PathBuf, Vec<TextEdit>),
    /// What the server says about the symbol at a place: its type, signature and docs.
    Hover(String),
    /// The server's fixes and refactors for a place, each a title and its edits.
    Actions(Vec<Fix>),
    /// A file's symbols, in order.
    Symbols(PathBuf, Vec<Symbol>),
    /// Where a file's code folds, as (first line, last line).
    Folds(PathBuf, Vec<(usize, usize)>),
    /// Symbols across the workspace matching what was asked for, which comes back with them.
    WorkspaceSymbols(String, Vec<Symbol>),
    /// The call at the caret, or None when there's no call there.
    Signature(Option<Signature>),
    /// The hints for part of a file, with the tag they were asked for under.
    InlayHints(PathBuf, u64, Vec<InlayHint>),
    /// The server refused a request, in its own words.
    Failed(String),
}

#[derive(Clone)]
enum Pending {
    Initialize,
    Definition,
    Completion,
    Rename,
    References,
    Formatting(PathBuf),
    Hover,
    Symbols(PathBuf),
    Folds(PathBuf),
    WorkspaceSymbols(String),
    Signature,
    InlayHints(PathBuf, u64),
    Actions,
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

/// The name the protocol knows a language by, where that isn't simply ours in lower case.
fn language_id(language: &str) -> String {
    match language {
        "C#" => "csharp".to_string(),
        "C++" => "cpp".to_string(),
        "Shell" => "shellscript".to_string(),
        "TSX" => "typescriptreact".to_string(),
        other => other.to_lowercase(),
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

/// A file's symbols, nested ones under their parent, whichever shape the server sends: the
/// tree of `DocumentSymbol`s or the flat `SymbolInformation` list.
fn parse_symbols(result: &Value) -> Vec<Symbol> {
    fn walk(items: &[Value], depth: usize, parent: &str, out: &mut Vec<Symbol>) {
        for item in items {
            let Some(mut symbol) = symbol_of(item) else {
                continue;
            };
            symbol.depth = depth;
            if symbol.container.is_empty() {
                symbol.container = parent.to_string();
            }
            let name = symbol.name.clone();
            out.push(symbol);
            if let Some(children) = item.get("children").and_then(Value::as_array) {
                walk(children, depth + 1, &name, out);
            }
        }
    }
    let mut out = Vec::new();
    if let Some(items) = result.as_array() {
        walk(items, 0, "", &mut out);
    }
    out.sort_by_key(|s| (s.line, s.column));
    out
}

/// Folding ranges as (first line, last line), leaving out any that fold nothing.
fn parse_folds(result: &Value) -> Vec<(usize, usize)> {
    let line = |item: &Value, key: &str| item[key].as_u64().map(|l| l as usize);
    let mut found: Vec<(usize, usize)> = result
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| Some((line(item, "startLine")?, line(item, "endLine")?)))
        .filter(|(start, end)| end > start)
        .collect();
    found.sort_unstable();
    found.dedup();
    found
}

/// One `DocumentSymbol`, `SymbolInformation` or `WorkspaceSymbol`, without its children.
fn symbol_of(item: &Value) -> Option<Symbol> {
    let name = item.get("name").and_then(Value::as_str)?;
    let location = item.get("location");
    let range = item
        .get("selectionRange")
        .or_else(|| item.get("range"))
        .or_else(|| location.and_then(|l| l.get("range")));
    let (line, column) = range.map_or((0, 0), |r| position(&r["start"]));
    let path = location.and_then(|l| l["uri"].as_str()).and_then(path_of);
    Some(Symbol {
        name: name.to_string(),
        kind: item["kind"].as_u64().unwrap_or(0),
        depth: 0,
        line,
        column,
        container: item["containerName"].as_str().unwrap_or("").to_string(),
        path,
    })
}

/// Workspace symbols that say which file they're in.
fn parse_workspace_symbols(result: &Value) -> Vec<Symbol> {
    result
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(symbol_of)
        .filter(|s| s.path.is_some())
        .collect()
}

/// Documentation as plain text, from a string or a `MarkupContent`.
fn plain(value: &Value) -> String {
    let text = match value {
        Value::String(text) => text.as_str(),
        Value::Object(map) => map.get("value").and_then(Value::as_str).unwrap_or(""),
        _ => "",
    };
    text.lines()
        .filter(|line| !line.trim_start().starts_with("```"))
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

/// The signature the server picked, with the parameter being typed found in its label.
fn parse_signature(result: &Value) -> Option<Signature> {
    let signatures = result.get("signatures")?.as_array()?;
    let count = signatures.len();
    let index =
        (result["activeSignature"].as_u64().unwrap_or(0) as usize).min(count.checked_sub(1)?);
    let signature = &signatures[index];
    let label = signature["label"].as_str()?.to_string();
    let parameters = signature["parameters"].as_array();
    let current = signature["activeParameter"]
        .as_u64()
        .or_else(|| result["activeParameter"].as_u64())
        .unwrap_or(0) as usize;
    let parameter = parameters.and_then(|p| p.get(current));
    let active = parameter.and_then(|p| match &p["label"] {
        Value::String(name) => {
            // A name can appear twice, as in `f(a, ab)`, so it's looked for after the `(`.
            let open = label.find('(').map_or(0, |at| at + 1);
            let at = label[open..]
                .match_indices(name.as_str())
                .map(|(at, _)| at + open)
                .find(|&at| {
                    let after = label[at + name.len()..].chars().next();
                    !after.is_some_and(|c| c.is_alphanumeric() || c == '_')
                })?;
            let start = label[..at].chars().count();
            Some((start, start + name.chars().count()))
        }
        Value::Array(range) => {
            let start = range.first()?.as_u64()? as usize;
            let end = range.get(1)?.as_u64()? as usize;
            Some((start, end))
        }
        _ => None,
    });
    let parameter_doc = parameter.map_or_else(String::new, |p| plain(&p["documentation"]));
    Some(Signature {
        label,
        active,
        parameter_doc,
        documentation: plain(&signature["documentation"]),
        index,
        count,
    })
}

fn parse_inlay_hints(result: &Value) -> Vec<InlayHint> {
    result
        .as_array()
        .into_iter()
        .flatten()
        .map(|hint| {
            let (line, column) = position(&hint["position"]);
            let text = match &hint["label"] {
                Value::String(text) => text.clone(),
                Value::Array(parts) => parts
                    .iter()
                    .filter_map(|part| part["value"].as_str())
                    .collect(),
                _ => String::new(),
            };
            let pad = |key: &str| if hint[key] == true { " " } else { "" };
            InlayHint {
                line,
                column,
                label: format!(
                    "{}{}{}",
                    pad("paddingLeft"),
                    text.trim(),
                    pad("paddingRight")
                ),
                kind: hint["kind"].as_u64().unwrap_or(0),
            }
        })
        .filter(|hint| !hint.label.trim().is_empty())
        .collect()
}

/// A hover's contents as plain lines: Markdown's code fences go, and the code stays.
fn parse_hover(result: &Value) -> Option<LspEvent> {
    let contents = result.get("contents")?;
    let piece = |value: &Value| -> String {
        match value {
            Value::String(text) => text.clone(),
            Value::Object(map) => map
                .get("value")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            _ => String::new(),
        }
    };
    let text = match contents {
        Value::Array(parts) => parts.iter().map(piece).collect::<Vec<_>>().join("\n\n"),
        other => piece(other),
    };
    // Fences and rules are Markdown's, and one blank line is enough between parts.
    let mut lines: Vec<&str> = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") || trimmed == "---" {
            continue;
        }
        if trimmed.is_empty() && lines.last().is_none_or(|l| l.trim().is_empty()) {
            continue;
        }
        lines.push(line);
    }
    let text = lines.join("\n").trim().to_string();
    (!text.is_empty()).then_some(LspEvent::Hover(text))
}

fn parse_references(result: &Value) -> LspEvent {
    let found = result
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|location| {
            let path = path_of(location["uri"].as_str()?)?;
            let (line, column) = position(&location["range"]["start"]);
            Some((path, line, column))
        });
    LspEvent::References(found.collect())
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
    LspEvent::Rename(workspace_edit(result))
}

/// The fixes that come with their edits. One that only names a command for the server to run
/// is left out, since Cellix can't show what it would do first.
fn parse_actions(result: &Value) -> Vec<Fix> {
    result
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|action| {
            let title = action["title"].as_str()?.to_string();
            let edit = action.get("edit")?;
            let files = workspace_edit(edit);
            (!files.is_empty()).then_some((title, files))
        })
        .collect()
}

/// A WorkspaceEdit's changes, by file, in either of its two shapes.
fn workspace_edit(result: &Value) -> Vec<(PathBuf, Vec<TextEdit>)> {
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
    files
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
            // Folds, hints and signatures are asked for in the background, not by the person, so
            // a refusal is no news to them; the next edit or scroll asks again.
            let background = matches!(
                kind,
                Some(Pending::Folds(_) | Pending::InlayHints(..) | Pending::Signature)
            );
            if !background {
                let _ = events.send(LspEvent::Failed(error.to_string()));
            }
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
            Some(Pending::Hover) => parse_hover(result),
            Some(Pending::Actions) => Some(LspEvent::Actions(parse_actions(result))),
            Some(Pending::Symbols(path)) => Some(LspEvent::Symbols(path, parse_symbols(result))),
            Some(Pending::Folds(path)) => Some(LspEvent::Folds(path, parse_folds(result))),
            Some(Pending::WorkspaceSymbols(query)) => Some(LspEvent::WorkspaceSymbols(
                query,
                parse_workspace_symbols(result),
            )),
            Some(Pending::Signature) => Some(LspEvent::Signature(parse_signature(result))),
            Some(Pending::InlayHints(path, tag)) => {
                Some(LspEvent::InlayHints(path, tag, parse_inlay_hints(result)))
            }
            Some(Pending::Completion) => Some(parse_completion(result)),
            Some(Pending::Rename) => Some(parse_rename(result)),
            Some(Pending::References) => Some(parse_references(result)),
            Some(Pending::Formatting(path)) => Some(LspEvent::Formatted(path, text_edits(result))),
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
                    "workspace": {"symbol": {}, "inlayHint": {"refreshSupport": false}},
                    "textDocument": {
                        "publishDiagnostics": {},
                        "definition": {},
                        "hover": {"contentFormat": ["plaintext", "markdown"]},
                        "documentSymbol": {"hierarchicalDocumentSymbolSupport": true},
                        "foldingRange": {"lineFoldingOnly": true},
                        "codeAction": {"codeActionLiteralSupport": {"codeActionKind": {"valueSet": [
                            "", "quickfix", "refactor", "refactor.extract", "refactor.inline",
                            "refactor.rewrite", "source", "source.organizeImports"
                        ]}}},
                        "completion": {},
                        "rename": {},
                        "references": {},
                        "formatting": {},
                        "signatureHelp": {"signatureInformation": {
                            "documentationFormat": ["plaintext", "markdown"],
                            "parameterInformation": {"labelOffsetSupport": true},
                            "activeParameterSupport": true,
                        }},
                        "inlayHint": {},
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
                "languageId": language_id(language),
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

    /// Asks for the fixes at a place, with the problems reported on its line.
    pub fn code_actions(
        &mut self,
        path: &Path,
        (line, column): (usize, usize),
        problems: &[(usize, usize, String, u64)],
    ) {
        let diagnostics: Vec<Value> = problems
            .iter()
            .map(|(line, column, message, severity)| {
                json!({
                    "range": {"start": {"line": line, "character": column},
                              "end": {"line": line, "character": column}},
                    "message": message,
                    "severity": severity,
                })
            })
            .collect();
        let at = json!({"line": line, "character": column});
        let params = json!({
            "textDocument": {"uri": uri(path)},
            "range": {"start": at, "end": at},
            "context": {"diagnostics": diagnostics},
        });
        self.request("textDocument/codeAction", params, Pending::Actions);
    }

    /// Asks for a file's symbols, for the Outline.
    pub fn symbols(&mut self, path: &Path) {
        let params = json!({"textDocument": {"uri": uri(path)}});
        self.request(
            "textDocument/documentSymbol",
            params,
            Pending::Symbols(path.to_path_buf()),
        );
    }

    /// Asks where a file's code folds.
    pub fn folding_ranges(&mut self, path: &Path) {
        let params = json!({"textDocument": {"uri": uri(path)}});
        self.request(
            "textDocument/foldingRange",
            params,
            Pending::Folds(path.to_path_buf()),
        );
    }

    /// Asks every symbol in the workspace whose name matches `query`, for Go to Symbol.
    pub fn workspace_symbols(&mut self, query: &str) {
        let params = json!({"query": query});
        let kind = Pending::WorkspaceSymbols(query.to_string());
        self.request("workspace/symbol", params, kind);
    }

    /// Asks about the call at a place, as a `(` or `,` is typed.
    pub fn signature_help(&mut self, path: &Path, line: usize, column: usize) {
        let params = Self::at(path, line, column);
        self.request("textDocument/signatureHelp", params, Pending::Signature);
    }

    /// Asks for the hints in lines `from..to`, tagged so an answer to an older question can
    /// be told apart.
    /// Hints from the start of line `from` to `end`, a line and a UTF-16 column that must be in
    /// the document: servers such as rust-analyzer refuse a range past its last line.
    pub fn inlay_hints(&mut self, path: &Path, from: usize, end: (usize, usize), tag: u64) {
        let params = json!({
            "textDocument": {"uri": uri(path)},
            "range": {"start": {"line": from, "character": 0},
                      "end": {"line": end.0, "character": end.1}},
        });
        let kind = Pending::InlayHints(path.to_path_buf(), tag);
        self.request("textDocument/inlayHint", params, kind);
    }

    pub fn hover(&mut self, path: &Path, line: usize, column: usize) {
        let params = Self::at(path, line, column);
        self.request("textDocument/hover", params, Pending::Hover);
    }

    pub fn definition(&mut self, path: &Path, line: usize, column: usize) {
        let params = Self::at(path, line, column);
        self.request("textDocument/definition", params, Pending::Definition);
    }

    pub fn completion(&mut self, path: &Path, line: usize, column: usize) {
        let params = Self::at(path, line, column);
        self.request("textDocument/completion", params, Pending::Completion);
    }

    pub fn references(&mut self, path: &Path, line: usize, column: usize) {
        let mut params = Self::at(path, line, column);
        params["context"] = json!({"includeDeclaration": true});
        self.request("textDocument/references", params, Pending::References);
    }

    pub fn format(&mut self, path: &Path, tab_size: usize) {
        let params = json!({
            "textDocument": {"uri": uri(path)},
            "options": {"tabSize": tab_size, "insertSpaces": true},
        });
        let kind = Pending::Formatting(path.to_path_buf());
        self.request("textDocument/formatting", params, kind);
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
    fn references_list_every_place() {
        let result = json!([
            {"uri": "file:///a.rs", "range": {"start": {"line": 1, "character": 2}}},
            {"uri": "file:///b.rs", "range": {"start": {"line": 3, "character": 4}}},
        ]);
        let LspEvent::References(found) = parse_references(&result) else {
            panic!("not references");
        };
        assert_eq!(found[1], (PathBuf::from("/b.rs"), 3, 4));
        assert!(matches!(parse_references(&Value::Null), LspEvent::References(f) if f.is_empty()));
    }

    #[test]
    fn languages_go_by_the_names_servers_expect() {
        assert_eq!(language_id("C#"), "csharp");
        assert_eq!(language_id("Shell"), "shellscript");
        assert_eq!(language_id("Rust"), "rust");
    }

    #[test]
    fn only_what_the_person_asked_for_reports_a_refusal() {
        // rust-analyzer refuses a hint range past the file's end with "Invalid offset LineCol";
        // that was asked in the background, so it must not reach the status bar. A refused
        // definition was asked for, so it still does.
        let (writer, _child) = writer_over_cat();
        let writer = Mutex::new(writer);
        let pending = Mutex::new(HashMap::from([
            (1, Pending::InlayHints(PathBuf::from("a.rs"), 3)),
            (2, Pending::Folds(PathBuf::from("a.rs"))),
            (3, Pending::Signature),
            (4, Pending::Definition),
        ]));
        let mut raw = String::new();
        for id in 1..=4 {
            let body = json!({"jsonrpc": "2.0", "id": id,
                "error": {"code": -32603, "message": format!("refused {id}")}})
            .to_string();
            raw.push_str(&format!("Content-Length: {}\r\n\r\n{body}", body.len()));
        }
        let (sender, events) = channel();
        read_loop(raw.as_bytes(), &writer, &pending, &sender);
        let said: Vec<String> = events
            .try_iter()
            .map(|event| match event {
                LspEvent::Failed(message) => message,
                _ => String::new(),
            })
            .collect();
        assert_eq!(said, ["refused 4"]);
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
    fn symbols_come_in_order_with_their_depth() {
        let tree = json!([{"name": "Queue", "kind": 23, "range": {"start": {"line": 4}},
            "selectionRange": {"start": {"line": 5}},
            "children": [{"name": "push", "kind": 6, "range": {"start": {"line": 9}}}]},
            {"name": "main", "kind": 12, "range": {"start": {"line": 1}}}]);
        let found = parse_symbols(&tree);
        let brief: Vec<(&str, usize, usize, u64, &str)> = found
            .iter()
            .map(|s| {
                (
                    s.name.as_str(),
                    s.depth,
                    s.line,
                    s.kind,
                    s.container.as_str(),
                )
            })
            .collect();
        assert_eq!(
            brief,
            vec![
                ("main", 0, 1, 12, ""),
                ("Queue", 0, 5, 23, ""),
                ("push", 1, 9, 6, "Queue")
            ]
        );
        let flat = json!([{"name": "f", "kind": 12, "containerName": "m",
            "location": {"uri": "file:///a", "range": {"start": {"line": 3, "character": 4}}}}]);
        let f = &parse_symbols(&flat)[0];
        assert_eq!((f.line, f.column, f.container.as_str()), (3, 4, "m"));
        assert_eq!(f.path, Some(PathBuf::from("/a")));
    }

    #[test]
    fn workspace_symbols_need_a_file() {
        let result = json!([
            {"name": "push", "kind": 6, "containerName": "Queue",
             "location": {"uri": "file:///q.rs", "range": {"start": {"line": 14, "character": 11}}}},
            {"name": "lost", "kind": 12, "location": {"range": {"start": {"line": 1}}}},
        ]);
        let found = parse_workspace_symbols(&result);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].path, Some(PathBuf::from("/q.rs")));
        assert_eq!((found[0].line, found[0].column), (14, 11));
    }

    #[test]
    fn the_parameter_being_typed_is_found_in_the_label() {
        let named = json!({"signatures": [{"label": "fn insert(&mut self, index: usize, value: T)",
            "parameters": [{"label": "&mut self"}, {"label": "index: usize"},
                {"label": "value: T", "documentation": "The element."}],
            "documentation": {"kind": "markdown", "value": "Inserts it."}}],
            "activeSignature": 0, "activeParameter": 2});
        let signature = parse_signature(&named).unwrap();
        let (start, end) = signature.active.unwrap();
        let chars: Vec<char> = signature.label.chars().collect();
        assert_eq!(chars[start..end].iter().collect::<String>(), "value: T");
        assert_eq!(signature.parameter_doc, "The element.");
        assert_eq!(signature.documentation, "Inserts it.");
        assert_eq!((signature.index, signature.count), (0, 1));
        // Offsets, and a name that's also the start of another.
        let offsets = json!({"signatures": [{"label": "f(a, ab)",
            "parameters": [{"label": [2, 3]}, {"label": "ab"}], "activeParameter": 1}]});
        assert_eq!(parse_signature(&offsets).unwrap().active, Some((5, 7)));
        let first = json!({"signatures": [{"label": "f(ab, a)",
            "parameters": [{"label": "ab"}, {"label": "a"}]}], "activeParameter": 1});
        assert_eq!(parse_signature(&first).unwrap().active, Some((6, 7)));
        assert_eq!(parse_signature(&json!({"signatures": []})), None);
        assert_eq!(parse_signature(&Value::Null), None);
    }

    #[test]
    fn inlay_hints_keep_their_padding() {
        let result = json!([
            {"position": {"line": 2, "character": 13}, "label": ": Queue<Job>", "kind": 1},
            {"position": {"line": 2, "character": 30}, "label": [{"value": "limit"}, {"value": ":"}],
             "kind": 2, "paddingRight": true},
            {"position": {"line": 3, "character": 0}, "label": ""},
        ]);
        let hints = parse_inlay_hints(&result);
        assert_eq!(hints.len(), 2);
        assert_eq!(hints[0].label, ": Queue<Job>");
        assert_eq!((hints[1].line, hints[1].column), (2, 30));
        assert_eq!(hints[1].label, "limit: ");
        assert_eq!(hints[1].kind, 2);
    }

    #[test]
    fn fixes_come_with_their_edits_and_commands_stay_out() {
        let actions = json!([
            {"title": "Add semicolon", "kind": "quickfix", "edit": {"changes": {"file:///a.c": [
                {"range": {"start": {"line": 2, "character": 9}, "end": {"line": 2, "character": 9}}, "newText": ";"}
            ]}}},
            {"title": "Run a command", "command": {"command": "x"}}
        ]);
        let found = parse_actions(&actions);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].0, "Add semicolon");
        assert_eq!(found[0].1[0].1[0].text, ";");
    }

    #[test]
    fn a_hover_reads_as_plain_lines() {
        let markup = json!({"contents": {"kind": "markdown", "value": "```rust\nfn add(a: i32) -> i32\n```\nAdds one."}});
        assert_eq!(
            parse_hover(&markup),
            Some(LspEvent::Hover("fn add(a: i32) -> i32\nAdds one.".into()))
        );
        let rust = json!({"contents": {"kind": "markdown", "value": "```rust\nhov\n```\n\n```rust\nfn add_one(x: i32) -> i32\n```\n\n---\n\nAdds one."}});
        assert_eq!(
            parse_hover(&rust),
            Some(LspEvent::Hover(
                "hov\n\nfn add_one(x: i32) -> i32\n\nAdds one.".into()
            ))
        );
        let old = json!({"contents": [{"language": "c", "value": "int total"}, "The sum."]});
        assert_eq!(
            parse_hover(&old),
            Some(LspEvent::Hover("int total\n\nThe sum.".into()))
        );
        assert_eq!(parse_hover(&json!({"contents": ""})), None);
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

    #[test]
    fn folding_ranges_keep_only_those_that_fold_something() {
        let result = json!([
            {"startLine": 4, "endLine": 9, "kind": "region"},
            {"startLine": 0, "endLine": 2},
            {"startLine": 3, "endLine": 3},
            {"startLine": 4, "endLine": 9}
        ]);
        assert_eq!(parse_folds(&result), vec![(0, 2), (4, 9)]);
        assert!(parse_folds(&json!(null)).is_empty());
    }
}
