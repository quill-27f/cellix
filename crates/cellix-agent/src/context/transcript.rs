//! What one Claude Code session worked from, from its transcript: the instruction files it
//! loaded as it says it loaded them, the listings it was given, what it read and searched, and
//! how full its context was at its last reply. The transcript is data; nothing in it is run.

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::{Scope, claude_project_dir, estimate_tokens};

/// A transcript line longer than this is skipped unread: a pasted image or a huge tool output.
const LONGEST_LINE: usize = 8 * 1024 * 1024;

/// Where Claude Code keeps session `session`'s transcript for a project.
pub fn transcript_path(claude: &Path, project: &Path, session: &str) -> PathBuf {
    claude_project_dir(claude, project).join(format!("{session}.jsonl"))
}

/// An instruction file as the agent got it.
#[derive(Clone, Debug, PartialEq)]
pub struct Loaded {
    pub path: PathBuf,
    pub scope: Scope,
    pub text: String,
}

/// A file the session read, searched in or changed.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FileRead {
    pub path: PathBuf,
    pub reads: usize,
    pub searches: usize,
    pub edits: usize,
    /// About how many tokens what it read of this file came to.
    pub tokens: usize,
    /// The turns it read the file in, 1-based, each once.
    pub turns: Vec<usize>,
    /// When it last read the file, in seconds since 1970, to tell whether it changed since.
    pub last_read: Option<u64>,
    /// When it last changed the file itself.
    pub last_edit: Option<u64>,
}

/// A search the session ran: Grep or Glob.
#[derive(Clone, Debug, PartialEq)]
pub struct Search {
    pub tool: String,
    pub pattern: String,
    pub path: Option<String>,
    pub tokens: usize,
    pub turn: usize,
}

/// What fills the context, in tokens. Everything but `total` is estimated from the text.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Breakdown {
    pub system: usize,
    pub tools: usize,
    pub instructions: usize,
    /// The skills and subagents it was told it could use.
    pub listings: usize,
    pub mcp: usize,
    pub files: usize,
    pub output: usize,
    pub messages: usize,
    /// The last reply's real count: input, cache written and cache read.
    pub total: usize,
}

impl Breakdown {
    /// Everything there before the first message.
    pub fn before(&self) -> usize {
        self.system + self.tools + self.instructions + self.listings + self.mcp
    }

    pub fn conversation(&self) -> usize {
        self.files + self.output + self.messages
    }
}

#[derive(Clone, Debug, Default)]
pub struct Session {
    pub loaded: Vec<Loaded>,
    /// The skills listing exactly as the agent was given it.
    pub skills: String,
    /// One line per subagent it was told of.
    pub subagents: Vec<String>,
    /// MCP servers that gave it instructions, and those instructions.
    pub mcp: Vec<(String, String)>,
    /// Skills it called on, by name, each once.
    pub invoked: Vec<String>,
    pub files: Vec<FileRead>,
    pub searches: Vec<Search>,
    pub turns: usize,
    pub model: Option<String>,
    pub breakdown: Breakdown,
    /// Times the context was compacted.
    pub compactions: usize,
}

impl Session {
    /// The model's context window: a million for a `[1m]` model, else 200k.
    pub fn window(&self) -> usize {
        // As the transcript page reads it: the long window when the model's name says so, or
        // once the context has gone past the short one.
        let long = self
            .model
            .as_ref()
            .is_some_and(|m| m.contains("[1m]") || m.ends_with("-1m"));
        if long || self.breakdown.total > 200_000 {
            1_000_000
        } else {
            200_000
        }
    }

    /// Files it read that have changed on disk since, most recent read first.
    pub fn stale(&self) -> Vec<&FileRead> {
        self.files
            .iter()
            .filter(|f| {
                let Some(read) = f.last_read else {
                    return false;
                };
                let own = f.last_edit.is_some_and(|e| e >= read);
                let modified = std::fs::metadata(&f.path).and_then(|m| m.modified()).ok();
                let modified = modified.and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok());
                !own && modified.is_some_and(|m| m.as_secs() > read + 1)
            })
            .collect()
    }
}

/// Seconds since 1970 for an RFC 3339 time in UTC, as transcripts write them.
pub(crate) fn unix_seconds(stamp: &str) -> Option<u64> {
    let (date, time) = stamp.split_once('T')?;
    let mut d = date.split('-').map(|p| p.parse::<i64>().ok());
    let (y, m, day) = (d.next()??, d.next()??, d.next()??);
    let time = time.trim_end_matches('Z');
    let time = time.split(['+', '-']).next()?;
    let mut t = time.split(':');
    let h: i64 = t.next()?.parse().ok()?;
    let min: i64 = t.next()?.parse().ok()?;
    let s: f64 = t.next().unwrap_or("0").parse().ok()?;
    // Days from the civil date, after Howard Hinnant.
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    u64::try_from(days * 86_400 + h * 3600 + min * 60 + s as i64).ok()
}

fn text_of(content: &Value) -> String {
    match content {
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter_map(|p| p["text"].as_str().or_else(|| p.as_str()))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

/// Reads a transcript. A line that isn't JSON, or a kind it doesn't know, is passed over, so a
/// newer Claude Code's transcript still reads as far as it can.
pub fn read_session(path: &Path) -> std::io::Result<Session> {
    let reader = BufReader::new(File::open(path)?);
    let mut session = Session::default();
    let mut index: HashMap<PathBuf, usize> = HashMap::new();
    // Tool calls waiting for their result: id to (tool, file or search).
    let mut pending: HashMap<String, (String, Option<usize>, Option<usize>)> = HashMap::new();
    let mut since_compact_files = 0;
    let mut since_compact_output = 0;
    let mut usage_total = 0;
    for line in reader.split(b'\n') {
        let Ok(line) = line else { break };
        if line.len() > LONGEST_LINE {
            continue;
        }
        let Ok(record) = serde_json::from_slice::<Value>(&line) else {
            continue;
        };
        if record["isSidechain"].as_bool() == Some(true) {
            continue;
        }
        let when = record["timestamp"].as_str().and_then(unix_seconds);
        match record["type"].as_str() {
            Some("attachment") => attachment(&record["attachment"], &mut session),
            Some("api-request-shape") => {
                let shape = &record["shape"];
                session.breakdown.system = estimate_tokens(&text_of(&shape["system"]));
                session.breakdown.tools = estimate_tokens(&shape["tools"].to_string());
            }
            Some("system") if record["subtype"] == "compact_boundary" => {
                session.compactions += 1;
                since_compact_files = 0;
                since_compact_output = 0;
            }
            Some("assistant") => {
                let message = &record["message"];
                if let Some(model) = message["model"].as_str().filter(|m| !m.starts_with('<')) {
                    session.model = Some(model.to_string());
                }
                let usage = &message["usage"];
                let total: u64 = [
                    "input_tokens",
                    "cache_creation_input_tokens",
                    "cache_read_input_tokens",
                ]
                .iter()
                .filter_map(|k| usage[*k].as_u64())
                .sum();
                if total > 0 {
                    usage_total = total as usize;
                }
                for block in message["content"].as_array().into_iter().flatten() {
                    if block["type"] == "tool_use" {
                        let id = block["id"].as_str().unwrap_or("").to_string();
                        let call = tool_call(block, &mut session, &mut index, when);
                        pending.insert(id, call);
                    }
                }
            }
            Some("user") => {
                let content = &record["message"]["content"];
                let prompt = match content {
                    Value::String(_) => true,
                    Value::Array(parts) => parts.iter().any(|p| p["type"] == "text"),
                    _ => false,
                };
                if prompt && record["isMeta"].as_bool() != Some(true) {
                    session.turns += 1;
                }
                for part in content.as_array().into_iter().flatten() {
                    if part["type"] != "tool_result" {
                        continue;
                    }
                    let tokens = estimate_tokens(&text_of(&part["content"]));
                    let id = part["tool_use_id"].as_str().unwrap_or("");
                    match pending.remove(id) {
                        Some((tool, Some(file), _)) if tool == "Read" => {
                            session.files[file].tokens += tokens;
                            since_compact_files += tokens;
                        }
                        Some((_, _, Some(search))) => {
                            session.searches[search].tokens += tokens;
                            since_compact_output += tokens;
                        }
                        _ => since_compact_output += tokens,
                    }
                }
            }
            _ => {}
        }
    }
    let b = &mut session.breakdown;
    b.instructions = session
        .loaded
        .iter()
        .map(|l| estimate_tokens(&l.text))
        .sum();
    b.listings = estimate_tokens(&session.skills)
        + session
            .subagents
            .iter()
            .map(|s| estimate_tokens(s))
            .sum::<usize>();
    b.mcp = session.mcp.iter().map(|(_, t)| estimate_tokens(t)).sum();
    b.total = usage_total;
    b.files = since_compact_files;
    b.output = since_compact_output;
    let known = b.before() + b.files + b.output;
    b.messages = usage_total.saturating_sub(known);
    session
        .files
        .sort_by(|a, b| b.tokens.cmp(&a.tokens).then(a.path.cmp(&b.path)));
    Ok(session)
}

fn attachment(a: &Value, session: &mut Session) {
    match a["type"].as_str() {
        Some("instructions") => {
            for file in a["files"].as_array().into_iter().flatten() {
                let Some(path) = file["path"].as_str() else {
                    continue;
                };
                let loaded = Loaded {
                    path: PathBuf::from(path),
                    scope: Scope::from_claude(file["type"].as_str().unwrap_or("")),
                    text: file["content"].as_str().unwrap_or("").to_string(),
                };
                match session.loaded.iter_mut().find(|l| l.path == loaded.path) {
                    Some(known) => *known = loaded,
                    None => session.loaded.push(loaded),
                }
            }
        }
        Some("skill_listing") => session.skills = a["content"].as_str().unwrap_or("").to_string(),
        Some("agent_listing_delta") => {
            for line in a["addedLines"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
            {
                if !session.subagents.iter().any(|s| s == line) {
                    session.subagents.push(line.to_string());
                }
            }
        }
        Some("mcp_instructions_delta") => {
            let names = a["addedNames"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str);
            let blocks = a["addedBlocks"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str);
            for (name, block) in names.zip(blocks) {
                session.mcp.retain(|(n, _)| n != name);
                session.mcp.push((name.to_string(), block.to_string()));
            }
        }
        Some("invoked_skills") => {
            for skill in a["skills"].as_array().into_iter().flatten() {
                if let Some(name) = skill["name"].as_str()
                    && !session.invoked.iter().any(|n| n == name)
                {
                    session.invoked.push(name.to_string());
                }
            }
        }
        _ => {}
    }
}

/// Notes a tool call: the file it reads or changes, or the search it runs.
fn tool_call(
    block: &Value,
    session: &mut Session,
    index: &mut HashMap<PathBuf, usize>,
    when: Option<u64>,
) -> (String, Option<usize>, Option<usize>) {
    let tool = block["name"].as_str().unwrap_or("").to_string();
    let input = &block["input"];
    let turn = session.turns.max(1);
    let mut file_at = |path: &str, session: &mut Session| -> usize {
        let path = PathBuf::from(path);
        *index.entry(path.clone()).or_insert_with(|| {
            session.files.push(FileRead {
                path,
                ..FileRead::default()
            });
            session.files.len() - 1
        })
    };
    match tool.as_str() {
        "Read" | "NotebookRead" => {
            let path = input["file_path"]
                .as_str()
                .or_else(|| input["notebook_path"].as_str());
            let Some(path) = path else {
                return (tool, None, None);
            };
            let i = file_at(path, session);
            let file = &mut session.files[i];
            file.reads += 1;
            file.last_read = when.or(file.last_read);
            if !file.turns.contains(&turn) {
                file.turns.push(turn);
            }
            (tool, Some(i), None)
        }
        "Edit" | "Write" | "MultiEdit" | "NotebookEdit" => {
            let path = input["file_path"]
                .as_str()
                .or_else(|| input["notebook_path"].as_str());
            if let Some(path) = path {
                let i = file_at(path, session);
                session.files[i].edits += 1;
                session.files[i].last_edit = when.or(session.files[i].last_edit);
            }
            (tool, None, None)
        }
        "Grep" | "Glob" => {
            let pattern = input["pattern"].as_str().unwrap_or("").to_string();
            let path = input["path"].as_str().map(str::to_string);
            if let Some(p) = &path
                && tool == "Grep"
                && Path::new(p).extension().is_some()
            {
                let i = file_at(p, session);
                session.files[i].searches += 1;
            }
            session.searches.push(Search {
                tool: tool.clone(),
                pattern,
                path,
                tokens: 0,
                turn,
            });
            let at = session.searches.len() - 1;
            (tool, None, Some(at))
        }
        _ => (tool, None, None),
    }
}
