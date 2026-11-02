//! `cellix hook <kind>`: what Claude Code's hooks call. Nothing here may disturb the agent,
//! so with no Cellix running every kind quietly does nothing but keep its checkpoints.

use std::path::{Path, PathBuf};
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
    let path = folder.join("Plan.md");
    std::fs::write(&path, plan).ok()?;
    tell(roots, input, AgentState::NeedsYou, PLAN_WAITING);
    let reply = ask(roots, &format!("plan {}", path.display()))?;
    let (verdict, text) = reply.split_once(' ').unwrap_or((&reply, "null"));
    Some((verdict.to_string(), serde_json::from_str(text).ok()?))
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
/// the user's to ask for in Codex.
fn on_codex_stop(roots: &[PathBuf], input: &Value) -> Option<String> {
    let plan = proposed_plan(input["last_assistant_message"].as_str()?)?;
    let (verdict, text) = review_plan(roots, input, plan)?;
    match (verdict.as_str(), text) {
        ("send-back", Some(reason)) => {
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
        "plan" if agent == "Gemini" => return on_gemini_plan(&roots, &input),
        "plan" => return on_plan(&roots, &input),
        "notify" => {
            let detail = input["message"].as_str().unwrap_or("Needs your attention");
            tell(&roots, &input, AgentState::NeedsYou, detail);
        }
        "tool-done" => on_tool(&roots, &input, true),
        "tool-failed" => on_tool(&roots, &input, false),
        "stop" => {
            tell(&roots, &input, AgentState::Idle, "Finished");
            if agent == "Codex" {
                return on_codex_stop(&roots, &input);
            }
        }
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
