//! Checks the shipped Claude Code plugin, since no build step touches it.

use std::path::Path;

fn plugin_file(path: &str) -> String {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../integrations/claude-code");
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
