//! An agent's transcript, read back as a page: what it was asked, what it said, each tool it
//! ran with what went in and what came back, in order. Claude Code keeps one JSON line per
//! event in `~/.claude/projects/<project>/<session>.jsonl`, Codex in a rollout log under
//! `~/.codex/sessions`. Everything in them is the agent's or a tool's text: it's shown, never
//! acted on, and what a tool printed goes in code blocks, so none of it is laid out as the
//! page's own.
//!
//! The same lines say how many tokens each reply took in, which is how full the agent's
//! context is, and that's what the agent's tab shows.

use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use serde_json::Value;

/// Where Cellix keeps the pages it makes of transcripts, inside the project's `.cellix`.
pub const PAGES: &str = ".cellix/transcripts";
/// A step's output past this many lines is cut to its last ones.
const SHOWN_LINES: usize = 12;
/// What an agent's window is taken to be when its log doesn't say.
const WINDOW: u64 = 200_000;
/// A Claude model with the long window, which its log only shows by using it.
const LONG_WINDOW: u64 = 1_000_000;
/// From here the tab warns that the agent will soon compact, and from `FULL` that it's about to.
pub const WARN: u8 = 80;
pub const FULL: u8 = 95;
/// How much of a long prompt or input a step's one line keeps.
const SUMMARY: usize = 80;

/// One thing the agent did or said within a turn.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Item {
    Text(String),
    Thinking(String),
    Tool(Tool),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Tool {
    pub name: String,
    /// The input on one line: the command, the file, the pattern.
    pub summary: String,
    /// What came back.
    pub output: String,
    pub failed: bool,
    /// A word on the result, as `42 lines` or `+18 −2`.
    pub note: String,
    /// Whether its output is worth showing: a command's is, a file read whole isn't.
    pub show_output: bool,
    /// Its id in the log, which its result names.
    id: String,
}

/// A prompt and everything the agent did about it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Turn {
    pub prompt: String,
    pub items: Vec<Item>,
    /// Context at the turn's first reply and its last.
    pub context: Option<(u64, u64)>,
}

/// How full the context is, and what the session has used.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Usage {
    /// Tokens the last reply took in, which is what the next one starts from.
    pub context: u64,
    pub window: u64,
    /// Every reply's input and output, added up.
    pub input: u64,
    pub output: u64,
    pub model: String,
}

impl Usage {
    /// How full the context is, in percent, once anything has been used.
    pub fn percent(&self) -> Option<u8> {
        if self.context == 0 || self.window == 0 {
            return None;
        }
        Some((self.context * 100 / self.window).min(100) as u8)
    }

    /// The tooltip on the agent's tab.
    pub fn describe(&self) -> String {
        let percent = self.percent().unwrap_or(0);
        let mut text = format!(
            "Context {} of {} ({percent}%)",
            tokens(self.context),
            tokens(self.window)
        );
        if percent >= FULL {
            text.push_str(", it compacts any moment");
        } else if percent >= WARN {
            text.push_str(", it compacts soon");
        }
        text.push_str(&format!(
            " · session {} in, {} out · right-click to open the transcript",
            tokens(self.input),
            tokens(self.output)
        ));
        text
    }
}

/// A count of tokens as a person reads it: 940, 41k, 3.1M.
pub fn tokens(count: u64) -> String {
    if count >= 1_000_000 {
        let tenths = (count + 50_000) / 100_000;
        if tenths.is_multiple_of(10) {
            format!("{}M", tenths / 10)
        } else {
            format!("{}.{}M", tenths / 10, tenths % 10)
        }
    } else if count >= 1_000 {
        format!("{}k", (count + 500) / 1_000)
    } else {
        count.to_string()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Transcript {
    pub title: String,
    pub agent: String,
    pub turns: Vec<Turn>,
    pub usage: Usage,
}

/// Reads a transcript, as Claude Code or Codex writes one, telling them apart by its lines.
pub fn read(path: &Path) -> std::io::Result<Transcript> {
    let text = std::fs::read_to_string(path)?;
    Ok(parse(&text))
}

pub fn parse(text: &str) -> Transcript {
    let codex = text
        .lines()
        .take(5)
        .any(|line| line.contains("\"payload\""));
    if codex {
        parse_codex(text)
    } else {
        parse_claude(text)
    }
}

/// Takes `<system-reminder>` and the like out of what the user is shown to have typed:
/// Claude Code adds them to a prompt, and the person never wrote them.
fn typed(text: &str) -> String {
    let mut rest = text;
    let mut kept = String::new();
    while let Some(start) = rest.find("<system-reminder>") {
        kept.push_str(&rest[..start]);
        match rest[start..].find("</system-reminder>") {
            Some(end) => rest = &rest[start + end + "</system-reminder>".len()..],
            None => rest = "",
        }
    }
    kept.push_str(rest);
    let kept = kept.trim();
    // A slash command shows as what was typed.
    if let Some(name) = between(kept, "<command-name>", "</command-name>") {
        let args = between(kept, "<command-args>", "</command-args>").unwrap_or("");
        return format!("{} {}", name.trim(), args.trim())
            .trim()
            .to_string();
    }
    if kept.starts_with("<local-command") || kept.starts_with("<command-") {
        return String::new();
    }
    kept.to_string()
}

fn between<'a>(text: &'a str, open: &str, close: &str) -> Option<&'a str> {
    let start = text.find(open)? + open.len();
    let end = text[start..].find(close)? + start;
    Some(&text[start..end])
}

/// The text of a tool's result, which is a string or a list of text blocks.
fn result_text(content: &Value) -> String {
    match content {
        Value::String(text) => text.clone(),
        Value::Array(blocks) => blocks
            .iter()
            .filter_map(|b| b["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

fn one_line(text: &str) -> String {
    let first = text.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
    let first = first.trim();
    let more = text.trim().lines().count() > 1;
    if first.chars().count() > SUMMARY {
        let cut: String = first.chars().take(SUMMARY).collect();
        format!("{cut}…")
    } else if more {
        format!("{first} …")
    } else {
        first.to_string()
    }
}

fn line_count(text: &str) -> usize {
    if text.is_empty() {
        0
    } else {
        text.lines().count()
    }
}

/// A tool call as a step: its one line, and whether what it returns is worth showing.
fn tool(name: &str, input: &Value, cwd: &str, id: &str) -> Tool {
    let text = |key: &str| input[key].as_str().unwrap_or("").to_string();
    let path = |key: &str| {
        let full = text(key);
        let relative = full
            .strip_prefix(cwd)
            .map(|r| r.trim_start_matches('/'))
            .filter(|r| !cwd.is_empty() && !r.is_empty());
        relative.map_or(full.clone(), str::to_string)
    };
    let (summary, note, show_output) = match name {
        "Bash" | "shell" | "exec_command" | "local_shell" => {
            let command = match &input["command"] {
                Value::Array(words) => words
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join(" "),
                _ => text("command").if_empty(text("cmd")),
            };
            (one_line(&command), String::new(), true)
        }
        "Read" => (path("file_path"), String::new(), false),
        "Edit" | "MultiEdit" => {
            let (old, new) = match input["edits"].as_array() {
                Some(edits) => edits.iter().fold((0, 0), |(o, n), edit| {
                    (
                        o + line_count(edit["old_string"].as_str().unwrap_or("")),
                        n + line_count(edit["new_string"].as_str().unwrap_or("")),
                    )
                }),
                None => (
                    line_count(&text("old_string")),
                    line_count(&text("new_string")),
                ),
            };
            (path("file_path"), format!("+{new} −{old}"), false)
        }
        "Write" => {
            let lines = line_count(&text("content"));
            (path("file_path"), format!("{lines} lines"), false)
        }
        "Grep" | "Glob" => {
            let place = path("path");
            let summary = if place.is_empty() {
                text("pattern")
            } else {
                format!("{} in {place}", text("pattern"))
            };
            (summary, String::new(), false)
        }
        "WebFetch" => (text("url"), String::new(), false),
        "WebSearch" => (text("query"), String::new(), false),
        "Task" | "Agent" => (one_line(&text("description")), String::new(), true),
        "TodoWrite" => {
            let count = input["todos"].as_array().map_or(0, Vec::len);
            (format!("{count} to-dos"), String::new(), false)
        }
        _ => {
            // The first text it was given says best what it was for.
            let first = input
                .as_object()
                .and_then(|o| o.values().find_map(Value::as_str))
                .unwrap_or("");
            (one_line(first), String::new(), true)
        }
    };
    Tool {
        name: name.to_string(),
        summary,
        note,
        show_output,
        id: id.to_string(),
        ..Tool::default()
    }
}

trait IfEmpty {
    fn if_empty(self, other: String) -> String;
}

impl IfEmpty for String {
    fn if_empty(self, other: String) -> String {
        if self.is_empty() { other } else { self }
    }
}

/// Puts a tool's result on its step, found by the id they share.
fn settle(turns: &mut [Turn], id: &str, output: String, failed: bool) {
    let steps = turns
        .iter_mut()
        .rev()
        .flat_map(|t| t.items.iter_mut().rev());
    for item in steps {
        if let Item::Tool(tool) = item
            && tool.id == id
        {
            if !failed {
                tool.note = match tool.name.as_str() {
                    "Read" => format!("{} lines", line_count(&output)),
                    // Claude Code's search says how much it found first.
                    "Grep" | "Glob" => match output.lines().next() {
                        Some(first) if first.starts_with("Found ") => first[6..].to_string(),
                        _ => format!("{} found", line_count(&output)),
                    },
                    _ if tool.show_output && tool.note.is_empty() => "ok".to_string(),
                    _ => std::mem::take(&mut tool.note),
                };
            }
            tool.output = output;
            tool.failed = failed;
            return;
        }
    }
}

/// Adds a reply's context to its turn: the first sets where it started, each sets the end.
fn note_context(turns: &mut [Turn], context: u64) {
    if let Some(turn) = turns.last_mut() {
        turn.context = Some(match turn.context {
            Some((start, _)) => (start, context),
            None => (context, context),
        });
    }
}

fn last_turn(turns: &mut Vec<Turn>) -> &mut Turn {
    if turns.is_empty() {
        turns.push(Turn::default());
    }
    turns.last_mut().expect("just made sure")
}

/// The window a Claude model has: the long one when its name says, or once it's used.
fn claude_window(model: &str, context: u64) -> u64 {
    if model.contains("[1m]") || model.ends_with("-1m") || context > WINDOW {
        LONG_WINDOW
    } else {
        WINDOW
    }
}

fn parse_claude(text: &str) -> Transcript {
    let mut transcript = Transcript {
        agent: "Claude Code".to_string(),
        ..Transcript::default()
    };
    let mut turns: Vec<Turn> = Vec::new();
    let mut counted = String::new();
    for line in text.lines() {
        let Ok(entry) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        // A subagent's own work, which its Task step stands for.
        if entry["isSidechain"].as_bool() == Some(true) {
            continue;
        }
        let cwd = entry["cwd"].as_str().unwrap_or("");
        match entry["type"].as_str() {
            Some("custom-title") => {
                if let Some(title) = entry["customTitle"].as_str() {
                    transcript.title = title.to_string();
                }
            }
            Some("summary") if transcript.title.is_empty() => {
                transcript.title = entry["summary"].as_str().unwrap_or("").to_string();
            }
            Some("user") if entry["isMeta"].as_bool() != Some(true) => {
                let content = &entry["message"]["content"];
                if let Some(said) = content.as_str() {
                    let said = typed(said);
                    if !said.is_empty() {
                        turns.push(Turn {
                            prompt: said,
                            ..Turn::default()
                        });
                    }
                    continue;
                }
                for block in content.as_array().into_iter().flatten() {
                    match block["type"].as_str() {
                        Some("tool_result") => {
                            let id = block["tool_use_id"].as_str().unwrap_or("");
                            let output = result_text(&block["content"]);
                            let failed = block["is_error"].as_bool() == Some(true);
                            settle(&mut turns, id, output, failed);
                        }
                        Some("text") => {
                            let said = typed(block["text"].as_str().unwrap_or(""));
                            if !said.is_empty() {
                                turns.push(Turn {
                                    prompt: said,
                                    ..Turn::default()
                                });
                            }
                        }
                        _ => {}
                    }
                }
            }
            Some("assistant") => {
                let message = &entry["message"];
                let model = message["model"].as_str().unwrap_or("");
                if !model.is_empty() && !model.starts_with('<') {
                    transcript.usage.model = model.to_string();
                }
                let usage = &message["usage"];
                let id = message["id"].as_str().unwrap_or("");
                if usage.is_object() && id != counted {
                    // One reply is written a block a line, each with the same usage.
                    counted = id.to_string();
                    let field = |key: &str| usage[key].as_u64().unwrap_or(0);
                    let context = field("input_tokens")
                        + field("cache_creation_input_tokens")
                        + field("cache_read_input_tokens");
                    if context > 0 {
                        transcript.usage.context = context;
                        transcript.usage.input += context;
                        transcript.usage.output += field("output_tokens");
                        note_context(&mut turns, context);
                    }
                }
                for block in message["content"].as_array().into_iter().flatten() {
                    let item = match block["type"].as_str() {
                        Some("text") => {
                            let said = block["text"].as_str().unwrap_or("").trim();
                            (!said.is_empty()).then(|| Item::Text(said.to_string()))
                        }
                        Some("thinking") => {
                            let thought = block["thinking"].as_str().unwrap_or("").trim();
                            (!thought.is_empty()).then(|| Item::Thinking(thought.to_string()))
                        }
                        Some("tool_use") => Some(Item::Tool(tool(
                            block["name"].as_str().unwrap_or("Tool"),
                            &block["input"],
                            cwd,
                            block["id"].as_str().unwrap_or(""),
                        ))),
                        _ => None,
                    };
                    if let Some(item) = item {
                        last_turn(&mut turns).items.push(item);
                    }
                }
            }
            _ => {}
        }
    }
    let usage = &mut transcript.usage;
    usage.window = claude_window(&usage.model, usage.context);
    transcript.turns = turns;
    transcript
}

fn parse_codex(text: &str) -> Transcript {
    let mut transcript = Transcript {
        agent: "Codex".to_string(),
        ..Transcript::default()
    };
    let mut turns: Vec<Turn> = Vec::new();
    let mut cwd = String::new();
    for line in text.lines() {
        let Ok(entry) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let payload = &entry["payload"];
        match (entry["type"].as_str(), payload["type"].as_str()) {
            (Some("session_meta"), _) => {
                cwd = payload["cwd"].as_str().unwrap_or("").to_string();
            }
            (Some("turn_context"), _) => {
                if let Some(model) = payload["model"].as_str() {
                    transcript.usage.model = model.to_string();
                }
            }
            (Some("response_item"), Some("message")) => {
                let said: Vec<&str> = payload["content"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|b| b["text"].as_str())
                    .collect();
                let said = said.join("\n");
                match payload["role"].as_str() {
                    Some("user") => {
                        let said = typed(&said);
                        // Codex hands the model its instructions and surroundings as user text.
                        let setup = said.starts_with("<environment_context>")
                            || said.starts_with("<user_instructions>")
                            || said.starts_with("# AGENTS.md");
                        if !said.is_empty() && !setup {
                            turns.push(Turn {
                                prompt: said,
                                ..Turn::default()
                            });
                        }
                    }
                    Some("assistant") if !said.trim().is_empty() => {
                        last_turn(&mut turns)
                            .items
                            .push(Item::Text(said.trim().to_string()));
                    }
                    _ => {}
                }
            }
            (Some("response_item"), Some("reasoning")) => {
                let thought: Vec<&str> = payload["summary"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|b| b["text"].as_str())
                    .collect();
                let thought = thought.join("\n");
                if !thought.trim().is_empty() {
                    last_turn(&mut turns)
                        .items
                        .push(Item::Thinking(thought.trim().to_string()));
                }
            }
            (
                Some("response_item"),
                Some(kind @ ("function_call" | "custom_tool_call" | "local_shell_call")),
            ) => {
                let id = payload["call_id"].as_str().unwrap_or("");
                let name = payload["name"].as_str().unwrap_or(match kind {
                    "local_shell_call" => "local_shell",
                    _ => "tool",
                });
                // A function's arguments come as JSON in a string, a custom tool's as text.
                let input = match kind {
                    "local_shell_call" => payload["action"].clone(),
                    "custom_tool_call" => {
                        serde_json::json!({ "input": payload["input"].as_str().unwrap_or("") })
                    }
                    _ => payload["arguments"]
                        .as_str()
                        .and_then(|a| serde_json::from_str(a).ok())
                        .unwrap_or(Value::Null),
                };
                last_turn(&mut turns)
                    .items
                    .push(Item::Tool(tool(name, &input, &cwd, id)));
            }
            (Some("response_item"), Some("function_call_output" | "custom_tool_call_output")) => {
                let id = payload["call_id"].as_str().unwrap_or("");
                // Shell output comes wrapped with its exit code.
                let raw = &payload["output"];
                let wrapped = raw
                    .as_str()
                    .and_then(|o| serde_json::from_str::<Value>(o).ok())
                    .filter(Value::is_object);
                let (output, failed) = match wrapped {
                    Some(o) => (
                        o["output"].as_str().unwrap_or("").to_string(),
                        o["metadata"]["exit_code"].as_i64().is_some_and(|c| c != 0),
                    ),
                    None => (result_text(raw), false),
                };
                settle(&mut turns, id, output, failed);
            }
            (Some("event_msg"), Some("token_count")) => {
                let info = &payload["info"];
                let last = &info["last_token_usage"];
                let total = &info["total_token_usage"];
                if let Some(context) = last["input_tokens"].as_u64() {
                    transcript.usage.context = context;
                    note_context(&mut turns, context);
                }
                if let Some(input) = total["input_tokens"].as_u64() {
                    transcript.usage.input = input;
                }
                if let Some(output) = total["output_tokens"].as_u64() {
                    transcript.usage.output = output;
                }
                if let Some(window) = info["model_context_window"].as_u64() {
                    transcript.usage.window = window;
                }
            }
            _ => {}
        }
    }
    if transcript.usage.window == 0 {
        transcript.usage.window = WINDOW;
    }
    transcript.turns = turns;
    transcript
}

/// Where a step starts on the page, and how a comment there names it.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Anchor {
    /// Counting from one.
    pub line: usize,
    pub name: String,
}

/// A fence longer than any run of backticks in `text`, so nothing in it can close it.
fn fence(text: &str) -> String {
    let mut longest = 0;
    let mut run = 0;
    for c in text.chars() {
        run = if c == '`' { run + 1 } else { 0 };
        longest = longest.max(run);
    }
    "`".repeat((longest + 1).max(3))
}

/// Text as inline code, whatever backticks it holds.
fn code(text: &str) -> String {
    let mut longest = 0;
    let mut run = 0;
    for c in text.chars() {
        run = if c == '`' { run + 1 } else { 0 };
        longest = longest.max(run);
    }
    let ticks = "`".repeat(longest + 1);
    if longest == 0 {
        format!("{ticks}{text}{ticks}")
    } else {
        format!("{ticks} {text} {ticks}")
    }
}

/// Builds the page, and where each step on it starts.
struct Page {
    text: String,
    line: usize,
    anchors: Vec<Anchor>,
    /// Steps in a row make one tight list, so they read as rows, not paragraphs.
    listing: bool,
}

impl Page {
    fn push(&mut self, block: &str) {
        self.end_list();
        self.text.push_str(block);
        self.text.push_str("\n\n");
        self.line += block.lines().count().max(1) + 1;
    }

    /// One row of a list of steps.
    fn item(&mut self, line: &str) {
        self.text.push_str("- ");
        self.text.push_str(line);
        self.text.push('\n');
        self.line += 1;
        self.listing = true;
    }

    fn end_list(&mut self) {
        if std::mem::take(&mut self.listing) {
            self.text.push('\n');
            self.line += 1;
        }
    }

    /// Marks the next block as the start of a step.
    fn anchor(&mut self, name: String) {
        self.anchors.push(Anchor {
            line: self.line,
            name,
        });
    }
}

/// The transcript as a Markdown page, and its steps' places on it.
pub fn page(transcript: &Transcript) -> (String, Vec<Anchor>) {
    let mut page = Page {
        text: String::new(),
        line: 1,
        anchors: Vec::new(),
        listing: false,
    };
    let title = if transcript.title.trim().is_empty() {
        let first = transcript.turns.first().map_or("", |t| t.prompt.as_str());
        let named = one_line(first);
        if named.is_empty() {
            format!("{} transcript", transcript.agent)
        } else {
            named
        }
    } else {
        transcript.title.trim().to_string()
    };
    page.anchor("the title".to_string());
    page.push(&format!("# {}", title.replace('\n', " ")));
    let steps: usize = transcript
        .turns
        .iter()
        .map(|t| {
            t.items
                .iter()
                .filter(|i| matches!(i, Item::Tool(_)))
                .count()
        })
        .sum();
    let turns = transcript.turns.len();
    let mut about = vec![transcript.agent.clone()];
    if !transcript.usage.model.is_empty() {
        about.push(model_name(&transcript.usage.model));
    }
    let plural = |n: usize, word: &str| format!("{n} {word}{}", if n == 1 { "" } else { "s" });
    about.push(format!(
        "{}, {}",
        plural(turns, "turn"),
        plural(steps, "step")
    ));
    if let Some(percent) = transcript.usage.percent() {
        about.push(format!(
            "context {} of {} ({percent}%)",
            tokens(transcript.usage.context),
            tokens(transcript.usage.window)
        ));
    }
    about.push("read only".to_string());
    page.push(&format!("*{}*", about.join(" · ")));
    for (number, turn) in transcript.turns.iter().enumerate() {
        let number = number + 1;
        let context = turn.context.map_or(String::new(), |(start, end)| {
            format!(" · context {} → {}", tokens(start), tokens(end))
        });
        page.anchor(format!("turn {number}"));
        page.push(&format!("## Turn {number}{context}"));
        if !turn.prompt.is_empty() {
            page.anchor(format!("turn {number}, your prompt"));
            let quoted: Vec<String> = turn.prompt.lines().map(|l| format!("> {l}")).collect();
            page.push(&format!("> [!NOTE] You\n{}", quoted.join("\n")));
        }
        let mut step = 0;
        for item in &turn.items {
            match item {
                Item::Text(said) => {
                    let after = if step == 0 {
                        String::new()
                    } else {
                        format!(" after step {step}")
                    };
                    page.anchor(format!("turn {number}, its reply{after}"));
                    page.push(said);
                }
                Item::Thinking(thought) => {
                    page.anchor(format!("turn {number}, its thinking"));
                    let first = one_line(thought);
                    page.push(&format!("*Thinking:* {}", first.replace('*', "\\*")));
                }
                Item::Tool(tool) => {
                    step += 1;
                    let name = format!(
                        "turn {number}, step {step} ({} {})",
                        tool.name, tool.summary
                    );
                    page.anchor(name);
                    let mut line = tool.name.clone();
                    if !tool.summary.is_empty() {
                        line.push(' ');
                        line.push_str(&code(&tool.summary));
                    }
                    if tool.failed {
                        // Marked, so the step that went wrong is the one the eye finds.
                        line.push_str(" · ==failed==");
                    } else if !tool.note.is_empty() {
                        line.push_str(&format!(" · {}", tool.note));
                    }
                    page.item(&line);
                    let output = tool.output.trim_end();
                    if (tool.show_output || tool.failed) && !output.trim().is_empty() {
                        let shown = shown_output(output, tool.failed);
                        let fence = fence(&shown);
                        page.push(&format!("{fence}\n{shown}\n{fence}"));
                    }
                }
            }
        }
    }
    page.end_list();
    (page.text, page.anchors)
}

/// What of a step's output the page shows: its last lines, and from a failure the lines
/// that say what failed too, with what's left out said where it was.
fn shown_output(output: &str, failed: bool) -> String {
    let lines: Vec<&str> = output.lines().collect();
    if lines.len() <= SHOWN_LINES {
        return lines.join("\n");
    }
    let mut keep = vec![false; lines.len()];
    for flag in keep.iter_mut().rev().take(SHOWN_LINES / 2) {
        *flag = true;
    }
    if failed {
        let telling = |line: &str| {
            let lower = line.to_lowercase();
            ["error", "fail", "panic", "exception"]
                .iter()
                .any(|word| lower.contains(word))
        };
        let mut kept = 0;
        for index in 0..lines.len() {
            if kept < SHOWN_LINES && telling(lines[index]) {
                keep[index] = true;
                if let Some(next) = keep.get_mut(index + 1) {
                    *next = true;
                }
                kept += 1;
            }
        }
    } else {
        for flag in keep.iter_mut().rev().take(SHOWN_LINES) {
            *flag = true;
        }
    }
    let mut shown = Vec::new();
    let mut gap = 0;
    for (line, kept) in lines.iter().zip(keep) {
        if kept {
            if gap > 0 {
                shown.push(format!("… {gap} lines left out"));
                gap = 0;
            }
            shown.push((*line).to_string());
        } else {
            gap += 1;
        }
    }
    shown.join("\n")
}

/// A Claude model as people say it: `claude-opus-5-5` is Opus 5.5. Others stay as named.
fn model_name(model: &str) -> String {
    let Some(rest) = model.strip_prefix("claude-") else {
        return model.to_string();
    };
    let mut parts = rest.split('-');
    let Some(family) = parts
        .next()
        .filter(|f| !f.is_empty() && f.chars().all(|c| c.is_ascii_alphabetic()))
    else {
        return model.to_string();
    };
    let version: Vec<&str> = parts
        .take_while(|p| p.len() <= 2 && p.chars().all(|c| c.is_ascii_digit()))
        .collect();
    let mut named: String = family[..1].to_uppercase() + &family[1..];
    if !version.is_empty() {
        named.push(' ');
        named.push_str(&version.join("."));
    }
    named
}

/// The name of the step a line of the page is in.
pub fn step_at(anchors: &[Anchor], line: usize) -> Option<&str> {
    anchors
        .iter()
        .take_while(|a| a.line <= line)
        .last()
        .map(|a| a.name.as_str())
}

/// The file beside a page that keeps its steps' places.
pub fn anchors_file(page: &Path) -> PathBuf {
    page.with_extension("steps.json")
}

/// The steps of a page written by `write_page`, if it is one.
pub fn anchors_of(page: &Path) -> Vec<Anchor> {
    std::fs::read(anchors_file(page))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

/// Writes the page of a transcript into the project, and where its steps are beside it.
pub fn write_page(root: &Path, name: &str, transcript: &Transcript) -> std::io::Result<PathBuf> {
    let folder = root.join(PAGES);
    std::fs::create_dir_all(&folder)?;
    // Like threads and plans, these are Cellix's notes, not the project's.
    let ignore = folder.join(".gitignore");
    if !ignore.exists() {
        std::fs::write(&ignore, "*\n")?;
    }
    let path = folder.join(format!("{name}.md"));
    let (text, anchors) = page(transcript);
    // Unchanged, it's left alone, so an open page isn't read again for nothing.
    if std::fs::read_to_string(&path).ok().as_deref() != Some(text.as_str()) {
        std::fs::write(&path, text)?;
        std::fs::write(anchors_file(&path), serde_json::to_vec(&anchors)?)?;
    }
    Ok(path)
}

/// Whether a file is one of these pages.
pub fn is_page(root: &Path, path: &Path) -> bool {
    path.starts_with(root.join(PAGES)) && path.extension().is_some_and(|e| e == "md")
}

/// The page's file name for an agent's session.
pub fn page_name(agent: &str, session: &str) -> String {
    let agent: String = agent
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .collect::<String>()
        .to_lowercase();
    let session: String = session
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .take(36)
        .collect();
    format!("{agent}-{session}")
}

/// What a page's tab says: whose transcript it is, from its file's name.
pub fn page_title(page: &Path) -> String {
    let stem = page.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    let agent = match stem.split('-').next().unwrap_or("") {
        "claudecode" => "Claude Code",
        "codex" => "Codex",
        "geminicli" => "Gemini CLI",
        _ => "Agent",
    };
    format!("Transcript · {agent}")
}

/// Where an agent keeps a session's transcript, when its hooks didn't say: Claude Code under
/// its projects folder, Codex in its dated sessions folders, each named by the session's id.
pub fn find(agent: &str, session: &str) -> Option<PathBuf> {
    if session.is_empty() {
        return None;
    }
    let var = |name: &str| std::env::var_os(name).map(PathBuf::from);
    let home = var("HOME").or_else(|| var("USERPROFILE"))?;
    match agent {
        "Claude Code" => {
            let base = var("CLAUDE_CONFIG_DIR").unwrap_or_else(|| home.join(".claude"));
            let wanted = format!("{session}.jsonl");
            std::fs::read_dir(base.join("projects"))
                .ok()?
                .flatten()
                .map(|project| project.path().join(&wanted))
                .find(|path| path.is_file())
        }
        "Codex" => {
            let base = var("CODEX_HOME").unwrap_or_else(|| home.join(".codex"));
            let ending = format!("{session}.jsonl");
            // sessions/YYYY/MM/DD/rollout-<time>-<id>.jsonl, newest days first.
            let mut found = None;
            walk(&base.join("sessions"), 4, &mut |path| {
                let named = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if named.starts_with("rollout-") && named.ends_with(&ending) {
                    found = Some(path.to_path_buf());
                    true
                } else {
                    false
                }
            });
            found
        }
        _ => None,
    }
}

/// Looks through a folder `depth` deep, newest names first, until `found` says stop.
fn walk(folder: &Path, depth: usize, found: &mut dyn FnMut(&Path) -> bool) -> bool {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return false;
    };
    let mut paths: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
    paths.sort();
    paths.reverse();
    for path in paths {
        if path.is_dir() {
            if depth > 0 && walk(&path, depth - 1, found) {
                return true;
            }
        } else if found(&path) {
            return true;
        }
    }
    false
}

/// Follows a transcript as it grows, reading only what's new, for the context on a tab.
#[derive(Debug, Default)]
pub struct Meter {
    pub path: PathBuf,
    read: u64,
    /// The end of a line cut off by the last read.
    partial: String,
    codex: Option<bool>,
    counted: String,
    pub usage: Usage,
}

impl Meter {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            ..Self::default()
        }
    }

    /// Reads what was added since last time. Cheap when nothing was.
    pub fn refresh(&mut self) {
        let Ok(mut file) = std::fs::File::open(&self.path) else {
            return;
        };
        let length = file.metadata().map_or(0, |m| m.len());
        if length < self.read {
            // Rewritten from the start.
            *self = Self::new(std::mem::take(&mut self.path));
        }
        if length == self.read || file.seek(SeekFrom::Start(self.read)).is_err() {
            return;
        }
        let mut bytes = Vec::new();
        if file.read_to_end(&mut bytes).is_err() {
            return;
        }
        self.read += bytes.len() as u64;
        let mut text = std::mem::take(&mut self.partial);
        text.push_str(&String::from_utf8_lossy(&bytes));
        let complete = text.rfind('\n').map_or(0, |end| end + 1);
        self.partial = text[complete..].to_string();
        for line in text[..complete].lines() {
            self.take(line);
        }
        if self.codex != Some(true) {
            self.usage.window = claude_window(&self.usage.model, self.usage.context);
        } else if self.usage.window == 0 {
            self.usage.window = WINDOW;
        }
    }

    fn take(&mut self, line: &str) {
        // Most lines say nothing about tokens, and aren't worth reading whole.
        let codex = *self
            .codex
            .get_or_insert_with(|| line.contains("\"payload\""));
        if codex {
            if !line.contains("token_count") && !line.contains("turn_context") {
                return;
            }
            let Ok(entry) = serde_json::from_str::<Value>(line) else {
                return;
            };
            let payload = &entry["payload"];
            if let Some(model) = payload["model"].as_str() {
                self.usage.model = model.to_string();
            }
            let info = &payload["info"];
            if let Some(context) = info["last_token_usage"]["input_tokens"].as_u64() {
                self.usage.context = context;
            }
            if let Some(input) = info["total_token_usage"]["input_tokens"].as_u64() {
                self.usage.input = input;
            }
            if let Some(output) = info["total_token_usage"]["output_tokens"].as_u64() {
                self.usage.output = output;
            }
            if let Some(window) = info["model_context_window"].as_u64() {
                self.usage.window = window;
            }
            return;
        }
        if !line.contains("\"usage\"") || line.contains("\"isSidechain\":true") {
            return;
        }
        let Ok(entry) = serde_json::from_str::<Value>(line) else {
            return;
        };
        let message = &entry["message"];
        let id = message["id"].as_str().unwrap_or("");
        if entry["type"].as_str() != Some("assistant") || id == self.counted {
            return;
        }
        self.counted = id.to_string();
        let model = message["model"].as_str().unwrap_or("");
        if !model.is_empty() && !model.starts_with('<') {
            self.usage.model = model.to_string();
        }
        let usage = &message["usage"];
        let field = |key: &str| usage[key].as_u64().unwrap_or(0);
        let context = field("input_tokens")
            + field("cache_creation_input_tokens")
            + field("cache_read_input_tokens");
        if context > 0 {
            self.usage.context = context;
            self.usage.input += context;
            self.usage.output += field("output_tokens");
        }
    }
}

/// What's known of the transcripts open as pages: each page's source, and how long it was
/// when the page was last written.
pub type Sources = HashMap<PathBuf, (PathBuf, u64)>;

#[cfg(test)]
pub mod tests {
    use super::*;

    /// A short Claude Code session: a prompt, a read, an edit, a failing command and a
    /// reply, with what each reply cost.
    pub const CLAUDE: &str = r#"{"type":"custom-title","customTitle":"Add a refund route","sessionId":"abc-123"}
{"type":"user","cwd":"/work/shop","message":{"role":"user","content":"<system-reminder>\nNot typed by anyone.\n</system-reminder>\nAdd a refund route, and a test."}}
{"type":"assistant","cwd":"/work/shop","message":{"id":"m1","model":"claude-opus-5-5","role":"assistant","content":[{"type":"text","text":"I'll look at how orders are routed first."}],"usage":{"input_tokens":2,"cache_creation_input_tokens":1000,"cache_read_input_tokens":40000,"output_tokens":50}}}
{"type":"assistant","cwd":"/work/shop","message":{"id":"m1","model":"claude-opus-5-5","role":"assistant","content":[{"type":"tool_use","id":"t1","name":"Read","input":{"file_path":"/work/shop/src/routes.rs"}}],"usage":{"input_tokens":2,"cache_creation_input_tokens":1000,"cache_read_input_tokens":40000,"output_tokens":50}}}
{"type":"user","cwd":"/work/shop","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","content":"use axum::Router;\n\npub fn routes() {}\n"}]}}
{"type":"assistant","cwd":"/work/shop","message":{"id":"m2","model":"claude-opus-5-5","role":"assistant","content":[{"type":"tool_use","id":"t2","name":"Edit","input":{"file_path":"/work/shop/src/orders.rs","old_string":"a\nb","new_string":"a\nb\nc\nd"}}],"usage":{"input_tokens":2,"cache_creation_input_tokens":2000,"cache_read_input_tokens":60000,"output_tokens":80}}}
{"type":"user","cwd":"/work/shop","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t2","content":"The file was updated."}]}}
{"type":"assistant","cwd":"/work/shop","message":{"id":"m3","model":"claude-opus-5-5","role":"assistant","content":[{"type":"tool_use","id":"t3","name":"Bash","input":{"command":"cargo test orders"}}],"usage":{"input_tokens":2,"cache_creation_input_tokens":3000,"cache_read_input_tokens":80000,"output_tokens":40}}}
{"type":"user","cwd":"/work/shop","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t3","is_error":true,"content":"running 15 tests\ntest orders::refund_full ... FAILED\nthread 'refund_full' panicked: left: Paid, right: Refunded"}]}}
{"type":"assistant","cwd":"/work/shop","message":{"id":"m4","model":"claude-opus-5-5","role":"assistant","content":[{"type":"text","text":"The refund never changed the state. Fixed, and the test passes."}],"usage":{"input_tokens":2,"cache_creation_input_tokens":4000,"cache_read_input_tokens":84000,"output_tokens":60}}}
{"type":"user","isSidechain":true,"message":{"role":"user","content":"a subagent's own prompt"}}
{"type":"user","cwd":"/work/shop","message":{"role":"user","content":"Also reject unpaid orders."}}
"#;

    #[test]
    fn a_claude_transcript_reads_as_turns_of_steps() {
        let t = parse(CLAUDE);
        assert_eq!(t.title, "Add a refund route");
        assert_eq!(t.turns.len(), 2, "{:#?}", t.turns);
        let first = &t.turns[0];
        assert_eq!(first.prompt, "Add a refund route, and a test.");
        let tools: Vec<&Tool> = first
            .items
            .iter()
            .filter_map(|i| match i {
                Item::Tool(tool) => Some(tool),
                _ => None,
            })
            .collect();
        assert_eq!(tools.len(), 3);
        assert_eq!(tools[0].summary, "src/routes.rs");
        assert_eq!(tools[0].note, "3 lines");
        assert_eq!(tools[1].note, "+4 −2");
        assert!(tools[2].failed);
        assert_eq!(first.context, Some((41_002, 88_002)));
        assert_eq!(t.usage.context, 88_002);
        assert_eq!(t.usage.window, 200_000);
        assert_eq!(t.usage.percent(), Some(44));
        assert_eq!(t.turns[1].prompt, "Also reject unpaid orders.");
    }

    #[test]
    fn the_page_names_each_step_and_keeps_output_in_code() {
        let (text, anchors) = page(&parse(CLAUDE));
        assert!(text.starts_with("# Add a refund route"), "{text}");
        assert!(
            text.contains("- Bash `cargo test orders` · ==failed=="),
            "{text}"
        );
        assert!(text.contains("```\nrunning 15 tests"), "{text}");
        assert!(
            text.contains("- Read `src/routes.rs` · 3 lines\n- Edit"),
            "steps in a row are rows: {text}"
        );
        // A read file's contents aren't repeated: the step says what was read.
        assert!(!text.contains("use axum::Router"), "{text}");
        let lines: Vec<&str> = text.lines().collect();
        let bash = lines.iter().position(|l| l.starts_with("- Bash")).unwrap() + 1;
        assert_eq!(
            step_at(&anchors, bash),
            Some("turn 1, step 3 (Bash cargo test orders)")
        );
        // Output under a step belongs to it.
        assert_eq!(
            step_at(&anchors, bash + 2),
            Some("turn 1, step 3 (Bash cargo test orders)")
        );
        let prompt = lines
            .iter()
            .position(|l| l.contains("Also reject"))
            .unwrap()
            + 1;
        assert_eq!(step_at(&anchors, prompt), Some("turn 2, your prompt"));
    }

    #[test]
    fn a_failures_telling_lines_are_kept_however_long_its_output() {
        let mut output: Vec<String> = (0..40).map(|n| format!("test case_{n} ... ok")).collect();
        output.insert(5, "error[E0308]: mismatched types".to_string());
        output.insert(6, "  --> src/orders.rs:12:5".to_string());
        let output = output.join("\n");
        let shown = shown_output(&output, true);
        assert!(shown.contains("error[E0308]"), "{shown}");
        assert!(
            shown.contains("--> src/orders.rs:12:5"),
            "the line after it: {shown}"
        );
        assert!(shown.contains("lines left out"), "{shown}");
        assert!(shown.ends_with("test case_39 ... ok"), "{shown}");
        assert!(shown.lines().count() < 20, "{shown}");
    }

    #[test]
    fn models_read_as_people_say_them() {
        assert_eq!(model_name("claude-opus-5-5"), "Opus 5.5");
        assert_eq!(model_name("claude-sonnet-4-5-20250929"), "Sonnet 4.5");
        assert_eq!(model_name("gpt-5-codex"), "gpt-5-codex");
    }

    #[test]
    fn backticks_in_output_cannot_close_its_block() {
        let output = "```\n# not a heading\n```";
        assert_eq!(fence(output), "````");
        assert_eq!(code("a`b"), "`` a`b ``");
    }

    #[test]
    fn the_meter_reads_only_what_was_added() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.jsonl");
        let half = CLAUDE.lines().take(5).collect::<Vec<_>>().join("\n") + "\n";
        std::fs::write(&path, &half).unwrap();
        let mut meter = Meter::new(path.clone());
        meter.refresh();
        assert_eq!(meter.usage.context, 41_002);
        std::fs::write(&path, CLAUDE).unwrap();
        meter.refresh();
        assert_eq!(meter.usage.context, 88_002);
        assert_eq!(meter.usage.input, 41_002 + 62_002 + 83_002 + 88_002);
        assert_eq!(meter.usage, parse(CLAUDE).usage);
    }

    #[test]
    fn a_codex_rollout_reads_too() {
        let log = r#"{"timestamp":"t","type":"session_meta","payload":{"id":"c-1","cwd":"/work/shop"}}
{"timestamp":"t","type":"turn_context","payload":{"model":"gpt-5-codex"}}
{"timestamp":"t","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"<environment_context>cwd</environment_context>"}]}}
{"timestamp":"t","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"Run the tests"}]}}
{"timestamp":"t","type":"response_item","payload":{"type":"function_call","name":"shell","arguments":"{\"command\":[\"bash\",\"-lc\",\"cargo test\"]}","call_id":"c1"}}
{"timestamp":"t","type":"response_item","payload":{"type":"function_call_output","call_id":"c1","output":"{\"output\":\"1 failed\",\"metadata\":{\"exit_code\":101}}"}}
{"timestamp":"t","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":9000,"output_tokens":300},"last_token_usage":{"input_tokens":7000},"model_context_window":272000}}}
{"timestamp":"t","type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"One test fails."}]}}
"#;
        let t = parse(log);
        assert_eq!(t.agent, "Codex");
        assert_eq!(t.turns.len(), 1);
        assert_eq!(t.turns[0].prompt, "Run the tests");
        let Item::Tool(shell) = &t.turns[0].items[0] else {
            panic!("{:?}", t.turns[0].items);
        };
        assert_eq!(shell.summary, "bash -lc cargo test");
        assert!(shell.failed);
        assert_eq!(t.usage.window, 272_000);
        assert_eq!(t.usage.percent(), Some(2));
    }

    #[test]
    fn counts_read_as_people_say_them() {
        assert_eq!(tokens(940), "940");
        assert_eq!(tokens(41_002), "41k");
        assert_eq!(tokens(3_120_000), "3.1M");
        assert_eq!(tokens(1_000_000), "1M");
    }
}
