//! Ask: a side question about what's selected, which is not a comment. Cellix runs the agent's
//! own command line once, in the background, read-only, in the project's folder, and shows its
//! answer as it streams in. It never types into the agent working in the side pane, and it
//! brings in no comments: the run carries `CELLIX_SIDE_QUESTION`, and every Cellix hook does
//! nothing under it, so the question neither collects the review's comments nor shows up as
//! an agent.
//!
//! What it knows, by layer:
//! 1. the whole document, with the selection marked ⟦like this⟧ and the headings it's under,
//!    a LaTeX paper's `\input` files read in, and its path, so it can read the rest;
//! 2. read-only tools to read, search and list files, and nothing that edits or runs;
//! 3. one side conversation per document, resumed by its session id, kept with Cellix's own
//!    state for the project, never in the project's files;
//! 4. on request, a copy of the conversation of the Claude Code agent working here
//!    (`--fork-session`), so it knows the work too, while that agent never sees the question.

use std::collections::HashMap;
use std::ffi::OsString;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command as Process, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::rendered::Dialect;

pub use cellix_agent::hook::SIDE_QUESTION_VAR;

/// Past this many letters, a document goes as the part the selection is in, with the
/// document's outline, and the rest is left for the tools to read.
pub const WHOLE_LIMIT: usize = 150_000;
/// How long an answer may take before it's stopped.
pub const TIME_LIMIT: Duration = Duration::from_secs(240);
/// The marks round the selection in the text the agent reads.
pub const OPEN_MARK: char = '⟦';
pub const CLOSE_MARK: char = '⟧';
/// Tools that read, and those that edit or run: the read-only set Claude Code is given.
const READ_TOOLS: &str = "Read,Grep,Glob";
const EDIT_TOOLS: &str = "Bash,Edit,Write,MultiEdit,NotebookEdit";
/// What a run is told before anything else, every time.
const RULES: &str = "You're answering a quick side question from someone reading in Cellix, an \
editor. Answer only this question, briefly and for the reader: a few sentences, or a short list \
if it needs one. Don't change, create or delete any file, and don't run anything that would. \
Review comments on the document aren't your task: ignore any you come across.";

/// The command lines that can answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Cli {
    Claude,
    Codex,
    Gemini,
}

impl Cli {
    /// In the order they're tried where nothing else says.
    pub const ALL: [Cli; 3] = [Cli::Claude, Cli::Codex, Cli::Gemini];

    pub fn program(self) -> &'static str {
        match self {
            Cli::Claude => "claude",
            Cli::Codex => "codex",
            Cli::Gemini => "gemini",
        }
    }

    /// As Cellix names it elsewhere.
    pub fn name(self) -> &'static str {
        match self {
            Cli::Claude => "Claude Code",
            Cli::Codex => "Codex",
            Cli::Gemini => "Gemini CLI",
        }
    }

    /// The `ask_agent` setting's words for it.
    pub fn from_setting(word: &str) -> Option<Cli> {
        match word.trim().to_lowercase().as_str() {
            "claude" | "claude-code" | "claude code" => Some(Cli::Claude),
            "codex" => Some(Cli::Codex),
            "gemini" | "gemini-cli" | "gemini cli" => Some(Cli::Gemini),
            _ => None,
        }
    }

    /// The kind of an agent's tab, by the name Cellix gives agents ("Claude Code", "Gemini").
    pub fn from_agent(name: &str) -> Option<Cli> {
        match name {
            "Claude Code" => Some(Cli::Claude),
            "Codex" => Some(Cli::Codex),
            "Gemini" | "Gemini CLI" => Some(Cli::Gemini),
            _ => None,
        }
    }

    /// Whether a conversation of its own can be copied to ask in, leaving the original as it
    /// was. Claude Code's `--fork-session`.
    pub fn copies(self) -> bool {
        self == Cli::Claude
    }
}

/// Which command line answers: the one the `ask_agent` setting names; else the kind of agent
/// in the side pane, where that's installed; else the first installed of Claude Code, Codex
/// and Gemini CLI. `Err` names what's missing: the one the setting names, or none at all.
pub fn choose(
    setting: Option<&str>,
    pane: Option<Cli>,
    installed: impl Fn(Cli) -> bool,
) -> Result<Cli, Option<Cli>> {
    if let Some(named) = setting.and_then(Cli::from_setting) {
        return if installed(named) {
            Ok(named)
        } else {
            Err(Some(named))
        };
    }
    if let Some(pane) = pane.filter(|cli| installed(*cli)) {
        return Ok(pane);
    }
    Cli::ALL.into_iter().find(|cli| installed(*cli)).ok_or(None)
}

/// Where a program is on `path`, a list of folders as PATH holds them.
pub fn find(program: &str, path: &OsString) -> Option<PathBuf> {
    let names: Vec<String> = if cfg!(windows) {
        ["", ".exe", ".cmd", ".bat"]
            .iter()
            .map(|ending| format!("{program}{ending}"))
            .collect()
    } else {
        vec![program.to_string()]
    };
    std::env::split_paths(path)
        .filter(|folder| !folder.as_os_str().is_empty())
        .flat_map(|folder| names.iter().map(move |name| folder.join(name)))
        .find(|candidate| candidate.is_file())
}

/// One run of a command line: what it's asked, where, and which conversation it goes on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Question {
    pub cli: Cli,
    /// The project's folder, where it runs.
    pub root: PathBuf,
    /// All it's told, given on its input rather than as an argument, which a long document
    /// would pass the system's limit on.
    pub prompt: String,
    /// The conversation to go on with, by its session id.
    pub resume: Option<String>,
    /// With `resume`, a copy of that conversation, which leaves the original as it was.
    pub fork: bool,
}

/// What the command line is given, after its name.
pub fn args(question: &Question) -> Vec<String> {
    let owned = |words: &[&str]| words.iter().map(|w| w.to_string()).collect::<Vec<_>>();
    match question.cli {
        // The prompt comes on its input. Partial messages stream the answer a few words at a
        // time; the tools are limited to reading, and editing and the shell refused outright.
        Cli::Claude => {
            let mut args = owned(&[
                "-p",
                "--output-format",
                "stream-json",
                "--verbose",
                "--include-partial-messages",
            ]);
            if let Some(id) = &question.resume {
                args.extend(["--resume".to_string(), id.clone()]);
                if question.fork {
                    args.push("--fork-session".to_string());
                }
            }
            args.extend(owned(&[
                "--allowedTools",
                READ_TOOLS,
                "--disallowedTools",
                EDIT_TOOLS,
            ]));
            args
        }
        // `exec resume` has no `--sandbox` or `-C`, so the sandbox is set as config and the
        // folder is where it runs. `-` reads the prompt from its input.
        Cli::Codex => match &question.resume {
            Some(id) => owned(&[
                "exec",
                "resume",
                "--json",
                "--skip-git-repo-check",
                "-c",
                "sandbox_mode=\"read-only\"",
                id,
                "-",
            ]),
            None => {
                let root = question.root.display().to_string();
                owned(&[
                    "exec",
                    "--sandbox",
                    "read-only",
                    "--json",
                    "--skip-git-repo-check",
                    "-C",
                    &root,
                    "-",
                ])
            }
        },
        // Without a person to ask, Gemini CLI leaves out every tool that would need one (the
        // shell, edits, writes) unless told to approve them, which it isn't. Its prompt comes
        // on its input, and `-p` adds a line to it.
        Cli::Gemini => {
            let mut args = owned(&[
                "--output-format",
                "stream-json",
                "--approval-mode",
                "default",
            ]);
            if let Some(id) = &question.resume {
                args.extend(["--resume".to_string(), id.clone()]);
            }
            args.extend(owned(&["-p", "Answer the question above."]));
            args
        }
    }
}

/// What a run says, as it comes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum News {
    /// The conversation's id, to go on with it.
    Session(String),
    /// A new message begins: what streamed before it was on the way, not the answer.
    Fresh,
    /// A few more words of the answer.
    Text(String),
    /// A message whole, in place of what streamed for it.
    Said(String),
    /// What it's doing meanwhile, as "Reading notes.md".
    Doing(String),
    /// Finished, with the answer whole where the command line gives it.
    Done(String),
    Failed(Failure),
}

/// Why there's no answer, in words for the card.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Failure {
    NotSignedIn,
    TimedOut,
    /// It couldn't be started at all.
    NotStarted(String),
    /// It said why, or exited without an answer.
    Said(String),
}

impl Failure {
    pub fn words(&self, cli: Cli) -> String {
        let name = cli.name();
        match self {
            Failure::NotSignedIn => {
                format!(
                    "{name} isn't signed in. Run {} in a terminal and sign in, then ask again.",
                    cli.program()
                )
            }
            Failure::TimedOut => format!("{name} took too long to answer, so it was stopped."),
            Failure::NotStarted(why) => format!("Couldn't start {name}: {why}"),
            Failure::Said(why) => format!("{name} failed: {why}"),
        }
    }
}

/// Words that say a command line isn't signed in, whichever says them.
fn reads_as_signed_out(text: &str) -> bool {
    let text = text.to_lowercase();
    [
        "not logged in",
        "/login",
        "please log in",
        "not signed in",
        "unauthorized",
        "401",
        "auth",
    ]
    .iter()
    .any(|word| text.contains(word))
}

/// What a tool call is doing, in a few words: "Reading notes.md".
fn doing(tool: &str, input: &serde_json::Value) -> String {
    let file = |key: &str| {
        input[key].as_str().map(|p| {
            Path::new(p)
                .file_name()
                .map_or(p.to_string(), |n| n.to_string_lossy().into())
        })
    };
    match tool {
        "Read" | "read_file" | "read_many_files" => {
            match file("file_path").or_else(|| file("path")) {
                Some(name) => format!("Reading {name}"),
                None => "Reading".to_string(),
            }
        }
        "Grep" | "search_file_content" | "grep" => "Searching the files".to_string(),
        "Glob" | "glob" | "list_directory" | "ls" => "Looking through the files".to_string(),
        "command_execution" => "Looking through the files".to_string(),
        _ => "Working".to_string(),
    }
}

/// Reads a command line's lines of JSON into news, keeping what it needs between them.
pub struct Reader {
    cli: Cli,
    /// Claude Code said it couldn't sign in, in a message before its result.
    signed_out: bool,
}

impl Reader {
    pub fn new(cli: Cli) -> Self {
        Self {
            cli,
            signed_out: false,
        }
    }

    pub fn line(&mut self, line: &str) -> Vec<News> {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line.trim()) else {
            return Vec::new();
        };
        match self.cli {
            Cli::Claude => self.claude(&value),
            Cli::Codex => codex(&value),
            Cli::Gemini => gemini(&value),
        }
    }

    fn claude(&mut self, value: &serde_json::Value) -> Vec<News> {
        let text_of = |message: &serde_json::Value| -> String {
            let blocks = message["content"].as_array().cloned().unwrap_or_default();
            blocks
                .iter()
                .filter(|b| b["type"] == "text")
                .filter_map(|b| b["text"].as_str())
                .collect::<Vec<_>>()
                .join("")
        };
        match value["type"].as_str().unwrap_or("") {
            "system" if value["subtype"] == "init" => value["session_id"]
                .as_str()
                .map(|id| vec![News::Session(id.to_string())])
                .unwrap_or_default(),
            "stream_event" => {
                let event = &value["event"];
                match event["type"].as_str().unwrap_or("") {
                    "message_start" => vec![News::Fresh],
                    "content_block_delta" if event["delta"]["type"] == "text_delta" => {
                        event["delta"]["text"]
                            .as_str()
                            .map(|t| vec![News::Text(t.to_string())])
                            .unwrap_or_default()
                    }
                    _ => Vec::new(),
                }
            }
            "assistant" => {
                if value["error"].as_str() == Some("authentication_failed") {
                    self.signed_out = true;
                }
                let message = &value["message"];
                let mut news = Vec::new();
                let blocks = message["content"].as_array().cloned().unwrap_or_default();
                for block in blocks.iter().filter(|b| b["type"] == "tool_use") {
                    let tool = block["name"].as_str().unwrap_or("");
                    news.push(News::Doing(doing(tool, &block["input"])));
                }
                let text = text_of(message);
                if !text.is_empty() && !self.signed_out {
                    news.push(News::Said(text));
                }
                news
            }
            "result" => {
                let said = value["result"].as_str().unwrap_or("").to_string();
                if value["is_error"] == true || value["subtype"] != "success" {
                    let failure = if self.signed_out || reads_as_signed_out(&said) {
                        Failure::NotSignedIn
                    } else if said.is_empty() {
                        Failure::Said(
                            value["subtype"]
                                .as_str()
                                .unwrap_or("error")
                                .replace('_', " "),
                        )
                    } else {
                        Failure::Said(said)
                    };
                    vec![News::Failed(failure)]
                } else {
                    vec![News::Done(said)]
                }
            }
            _ => Vec::new(),
        }
    }
}

fn codex(value: &serde_json::Value) -> Vec<News> {
    let item = &value["item"];
    match value["type"].as_str().unwrap_or("") {
        "thread.started" => value["thread_id"]
            .as_str()
            .map(|id| vec![News::Session(id.to_string())])
            .unwrap_or_default(),
        "item.started" if item["type"] == "command_execution" => {
            vec![News::Doing(doing("command_execution", item))]
        }
        "item.completed" if item["type"] == "agent_message" => item["text"]
            .as_str()
            .map(|t| vec![News::Fresh, News::Said(t.to_string())])
            .unwrap_or_default(),
        "turn.completed" => vec![News::Done(String::new())],
        "turn.failed" | "error" => {
            let said = value["error"]["message"]
                .as_str()
                .or(value["message"].as_str())
                .unwrap_or("it stopped")
                .to_string();
            let failure = if reads_as_signed_out(&said) {
                Failure::NotSignedIn
            } else {
                Failure::Said(said)
            };
            vec![News::Failed(failure)]
        }
        _ => Vec::new(),
    }
}

fn gemini(value: &serde_json::Value) -> Vec<News> {
    match value["type"].as_str().unwrap_or("") {
        "init" => value["session_id"]
            .as_str()
            .map(|id| vec![News::Session(id.to_string())])
            .unwrap_or_default(),
        "message" if value["role"] == "assistant" => {
            let text = value["content"].as_str().unwrap_or("").to_string();
            if value["delta"] == true {
                vec![News::Text(text)]
            } else {
                vec![News::Said(text)]
            }
        }
        "tool_use" => {
            let tool = value["tool_name"].as_str().unwrap_or("");
            vec![News::Doing(doing(tool, &value["parameters"]))]
        }
        "result" if value["status"] == "success" => vec![News::Done(String::new())],
        "result" | "error" => {
            let said = value["error"]["message"]
                .as_str()
                .or(value["message"].as_str())
                .unwrap_or("it stopped")
                .to_string();
            let failure = if reads_as_signed_out(&said) {
                Failure::NotSignedIn
            } else {
                Failure::Said(said)
            };
            vec![News::Failed(failure)]
        }
        _ => Vec::new(),
    }
}

/// The variables a run leaves out: those saying it's held by Cellix or named by it, or inside
/// another agent's session, which a side question isn't.
const LEFT_OUT: [&str; 6] = [
    cellix_agent::hook::HELD_VAR,
    cellix_agent::team::NAME_VAR,
    "CELLIX_MOD",
    "CLAUDECODE",
    "CLAUDE_CODE_ENTRYPOINT",
    "CLAUDE_CODE_SESSION_ID",
];

/// A run under way. Dropped, it's stopped.
pub struct Running {
    pub news: Receiver<News>,
    child: Arc<Mutex<Option<Child>>>,
    ended: Arc<AtomicBool>,
}

impl Drop for Running {
    fn drop(&mut self) {
        self.ended.store(true, Ordering::SeqCst);
        if let Ok(mut child) = self.child.lock()
            && let Some(child) = child.as_mut()
        {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// Starts `program` on `question`, with `path` as its PATH, stopped after `limit`. What it says
/// comes on `news`, ending with `Done` or `Failed`, each waking the loop.
pub fn start(program: &Path, question: &Question, path: &OsString, limit: Duration) -> Running {
    let run = Run {
        cli: question.cli,
        args: args(question),
        root: &question.root,
        prompt: &question.prompt,
    };
    start_run(program, run, path, limit)
}

/// One run of a command line, said whole: which it is, its arguments after its name, the
/// folder it runs in and what it's told on its input. Ask builds one from a question; a
/// remote doc's reads and posts build their own, with the tools they allow.
pub struct Run<'a> {
    pub cli: Cli,
    pub args: Vec<String>,
    pub root: &'a Path,
    pub prompt: &'a str,
}

/// Starts `program` as `run` says, with `path` as its PATH, stopped after `limit`, its news
/// read as `start`'s is.
pub fn start_run(program: &Path, run: Run, path: &OsString, limit: Duration) -> Running {
    let (sender, news) = crate::wake::channel();
    let ended = Arc::new(AtomicBool::new(false));
    let shared = Arc::new(Mutex::new(None));
    let running = Running {
        news,
        child: shared.clone(),
        ended: ended.clone(),
    };
    let mut command = Process::new(program);
    command
        .args(&run.args)
        .current_dir(run.root)
        .env("PATH", path)
        .env(SIDE_QUESTION_VAR, "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for name in LEFT_OUT {
        command.env_remove(name);
    }
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(e) => {
            let _ = sender.send(News::Failed(Failure::NotStarted(e.to_string())));
            return running;
        }
    };
    let input = child.stdin.take();
    let output = child.stdout.take();
    let errors = child.stderr.take();
    if let Ok(mut held) = shared.lock() {
        *held = Some(child);
    }
    // On a thread, so a long document can't fill the pipe while nothing reads the other end.
    if let Some(mut input) = input {
        let prompt = run.prompt.to_string();
        std::thread::spawn(move || {
            let _ = input.write_all(prompt.as_bytes());
        });
    }
    let said = Arc::new(Mutex::new(String::new()));
    if let Some(mut errors) = errors {
        let said = said.clone();
        std::thread::spawn(move || {
            let mut text = String::new();
            let _ = errors.read_to_string(&mut text);
            if let Ok(mut said) = said.lock() {
                *said = text;
            }
        });
    }
    // The clock: past the limit it's stopped, and says so, unless it ended first.
    {
        let (ended, sender, child) = (ended.clone(), sender.clone(), shared.clone());
        std::thread::spawn(move || {
            let step = Duration::from_millis(50);
            let mut waited = Duration::ZERO;
            while waited < limit {
                if ended.load(Ordering::SeqCst) {
                    return;
                }
                std::thread::sleep(step);
                waited += step;
            }
            if !ended.swap(true, Ordering::SeqCst) {
                if let Ok(mut child) = child.lock()
                    && let Some(child) = child.as_mut()
                {
                    let _ = child.kill();
                }
                let _ = sender.send(News::Failed(Failure::TimedOut));
            }
        });
    }
    let cli = run.cli;
    std::thread::spawn(move || {
        let mut reader = Reader::new(cli);
        let mut finished = false;
        if let Some(output) = output {
            for line in BufReader::new(output).lines() {
                let Ok(line) = line else { break };
                for item in reader.line(&line) {
                    if matches!(item, News::Done(_) | News::Failed(_)) {
                        if ended.swap(true, Ordering::SeqCst) {
                            return;
                        }
                        finished = true;
                    }
                    if sender.send(item).is_err() {
                        return;
                    }
                }
            }
        }
        if finished {
            return;
        }
        // It ended without a result: by its exit, and what it said on its errors. Looked at
        // without holding the child, which dismissing the card or the clock may want to stop.
        let status = loop {
            let exited = shared
                .lock()
                .ok()
                .and_then(|mut child| child.as_mut().and_then(|c| c.try_wait().ok().flatten()));
            if exited.is_some() || ended.load(Ordering::SeqCst) {
                break exited;
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        if ended.swap(true, Ordering::SeqCst) {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
        let errors = said.lock().map(|s| s.clone()).unwrap_or_default();
        let last = errors.lines().rev().find(|l| !l.trim().is_empty());
        let news = match status {
            Some(status) if status.success() => News::Done(String::new()),
            _ if reads_as_signed_out(&errors) => News::Failed(Failure::NotSignedIn),
            Some(status) => News::Failed(Failure::Said(match last {
                Some(line) => line.trim().to_string(),
                None => format!("it exited with {}", status.code().unwrap_or(-1)),
            })),
            None => News::Failed(Failure::Said("it stopped".to_string())),
        };
        let _ = sender.send(news);
    });
    running
}

/// A side conversation kept for a document: which command line, its session, and the
/// document's text as it last went, by its hash, so a question on the same text doesn't
/// send it again.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Kept {
    pub cli: Cli,
    pub session: String,
    pub version: u64,
}

/// Each document's side conversation, by its path in the project.
pub type Conversations = HashMap<String, Kept>;

/// Where they're kept: with Cellix's own state for the project, beside its sessions.
fn kept_file(root: &Path) -> PathBuf {
    crate::hold::folder_of(root).join("asks.json")
}

pub fn load(root: &Path) -> Conversations {
    std::fs::read(kept_file(root))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

pub fn save(root: &Path, kept: &Conversations) {
    let file = kept_file(root);
    if let Some(folder) = file.parent() {
        let _ = std::fs::create_dir_all(folder);
    }
    if let Ok(json) = serde_json::to_vec_pretty(kept) {
        let _ = std::fs::write(file, json);
    }
}

/// A text's hash, to tell one version from another.
pub fn version(text: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish()
}

/// What a question is about, on a page or in code.
pub struct Doc<'a> {
    /// Its path in the project, as the agent is told it.
    pub path: &'a str,
    /// The folder a paper's `\input` names are read from.
    pub folder: Option<&'a Path>,
    pub text: &'a str,
    /// How it reads as a page; none for code.
    pub dialect: Option<Dialect>,
    /// The selection, as letters from the start.
    pub selection: (usize, usize),
}

/// The text with the selection marked.
pub fn marked(text: &str, (start, end): (usize, usize)) -> String {
    let mut out = String::with_capacity(text.len() + 8);
    for (at, c) in text.chars().enumerate() {
        if at == start {
            out.push(OPEN_MARK);
        }
        out.push(c);
        if at + 1 == end {
            out.push(CLOSE_MARK);
        }
    }
    if start >= text.chars().count() {
        out.push(OPEN_MARK);
        out.push(CLOSE_MARK);
    }
    out
}

/// A paper with its `\input` and `\include` files read in where they're named, each between
/// lines saying which file it is, so the agent reads the paper as LaTeX does. A file that reads
/// itself in is read once.
pub fn with_inputs(text: &str, folder: &Path) -> String {
    fn read_in(text: &str, folder: &Path, reading: &mut Vec<PathBuf>, out: &mut String) {
        for line in text.split_inclusive('\n') {
            let code = cellix_doc::paper::uncommented(line);
            let found = cellix_doc::paper::inputs(code);
            if found.is_empty() || reading.len() >= 16 {
                out.push_str(line);
                continue;
            }
            let mut from = 0;
            for (at, name, end) in found {
                out.push_str(&code[from..at]);
                let path = cellix_doc::paper::resolve(folder, name);
                let canonical = path.canonicalize().unwrap_or_else(|_| path.clone());
                match std::fs::read_to_string(&path) {
                    Ok(inner) if !reading.contains(&canonical) => {
                        let shown = path.strip_prefix(folder).unwrap_or(&path).display();
                        out.push_str(&format!("\n%% ── {shown}, read in here ──\n"));
                        reading.push(canonical);
                        read_in(&inner, folder, reading, out);
                        reading.pop();
                        out.push_str(&format!("\n%% ── end of {shown} ──\n"));
                    }
                    _ => out.push_str(&code[at..end]),
                }
                from = end;
            }
            out.push_str(&code[from..]);
            if line.ends_with('\n') && !code.ends_with('\n') {
                out.push('\n');
            }
        }
    }
    let mut out = String::with_capacity(text.len());
    read_in(text, folder, &mut Vec::new(), &mut out);
    out
}

/// The headings a line comes under, outermost first.
fn path_at(headings: &[crate::outline::Heading], line: usize, numbered: bool) -> Vec<String> {
    let mut numbers = [0usize; 8];
    let mut path: Vec<(u8, String)> = Vec::new();
    for heading in headings.iter().filter(|h| h.line <= line) {
        let level = heading.level.clamp(1, 7);
        numbers[level as usize] += 1;
        for deeper in numbers.iter_mut().skip(level as usize + 1) {
            *deeper = 0;
        }
        path.retain(|(l, _)| *l < level);
        let title = if numbered {
            // A paper's sections are numbered from its top level down, as LaTeX numbers them.
            let top = headings.iter().map(|h| h.level).min().unwrap_or(level);
            let number: Vec<String> = numbers[top as usize..=level as usize]
                .iter()
                .map(usize::to_string)
                .collect();
            format!("{} {}", number.join("."), heading.title)
        } else {
            heading.title.clone()
        };
        path.push((level, title));
    }
    path.into_iter().map(|(_, title)| title).collect()
}

/// The line, counting from zero, a letter offset is on.
fn line_of(text: &str, at: usize) -> usize {
    text.chars().take(at).filter(|c| *c == '\n').count()
}

/// What the first question about a document tells the agent: the rules, where the document is,
/// where the selection is in it, and the document itself with the selection marked; past
/// `WHOLE_LIMIT` letters, the part it's in with the outline. Then the question.
pub fn first_prompt(doc: &Doc, question: &str, copy: bool) -> String {
    let (text, section, whole) = document(doc);
    let quote = selected(doc);
    let mut prompt = String::from(RULES);
    if copy {
        prompt.push_str(
            " You're a copy of the conversation the reader's agent is having about this work, \
             so answer from what you know of it as well as from the document; the agent itself \
             won't see this.",
        );
    }
    prompt.push_str(&format!(
        "\n\nThe document is {path}, in the project you're in. Read it, or files it names such \
         as its bibliography, with your tools if you need more.\n",
        path = doc.path
    ));
    if !section.is_empty() {
        prompt.push_str(&format!("The selection is under: {section}\n"));
    }
    if whole {
        prompt.push_str(&format!(
            "\nThe document, with the selection marked {OPEN_MARK}like this{CLOSE_MARK}:\n\n{text}\n"
        ));
    } else {
        prompt.push_str(&format!(
            "\nThe document is long, so here is the part the selection is in, marked \
             {OPEN_MARK}like this{CLOSE_MARK}, with the document's outline. The rest is in the \
             file.\n\n{text}\n"
        ));
    }
    prompt.push_str(&format!(
        "\nThe question, about {OPEN_MARK}{quote}{CLOSE_MARK}: {question}\n"
    ));
    prompt
}

/// What a later question about the same document says, the document unchanged and already in
/// the conversation.
pub fn next_prompt(doc: &Doc, question: &str) -> String {
    let (_, section, _) = document(doc);
    let quote = selected(doc);
    let under = if section.is_empty() {
        String::new()
    } else {
        format!(" (under {section})")
    };
    format!(
        "Another side question about {path}, on {OPEN_MARK}{quote}{CLOSE_MARK}{under}. The same \
         rules: answer only this, briefly and for the reader, and change nothing.\n\n{question}\n",
        path = doc.path
    )
}

/// A question that follows an answer on the same card.
pub fn follow_up_prompt(question: &str) -> String {
    format!(
        "{question}\n\n(The same rules: answer briefly and for the reader, and change nothing.)"
    )
}

/// What was selected, on one line, as the agent is told it.
fn selected(doc: &Doc) -> String {
    let (start, end) = doc.selection;
    let quote: String = doc
        .text
        .chars()
        .skip(start)
        .take(end.saturating_sub(start))
        .collect();
    quote.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The document as it goes, the headings the selection is under, and whether it's whole.
fn document(doc: &Doc) -> (String, String, bool) {
    let mut text = marked(doc.text, doc.selection);
    if let (Some(Dialect::Latex), Some(folder)) = (doc.dialect, doc.folder) {
        text = with_inputs(&text, folder);
    }
    let at = text.chars().position(|c| c == OPEN_MARK).unwrap_or(0);
    let line = line_of(&text, at);
    let headings = match doc.dialect {
        Some(dialect) => crate::outline::of_in(&text, dialect, None),
        None => Vec::new(),
    };
    let numbered = doc.dialect == Some(Dialect::Latex);
    let section = path_at(&headings, line, numbered).join(" › ");
    let section = if section.is_empty() && doc.dialect.is_none() {
        format!("line {}", line_of(doc.text, doc.selection.0) + 1)
    } else {
        section
    };
    if text.chars().count() <= WHOLE_LIMIT {
        return (text, section, true);
    }
    (part_of(&text, &headings, line), section, false)
}

/// The part of a long text a line is in: the widest section round it that fits, else as much
/// round the line as fits; after the document's outline.
fn part_of(text: &str, headings: &[crate::outline::Heading], line: usize) -> String {
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let span = |from: usize, to: usize| lines[from.min(lines.len())..to.min(lines.len())].concat();
    let mut part = None;
    // Outermost first: the first section round the line that fits is the widest.
    let mut levels: Vec<u8> = headings.iter().map(|h| h.level).collect();
    levels.sort_unstable();
    levels.dedup();
    for level in levels {
        let Some(start) = headings
            .iter()
            .filter(|h| h.level == level && h.line <= line)
            .map(|h| h.line)
            .max()
        else {
            continue;
        };
        let end = headings
            .iter()
            .filter(|h| h.line > start && h.level <= level)
            .map(|h| h.line)
            .min()
            .unwrap_or(lines.len());
        if end <= line {
            continue;
        }
        let candidate = span(start, end);
        if candidate.chars().count() <= WHOLE_LIMIT {
            part = Some(candidate);
            break;
        }
    }
    let part = part.unwrap_or_else(|| {
        // As many letters either side of the selection as fit.
        let at = text.chars().position(|c| c == OPEN_MARK).unwrap_or(0);
        let half = WHOLE_LIMIT / 2;
        let from = at.saturating_sub(half);
        text.chars().skip(from).take(WHOLE_LIMIT).collect()
    });
    let outline: String = headings
        .iter()
        .map(|h| {
            let indent = "  ".repeat(usize::from(h.level.saturating_sub(1)));
            format!("{indent}{} (line {})\n", h.title, h.line + 1)
        })
        .collect();
    format!("Outline:\n{outline}\n…\n{part}\n…")
}

/// What a question about terminal output tells the agent: the rules, where the output came
/// from, the output, and the question.
pub fn output_prompt(
    tab: &str,
    command: &str,
    places: &[String],
    output: &str,
    question: &str,
) -> String {
    let from = if command.is_empty() {
        String::new()
    } else {
        format!(" from `{command}`")
    };
    let places = if places.is_empty() {
        String::new()
    } else {
        format!(" It names {}.", places.join(", "))
    };
    format!(
        "{RULES}\n\nThe reader selected output in the terminal tab {tab}{from}, in the project \
         you're in.{places} You can read the project's files with your tools.\n\nThe output:\n\n\
         {output}\n\nThe question, about that output: {question}\n"
    )
}

/// The quick questions under the field, and what each asks. Enter with nothing typed asks the
/// first.
pub const QUICK: [(&str, &str); 3] = [
    ("Explain", "Explain this, as it's meant here."),
    ("Define", "Define this term, as it's used here."),
    ("Simplify", "Say this more simply."),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_setting_then_the_pane_then_the_first_installed_answers() {
        let all = |_: Cli| true;
        assert_eq!(
            choose(Some("codex"), Some(Cli::Gemini), all),
            Ok(Cli::Codex)
        );
        assert_eq!(choose(None, Some(Cli::Gemini), all), Ok(Cli::Gemini));
        assert_eq!(choose(None, None, all), Ok(Cli::Claude));
        let codex_only = |cli: Cli| cli == Cli::Codex;
        assert_eq!(choose(None, Some(Cli::Claude), codex_only), Ok(Cli::Codex));
        assert_eq!(
            choose(Some("gemini"), None, codex_only),
            Err(Some(Cli::Gemini))
        );
        assert_eq!(choose(None, None, |_| false), Err(None));
    }

    #[test]
    fn claude_streams_its_answer_and_says_its_session() {
        let mut reader = Reader::new(Cli::Claude);
        let lines = [
            r#"{"type":"system","subtype":"init","session_id":"s-1","cwd":"/p"}"#,
            r#"{"type":"stream_event","event":{"type":"message_start"}}"#,
            r#"{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"In-context"}}}"#,
            r#"{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":" learning."}}}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Read","input":{"file_path":"/p/refs.bib"}}]}}"#,
            r#"{"type":"result","subtype":"success","is_error":false,"result":"In-context learning."}"#,
        ];
        let news: Vec<News> = lines.iter().flat_map(|l| reader.line(l)).collect();
        assert_eq!(
            news,
            [
                News::Session("s-1".into()),
                News::Fresh,
                News::Text("In-context".into()),
                News::Text(" learning.".into()),
                News::Doing("Reading refs.bib".into()),
                News::Done("In-context learning.".into()),
            ]
        );
    }

    #[test]
    fn a_signed_out_claude_says_so() {
        let mut reader = Reader::new(Cli::Claude);
        let lines = [
            r#"{"type":"assistant","error":"authentication_failed","message":{"content":[{"type":"text","text":"Not logged in · Please run /login"}]}}"#,
            r#"{"type":"result","subtype":"success","is_error":true,"result":"Not logged in · Please run /login"}"#,
        ];
        let news: Vec<News> = lines.iter().flat_map(|l| reader.line(l)).collect();
        assert_eq!(news, [News::Failed(Failure::NotSignedIn)]);
    }

    #[test]
    fn codex_says_its_thread_and_its_message() {
        let mut reader = Reader::new(Cli::Codex);
        let lines = [
            r#"{"type":"thread.started","thread_id":"t-9"}"#,
            r#"{"type":"turn.started"}"#,
            r#"{"type":"item.started","item":{"id":"item_0","type":"command_execution","command":"rg ICL","status":"in_progress"}}"#,
            r#"{"type":"item.completed","item":{"id":"item_1","type":"agent_message","text":"In-context learning."}}"#,
            r#"{"type":"turn.completed","usage":{}}"#,
        ];
        let news: Vec<News> = lines.iter().flat_map(|l| reader.line(l)).collect();
        assert_eq!(
            news,
            [
                News::Session("t-9".into()),
                News::Doing("Looking through the files".into()),
                News::Fresh,
                News::Said("In-context learning.".into()),
                News::Done(String::new()),
            ]
        );
    }

    #[test]
    fn gemini_streams_and_fails_in_words() {
        let mut reader = Reader::new(Cli::Gemini);
        let lines = [
            r#"{"type":"init","session_id":"g-2","model":"m"}"#,
            r#"{"type":"message","role":"user","content":"q"}"#,
            r#"{"type":"message","role":"assistant","content":"In-context","delta":true}"#,
            r#"{"type":"result","status":"error","error":{"message":"quota exceeded"}}"#,
        ];
        let news: Vec<News> = lines.iter().flat_map(|l| reader.line(l)).collect();
        assert_eq!(
            news,
            [
                News::Session("g-2".into()),
                News::Text("In-context".into()),
                News::Failed(Failure::Said("quota exceeded".into())),
            ]
        );
    }

    /// A run is the program given read-only arguments, told it's a side question, with the
    /// prompt on its input; what it prints comes back as news, its session first.
    #[cfg(unix)]
    #[test]
    fn a_run_is_read_only_told_it_is_a_side_question_and_streams_back() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let program = dir.path().join("claude");
        let record = dir.path().join("record");
        let lines = [
            r#"{"type":"system","subtype":"init","session_id":"fork-1"}"#,
            r#"{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"Short."}}}"#,
            r#"{"type":"result","subtype":"success","is_error":false,"result":"Short."}"#,
        ];
        let out = dir.path().join("out");
        std::fs::write(&out, lines.join("\n") + "\n").unwrap();
        let script = format!(
            "#!/bin/sh\nprintf '%s\\n' \"$@\" > '{r}.args'\nprintf '%s' \"$CELLIX_SIDE_QUESTION\" > '{r}.env'\n/bin/cat > '{r}.prompt'\n/bin/cat '{o}'\n",
            r = record.display(),
            o = out.display()
        );
        std::fs::write(&program, script).unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        let question = Question {
            cli: Cli::Claude,
            root: dir.path().to_path_buf(),
            prompt: "The whole prompt.".to_string(),
            resume: Some("main-1".to_string()),
            fork: true,
        };
        let path = OsString::from(dir.path());
        let running = start(&program, &question, &path, Duration::from_secs(120));
        let mut news = Vec::new();
        while let Ok(item) = running.news.recv_timeout(Duration::from_secs(120)) {
            let end = matches!(item, News::Done(_) | News::Failed(_));
            news.push(item);
            if end {
                break;
            }
        }
        assert_eq!(
            news,
            [
                News::Session("fork-1".into()),
                News::Text("Short.".into()),
                News::Done("Short.".into()),
            ]
        );
        let read = |end: &str| std::fs::read_to_string(format!("{}.{end}", record.display()));
        let args: Vec<String> = read("args").unwrap().lines().map(str::to_string).collect();
        assert_eq!(
            args,
            [
                "-p",
                "--output-format",
                "stream-json",
                "--verbose",
                "--include-partial-messages",
                "--resume",
                "main-1",
                "--fork-session",
                "--allowedTools",
                "Read,Grep,Glob",
                "--disallowedTools",
                "Bash,Edit,Write,MultiEdit,NotebookEdit",
            ]
        );
        assert_eq!(read("env").unwrap(), "1");
        assert_eq!(read("prompt").unwrap(), "The whole prompt.");
    }

    #[test]
    fn a_paper_reads_its_inputs_in_and_the_selection_is_marked_under_its_sections() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("method.tex"),
            "\\subsection{Training}\nWe rely on ICL here.\n",
        )
        .unwrap();
        let text = "\\section{Intro}\nHello.\n\\section{Method}\n\\input{method}\n";
        let at = text[..text.find("Hello.").unwrap()].chars().count();
        let doc = Doc {
            path: "paper.tex",
            folder: Some(dir.path()),
            text,
            dialect: Some(Dialect::Latex),
            selection: (at, at + "Hello.".len()),
        };
        let prompt = first_prompt(&doc, "Is this the right word?", false);
        assert!(prompt.contains("⟦Hello.⟧"), "{prompt}");
        assert!(
            prompt.contains("We rely on ICL here."),
            "the input is read in: {prompt}"
        );
        assert!(
            prompt.contains("The selection is under: 1 Intro"),
            "{prompt}"
        );
        assert!(prompt.contains("paper.tex"), "{prompt}");
        assert!(prompt.contains("Is this the right word?"), "{prompt}");
    }
}
