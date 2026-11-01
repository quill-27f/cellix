//! `cellix hook <kind>`: what Claude Code's hooks call. Nothing here may disturb the agent,
//! so with no Cellix running every kind quietly does nothing but keep its checkpoints.

use std::path::PathBuf;
use std::process::Command;

use cellix_review::{Comment, checkpoint};
use serde_json::{Value, json};

use crate::{AgentEvent, AgentState, TestRun, request, socket_path};

/// Longest test output kept, from the end, where failures are.
const MAX_OUTPUT: usize = 20_000;
const LABEL_LENGTH: usize = 60;

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
    let cwd = cwd.canonicalize().unwrap_or(cwd);
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
        agent: "Claude Code".to_string(),
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
    let reply = ask(roots, "take")?;
    let comments: Vec<Comment> = serde_json::from_str(&reply).ok()?;
    if comments.is_empty() {
        return None;
    }
    let output = json!({"hookSpecificOutput": {
        "hookEventName": "UserPromptSubmit",
        "additionalContext": comments_message(&comments),
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
    let path = socket.with_file_name(format!("plan-{session}.md"));
    std::fs::write(&path, plan).ok()?;
    tell(
        roots,
        input,
        AgentState::NeedsYou,
        "Plan waiting for review",
    );
    let reply = ask(roots, &format!("plan {}", path.display()))?;
    let output = match reply.split_once(' ') {
        Some(("send-back", feedback)) => {
            let reason: String = serde_json::from_str(feedback).ok()?;
            json!({"hookSpecificOutput": {
                "hookEventName": "PreToolUse",
                "permissionDecision": "deny",
                "permissionDecisionReason": reason,
            }})
        }
        _ if reply == "approve" => json!({"hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "permissionDecision": "allow",
            "permissionDecisionReason": "Approved in Cellix",
        }}),
        _ => return None,
    };
    Some(output.to_string())
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
        .or_else(|| response.as_str())
        .unwrap_or("");
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

/// Handles one hook, and returns what to print for Claude Code, if anything.
pub fn handle(kind: &str, stdin: &str) -> Option<String> {
    let input: Value = serde_json::from_str(stdin).unwrap_or(Value::Null);
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
        assert_eq!(answer["permissionDecision"], "deny");
        assert_eq!(answer["permissionDecisionReason"], "Split step two");
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
