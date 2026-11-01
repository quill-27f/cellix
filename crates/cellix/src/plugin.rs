use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Carried in the binary, so a fresh server needs nothing but Cellix.
const FILES: [(&str, &str); 5] = [
    (
        ".claude-plugin/plugin.json",
        include_str!("../../../integrations/claude-code/.claude-plugin/plugin.json"),
    ),
    (
        "hooks/hooks.json",
        include_str!("../../../integrations/claude-code/hooks/hooks.json"),
    ),
    (
        "hooks/register.tsx",
        include_str!("../../../integrations/claude-code/hooks/register.tsx"),
    ),
    (
        "hooks/pane-input.tsx",
        include_str!("../../../integrations/claude-code/hooks/pane-input.tsx"),
    ),
    (
        "skills/cellix-review/SKILL.md",
        include_str!("../../../integrations/claude-code/skills/cellix-review/SKILL.md"),
    ),
];

fn data_dir(var: impl Fn(&str) -> Option<String>) -> Option<PathBuf> {
    let data = var("XDG_DATA_HOME")
        .filter(|d| !d.is_empty())
        .map(PathBuf::from);
    data.or_else(|| Some(PathBuf::from(var("HOME")?).join(".local/share")))
}

pub(crate) fn write_to(folder: &Path) -> io::Result<()> {
    for (name, text) in FILES {
        let path = folder.join(name);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, text)?;
    }
    Ok(())
}

/// Where the Claude Code plugin lives.
pub(crate) fn folder(var: impl Fn(&str) -> Option<String>) -> Option<PathBuf> {
    Some(data_dir(var)?.join("cellix/claude-code"))
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

    #[test]
    fn it_goes_in_the_users_data_folder() {
        let none = |_: &str| None;
        assert_eq!(data_dir(none), None);
        let home = |name: &str| (name == "HOME").then(|| "/h".to_string());
        assert_eq!(data_dir(home), Some(PathBuf::from("/h/.local/share")));
    }
}
