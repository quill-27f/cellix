//! `cellix hook <kind>`: what Claude Code's hooks call. Nothing here may disturb the agent,
//! so with no Cellix running every kind quietly does nothing but keep its checkpoints.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use cellix_review::tasks::{self, Event};
use cellix_review::{Comment, Thread, threads};
use serde_json::{Value, json};

use crate::{AgentEvent, AgentState, TestRun, request, socket_path};

/// Longest test output kept, from the end, where failures are.
const MAX_OUTPUT: usize = 20_000;
const LABEL_LENGTH: usize = 60;
/// Where `handle_for` leaves the agent's name in a hook's input.
const AGENT_KEY: &str = "cellix_agent";
/// Set by Cellix's session host to its socket, for a program it holds.
pub const HELD_VAR: &str = "CELLIX_HELD";

/// How to resume an agent held by Cellix, left by its hooks beside the host's socket so it
/// outlives the host, as when the machine restarts.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Resumable {
    /// The agent, as Cellix names it: "Claude Code", "Codex".
    pub agent: String,
    /// The agent's own session id.
    pub session: String,
    /// Where the agent keeps the session's transcript, when its hook says.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub transcript: String,
}

/// Leaves word beside `held`, the host's socket, of the agent session running there.
/// Unchanged word isn't written again.
pub fn remember(held: &Path, agent: &str, session: &str, transcript: &str) {
    if session.is_empty() || !held.extension().is_some_and(|e| e == "sock") {
        return;
    }
    let word = Resumable {
        agent: agent.to_string(),
        session: session.to_string(),
        transcript: transcript.to_string(),
    };
    let path = held.with_extension("agent");
    let known = std::fs::read(&path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Resumable>(&bytes).ok());
    if known.as_ref() == Some(&word) {
        return;
    }
    if let Ok(json) = serde_json::to_vec(&word) {
        let _ = std::fs::write(path, json);
    }
}
/// What an agent reports while its plan is with the reviewer.
pub const PLAN_WAITING: &str = "Plan waiting for review";
/// How long the prompt hook waits for its checkpoint. An agent gives a hook a fixed time and
/// drops what it says after that, comments included, and a first checkpoint of a large tree
/// can take longer; past this it finishes on its own.
const CHECKPOINT_WAIT: Duration = Duration::from_secs(3);

/// Commands whose results belong beside the diff.
const TEST_COMMANDS: [&str; 13] = [
    "cargo test",
    "cargo nextest",
    "pytest",
    "npm test",
    "npm run test",
    "pnpm test",
    "yarn test",
    "vitest",
    "jest",
    "go test",
    "rspec",
    "make test",
    "harness/x",
];

pub fn is_test_command(command: &str) -> bool {
    TEST_COMMANDS.iter().any(|c| command.contains(c))
}

/// The folders a Cellix for this session might be serving, most specific first.
fn roots(input: &Value) -> Vec<PathBuf> {
    let cwd = input["cwd"]
        .as_str()
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_default();
    workspaces(&cwd)
}

/// The folders a Cellix for `folder` might be serving: it, then its repository's root.
pub fn workspaces(folder: &std::path::Path) -> Vec<PathBuf> {
    let cwd = dunce::canonicalize(folder).unwrap_or_else(|_| folder.to_path_buf());
    let mut roots = vec![cwd.clone()];
    let top = Command::new("git")
        .arg("-C")
        .arg(&cwd)
        .args(["rev-parse", "--show-toplevel"])
        .output();
    if let Ok(output) = top
        && output.status.success()
    {
        let text = String::from_utf8_lossy(&output.stdout);
        let top = PathBuf::from(text.trim());
        // git on Windows writes `C:/x`, and the socket is found by the path as Cellix has it.
        let top = if cfg!(windows) {
            dunce::canonicalize(&top).unwrap_or(top)
        } else {
            top
        };
        roots.push(top);
    }
    roots
}

fn ask(roots: &[PathBuf], line: &str) -> Option<String> {
    roots.iter().find_map(|root| request(root, line).ok())
}

/// The agent's own id for the session a hook is about.
fn session_of(input: &Value) -> String {
    input["session_id"].as_str().unwrap_or("").to_string()
}

/// The agent a hook came from, as Cellix names it.
fn agent_of(input: &Value) -> String {
    input[AGENT_KEY]
        .as_str()
        .unwrap_or("Claude Code")
        .to_string()
}

/// Where the project's task log is: with the Cellix serving it, or else at the top of its
/// repository. Outside both, as in a home folder, nothing is logged.
fn task_root(roots: &[PathBuf]) -> Option<PathBuf> {
    roots
        .iter()
        .find(|r| socket_path(r).exists())
        .or_else(|| roots.get(1))
        .or_else(|| roots.first().filter(|r| r.join(".git").exists()))
        .cloned()
}

/// Adds to the task log of the project the hook is about. Never in the agent's way.
fn log(roots: &[PathBuf], event: &Event) {
    if let Some(root) = task_root(roots) {
        let _ = tasks::record(&root, event);
    }
}

fn tell(roots: &[PathBuf], input: &Value, state: AgentState, detail: &str) {
    let event = AgentEvent {
        session: session_of(input),
        agent: agent_of(input),
        state,
        detail: detail.to_string(),
        folder: roots
            .first()
            .map(|r| r.display().to_string())
            .unwrap_or_default(),
        pid: agent_pid(),
        transcript: input["transcript_path"].as_str().unwrap_or("").to_string(),
    };
    if let Ok(json) = serde_json::to_string(&event) {
        ask(roots, &format!("event {json}"));
    }
}

/// The agent this hook runs for. An agent runs a hook's command through a shell, which waits
/// while `cellix` runs, so the agent is the shell's parent; run directly, it's this one's.
#[cfg(unix)]
fn agent_pid() -> Option<u32> {
    let parent = std::os::unix::process::parent_id();
    let (above, name) = process_of(parent)?;
    let shell = ["sh", "bash", "zsh", "dash", "fish", "ksh"];
    let name = name
        .rsplit('/')
        .next()
        .unwrap_or(&name)
        .trim_start_matches('-');
    Some(if shell.contains(&name) { above } else { parent })
}

#[cfg(not(unix))]
fn agent_pid() -> Option<u32> {
    None
}

/// A process's parent and its command's name: from `/proc` where there is one, else `ps`.
#[cfg(unix)]
fn process_of(pid: u32) -> Option<(u32, String)> {
    if let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) {
        // `pid (name) state ppid ...`, and the name can hold spaces and brackets.
        let (head, rest) = stat.rsplit_once(')')?;
        let name = head.split_once('(')?.1.to_string();
        let ppid = rest.split_whitespace().nth(1)?.parse().ok()?;
        return Some((ppid, name));
    }
    let output = Command::new("ps")
        .args(["-o", "ppid=,comm=", "-p", &pid.to_string()])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    let (ppid, name) = text.trim().split_once(char::is_whitespace)?;
    Some((ppid.trim().parse().ok()?, name.trim().to_string()))
}

/// Whether a notification asks the person for something. Claude Code also sends one when it
/// has sat idle after a turn ("waiting for your input"), on signing in, and when a question
/// was answered (`elicitation_complete`, `elicitation_response`), and those ask for nothing.
/// Its `notification_type` says which, as Gemini's does; without it, from older versions, the
/// words do.
fn asks(input: &Value) -> bool {
    match input["notification_type"].as_str() {
        Some(kind) => {
            let kind = kind.to_lowercase();
            kind.contains("permission")
                || kind == "agent_needs_input"
                || (kind.starts_with("elicitation") && kind.ends_with("dialog"))
        }
        None => {
            let message = input["message"].as_str().unwrap_or("").to_lowercase();
            !message.contains("waiting for your input")
        }
    }
}

/// Whether a notification says the agent is waiting with nothing to do: after a turn, and
/// after one was interrupted, when no Stop comes.
fn idles(input: &Value) -> bool {
    matches!(
        input["notification_type"].as_str(),
        Some("idle_prompt" | "agent_completed")
    )
}

/// Why a turn stopped on an error, in a word for the agent's tile.
fn failure_words(input: &Value) -> String {
    let reason = ["error_type", "error", "reason", "message"]
        .iter()
        .find_map(|field| input[*field].as_str().filter(|s| !s.is_empty()));
    match reason {
        Some(reason) => format!("Stopped: {}", reason.replace('_', " ")),
        None => "Stopped on an error".to_string(),
    }
}

/// What a request says: its own words, or for Codex, which asks through a permission hook
/// with none, the command it wants to run.
fn request_words(input: &Value, agent: &str) -> String {
    if let Some(message) = input["message"].as_str().filter(|m| !m.is_empty()) {
        return message.to_string();
    }
    match input["tool_input"]["command"].as_str() {
        Some(command) => format!("{agent} asks to run {command}"),
        None => "Needs your attention".to_string(),
    }
}

/// Comments as a message an agent can act on.
pub fn comments_message(comments: &[Comment]) -> String {
    let mut text = String::from("Review comments on your changes:\n");
    // In file and line order, as they'd be read, not the order they were written.
    let mut sorted: Vec<&Comment> = comments.iter().collect();
    sorted.sort_by(|a, b| (&a.file, a.line).cmp(&(&b.file, b.line)));
    for comment in sorted {
        text.push_str(&format!("- {}\n", comment.message()));
    }
    text
}

fn label(prompt: &str) -> String {
    let line = prompt.lines().next().unwrap_or("Prompt").trim();
    let mut label: String = line.chars().take(LABEL_LENGTH).collect();
    if line.chars().count() > LABEL_LENGTH {
        label.push('…');
    }
    label
}

fn on_prompt(roots: &[PathBuf], input: &Value) -> Option<String> {
    if let Some(root) = roots.first() {
        let prompt = input["prompt"].as_str().unwrap_or("Prompt");
        // The newest thread now, so the task view can tell which were made during this turn.
        let event = task_root(roots).map(|log| {
            let threads = threads::load(&log)
                .map(|t| threads::next_id(&t) - 1)
                .unwrap_or(0);
            let event = Event::Prompt {
                at: tasks::now(),
                session: session_of(input),
                agent: agent_of(input),
                text: tasks::clip_prompt(prompt),
                commit: None,
                threads,
            };
            (log, event)
        });
        checkpoint_briefly(root, &label(prompt), event);
    }
    tell(roots, input, AgentState::Working, "Working");
    let comments: Vec<Comment> = ask(roots, "take")
        .and_then(|reply| serde_json::from_str(&reply).ok())
        .unwrap_or_default();
    let threads: Vec<Thread> = ask(roots, "take-threads")
        .and_then(|reply| serde_json::from_str(&reply).ok())
        .unwrap_or_default();
    let mut context = String::new();
    if !comments.is_empty() {
        context.push_str(&comments_message(&comments));
    }
    for thread in &threads {
        context.push_str(&thread.message());
        context.push('\n');
    }
    if context.is_empty() {
        return None;
    }
    let output = json!({"hookSpecificOutput": {
        "hookEventName": "UserPromptSubmit",
        "additionalContext": context,
    }});
    Some(output.to_string())
}

/// Checkpoints in a process of its own, waiting at most `CHECKPOINT_WAIT` for it. That process
/// also logs the prompt for the task view, with the checkpoint it took.
fn checkpoint_briefly(root: &Path, label: &str, prompt: Option<(PathBuf, Event)>) {
    // Only the real binary has a `checkpoint` command; a test runs it here instead.
    let exe = std::env::current_exe()
        .ok()
        .filter(|exe| exe.file_stem().is_some_and(|stem| stem == "cellix"));
    let Some(exe) = exe else {
        let prompt = prompt.as_ref().map(|(log, e)| (log.as_path(), e.clone()));
        tasks::checkpoint_prompt(root, label, prompt);
        return;
    };
    let mut command = Command::new(exe);
    command.args(["checkpoint", label]);
    if let Some((log, event)) = &prompt {
        command.env(tasks::ROOT_VAR, log).env(
            tasks::EVENT_VAR,
            serde_json::to_string(event).unwrap_or_default(),
        );
    }
    command
        .current_dir(root)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // Its own group, so it isn't stopped with the hook that started it.
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut command, 0);
    let Ok(mut child) = command.spawn() else {
        return;
    };
    let started = Instant::now();
    while started.elapsed() < CHECKPOINT_WAIT {
        if !matches!(child.try_wait(), Ok(None)) {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Holds a plan until the reviewer decides, and gives back the verdict and any text with it.
fn review_plan(roots: &[PathBuf], input: &Value, plan: &str) -> Option<(String, Option<String>)> {
    let socket = roots.iter().map(|r| socket_path(r)).find(|s| s.exists())?;
    let session: String = input["session_id"]
        .as_str()
        .unwrap_or("session")
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .collect();
    // Beside the socket, because that folder is private and /tmp on a server isn't.
    let folder = socket.with_extension("").join(session);
    std::fs::create_dir_all(&folder).ok()?;
    let path = keep_versions(&folder, plan).ok()?;
    tell(roots, input, AgentState::NeedsYou, PLAN_WAITING);
    let file = path.display().to_string();
    log(
        roots,
        &Event::Plan {
            at: tasks::now(),
            session: session_of(input),
            agent: agent_of(input),
            file: file.clone(),
        },
    );
    let reply = ask(roots, &format!("plan {file}"))?;
    let (verdict, text) = reply.split_once(' ').unwrap_or((&reply, "null"));
    let text: Option<String> = serde_json::from_str(text).ok()?;
    // An approved plan is done with, so the next one the session sends starts at version one.
    let kept = if verdict == "approve" {
        put_versions_away(&folder).ok().flatten()
    } else {
        None
    };
    if matches!(verdict, "approve" | "send-back") {
        log(
            roots,
            &Event::Answer {
                at: tasks::now(),
                file,
                approved: verdict == "approve",
                note: if verdict == "approve" {
                    String::new()
                } else {
                    text.clone().unwrap_or_default()
                },
                kept: kept.map(|k| k.display().to_string()),
            },
        );
    }
    Some((verdict.to_string(), text))
}

/// Moves an approved plan and its earlier versions into `Approved 1`, `Approved 2` and on, so
/// a plan sent after it isn't taken for its next version. Cellix keeps the approved one with
/// the project, and its comments with it. Gives the folder they went to.
pub fn put_versions_away(folder: &Path) -> std::io::Result<Option<PathBuf>> {
    let plan = folder.join("Plan.md");
    if !plan.exists() {
        return Ok(None);
    }
    let mut number = 1;
    while folder.join(format!("Approved {number}")).exists() {
        number += 1;
    }
    let away = folder.join(format!("Approved {number}"));
    std::fs::create_dir_all(&away)?;
    let mut version = 1;
    loop {
        let name = format!("Plan v{version}.md");
        if !folder.join(&name).exists() {
            break;
        }
        std::fs::rename(folder.join(&name), away.join(&name))?;
        version += 1;
    }
    std::fs::rename(plan, away.join("Plan.md"))?;
    Ok(Some(away))
}

/// Writes the plan as `Plan.md`, keeping the one it replaces as `Plan v1.md`, `Plan v2.md` and on,
/// so a reviewer can see what changed since they sent it back. Comments stay on `Plan.md`.
pub fn keep_versions(folder: &Path, plan: &str) -> std::io::Result<PathBuf> {
    let path = folder.join("Plan.md");
    if let Ok(old) = std::fs::read_to_string(&path)
        && old != plan
    {
        let mut version = 1;
        while folder.join(format!("Plan v{version}.md")).exists() {
            version += 1;
        }
        std::fs::write(folder.join(format!("Plan v{version}.md")), old)?;
    }
    std::fs::write(&path, plan)?;
    Ok(path)
}

/// Claude Code's plan, from its permission request. With no Cellix, it asks as usual.
fn on_plan(roots: &[PathBuf], input: &Value) -> Option<String> {
    let plan = input["tool_input"]["plan"].as_str()?;
    let (verdict, text) = review_plan(roots, input, plan)?;
    // A PreToolUse answer wouldn't do: Claude Code still shows its own plan dialog.
    let decision = match (verdict.as_str(), text) {
        ("send-back", Some(message)) => json!({"behavior": "deny", "message": message}),
        ("approve", edited) => {
            // Claude Code only counts an approval as the user's answer if it carries the input.
            let mut plan_input = input["tool_input"].clone();
            if let Some(edited) = edited {
                plan_input["plan"] = json!(edited);
            }
            json!({"behavior": "allow", "updatedInput": plan_input})
        }
        _ => return None,
    };
    let output = json!({"hookSpecificOutput": {
        "hookEventName": "PermissionRequest",
        "decision": decision,
    }});
    Some(output.to_string())
}

/// Gemini CLI's plan, as it goes to leave plan mode. It keeps plans in a file, so an edited
/// one is written back there, and Gemini still asks for its own approval afterwards.
fn on_gemini_plan(roots: &[PathBuf], input: &Value) -> Option<String> {
    let path = PathBuf::from(input["tool_input"]["plan_path"].as_str()?);
    let path = match input["cwd"].as_str() {
        Some(folder) => Path::new(folder).join(path),
        None => path,
    };
    let plan = std::fs::read_to_string(&path).ok()?;
    let (verdict, text) = review_plan(roots, input, &plan)?;
    let output = match (verdict.as_str(), text) {
        ("send-back", Some(reason)) => json!({"decision": "deny", "reason": reason}),
        ("approve", edited) => {
            if let Some(edited) = edited {
                std::fs::write(&path, edited).ok()?;
            }
            json!({"decision": "allow"})
        }
        _ => return None,
    };
    Some(output.to_string())
}

/// The plan in a message from Codex, which has no tool for proposing one.
fn proposed_plan(message: &str) -> Option<&str> {
    let (_, rest) = message.split_once("<proposed_plan>")?;
    let (plan, _) = rest.split_once("</proposed_plan>")?;
    Some(plan.trim())
}

/// Codex ends its turn on a plan, so that's when it's held. Sending it back starts another
/// turn with the reviewer's words. Approving lets the turn end, and implementing it is still
/// the user's to ask for in Codex; approving with changes or notes starts one more turn, so
/// Codex has them before it's asked.
fn on_codex_stop(roots: &[PathBuf], input: &Value) -> Option<String> {
    let plan = proposed_plan(input["last_assistant_message"].as_str()?)?;
    let (verdict, text) = review_plan(roots, input, plan)?;
    match (verdict.as_str(), text) {
        ("send-back", Some(reason)) => {
            Some(json!({"decision": "block", "reason": reason}).to_string())
        }
        ("approve", Some(changed)) if changed.trim() != plan.trim() => {
            let reason = format!(
                "The reviewer approved your plan with changes. This is the plan to follow \
                 now; implement it when asked.\n\n{changed}"
            );
            Some(json!({"decision": "block", "reason": reason}).to_string())
        }
        _ => None,
    }
}

/// Whether a command's exit is really that of its last stage, such as `tail`, and not the
/// tests'. With `pipefail` the shell reports the failure itself.
fn piped(command: &str) -> bool {
    let bare = command.replace("||", "");
    bare.contains('|') && !command.contains("pipefail")
}

/// What failing tests print, for when the exit can't be trusted or isn't said: a mark, or a
/// count of failures that isn't nought ("2 failed", not cargo's "0 failed").
fn reads_as_failed(output: &str) -> bool {
    let marked = ["FAILED", "failures:", "test result: F", "FAIL "]
        .iter()
        .any(|mark| output.contains(mark));
    marked
        || output.match_indices(" failed").any(|(at, _)| {
            let count = output[..at].split_whitespace().next_back().unwrap_or("");
            count.parse::<u64>().is_ok_and(|n| n > 0)
        })
}

/// The exit code a result says in its words, as Gemini CLI's does ("Exit Code: 1").
fn said_exit(text: &str) -> Option<i64> {
    let lower = text.to_lowercase();
    ["exit code: ", "exit code ", "exited with code "]
        .iter()
        .find_map(|mark| {
            let at = lower.find(mark)? + mark.len();
            let digits: String = lower[at..]
                .chars()
                .take_while(|c| c.is_ascii_digit() || *c == '-')
                .collect();
            digits.parse().ok()
        })
}

fn on_tool(roots: &[PathBuf], input: &Value, passed: bool) {
    // A tool ran, so whatever the agent asked leave for has been answered.
    tell(roots, input, AgentState::Working, "Working");
    let command = input["tool_input"]["command"].as_str().unwrap_or("");
    if !is_test_command(command) {
        return;
    }
    // Codex may send its result as JSON inside a string.
    let unpacked: Option<Value> = input["tool_response"]
        .as_str()
        .and_then(|text| serde_json::from_str(text).ok())
        .filter(Value::is_object);
    let response = unpacked.as_ref().unwrap_or(&input["tool_response"]);
    let text = response["stdout"]
        .as_str()
        .or_else(|| input["error"].as_str())
        .or_else(|| response["returnDisplay"].as_str())
        .or_else(|| response["output"].as_str())
        .or_else(|| response["llmContent"].as_str())
        .or_else(|| response.as_str())
        .unwrap_or("");
    // Agents without a failure hook say so in the result: as a field, or in its words.
    let exit = response["exit_code"]
        .as_i64()
        .or(response["exitCode"].as_i64())
        .or_else(|| response["llmContent"].as_str().and_then(said_exit))
        .or_else(|| said_exit(text));
    // Claude Code has a hook of its own for a failed run. The others, when the result doesn't
    // say how it exited, are judged by what the run printed, never taken to have passed.
    let unsaid = exit.is_none() && agent_of(input) != "Claude Code";
    let passed = passed
        && exit.unwrap_or(0) == 0
        && response["error"].is_null()
        && !((piped(command) || unsaid) && reads_as_failed(text));
    let start = text
        .char_indices()
        .rev()
        .nth(MAX_OUTPUT)
        .map_or(0, |(at, _)| at);
    log(
        roots,
        &Event::Test {
            at: tasks::now(),
            session: session_of(input),
            command: command.to_string(),
            passed,
            output: tasks::clip_output(text),
        },
    );
    let run = TestRun {
        command: command.to_string(),
        passed,
        output: text[start..].to_string(),
    };
    if let Ok(json) = serde_json::to_string(&run) {
        ask(roots, &format!("test {json}"));
    }
}

/// Handles one hook from Claude Code.
pub fn handle(kind: &str, stdin: &str) -> Option<String> {
    handle_for("Claude Code", kind, stdin)
}

/// Handles one hook from a named agent, and returns what to print for it, if anything.
pub fn handle_for(agent: &str, kind: &str, stdin: &str) -> Option<String> {
    // Without its details this can't be told from a stray call, and nothing here should
    // guess which project it's about, least of all from the folder it happens to run in.
    let mut input: Value = serde_json::from_str(stdin).ok()?;
    if !input.is_object() {
        return None;
    }
    input[AGENT_KEY] = json!(agent);
    let roots = roots(&input);
    if matches!(kind, "session-start" | "prompt")
        && let Some(held) = std::env::var_os(HELD_VAR)
    {
        remember(
            Path::new(&held),
            agent,
            input["session_id"].as_str().unwrap_or(""),
            input["transcript_path"].as_str().unwrap_or(""),
        );
    }
    match kind {
        "session-start" => tell(&roots, &input, AgentState::Idle, "Ready"),
        "prompt" => return on_prompt(&roots, &input),
        "plan" if agent == "Gemini" => return on_gemini_plan(&roots, &input),
        "plan" => return on_plan(&roots, &input),
        "notify" if asks(&input) => {
            let detail = request_words(&input, agent);
            tell(&roots, &input, AgentState::NeedsYou, &detail);
        }
        // Waiting after a turn, which after an interrupt is the only word that it stopped.
        "notify" if idles(&input) => tell(&roots, &input, AgentState::Idle, "Waiting for you"),
        // A sign-in or an answered question: nothing to answer, so the state stays as it was.
        "notify" => {}
        "tool-done" => on_tool(&roots, &input, true),
        "tool-failed" => on_tool(&roots, &input, false),
        "stop" => {
            tell(&roots, &input, AgentState::Idle, "Finished");
            if agent == "Codex" {
                return on_codex_stop(&roots, &input);
            }
        }
        // A turn cut short by the API: a limit, an outage, a sign-in. It isn't working.
        "stop-failed" => tell(&roots, &input, AgentState::Idle, &failure_words(&input)),
        "session-end" => tell(&roots, &input, AgentState::Ended, "Ended"),
        _ => {}
    }
    None
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_held_agent_leaves_word_of_how_to_resume_it() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("1-2.sock");
        super::remember(&socket, "Claude Code", "abc-123", "/t/abc-123.jsonl");
        let word: super::Resumable =
            serde_json::from_slice(&std::fs::read(dir.path().join("1-2.agent")).unwrap()).unwrap();
        assert_eq!(word.agent, "Claude Code");
        assert_eq!(word.session, "abc-123");
        assert_eq!(word.transcript, "/t/abc-123.jsonl");
        // A /clear starts a new session, which replaces it.
        super::remember(&socket, "Claude Code", "def-456", "");
        let text = std::fs::read_to_string(dir.path().join("1-2.agent")).unwrap();
        assert!(text.contains("def-456"), "{text}");
        // Nothing without an id, and nothing beside what isn't a host's socket.
        super::remember(&socket, "Claude Code", "", "");
        assert!(
            std::fs::read_to_string(dir.path().join("1-2.agent"))
                .unwrap()
                .contains("def-456")
        );
        super::remember(&dir.path().join("notes.txt"), "Claude Code", "x", "");
        assert!(!dir.path().join("notes.agent").exists());
    }

    use super::*;
    use crate::{Decision, Request, Server, SharedComments, start};
    use std::path::Path;

    #[test]
    fn each_plan_keeps_the_one_before_it() {
        let dir = tempfile::tempdir().unwrap();
        keep_versions(dir.path(), "first").unwrap();
        keep_versions(dir.path(), "first").unwrap();
        assert!(
            !dir.path().join("Plan v1.md").exists(),
            "the same plan again is no version"
        );
        keep_versions(dir.path(), "second").unwrap();
        let path = keep_versions(dir.path(), "third").unwrap();
        assert_eq!(std::fs::read_to_string(path).unwrap(), "third");
        let read = |name: &str| std::fs::read_to_string(dir.path().join(name)).unwrap();
        assert_eq!(read("Plan v1.md"), "first");
        assert_eq!(read("Plan v2.md"), "second");
    }

    #[test]
    fn a_pipe_hides_a_failed_test_and_the_output_gives_it_away() {
        assert!(piped("cargo test | tail"));
        assert!(!piped("cargo test || true"));
        assert!(!piped("set -o pipefail; cargo test | tail"));
        assert!(reads_as_failed("test result: FAILED. 1 passed; 2 failed"));
        assert!(!reads_as_failed("test result: ok. 9 passed; 0 ignored"));
        assert!(!reads_as_failed(
            "test result: ok. 9 passed; 0 failed; 0 ignored"
        ));
        assert!(reads_as_failed("Tests: 1 failed, 5 passed, 6 total"));
    }

    #[test]
    fn a_failed_run_in_codex_or_gemini_is_not_taken_for_a_pass() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let server = start(&root, Arc::default()).unwrap();
        let run = |response: Value| {
            input(
                &root,
                json!({"tool_input": {"command": "cargo test"}, "tool_response": response}),
            )
        };
        // Codex's result as JSON in a string, with its exit code.
        let codex = json!({"exit_code": 101, "stdout": "error: test failed"}).to_string();
        handle_for("Codex", "tool-done", &run(Value::String(codex)));
        assert!(!next_test(&server).passed, "Codex's exit code");
        // Gemini CLI says the exit code in its words.
        let gemini =
            json!({"llmContent": "Command: cargo test\nExit Code: 101", "returnDisplay": "…"});
        handle_for("Gemini", "tool-done", &run(gemini));
        assert!(!next_test(&server).passed, "Gemini's words");
        // With no exit code at all, what the run printed decides.
        let failing = json!({"output": "test result: FAILED. 3 passed; 1 failed"});
        handle_for("Codex", "tool-done", &run(failing));
        assert!(!next_test(&server).passed, "the output");
        let passing = json!({"output": "test result: ok. 3 passed; 0 failed"});
        handle_for("Codex", "tool-done", &run(passing));
        assert!(next_test(&server).passed, "a pass still passes");
    }
    use std::sync::Arc;
    use std::time::Duration;

    fn input(dir: &Path, extra: Value) -> String {
        let mut value = json!({"cwd": dir, "session_id": "s1"});
        value
            .as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        value.to_string()
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
    fn comments_reach_an_agent_in_order_with_what_they_are_about() {
        let later = Comment {
            file: "b.rs".into(),
            line: 9,
            text: "wall clock is wrong here".into(),
            quote: "now = time.time()".into(),
            removed: true,
        };
        let first = Comment {
            quote: "fn main() {".into(),
            ..comment()
        };
        let message = comments_message(&[later, first]);
        let a = message
            .find("a.rs line 3, on \"fn main() {\": why?")
            .expect(&message);
        let b = message.find("b.rs line 9 (a line your change removed), on \"now = time.time()\"");
        assert!(b.is_some_and(|b| b > a), "{message}");
    }

    #[test]
    fn nothing_breaks_with_no_cellix_running() {
        let dir = tempfile::tempdir().unwrap();
        for kind in ["session-start", "prompt", "notify", "stop", "session-end"] {
            assert_eq!(handle(kind, &input(dir.path(), json!({}))), None);
        }
        let plan = input(dir.path(), json!({"tool_input": {"plan": "# P"}}));
        assert_eq!(handle("plan", &plan), None, "Claude Code asks as usual");
        assert_eq!(handle("prompt", "not json"), None);
    }

    #[test]
    fn a_hook_with_no_details_asks_nothing_of_a_running_cellix() {
        for kind in ["prompt", "plan", "stop"] {
            assert_eq!(handle(kind, "not json"), None);
            assert_eq!(handle(kind, "[1, 2]"), None);
        }
    }

    #[test]
    fn the_next_prompt_carries_pending_comments() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let shared: SharedComments = Arc::new(std::sync::Mutex::new(vec![comment()]));
        let _server = start(&root, shared.clone()).unwrap();
        let hook = input(&root, json!({"prompt": "carry on"}));
        let output = handle("prompt", &hook).unwrap();
        let parsed: Value = serde_json::from_str(&output).unwrap();
        let context = parsed["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .unwrap();
        assert!(context.contains("a.rs line 3: why?"), "{context}");
        assert!(shared.lock().unwrap().is_empty(), "delivered once");
        assert_eq!(handle("prompt", &hook), None);
    }

    #[test]
    fn plan_decision_reaches_the_hook() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let server = start(&root, Arc::default()).unwrap();
        let hook = input(&root, json!({"tool_input": {"plan": "# Plan\n"}}));
        let waiting = std::thread::spawn(move || handle("plan", &hook));
        loop {
            match server
                .requests
                .recv_timeout(Duration::from_secs(5))
                .unwrap()
            {
                Request::Plan { path, reply } => {
                    assert_eq!(std::fs::read_to_string(path).unwrap(), "# Plan\n");
                    reply
                        .send(Decision::SendBack("Split step two".into()))
                        .unwrap();
                    break;
                }
                _ => continue,
            }
        }
        let output = waiting.join().unwrap().unwrap();
        let parsed: Value = serde_json::from_str(&output).unwrap();
        let answer = &parsed["hookSpecificOutput"];
        assert_eq!(answer["hookEventName"], "PermissionRequest");
        assert_eq!(answer["decision"]["behavior"], "deny");
        assert_eq!(answer["decision"]["message"], "Split step two");
    }

    #[test]
    fn a_plan_after_an_approved_one_starts_at_version_one() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let server = start(&root, Arc::default()).unwrap();
        // Sent back once, approved, then a new plan in the same session.
        let answers = [
            ("# One\n", Decision::SendBack("Fewer steps".into())),
            ("# One, shorter\n", Decision::Approve(None)),
            ("# Two\n", Decision::SendBack("Later".into())),
        ];
        let mut held = Vec::new();
        for (plan, decision) in answers {
            let hook = input(&root, json!({"tool_input": {"plan": plan}}));
            let waiting = std::thread::spawn(move || handle("plan", &hook));
            loop {
                if let Request::Plan { path, reply } = server
                    .requests
                    .recv_timeout(Duration::from_secs(5))
                    .unwrap()
                {
                    let versions = (1..)
                        .map(|v| path.with_file_name(format!("Plan v{v}.md")))
                        .take_while(|p| p.exists())
                        .count();
                    held.push((path, versions));
                    reply.send(decision).unwrap();
                    break;
                }
            }
            waiting.join().unwrap();
        }
        assert_eq!(held[1].1, 1, "sent back, so the second is version 2");
        assert_eq!(held[2].1, 0, "after approval, a new plan is version 1");
        let away = held[2].0.with_file_name("Approved 1");
        let read = |name: &str| std::fs::read_to_string(away.join(name)).unwrap();
        assert_eq!(
            read("Plan.md"),
            "# One, shorter\n",
            "the approved plan is put away"
        );
        assert_eq!(read("Plan v1.md"), "# One\n");
    }

    /// Runs a hook on a thread, and answers the plan it holds with `decision`.
    fn answered(
        agent: &'static str,
        kind: &'static str,
        hook: String,
        decision: Decision,
    ) -> (Option<String>, String) {
        let root = hook_root(&hook);
        let server = start(&root, Arc::default()).unwrap();
        let waiting = std::thread::spawn(move || handle_for(agent, kind, &hook));
        loop {
            if let Request::Plan { path, reply } = server
                .requests
                .recv_timeout(Duration::from_secs(5))
                .unwrap()
            {
                let held = std::fs::read_to_string(path).unwrap();
                reply.send(decision).unwrap();
                return (waiting.join().unwrap(), held);
            }
        }
    }

    fn hook_root(hook: &str) -> PathBuf {
        let parsed: Value = serde_json::from_str(hook).unwrap();
        PathBuf::from(parsed["cwd"].as_str().unwrap())
    }

    fn gemini_plan(root: &Path, text: &str) -> String {
        let plan = root.join("plan.md");
        std::fs::write(&plan, text).unwrap();
        input(root, json!({"tool_input": {"plan_path": plan}}))
    }

    #[test]
    fn gemini_is_held_at_leaving_plan_mode_and_told_what_to_change() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let hook = gemini_plan(&root, "# G\n");
        let sent = Decision::SendBack("Fewer steps".into());
        let (output, held) = answered("Gemini", "plan", hook, sent);
        assert_eq!(held, "# G\n");
        let parsed: Value = serde_json::from_str(&output.unwrap()).unwrap();
        assert_eq!(parsed["decision"], "deny");
        assert_eq!(parsed["reason"], "Fewer steps");
    }

    #[test]
    fn an_edit_to_geminis_plan_goes_back_in_its_file() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let hook = gemini_plan(&root, "# Old\n");
        let edited = Decision::Approve(Some("# New\n".into()));
        let (output, _) = answered("Gemini", "plan", hook, edited);
        let parsed: Value = serde_json::from_str(&output.unwrap()).unwrap();
        assert_eq!(parsed["decision"], "allow");
        assert_eq!(
            std::fs::read_to_string(root.join("plan.md")).unwrap(),
            "# New\n"
        );
    }

    #[test]
    fn a_plan_from_codex_is_held_when_its_turn_ends() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let message = "Here it is.\n\n<proposed_plan>\n# C\n</proposed_plan>\n";
        let hook = input(&root, json!({"last_assistant_message": message}));
        let sent = Decision::SendBack("Say why".into());
        let (output, held) = answered("Codex", "stop", hook, sent);
        assert_eq!(held, "# C");
        let parsed: Value = serde_json::from_str(&output.unwrap()).unwrap();
        assert_eq!(parsed["decision"], "block");
        assert_eq!(parsed["reason"], "Say why");
    }

    #[test]
    fn approving_a_codex_plan_with_changes_hands_them_over() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let message = "<proposed_plan>\n# C\n</proposed_plan>";
        let hook = input(&root, json!({"last_assistant_message": message}));
        let changed = "# C\n\n## Reviewer notes\n- Keep the old API.".to_string();
        let (output, _) = answered("Codex", "stop", hook, Decision::Approve(Some(changed)));
        let parsed: Value = serde_json::from_str(&output.unwrap()).unwrap();
        assert_eq!(parsed["decision"], "block");
        assert!(
            parsed["reason"]
                .as_str()
                .unwrap()
                .contains("Keep the old API.")
        );
    }

    #[test]
    fn approving_a_codex_plan_lets_its_turn_end() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let message = "<proposed_plan>\n# C\n</proposed_plan>";
        let hook = input(&root, json!({"last_assistant_message": message}));
        let (output, _) = answered("Codex", "stop", hook, Decision::Approve(None));
        assert_eq!(output, None);
    }

    #[test]
    fn an_ordinary_codex_turn_isnt_held() {
        assert_eq!(proposed_plan("Done, the tests pass."), None);
        assert_eq!(proposed_plan("<proposed_plan>cut off"), None);
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let _server = start(&root, Arc::default()).unwrap();
        let hook = input(&root, json!({"last_assistant_message": "All done."}));
        assert_eq!(handle_for("Codex", "stop", &hook), None);
    }

    #[test]
    fn plan_hooks_are_given_time_for_a_person_to_read() {
        let hooks = |text: &str| serde_json::from_str::<Value>(text).unwrap();
        let gemini = hooks(include_str!(
            "../../cellix/integrations/gemini/hooks/hooks.json"
        ));
        let before = &gemini["hooks"]["BeforeTool"][0];
        assert_eq!(before["matcher"], "exit_plan_mode");
        assert_eq!(
            before["hooks"][0]["timeout"], 86_400_000,
            "Gemini counts milliseconds"
        );
        let codex = hooks(include_str!(
            "../../cellix/integrations/codex/hooks/hooks.json"
        ));
        assert_eq!(codex["hooks"]["Stop"][0]["hooks"][0]["timeout"], 86_400);
    }

    #[test]
    fn an_edited_plan_is_approved_as_edited() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let server = start(&root, Arc::default()).unwrap();
        let hook = input(
            &root,
            json!({"tool_input": {"plan": "# Old", "planFilePath": "/p.md"}}),
        );
        let waiting = std::thread::spawn(move || handle("plan", &hook));
        loop {
            if let Request::Plan { reply, .. } = server
                .requests
                .recv_timeout(Duration::from_secs(5))
                .unwrap()
            {
                reply.send(Decision::Approve(Some("# New".into()))).unwrap();
                break;
            }
        }
        let output = waiting.join().unwrap().unwrap();
        let parsed: Value = serde_json::from_str(&output).unwrap();
        let answer = &parsed["hookSpecificOutput"]["decision"];
        assert_eq!(answer["behavior"], "allow");
        assert_eq!(answer["updatedInput"]["plan"], "# New");
        assert_eq!(answer["updatedInput"]["planFilePath"], "/p.md");
    }

    #[test]
    fn events_carry_the_agents_name() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let server = start(&root, Arc::default()).unwrap();
        handle_for("Codex", "session-start", &input(&root, json!({})));
        match server
            .requests
            .recv_timeout(Duration::from_secs(5))
            .unwrap()
        {
            Request::Agent(event) => assert_eq!(event.agent, "Codex"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_failed_command_is_read_from_the_result() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let server = start(&root, Arc::default()).unwrap();
        let run = input(
            &root,
            json!({"tool_input": {"command": "cargo test"},
                   "tool_response": {"returnDisplay": "boom", "error": "exit 1"}}),
        );
        handle_for("Gemini", "tool-done", &run);
        let run = next_test(&server);
        assert!(!run.passed);
        assert_eq!(run.output, "boom");
    }

    #[test]
    fn test_runs_are_reported_and_other_commands_are_not() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let server = start(&root, Arc::default()).unwrap();
        let other = input(&root, json!({"tool_input": {"command": "ls"}}));
        handle("tool-done", &other);
        let run = input(
            &root,
            json!({"tool_input": {"command": "cargo test"}, "tool_response": {"stdout": "ok"}}),
        );
        handle("tool-failed", &run);
        // The first test run to arrive is the second command's: `ls` only says it's working.
        let run = next_test(&server);
        assert_eq!(run.command, "cargo test");
        assert!(!run.passed);
    }

    /// The next test run reported, past news of the agent working, which every tool sends.
    fn next_test(server: &Server) -> TestRun {
        loop {
            match server
                .requests
                .recv_timeout(Duration::from_secs(5))
                .unwrap()
            {
                Request::Test(run) => return run,
                Request::Agent(event) => assert_eq!(event.state, AgentState::Working),
                other => panic!("{other:?}"),
            }
        }
    }

    /// The next news of an agent, or None if nothing comes for a while.
    fn next_event(server: &Server, wait: Duration) -> Option<AgentEvent> {
        match server.requests.recv_timeout(wait) {
            Ok(Request::Agent(event)) => Some(event),
            Ok(other) => panic!("{other:?}"),
            Err(_) => None,
        }
    }

    #[test]
    fn only_a_request_counts_as_needing_you() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let server = start(&root, Arc::default()).unwrap();
        // Claude Code's reminder after an idle turn asks for nothing: it says the agent is
        // waiting, which after an interrupt nothing else does.
        let idle = "Claude is waiting for your input";
        handle(
            "notify",
            &input(
                &root,
                json!({"notification_type": "idle_prompt", "message": idle}),
            ),
        );
        let event = next_event(&server, Duration::from_secs(5)).unwrap();
        assert_eq!(
            (event.state, event.detail.as_str()),
            (AgentState::Idle, "Waiting for you")
        );
        // Nor does the reminder without its type, a sign-in, or a question already answered,
        // and none is told.
        handle("notify", &input(&root, json!({"message": idle})));
        for kind in [
            "auth_success",
            "elicitation_complete",
            "elicitation_response",
        ] {
            handle(
                "notify",
                &input(&root, json!({"notification_type": kind, "message": "Done"})),
            );
        }
        // A permission prompt is, and comes first.
        let asking = "Claude needs your permission to use Bash";
        handle(
            "notify",
            &input(
                &root,
                json!({"notification_type": "permission_prompt", "message": asking}),
            ),
        );
        let event = next_event(&server, Duration::from_secs(5)).unwrap();
        assert_eq!(
            (event.state, event.detail.as_str()),
            (AgentState::NeedsYou, asking)
        );
        // Codex asks through its permission hook, with the command and no words of its own.
        let codex = input(
            &root,
            json!({"tool_name": "Bash", "tool_input": {"command": "cargo publish"}}),
        );
        handle_for("Codex", "notify", &codex);
        let event = next_event(&server, Duration::from_secs(5)).unwrap();
        assert_eq!(event.state, AgentState::NeedsYou);
        assert_eq!(event.detail, "Codex asks to run cargo publish");
        // Gemini says what kind of notification it sends.
        let gemini = input(
            &root,
            json!({"notification_type": "ToolPermission", "message": "Allow rm?"}),
        );
        handle_for("Gemini", "notify", &gemini);
        assert_eq!(
            next_event(&server, Duration::from_secs(5)).unwrap().state,
            AgentState::NeedsYou
        );
        assert_eq!(next_event(&server, Duration::from_millis(300)), None);
    }

    #[test]
    fn a_turn_the_api_cut_short_is_not_left_working() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let server = start(&root, Arc::default()).unwrap();
        // Claude Code's StopFailure, where a Stop would have come.
        handle(
            "stop-failed",
            &input(&root, json!({"error_type": "rate_limit"})),
        );
        let event = next_event(&server, Duration::from_secs(5)).unwrap();
        assert_eq!(
            (event.state, event.detail.as_str()),
            (AgentState::Idle, "Stopped: rate limit")
        );
        // And an elicitation is a question, as a permission prompt is.
        let asking = json!({"notification_type": "elicitation_dialog", "message": "Which port?"});
        handle("notify", &input(&root, asking));
        let event = next_event(&server, Duration::from_secs(5)).unwrap();
        assert_eq!(event.state, AgentState::NeedsYou);
    }

    #[test]
    fn a_tool_that_ran_means_the_request_was_answered() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let server = start(&root, Arc::default()).unwrap();
        let asking = json!({"notification_type": "permission_prompt", "message": "Claude needs your permission to use Edit"});
        handle("notify", &input(&root, asking));
        let event = next_event(&server, Duration::from_secs(5)).unwrap();
        assert_eq!(event.state, AgentState::NeedsYou);
        // On Unix, it says which process it is, so one that's gone can be let go.
        assert_eq!(event.pid.is_some(), cfg!(unix));
        // Approved in the agent, the tool runs, and its hook says the agent is working.
        let edited = json!({"tool_name": "Edit", "tool_input": {"file_path": "notes.txt"}});
        handle("tool-done", &input(&root, edited));
        let event = next_event(&server, Duration::from_secs(5)).unwrap();
        assert_eq!(event.state, AgentState::Working);
    }

    #[test]
    fn an_agents_work_is_logged_for_the_task_view() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let git = |args: &[&str]| {
            let status = Command::new("git").arg("-C").arg(&root).args(args).status();
            assert!(status.unwrap().success());
        };
        git(&["init", "-q"]);
        std::fs::write(root.join("a.rs"), "fn a() {}\n").unwrap();
        git(&["add", "."]);
        git(&[
            "-c",
            "user.name=T",
            "-c",
            "user.email=t@example.com",
            "commit",
            "-qm",
            "a",
        ]);
        let server = start(&root, Arc::default()).unwrap();
        handle(
            "prompt",
            &input(&root, json!({"prompt": "Add a refund route"})),
        );
        // Plans sent back once and then approved, as the reviewer answers them.
        for decision in [
            Decision::SendBack("Only paid orders".into()),
            Decision::Approve(None),
        ] {
            let hook = input(&root, json!({"tool_input": {"plan": "# Refunds\n"}}));
            let waiting = std::thread::spawn(move || handle("plan", &hook));
            loop {
                let request = server
                    .requests
                    .recv_timeout(Duration::from_secs(5))
                    .unwrap();
                if let Request::Plan { reply, .. } = request {
                    reply.send(decision).unwrap();
                    break;
                }
            }
            waiting.join().unwrap();
        }
        let run = input(
            &root,
            json!({"tool_input": {"command": "cargo test"}, "tool_response": {"stdout": "FAILED"}}),
        );
        handle("tool-failed", &run);

        let events = tasks::load(&root);
        let found = tasks::tasks(&events, |_| None);
        assert_eq!(found.len(), 1, "{events:?}");
        let task = &found[0];
        assert_eq!(
            (task.name.as_str(), task.agent.as_str()),
            ("Add a refund route", "Claude Code")
        );
        let turn = task.turns().next().unwrap();
        assert!(
            turn.commit.is_some(),
            "the prompt is logged with its checkpoint"
        );
        assert_eq!(turn.tests.len(), 1);
        assert!(!turn.tests[0].passed);
        let outcomes: Vec<&tasks::Outcome> = task.plans().map(|p| &p.outcome).collect();
        assert_eq!(
            outcomes[0],
            &tasks::Outcome::SentBack("Only paid orders".into())
        );
        assert!(matches!(outcomes[1], tasks::Outcome::Approved(Some(_))));
        // The log stays out of the project's changes.
        let ignore = std::fs::read_to_string(root.join(".cellix/.gitignore")).unwrap();
        assert!(ignore.contains("tasks.jsonl"));
    }
}
