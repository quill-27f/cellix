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
