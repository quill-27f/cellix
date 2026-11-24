//! Reviewing a document that lives elsewhere, behind a doc tool Cellix can't reach itself.
//! The person's agent is the bridge: Cellix runs Claude Code in the background, as Ask does,
//! with a fixed instruction and only the doc tool's MCP server's tools allowed, and never
//! speaks that tool's protocol, whose tool names it doesn't know. The agent answers with data
//! and Cellix writes the files: the doc as `.cellix/remote/<slug>/doc.md`, and what it is in
//! `remote.json` beside it. The comments are threads, someone else's under their names.
//!
//! Nothing is ever posted but by a press on that one comment's Post button: a read is told to
//! use the tool's read tools only, and a post carries one comment and asks for its id back.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use cellix_review::threads::{Remote, Reply, next_id};
use cellix_review::{Thread, YOU};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Where a project keeps the docs read from elsewhere, a folder each.
pub const FOLDER: &str = ".cellix/remote";
/// The doc as Markdown, and what it is, in its folder.
pub const DOC: &str = "doc.md";
const META: &str = "remote.json";
/// How long a read or a post may take before it's stopped: a long doc with many comments
/// takes the agent several calls.
pub const TIME_LIMIT: Duration = Duration::from_secs(600);
/// Claude Code's own tools that could change something, refused outright on every run, as
/// Ask refuses them. `--tools ""` already leaves the built-in set out.
const REFUSED: &str = "Bash,Edit,Write,MultiEdit,NotebookEdit,WebFetch,WebSearch,Task";
/// What a card says when the doc tool's sign-in has run out.
pub const SIGN_IN_AGAIN: &str =
    "The doc tool asked to sign in again. Sign in (your security key), then Retry.";

/// What a remote doc is, kept beside it: where it's from, which version was read, when, and
/// through which MCP server.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Meta {
    pub title: String,
    pub link: String,
    pub version: String,
    /// The MCP server it was read through.
    pub server: String,
    /// When it was last read, in seconds since 1970.
    pub synced: u64,
    /// How many comments on it are someone else's, as the strip counts them.
    pub others: usize,
}

/// A comment as the agent reads it from the doc.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Comment {
    pub id: String,
    pub author: String,
    pub quote: String,
    pub text: String,
    pub at: u64,
    pub resolved: bool,
    pub replies: Vec<Answer>,
}

/// A reply under one.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Answer {
    pub id: String,
    pub author: String,
    pub text: String,
    pub at: u64,
}

/// All a read brings back.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Fetched {
    pub title: String,
    pub link: String,
    pub version: String,
    pub markdown: String,
    pub comments: Vec<Comment>,
}

/// The name Claude Code gives a server's tools: `mcp__<server>__<tool>`, with anything but
/// letters, digits, `_` and `-` in the server's name made `_`.
pub fn tool_prefix(server: &str) -> String {
    let name: String = server
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    format!("mcp__{name}__")
}

/// The MCP servers Claude Code knows here, by name: the user's (`mcpServers` in its
/// `.claude.json`, in `CLAUDE_CONFIG_DIR` when that's set and the home folder otherwise),
/// those it keeps for this project there, and the project's own `.mcp.json`. Read only.
pub fn servers(var: &impl Fn(&str) -> Option<String>, root: &Path) -> Vec<String> {
    let file = match var("CLAUDE_CONFIG_DIR").filter(|d| !d.is_empty()) {
        Some(dir) => Some(PathBuf::from(dir).join(".claude.json")),
        None => var("HOME").map(|home| PathBuf::from(home).join(".claude.json")),
    };
    let read = |path: &Path| -> Value {
        fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or(Value::Null)
    };
    let names = |value: &Value| -> Vec<String> {
        value["mcpServers"]
            .as_object()
            .map(|servers| servers.keys().cloned().collect())
            .unwrap_or_default()
    };
    let mut found = Vec::new();
    if let Some(file) = file {
        let config = read(&file);
        found.extend(names(&config));
        let here = root.display().to_string();
        if let Some(projects) = config["projects"].as_object() {
            for (path, project) in projects {
                if Path::new(path) == root || *path == here {
                    found.extend(names(project));
                }
            }
        }
    }
    found.extend(names(&read(&root.join(".mcp.json"))));
    let mut seen = Vec::new();
    for name in found {
        if !seen.contains(&name) {
            seen.push(name);
        }
    }
    seen
}

/// What Claude Code is given to read the doc or post to it: its own tools left out, those that
/// change things refused, every other server's tools refused, and only `server`'s allowed.
pub fn tool_args(server: &str, others: &[String]) -> Vec<String> {
    let mut refused = REFUSED.to_string();
    for other in others.iter().filter(|o| o.as_str() != server) {
        refused.push(',');
        refused.push_str(&tool_prefix(other));
        refused.push('*');
    }
    vec![
        "-p".into(),
        "--output-format".into(),
        "stream-json".into(),
        "--verbose".into(),
        "--tools".into(),
        String::new(),
        "--allowedTools".into(),
        format!("{}*", tool_prefix(server)),
        "--disallowedTools".into(),
        refused,
    ]
}

/// What Polish runs with: no tools at all, built in or from any server.
pub fn polish_args() -> Vec<String> {
    vec![
        "-p".into(),
        "--output-format".into(),
        "stream-json".into(),
        "--verbose".into(),
        "--tools".into(),
        String::new(),
        "--strict-mcp-config".into(),
        "--disallowedTools".into(),
        REFUSED.into(),
    ]
}

/// The JSON a read answers with, as the agent is shown it.
const SHAPE: &str = r#"{"title": "the doc's title", "link": "its link", "version": "its version or revision, if the tool says one", "markdown": "the whole document as Markdown", "comments": [{"id": "the comment's id", "author": "who wrote it", "quote": "the words it's on, exactly as they are in the markdown", "text": "what it says", "at": "when, as ISO 8601", "resolved": false, "replies": [{"id": "the reply's id", "author": "who", "text": "what it says", "at": "when"}]}]}"#;

const ERROR: &str = r#"If you can't, answer {"error": "what the tool said"} instead: if it asks to sign in again, say so in its words."#;

/// What the agent is told to read the doc with.
pub fn read_prompt(server: &str, link: &str) -> String {
    format!(
        "You're reading a document for someone reviewing it in Cellix, an editor. Using the \
         tools of the MCP server \"{server}\", read the document at this link, with all its \
         comments and their replies:\n\n{link}\n\nUse only the tools that read. Don't create, \
         change, resolve or delete anything, and post nothing. Your answer is read by a \
         program: answer with one JSON object and nothing else, shaped like this:\n\n{SHAPE}\n\n\
         {ERROR}"
    )
}

/// What the agent is told to post one comment with.
pub fn post_prompt(server: &str, link: &str, quote: &str, text: &str) -> String {
    format!(
        "Using the tools of the MCP server \"{server}\", add this one comment to the document \
         at {link}, anchored on the words quoted, as the person you're working for:\n\n\
         Quoted words: {quote}\n\nComment: {text}\n\nAdd only this comment, once. Change, \
         resolve and delete nothing else. Your answer is read by a program: answer with one \
         JSON object and nothing else, {{\"id\": \"the new comment's id\"}}. {ERROR}"
    )
}

/// What the agent is told to post one reply with.
pub fn reply_prompt(server: &str, link: &str, comment: &str, text: &str) -> String {
    format!(
        "Using the tools of the MCP server \"{server}\", add this one reply to the comment \
         whose id is {comment} on the document at {link}, as the person you're working for:\n\n\
         Reply: {text}\n\nAdd only this reply, once. Change, resolve and delete nothing else. \
         Your answer is read by a program: answer with one JSON object and nothing else, \
         {{\"id\": \"the new reply's id\"}}. {ERROR}"
    )
}

/// What Polish asks of a comment, with the words it's on and the paragraph round them.
pub fn polish_prompt(text: &str, quote: &str, paragraph: &str) -> String {
    format!(
        "Make this review comment short, clear and kind; keep its meaning; answer with the \
         text only.\n\nThe paragraph it's on:\n{paragraph}\n\nThe words it's about: {quote}\n\n\
         The comment:\n{text}"
    )
}

/// The one JSON object in an answer, wherever it stands in it: alone, in a fence, or after
/// a line of words. An `error` it gives is the error.
pub fn answer_json(answer: &str) -> Result<Value, String> {
    let (Some(start), Some(end)) = (answer.find('{'), answer.rfind('}')) else {
        return Err(failure_words(answer.trim()));
    };
    let value: Value = serde_json::from_str(&answer[start..=end])
        .map_err(|_| "The agent's answer wasn't the JSON it was asked for".to_string())?;
    if let Some(error) = value.get("error").filter(|e| !e.is_null()) {
        let said = error
            .as_str()
            .map_or_else(|| error.to_string(), str::to_string);
        return Err(failure_words(&said));
    }
    Ok(value)
}

/// What the agent said went wrong, in a card's words: a sign-in that ran out reads as one.
pub fn failure_words(said: &str) -> String {
    let lower = said.to_lowercase();
    let signs = [
        "sign in",
        "sign-in",
        "signin",
        "log in",
        "login",
        "re-authenticate",
        "reauthenticate",
        "authenticat",
        "security key",
        "token expired",
        "session expired",
        "expired",
        "unauthorized",
        "401",
    ];
    if signs.iter().any(|sign| lower.contains(sign)) {
        SIGN_IN_AGAIN.to_string()
    } else if said.is_empty() {
        "The agent gave no answer".to_string()
    } else {
        said.to_string()
    }
}

fn text_of(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        _ => String::new(),
    }
}

/// When, from seconds since 1970 or an ISO 8601 time; nothing where it's neither.
pub fn time_of(value: &Value) -> u64 {
    match value {
        Value::Number(n) => {
            let n = n.as_u64().unwrap_or(0);
            // Milliseconds, as JavaScript counts.
            if n > 100_000_000_000 { n / 1000 } else { n }
        }
        Value::String(s) => iso_time(s).unwrap_or(0),
        _ => 0,
    }
}

/// Seconds since 1970 from `2026-11-24T09:30:00Z`, with a fraction or an offset or neither.
fn iso_time(text: &str) -> Option<u64> {
    let text = text.trim();
    let number =
        |from: usize, len: usize| -> Option<i64> { text.get(from..from + len)?.parse().ok() };
    let (year, month, day) = (number(0, 4)?, number(5, 2)?, number(8, 2)?);
    let (hour, minute, second) = if text.len() >= 19 {
        (number(11, 2)?, number(14, 2)?, number(17, 2)?)
    } else {
        (0, 0, 0)
    };
    // Days from 1970 by the civil calendar (Howard Hinnant's).
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    let mut seconds = days * 86_400 + hour * 3600 + minute * 60 + second;
    // An offset after the time: what it's ahead of UTC is taken off.
    let rest = text.get(19..).unwrap_or("");
    let rest = rest.trim_start_matches(|c: char| c == '.' || c.is_ascii_digit());
    if let Some(sign) = rest.chars().next().filter(|c| *c == '+' || *c == '-') {
        let offset = &rest[1..];
        let hours: i64 = offset.get(0..2)?.parse().ok()?;
        let minutes: i64 = offset
            .get(2..)
            .map(|m| m.trim_start_matches(':'))
            .and_then(|m| m.get(0..2))
            .and_then(|m| m.parse().ok())
            .unwrap_or(0);
        let ahead = hours * 3600 + minutes * 60;
        seconds += if sign == '+' { -ahead } else { ahead };
    }
    u64::try_from(seconds).ok()
}

/// A read's answer, checked: it must hold the doc's Markdown. A comment with no id is given
/// one of its words, so a second read knows it again.
pub fn fetched(value: &Value, link: &str) -> Result<Fetched, String> {
    let markdown = value["markdown"]
        .as_str()
        .filter(|m| !m.trim().is_empty())
        .ok_or("The agent's answer had no document in it")?;
    let title = value["title"].as_str().unwrap_or("").trim().to_string();
    let title = if title.is_empty() {
        markdown
            .lines()
            .find_map(|line| line.strip_prefix("# "))
            .unwrap_or("Remote doc")
            .trim()
            .to_string()
    } else {
        title
    };
    let given = value["link"].as_str().unwrap_or("").trim();
    let comments = value["comments"]
        .as_array()
        .map(|list| {
            list.iter()
                .filter(|c| !text_of(&c["text"]).trim().is_empty())
                .map(|c| {
                    let replies = c["replies"]
                        .as_array()
                        .map(|list| {
                            list.iter()
                                .filter(|r| !text_of(&r["text"]).trim().is_empty())
                                .map(|r| Answer {
                                    id: id_of(r),
                                    author: author_of(r),
                                    text: text_of(&r["text"]),
                                    at: time_of(&r["at"]),
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    Comment {
                        id: id_of(c),
                        author: author_of(c),
                        quote: text_of(&c["quote"]).trim().to_string(),
                        text: text_of(&c["text"]),
                        at: time_of(&c["at"]),
                        resolved: c["resolved"].as_bool().unwrap_or(false),
                        replies,
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(Fetched {
        title,
        link: if given.is_empty() { link } else { given }.to_string(),
        version: text_of(&value["version"]),
        markdown: markdown.to_string(),
        comments,
    })
}

fn author_of(value: &Value) -> String {
    let author = text_of(&value["author"]).trim().to_string();
    if author.is_empty() || author == YOU {
        "Someone".to_string()
    } else {
        author
    }
}

/// Its id, or one made of its words where the doc gave none.
fn id_of(value: &Value) -> String {
    let id = text_of(&value["id"]);
    if !id.trim().is_empty() {
        return id;
    }
    let words = format!(
        "{}\u{1}{}\u{1}{}",
        text_of(&value["author"]),
        text_of(&value["quote"]),
        text_of(&value["text"])
    );
    format!("cellix-{:016x}", crate::ask::version(&words))
}

/// The id a post answers with.
pub fn posted_id(answer: &str) -> Result<String, String> {
    let value = answer_json(answer)?;
    let id = text_of(&value["id"]);
    if id.trim().is_empty() {
        Err("The agent didn't say the doc's id for it, so it may not have posted".to_string())
    } else {
        Ok(id)
    }
}

/// A folder's name for a link: its words that say the most, in small letters, and a few
/// letters of its hash so two links never share one.
pub fn slug(link: &str) -> String {
    let bare = link
        .trim()
        .split_once("://")
        .map_or(link.trim(), |(_, rest)| rest);
    let mut words = String::new();
    for c in bare.chars() {
        if c.is_ascii_alphanumeric() {
            words.push(c.to_ascii_lowercase());
        } else if !words.ends_with('-') {
            words.push('-');
        }
    }
    let words: String = words.trim_matches('-').chars().rev().take(32).collect();
    let words: String = words.chars().rev().collect();
    let words = words.trim_matches('-');
    let hash = format!("{:016x}", crate::ask::version(link.trim()));
    if words.is_empty() {
        format!("doc-{}", &hash[..8])
    } else {
        format!("{words}-{}", &hash[..6])
    }
}

/// The doc's path in the project, as threads name it.
pub fn doc_file(slug: &str) -> String {
    format!("{FOLDER}/{slug}/{DOC}")
}

/// The slug of a remote doc's path in the project, if it's one.
pub fn slug_of(file: &str) -> Option<&str> {
    let rest = file.strip_prefix(FOLDER)?.strip_prefix('/')?;
    let (slug, doc) = rest.split_once('/')?;
    (doc == DOC && !slug.is_empty()).then_some(slug)
}

pub fn folder(root: &Path, slug: &str) -> PathBuf {
    root.join(FOLDER).join(slug)
}

pub fn load(root: &Path, slug: &str) -> Option<Meta> {
    let text = fs::read_to_string(folder(root, slug).join(META)).ok()?;
    serde_json::from_str(&text).ok()
}

/// Every remote doc the project has, by slug, the first read first.
pub fn load_all(root: &Path) -> Vec<(String, Meta)> {
    let Ok(entries) = fs::read_dir(root.join(FOLDER)) else {
        return Vec::new();
    };
    let mut found: Vec<(String, Meta)> = entries
        .flatten()
        .filter_map(|entry| {
            let slug = entry.file_name().to_string_lossy().into_owned();
            let meta = load(root, &slug)?;
            entry.path().join(DOC).is_file().then_some((slug, meta))
        })
        .collect();
    found.sort_by(|a, b| a.0.cmp(&b.0));
    found
}

/// Writes what was read: the doc as Markdown and what it is beside it. The folder keeps out
/// of git whole: it's someone's company's content, never the project's.
pub fn write(root: &Path, slug: &str, read: &Fetched, meta: &Meta) -> io::Result<()> {
    let folder = folder(root, slug);
    fs::create_dir_all(&folder)?;
    let ignore = root.join(FOLDER).join(".gitignore");
    if !ignore.exists() {
        fs::write(ignore, "*\n")?;
    }
    let text = if read.markdown.ends_with('\n') {
        read.markdown.clone()
    } else {
        format!("{}\n", read.markdown)
    };
    let scratch = folder.join("doc.md.tmp");
    fs::write(&scratch, text)?;
    fs::rename(scratch, folder.join(DOC))?;
    let json = serde_json::to_string_pretty(meta).map_err(io::Error::other)?;
    fs::write(folder.join(META), json)
}

/// Takes what a read brought into the threads on `file`: a comment already here, by its id,
/// has its words, its being resolved and its replies brought up to date, and new replies added
/// by theirs; one not here comes in under its author's name. The reviewer's drafts are left as
/// they are. Then every thread on the doc is found again by its quote, and one whose words
/// have gone says so. Gives how many came in.
pub fn merge(threads: &mut Vec<Thread>, file: &str, read: &Fetched) -> usize {
    let mut added = 0;
    for comment in &read.comments {
        let known = threads
            .iter_mut()
            .find(|t| t.file == file && t.remote.as_ref().is_some_and(|r| r.id == comment.id));
        match known {
            Some(thread) => {
                // Someone's own words can change in the doc; the reviewer's stay as posted.
                if !thread.author.is_empty() {
                    thread.author = comment.author.clone();
                    thread.text = comment.text.clone();
                    if !comment.quote.is_empty() {
                        thread.quote = comment.quote.clone();
                    }
                }
                thread.resolved = comment.resolved;
                for answer in &comment.replies {
                    let had = thread.replies.iter_mut().find(|r| {
                        r.remote
                            .as_ref()
                            .is_some_and(|remote| !remote.id.is_empty() && remote.id == answer.id)
                    });
                    match had {
                        Some(reply) if reply.author != YOU => reply.text = answer.text.clone(),
                        Some(_) => {}
                        None => thread.replies.push(reply_of(answer)),
                    }
                }
            }
            None => {
                added += 1;
                let id = next_id(threads);
                threads.push(Thread {
                    id,
                    file: file.to_string(),
                    line: 1,
                    quote: comment.quote.clone(),
                    text: comment.text.clone(),
                    replies: comment.replies.iter().map(reply_of).collect(),
                    resolved: comment.resolved,
                    at: comment.at,
                    author: comment.author.clone(),
                    remote: Some(Remote::posted(comment.id.clone(), comment.at)),
                    // Nobody here is to answer it, so no agent waits on it.
                    told: Some(comment.replies.len()),
                    ..Thread::default()
                });
            }
        }
    }
    for thread in threads.iter_mut().filter(|t| t.file == file) {
        // Its words were in the doc when it was made, so their going is news.
        if !thread.quote.trim().is_empty() {
            thread.placed = true;
        }
        thread.follow(&read.markdown);
    }
    added
}

fn reply_of(answer: &Answer) -> Reply {
    Reply {
        author: answer.author.clone(),
        text: answer.text.clone(),
        at: answer.at,
        remote: Some(Remote::posted(answer.id.clone(), answer.at)),
    }
}

/// How many words a text has, as Polish counts them.
pub fn words(text: &str) -> usize {
    text.split_whitespace().count()
}

/// The paragraph round the first place `quote` stands in `text`: the lines between the
/// blank ones either side of it. The whole text where the quote isn't in it.
pub fn paragraph_around(text: &str, quote: &str) -> String {
    let Some(at) = text.find(quote).filter(|_| !quote.is_empty()) else {
        return text.chars().take(2000).collect();
    };
    let start = text[..at].rfind("\n\n").map_or(0, |i| i + 2);
    let end = text[at..].find("\n\n").map_or(text.len(), |i| at + i);
    text[start..end].trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read_of(json: &str) -> Fetched {
        fetched(&answer_json(json).unwrap(), "https://docs.example/d/1").unwrap()
    }

    /// The servers come from the user's config and the project's, under `CLAUDE_CONFIG_DIR`
    /// when it's set, each once; the files are only read.
    #[test]
    fn the_servers_are_read_from_claude_codes_config_and_the_project() {
        let home = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        let config = r#"{"mcpServers":{"docs":{"command":"x"},"tracker":{}},
            "projects":{"PROJECT":{"mcpServers":{"wiki":{}}}}}"#
            .replace("PROJECT", &project.path().display().to_string());
        fs::write(home.path().join(".claude.json"), &config).unwrap();
        fs::write(
            project.path().join(".mcp.json"),
            r#"{"mcpServers":{"docs":{},"local":{}}}"#,
        )
        .unwrap();
        let root = home.path().display().to_string();
        let var = |name: &str| (name == "HOME").then(|| root.clone());
        let found = servers(&var, project.path());
        assert_eq!(found, ["docs", "tracker", "wiki", "local"]);
        assert_eq!(
            fs::read_to_string(home.path().join(".claude.json")).unwrap(),
            config,
            "read, never written"
        );
        // CLAUDE_CONFIG_DIR, where it's set, holds the config in place of home.
        let other = tempfile::tempdir().unwrap();
        fs::write(
            other.path().join(".claude.json"),
            r#"{"mcpServers":{"notes":{}}}"#,
        )
        .unwrap();
        let dir = other.path().display().to_string();
        let var = |name: &str| match name {
            "HOME" => Some(root.clone()),
            "CLAUDE_CONFIG_DIR" => Some(dir.clone()),
            _ => None,
        };
        assert_eq!(servers(&var, project.path()), ["notes", "docs", "local"]);
    }

    /// A read allows only the doc tool's server, refuses every other one and Claude Code's
    /// own tools that change things, and never skips a permission.
    #[test]
    fn a_read_allows_only_the_doc_tools_server() {
        let others = vec![
            "docs".to_string(),
            "tracker".to_string(),
            "my.wiki".to_string(),
        ];
        let args = tool_args("docs", &others);
        let allowed: Vec<&String> = args
            .iter()
            .zip(args.iter().skip(1))
            .filter(|(flag, _)| flag.as_str() == "--allowedTools")
            .map(|(_, value)| value)
            .collect();
        assert_eq!(allowed, ["mcp__docs__*"]);
        let refused = &args[args.iter().position(|a| a == "--disallowedTools").unwrap() + 1];
        assert!(refused.contains("mcp__tracker__*"), "{refused}");
        assert!(refused.contains("mcp__my_wiki__*"), "{refused}");
        assert!(!refused.contains("mcp__docs__"), "{refused}");
        assert!(
            refused.contains("Write") && refused.contains("Bash"),
            "{refused}"
        );
        let tools = &args[args.iter().position(|a| a == "--tools").unwrap() + 1];
        assert!(tools.is_empty(), "no built-in tools");
        let all = args.join(" ");
        assert!(!all.contains("dangerously"), "{all}");
        assert!(!all.contains("bypassPermissions"), "{all}");
        assert!(polish_args().contains(&"--strict-mcp-config".to_string()));
    }

    #[test]
    fn a_reads_answer_is_checked_and_its_times_read() {
        let read = read_of(
            "Here it is:\n```json\n{\"title\":\"Queue service launch\",\"version\":12,\
             \"markdown\":\"# Queue\\n\\nThree times last year's peak.\\n\",\
             \"comments\":[{\"id\":3,\"author\":\"Priya\",\"quote\":\"last year's peak\",\
             \"text\":\"Where does 3x come from?\",\"at\":\"2026-11-24T09:30:00Z\",\
             \"replies\":[{\"id\":\"r1\",\"author\":\"Sam\",\"text\":\"Growth.\",\
             \"at\":\"2026-11-24T10:30:00+01:00\"}]}]}\n```",
        );
        assert_eq!(read.title, "Queue service launch");
        assert_eq!(read.version, "12");
        assert_eq!(read.link, "https://docs.example/d/1");
        assert_eq!(read.comments[0].id, "3");
        assert_eq!(read.comments[0].at, 1_795_512_600);
        assert_eq!(read.comments[0].replies[0].at, 1_795_512_600);
        assert!(fetched(&serde_json::json!({"title": "x"}), "l").is_err());
        assert_eq!(
            answer_json(r#"{"error":"Session expired: please sign in again"}"#).unwrap_err(),
            SIGN_IN_AGAIN
        );
        assert_eq!(
            answer_json(r#"{"error":"No such document"}"#).unwrap_err(),
            "No such document"
        );
        assert_eq!(posted_id(r#"{"id": "c-41"}"#).unwrap(), "c-41");
        assert!(posted_id("I posted it.").is_err());
    }

    #[test]
    fn a_slug_is_short_and_safe() {
        let slug = slug("https://docs.example/d/Queue-Service_Launch?x=1");
        assert!(
            slug.starts_with("docs-example-d-queue-service-launch-x-1-") || slug.len() < 48,
            "{slug}"
        );
        assert!(
            slug.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'),
            "{slug}"
        );
        assert_ne!(
            slug,
            super::slug("https://docs.example/d/Queue-Service_Launch?x=2")
        );
        assert_eq!(slug_of(&doc_file(&slug)), Some(slug.as_str()));
        assert_eq!(slug_of("notes.md"), None);
    }

    /// Sync adds new comments and replies by their ids, brings changed ones up to date, and
    /// leaves the reviewer's drafts as they are; a quote that's gone says so.
    #[test]
    fn a_read_merges_by_id_and_keeps_drafts() {
        let file = doc_file("d");
        let first = read_of(
            r#"{"title":"T","markdown":"Alpha beta.\n\nGamma delta.\n","comments":[
              {"id":"c1","author":"Priya","quote":"Alpha","text":"Why alpha?","replies":[]}]}"#,
        );
        let mut threads = Vec::new();
        assert_eq!(merge(&mut threads, &file, &first), 1);
        threads.push(Thread {
            id: 9,
            file: file.clone(),
            line: 3,
            quote: "Gamma".into(),
            text: "My draft".into(),
            remote: Some(Remote::draft()),
            ..Thread::default()
        });
        let second = read_of(
            r#"{"title":"T","markdown":"Alpha beta.\n\nDelta only.\n","comments":[
              {"id":"c1","author":"Priya","quote":"Alpha","text":"Why alpha, really?",
               "resolved":true,"replies":[{"id":"r1","author":"Sam","text":"Because."}]},
              {"id":"c2","author":"Sam","quote":"beta","text":"And beta?"}]}"#,
        );
        assert_eq!(merge(&mut threads, &file, &second), 1);
        assert_eq!(merge(&mut threads, &file, &second), 0, "nothing twice");
        let c1 = threads.iter().find(|t| t.author == "Priya").unwrap();
        assert_eq!(c1.text, "Why alpha, really?");
        assert!(c1.resolved);
        assert_eq!(c1.replies.len(), 1);
        let draft = threads.iter().find(|t| t.id == 9).unwrap();
        assert_eq!(draft.text, "My draft");
        assert_eq!(draft.remote, Some(Remote::draft()));
        assert!(draft.gone, "Gamma left the doc");
        assert_eq!(threads.len(), 3);
    }

    #[test]
    fn a_paragraph_is_found_round_its_words() {
        let text = "# H\n\nOne two.\nThree four.\n\nFive.\n";
        assert_eq!(paragraph_around(text, "Three"), "One two.\nThree four.");
        assert_eq!(words("One slow  consumer\nstalls."), 4);
    }
}
