//! Checks the shipped Claude Code plugin, since no build step touches it.

use std::path::Path;

fn plugin_file(path: &str) -> String {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("integrations/claude-code");
    std::fs::read_to_string(root.join(path)).unwrap()
}

#[test]
fn plugin_manifest_is_valid() {
    let manifest: serde_json::Value =
        serde_json::from_str(&plugin_file(".claude-plugin/plugin.json")).unwrap();
    assert_eq!(manifest["name"], "cellix");
    assert_eq!(manifest["version"], env!("CARGO_PKG_VERSION"));
    let skill = plugin_file("skills/cellix-review/SKILL.md");
    assert!(skill.starts_with("---\nname: cellix-review\ndescription: "));
    assert!(skill.contains("cellix comments --take"));
}

#[test]
fn plugin_hooks_are_wired_and_never_fail_the_agent() {
    let hooks: serde_json::Value = serde_json::from_str(&plugin_file("hooks/hooks.json")).unwrap();
    let events = hooks["hooks"].as_object().unwrap();
    for name in ["UserPromptSubmit", "PermissionRequest", "Stop"] {
        assert!(events.contains_key(name), "{name}");
    }
    for entries in events.values() {
        for entry in entries.as_array().unwrap() {
            for hook in entry["hooks"].as_array().unwrap() {
                let command = hook["command"].as_str().unwrap();
                assert!(command.starts_with("cellix hook "), "{command}");
                assert!(command.ends_with("|| true"), "{command}");
                // Claude Code's PATH may lack Cellix; setup says where it is.
                assert!(command.contains(" || \"$CELLIX_BIN\" hook "), "{command}");
            }
        }
    }
    let plan = &events["PermissionRequest"][0];
    assert_eq!(plan["matcher"], "ExitPlanMode");
    assert!(
        plan["hooks"][0]["timeout"].as_i64().unwrap() > 3600,
        "a review takes time"
    );
}

#[test]
fn claude_mod_bridge_loads() {
    let hooks: serde_json::Value = serde_json::from_str(&plugin_file("hooks/hooks.json")).unwrap();
    assert_eq!(hooks["modules"][0], "./register.tsx");
    let module = plugin_file("hooks/register.tsx");
    assert!(module.contains("export const register"));
    for command in ["cellix", "cellix-comments", "cellix-open", "cellix-send"] {
        assert!(module.contains(&format!("name: '{command}'")), "{command}");
    }
    // The socket is the mod's only way in, so a Cellix that isn't running must read as that.
    assert!(module.contains("Cellix is not running"));
    // A second session in the same folder can't start one, and must say so, not wait.
    assert!(module.contains("already open for this folder"));
}

/// The mod's own tests, `hooks/*.test.tsx`, run in Claude Code's kit, which loads the mod as a
/// session does and answers its calls. Skipped where Claude Code isn't installed.
#[test]
fn the_mod_passes_its_own_tests_in_claude_code() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("integrations/claude-code");
    let run = std::process::Command::new("claude")
        .args(["plugin", "test"])
        .arg(&root)
        .output();
    let Ok(run) = run else {
        eprintln!("skipped: Claude Code isn't installed");
        return;
    };
    let said = format!(
        "{}{}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );
    assert!(run.status.success(), "{said}");
    assert!(said.contains(" 0 fail"), "{said}");
}

/// OpenCode loads Cellix's plugin as a module and hands it its events. Run under node, as
/// OpenCode would run it under Bun, with a stand-in `cellix` that keeps each call it gets in a
/// file of its own, each event becomes `cellix hook --agent OpenCode <kind>` with the session
/// and folder, and a test command's run with what it printed. Skipped where node isn't
/// installed; OpenCode itself isn't run anywhere.
#[test]
#[cfg(unix)]
fn the_opencode_plugin_hands_its_events_to_cellix() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let calls = dir.path().join("calls");
    std::fs::create_dir(&calls).unwrap();
    let stand_in = dir.path().join("cellix");
    let script = format!(
        "#!/bin/sh\nf=$(mktemp '{}/call.XXXXXX')\necho \"$*\" > \"$f\"\ncat >> \"$f\"\n",
        calls.display()
    );
    std::fs::write(&stand_in, script).unwrap();
    std::fs::set_permissions(&stand_in, std::fs::Permissions::from_mode(0o755)).unwrap();
    // As a module of its own, which is how OpenCode reads it.
    let plugin = dir.path().join("cellix.mjs");
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("integrations/opencode/cellix.js");
    std::fs::copy(source, &plugin).unwrap();
    let events = r#"
        const { Cellix } = await import(process.argv[1])
        const hooks = await Cellix({ directory: "/work/project", worktree: "/work/project" })
        const event = (type, properties) => hooks.event({ event: { type, properties } })
        await event("session.created", { info: { id: "ses_1" } })
        await event("session.status", { sessionID: "ses_1", status: { type: "busy" } })
        await event("session.status", { sessionID: "ses_1", status: { type: "idle" } })
        await event("permission.asked", { sessionID: "ses_1", permission: "bash", patterns: ["cargo publish"] })
        await hooks["tool.execute.before"]({ tool: "bash", sessionID: "ses_1", callID: "c1" }, { args: { command: "cargo test" } })
        await hooks["tool.execute.after"]({ tool: "bash", sessionID: "ses_1", callID: "c1" }, { title: "", output: "test result: ok", metadata: { exit: 0 } })
        await event("session.idle", { sessionID: "ses_1" })
        await event("message.part.updated", { sessionID: "ses_1" })
    "#;
    let run = std::process::Command::new("node")
        .args(["--input-type=module", "-e", events])
        .arg(&plugin)
        .env("CELLIX_BIN", &stand_in)
        .output();
    let Ok(run) = run else {
        eprintln!("skipped: node isn't installed");
        return;
    };
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    // Each call is a process of its own, which the plugin doesn't wait for.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let mut told: Vec<(String, serde_json::Value)> = Vec::new();
    while told.len() < 5 {
        assert!(std::time::Instant::now() < deadline, "only {told:?}");
        std::thread::sleep(std::time::Duration::from_millis(50));
        told = std::fs::read_dir(&calls)
            .unwrap()
            .flatten()
            .filter_map(|file| {
                let text = std::fs::read_to_string(file.path()).ok()?;
                let (args, stdin) = text.split_once('\n')?;
                Some((args.to_string(), serde_json::from_str(stdin).ok()?))
            })
            .collect();
    }
    std::thread::sleep(std::time::Duration::from_millis(300));
    assert_eq!(
        std::fs::read_dir(&calls).unwrap().count(),
        5,
        "nothing for idle or a part"
    );
    let call = |kind: &str| {
        let wanted = format!("hook --agent OpenCode {kind}");
        let found = told.iter().find(|(args, _)| *args == wanted);
        found
            .unwrap_or_else(|| panic!("no {kind} in {told:?}"))
            .1
            .clone()
    };
    for kind in ["session-start", "working", "notify", "tool-done", "stop"] {
        let said = call(kind);
        assert_eq!(said["session_id"], "ses_1", "{kind}");
        assert_eq!(said["cwd"], "/work/project", "{kind}");
    }
    let asked = call("notify");
    assert_eq!(asked["notification_type"], "permission_prompt");
    assert_eq!(asked["message"], "OpenCode asks to run cargo publish");
    let ran = call("tool-done");
    assert_eq!(ran["tool_input"]["command"], "cargo test");
    assert_eq!(ran["tool_response"]["output"], "test result: ok");
    assert_eq!(ran["tool_response"]["exit_code"], 0);
}
