use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Carried in the binary, so a fresh server needs nothing but Cellix.
const FILES: [(&str, &str); 5] = [
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
];

/// The Codex plugin, under the marketplace folder Codex reads it from.
const CODEX_FILES: [(&str, &str); 4] = [
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
    (
        ".agents/plugins/marketplace.json",
        include_str!("../integrations/codex/marketplace.json"),
    ),
];

const GEMINI_FILES: [(&str, &str); 3] = [
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
];

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
    if let Some(folder) = folder(|name| std::env::var(name).ok())
        && folder.join(".claude-plugin/plugin.json").exists()
    {
        let _ = write_to(&folder);
    }
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
