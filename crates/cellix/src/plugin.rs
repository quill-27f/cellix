use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// How agents start, message and wait for each other in Cellix: the same guide for every
/// agent, as a skill each reads, and what `cellix agent --skill` prints.
pub const AGENT_SKILL: &str = include_str!("../integrations/skills/cellix-agents/SKILL.md");

/// The guide without the lines that make it a skill, as an agent reads it on its own.
pub fn agent_guide() -> &'static str {
    let body = AGENT_SKILL
        .strip_prefix("---\n")
        .and_then(|rest| rest.split_once("\n---\n"));
    body.map_or(AGENT_SKILL, |(_, body)| body.trim_start())
}

/// Carried in the binary, so a fresh server needs nothing but Cellix.
const FILES: [(&str, &str); 6] = [
    (
        ".claude-plugin/plugin.json",
        include_str!("../integrations/claude-code/.claude-plugin/plugin.json"),
    ),
    (
        "hooks/hooks.json",
        include_str!("../integrations/claude-code/hooks/hooks.json"),
    ),
    (
        "hooks/register.tsx",
        include_str!("../integrations/claude-code/hooks/register.tsx"),
    ),
    (
        "hooks/pane-input.tsx",
        include_str!("../integrations/claude-code/hooks/pane-input.tsx"),
    ),
    (
        "skills/cellix-review/SKILL.md",
        include_str!("../integrations/claude-code/skills/cellix-review/SKILL.md"),
    ),
    ("skills/cellix-agents/SKILL.md", AGENT_SKILL),
];

/// The Codex plugin, under the marketplace folder Codex reads it from.
const CODEX_FILES: [(&str, &str); 5] = [
    (
        "plugins/cellix/plugin.json",
        include_str!("../integrations/codex/plugin.json"),
    ),
    (
        "plugins/cellix/hooks/hooks.json",
        include_str!("../integrations/codex/hooks/hooks.json"),
    ),
    (
        "plugins/cellix/skills/cellix-review/SKILL.md",
        include_str!("../integrations/codex/skills/cellix-review/SKILL.md"),
    ),
    ("plugins/cellix/skills/cellix-agents/SKILL.md", AGENT_SKILL),
    (
        ".agents/plugins/marketplace.json",
        include_str!("../integrations/codex/marketplace.json"),
    ),
];

/// Gemini CLI reads an extension's skills from its `skills` folder, and its GEMINI.md in every
/// session.
const GEMINI_FILES: [(&str, &str); 4] = [
    (
        "gemini-extension.json",
        include_str!("../integrations/gemini/gemini-extension.json"),
    ),
    (
        "hooks/hooks.json",
        include_str!("../integrations/gemini/hooks/hooks.json"),
    ),
    (
        "GEMINI.md",
        include_str!("../integrations/gemini/GEMINI.md"),
    ),
    ("skills/cellix-agents/SKILL.md", AGENT_SKILL),
];

/// OpenCode's plugin and Copilot CLI's hooks: one file each, in a folder of the agent's that
/// it reads every file of.
pub(crate) const OPENCODE_PLUGIN: &str = include_str!("../integrations/opencode/cellix.js");
pub(crate) const COPILOT_HOOKS: &str = include_str!("../integrations/copilot/cellix.json");
/// Cursor's hooks, which go into its one file beside the user's own.
const CURSOR_HOOKS: &str = include_str!("../integrations/cursor/hooks.json");
/// What the prompt hook says when Cellix isn't there, since Cursor waits on its word.
const CURSOR_GO_ON: &str = " 2>/dev/null || echo '{\\\"continue\\\": true}'";

/// Cursor's hooks as this platform runs them: PowerShell has no `/dev/null` and no `||`.
pub(crate) fn cursor_hooks() -> String {
    if cfg!(windows) {
        CURSOR_HOOKS.replace(QUIET, "").replace(CURSOR_GO_ON, "")
    } else {
        CURSOR_HOOKS.to_string()
    }
}

/// Kiro CLI's hooks: one file of Cellix's own in its global hooks folder, which its V3 chat
/// reads in every workspace beside the workspace's own.
const KIRO_HOOKS: &str = include_str!("../integrations/kiro/cellix.json");
/// The trigger Kiro CLI has only from 3.4, which runs as a call is about to ask the person.
pub(crate) const KIRO_ASKS: &str = "PermissionRequest";

/// Kiro CLI's hooks as this machine's Kiro runs them. `asks`: whether its Kiro has the
/// `PermissionRequest` trigger, left out where it's known not to, as a trigger it doesn't know
/// may keep the file from loading. On Windows each is the bare command, as Cursor's are.
pub(crate) fn kiro_hooks(asks: bool) -> String {
    let mut file: serde_json::Value =
        serde_json::from_str(KIRO_HOOKS).expect("Kiro's hooks are JSON");
    if let Some(hooks) = file["hooks"].as_array_mut() {
        hooks.retain(|hook| asks || hook["trigger"] != KIRO_ASKS);
        for hook in hooks.iter_mut().filter(|_| cfg!(windows)) {
            let command = hook["action"]["command"].as_str().unwrap_or("");
            let bare = command.split(" 2>/dev/null").next().unwrap_or(command);
            hook["action"]["command"] = serde_json::json!(bare);
        }
    }
    let mut text = serde_json::to_string_pretty(&file).expect("JSON is written");
    text.push('\n');
    text
}

/// Writes one file, and its folder, leaving it alone when it already says that.
pub(crate) fn write_file(path: &Path, text: &str) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    if fs::read_to_string(path).is_ok_and(|now| now == text) {
        return Ok(());
    }
    fs::write(path, text)
}

fn data_dir(var: impl Fn(&str) -> Option<String>) -> Option<PathBuf> {
    let data = var("XDG_DATA_HOME")
        .filter(|d| !d.is_empty())
        .map(PathBuf::from);
    data.or_else(|| Some(PathBuf::from(var("HOME")?).join(".local/share")))
}

pub(crate) fn write_to(folder: &Path) -> io::Result<()> {
    // Claude Code runs hooks in Git Bash, on Windows too.
    write_files_for(folder, &FILES, false)
}

/// Writes an agent's hooks as this platform runs them.
fn write_files(folder: &Path, files: &[(&str, &str)]) -> io::Result<()> {
    write_files_for(folder, files, cfg!(windows))
}

/// What keeps a hook quiet where it runs in a Unix shell, when Cellix isn't installed.
const QUIET: &str = " 2>/dev/null || true";

/// A hook file as `windows` runs it. Codex and Gemini run hooks in PowerShell there, where
/// `/dev/null` doesn't exist and Windows PowerShell can't read `||`, so the hook would never
/// run. `cellix hook` itself always succeeds quietly, so the bare command is what's left.
fn for_platform<'a>(name: &str, text: &'a str, windows: bool) -> std::borrow::Cow<'a, str> {
    if windows && name.ends_with("hooks/hooks.json") {
        text.replace(QUIET, "").into()
    } else {
        text.into()
    }
}

fn write_files_for(folder: &Path, files: &[(&str, &str)], windows: bool) -> io::Result<()> {
    for (name, text) in files {
        let path = folder.join(name);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let text = for_platform(name, text, windows);
        // Left alone when it's the same, so whatever watches the folder sees no change.
        if fs::read_to_string(&path).is_ok_and(|now| now == text.as_ref()) {
            continue;
        }
        fs::write(path, text.as_ref())?;
    }
    Ok(())
}

/// Writes the Codex marketplace, and returns its folder.
pub(crate) fn write_codex(var: impl Fn(&str) -> Option<String>) -> io::Result<PathBuf> {
    let folder = data_dir(var)
        .ok_or(io::ErrorKind::NotFound)?
        .join("cellix/codex");
    write_files(&folder, &CODEX_FILES)?;
    Ok(folder)
}

/// Writes the Gemini extension, and returns its folder.
pub(crate) fn write_gemini(var: impl Fn(&str) -> Option<String>) -> io::Result<PathBuf> {
    let folder = data_dir(var)
        .ok_or(io::ErrorKind::NotFound)?
        .join("cellix/gemini");
    write_files(&folder, &GEMINI_FILES)?;
    Ok(folder)
}

/// Where the Claude Code plugin lives.
pub(crate) fn folder(var: impl Fn(&str) -> Option<String>) -> Option<PathBuf> {
    Some(data_dir(var)?.join("cellix/claude-code"))
}

/// Brings a plugin written by an older Cellix up to this one's, as each Claude Code session
/// starts: an upgrade replaces the binary, not the files it wrote. Nothing is written where
/// the plugin never was.
pub fn refresh() {
    if let Some(folder) = folder(|name| std::env::var(name).ok()) {
        refresh_in(&folder, env!("CARGO_PKG_VERSION"));
    }
}

/// `refresh`, for the plugin in `folder` and a Cellix of `version`. An older Cellix still on
/// the PATH runs the hooks first, so it must not put its own files back over a newer one's:
/// the newer Cellix's mod would then refuse it as another protocol, every session.
fn refresh_in(folder: &Path, version: &str) {
    let manifest = folder.join(".claude-plugin/plugin.json");
    let Ok(text) = fs::read_to_string(&manifest) else {
        return;
    };
    let installed = serde_json::from_str::<serde_json::Value>(&text)
        .ok()
        .and_then(|m| m["version"].as_str().map(numbers));
    if installed.is_some_and(|installed| installed > numbers(version)) {
        return;
    }
    let _ = write_to(folder);
}

/// A version's numbers, to compare: `0.6.10` after `0.6.9`.
fn numbers(version: &str) -> Vec<u64> {
    version
        .split(['.', '-', '+'])
        .map_while(|part| part.parse().ok())
        .collect()
}

/// Writes the Claude Code plugin to the user's data folder, and returns where.
pub fn install() -> Option<PathBuf> {
    let folder = folder(|name| std::env::var(name).ok())?;
    write_to(&folder).ok()?;
    Some(folder)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_older_cellix_leaves_a_newer_plugin_alone() {
        let dir = tempfile::tempdir().unwrap();
        write_to(dir.path()).unwrap();
        let manifest = dir.path().join(".claude-plugin/plugin.json");
        let hooks = dir.path().join("hooks/hooks.json");
        let newer = fs::read_to_string(&manifest)
            .unwrap()
            .replace(env!("CARGO_PKG_VERSION"), "99.0.0");
        fs::write(&manifest, &newer).unwrap();
        fs::write(&hooks, "{\"from\": \"a newer Cellix\"}").unwrap();
        refresh_in(dir.path(), "0.6.10");
        assert_eq!(
            fs::read_to_string(&hooks).unwrap(),
            "{\"from\": \"a newer Cellix\"}"
        );
        // An older plugin is brought up to this Cellix's.
        let older = newer.replace("99.0.0", "0.5.4");
        fs::write(&manifest, older).unwrap();
        refresh_in(dir.path(), env!("CARGO_PKG_VERSION"));
        assert_ne!(
            fs::read_to_string(&hooks).unwrap(),
            "{\"from\": \"a newer Cellix\"}"
        );
        assert!(numbers("0.6.10") > numbers("0.6.9"));
    }

    #[test]
    fn the_plugin_unpacks_whole() {
        let dir = tempfile::tempdir().unwrap();
        write_to(dir.path()).unwrap();
        for (name, _) in FILES {
            assert!(dir.path().join(name).exists(), "{name}");
        }
    }

    /// Codex and Gemini run hooks in PowerShell on Windows, which can't run the Unix form.
    #[test]
    fn codex_and_gemini_hooks_run_in_powershell_on_windows() {
        for files in [&CODEX_FILES[..], &GEMINI_FILES[..]] {
            let dir = tempfile::tempdir().unwrap();
            write_files_for(dir.path(), files, true).unwrap();
            let (name, unix) = files
                .iter()
                .find(|(name, _)| name.ends_with("hooks/hooks.json"))
                .unwrap();
            let written = fs::read_to_string(dir.path().join(name)).unwrap();
            let hooks: serde_json::Value = serde_json::from_str(&written).unwrap();
            let mut commands = Vec::new();
            for groups in hooks["hooks"].as_object().unwrap().values() {
                for group in groups.as_array().unwrap() {
                    for hook in group["hooks"].as_array().unwrap() {
                        commands.push(hook["command"].as_str().unwrap().to_string());
                    }
                }
            }
            assert!(!commands.is_empty());
            for command in &commands {
                // Words PowerShell, cmd and bash all run the same way.
                assert!(command.starts_with("cellix hook "), "{command}");
                assert!(
                    command
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || " -".contains(c)),
                    "{command}"
                );
            }
            // Elsewhere they keep quiet when Cellix isn't installed.
            assert_eq!(commands.len(), unix.matches(QUIET).count());
            let dir = tempfile::tempdir().unwrap();
            write_files_for(dir.path(), files, false).unwrap();
            assert_eq!(fs::read_to_string(dir.path().join(name)).unwrap(), *unix);
        }
    }

    #[test]
    fn an_older_plugin_is_brought_up_to_date_and_a_current_one_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        write_to(dir.path()).unwrap();
        let module = dir.path().join("hooks/register.tsx");
        fs::write(&module, "an older mod").unwrap();
        let skill = dir.path().join("skills/cellix-review/SKILL.md");
        let before = fs::metadata(&skill).unwrap().modified().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        write_to(dir.path()).unwrap();
        assert_eq!(fs::read_to_string(&module).unwrap(), FILES[2].1);
        assert_eq!(fs::metadata(&skill).unwrap().modified().unwrap(), before);
    }

    /// Every agent gets the same guide to starting and messaging others, each in the form it
    /// reads, and the guide names only commands `cellix agent` has.
    #[test]
    fn every_agent_has_the_guide_to_other_agents() {
        for files in [&FILES[..], &CODEX_FILES[..], &GEMINI_FILES[..]] {
            let dir = tempfile::tempdir().unwrap();
            write_files_for(dir.path(), files, false).unwrap();
            let skill = files
                .iter()
                .find(|(name, _)| name.ends_with("cellix-agents/SKILL.md"))
                .expect("the guide is shipped");
            let written = fs::read_to_string(dir.path().join(skill.0)).unwrap();
            assert!(written.starts_with("---\nname: cellix-agents\ndescription: "));
        }
        let guide = agent_guide();
        assert!(guide.starts_with("# Working with other agents"), "{guide}");
        for command in ["run", "send", "read", "wait", "ls", "stop"] {
            assert!(
                guide.contains(&format!("cellix agent {command}")),
                "{command}"
            );
        }
        let gemini = include_str!("../integrations/gemini/GEMINI.md");
        assert!(gemini.contains("cellix agent --skill"));
    }

    #[test]
    fn it_goes_in_the_users_data_folder() {
        let none = |_: &str| None;
        assert_eq!(data_dir(none), None);
        let home = |name: &str| (name == "HOME").then(|| "/h".to_string());
        assert_eq!(data_dir(home), Some(PathBuf::from("/h/.local/share")));
    }

    #[test]
    fn every_agent_is_told_the_commands_that_exist() {
        let texts = [
            include_str!("../integrations/claude-code/skills/cellix-review/SKILL.md"),
            include_str!("../integrations/codex/skills/cellix-review/SKILL.md"),
            include_str!("../integrations/gemini/GEMINI.md"),
        ];
        let main = include_str!("main.rs");
        for text in texts {
            for command in ["threads", "reply", "plan", "comments", "open"] {
                assert!(text.contains(&format!("`cellix {command}")), "{command}");
                assert!(main.contains(&format!("[\"{command}\"")), "{command}");
            }
        }
    }
}
