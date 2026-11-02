//! `cellix hook <kind>`: what Claude Code's hooks call. Nothing here may disturb the agent,
//! so with no Cellix running every kind quietly does nothing but keep its checkpoints.

use std::path::PathBuf;
use std::process::Command;

use cellix_review::{Comment, Thread, checkpoint};
use serde_json::{Value, json};

use crate::{AgentEvent, AgentState, TestRun, request, socket_path};

/// Longest test output kept, from the end, where failures are.
const MAX_OUTPUT: usize = 20_000;
const LABEL_LENGTH: usize = 60;
/// Where `handle_for` leaves the agent's name in a hook's input.
const AGENT_KEY: &str = "cellix_agent";
/// What an agent reports while its plan is with the reviewer.
pub const PLAN_WAITING: &str = "Plan waiting for review";

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
    let cwd = folder
        .canonicalize()
        .unwrap_or_else(|_| folder.to_path_buf());
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
        roots.push(PathBuf::from(text.trim()));
    }
    roots
}

fn ask(roots: &[PathBuf], line: &str) -> Option<String> {
    roots.iter().find_map(|root| request(root, line).ok())
}

fn tell(roots: &[PathBuf], input: &Value, state: AgentState, detail: &str) {
    let event = AgentEvent {
        session: input["session_id"].as_str().unwrap_or("").to_string(),
        agent: input[AGENT_KEY]
            .as_str()
            .unwrap_or("Claude Code")
            .to_string(),
        state,
        detail: detail.to_string(),
        folder: roots
            .first()
            .map(|r| r.display().to_string())
            .unwrap_or_default(),
    };
    if let Ok(json) = serde_json::to_string(&event) {
        ask(roots, &format!("event {json}"));
    }
}

/// Comments as a message an agent can act on.
pub fn comments_message(comments: &[Comment]) -> String {
    let mut text = String::from("Review comments on your changes:\n");
    for comment in comments {
        text.push_str(&format!(
            "- {}:{}: {}\n",
            comment.file, comment.line, comment.text
        ));
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
        checkpoint(root, &label(prompt));
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

/// Holds the plan until the reviewer decides. With no Cellix, Claude Code asks as usual.
fn on_plan(roots: &[PathBuf], input: &Value) -> Option<String> {
    let plan = input["tool_input"]["plan"].as_str()?;
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
    let path = folder.join("Plan.md");
    std::fs::write(&path, plan).ok()?;
    tell(roots, input, AgentState::NeedsYou, PLAN_WAITING);
    let reply = ask(roots, &format!("plan {}", path.display()))?;
    // A PreToolUse answer wouldn't do: Claude Code still shows its own plan dialog.
    let (verdict, text) = reply.split_once(' ').unwrap_or((&reply, "null"));
    let text: Option<String> = serde_json::from_str(text).ok()?;
    let decision = match (verdict, text) {
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

/// Whether a command's exit is really that of its last stage, such as `tail`, and not the
/// tests'. With `pipefail` the shell reports the failure itself.
fn piped(command: &str) -> bool {
    let bare = command.replace("||", "");
    bare.contains('|') && !command.contains("pipefail")
}

/// What failing tests print, for when the exit can't be trusted.
fn reads_as_failed(output: &str) -> bool {
    ["FAILED", "failures:", "test result: F", " failed", "FAIL "]
        .iter()
        .any(|mark| output.contains(mark))
}

fn on_tool(roots: &[PathBuf], input: &Value, passed: bool) {
    let command = input["tool_input"]["command"].as_str().unwrap_or("");
    if !is_test_command(command) {
        return;
    }
    let response = &input["tool_response"];
    let text = response["stdout"]
        .as_str()
        .or_else(|| input["error"].as_str())
        .or_else(|| response["returnDisplay"].as_str())
        .or_else(|| response["output"].as_str())
        .or_else(|| response.as_str())
        .unwrap_or("");
    // Agents without a failure hook say so in the result.
    let exit = response["exit_code"]
        .as_i64()
        .or(response["exitCode"].as_i64());
    let passed = passed
        && exit.unwrap_or(0) == 0
        && response["error"].is_null()
        && !(piped(command) && reads_as_failed(text));
    let start = text
        .char_indices()
        .rev()
        .nth(MAX_OUTPUT)
        .map_or(0, |(at, _)| at);
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
    match kind {
        "session-start" => tell(&roots, &input, AgentState::Idle, "Ready"),
        "prompt" => return on_prompt(&roots, &input),
        "plan" => return on_plan(&roots, &input),
        "notify" => {
            let detail = input["message"].as_str().unwrap_or("Needs your attention");
            tell(&roots, &input, AgentState::NeedsYou, detail);
        }
        "tool-done" => on_tool(&roots, &input, true),
        "tool-failed" => on_tool(&roots, &input, false),
        "stop" => tell(&roots, &input, AgentState::Idle, "Finished"),
        "session-end" => tell(&roots, &input, AgentState::Ended, "Ended"),
        _ => {}
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Decision, Request, SharedComments, start};
    use std::path::Path;

    #[test]
    fn a_pipe_hides_a_failed_test_and_the_output_gives_it_away() {
        assert!(piped("cargo test | tail"));
        assert!(!piped("cargo test || true"));
        assert!(!piped("set -o pipefail; cargo test | tail"));
        assert!(reads_as_failed("test result: FAILED. 1 passed; 2 failed"));
        assert!(!reads_as_failed("test result: ok. 9 passed; 0 ignored"));
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
        }
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
        assert!(context.contains("a.rs:3: why?"), "{context}");
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
        match server
            .requests
            .recv_timeout(Duration::from_secs(5))
            .unwrap()
        {
            Request::Test(run) => {
                assert!(!run.passed);
                assert_eq!(run.output, "boom");
            }
            other => panic!("{other:?}"),
        }
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
        match server
            .requests
            .recv_timeout(Duration::from_secs(5))
            .unwrap()
        {
            Request::Test(run) => {
                assert_eq!(run.command, "cargo test");
                assert!(!run.passed);
            }
            other => panic!("{other:?}"),
        }
    }
}
