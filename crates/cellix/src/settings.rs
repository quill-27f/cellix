use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

/// Read from `~/.config/cellix/settings.json`, then `.cellix/settings.json` in the
/// workspace, with later files replacing whole keys.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct Settings {
    /// `auto` (the default) follows the terminal. `dark` and `light` choose.
    pub theme: Option<String>,
    /// `simple` (the default), `vim`, or `neovim`, which needs `nvim` on the host.
    pub keymap: Option<String>,
    /// `direct`, `placeholder` or `cells`.
    pub tier: Option<String>,
    /// `false` makes the wheel jump by lines, for a slow link.
    pub smooth_scroll: Option<bool>,
    /// `false` hides the minimap, however wide the window.
    pub minimap: Option<bool>,
    /// What opens web pages. Without it, terminal-browser opens them in a split.
    pub browser: Option<Vec<String>>,
    /// Language name to server command, overriding the built-in table.
    pub language_servers: HashMap<String, Vec<String>>,
}

fn read(path: &Path) -> Option<Settings> {
    serde_json::from_str(&fs::read_to_string(path).ok()?).ok()
}

impl Settings {
    fn merge(&mut self, other: Self) {
        self.theme = other.theme.or(self.theme.take());
        self.smooth_scroll = other.smooth_scroll.or(self.smooth_scroll.take());
        self.minimap = other.minimap.or(self.minimap.take());
        self.keymap = other.keymap.or(self.keymap.take());
        self.tier = other.tier.or(self.tier.take());
        self.browser = other.browser.or(self.browser.take());
        self.language_servers.extend(other.language_servers);
    }

    /// The user's own settings file.
    fn user_file() -> Option<PathBuf> {
        let home = PathBuf::from(std::env::var_os("HOME")?);
        Some(home.join(".config/cellix/settings.json"))
    }

    /// Saves one setting to the user's file, keeping everything else in it.
    pub fn store(key: &str, value: serde_json::Value) -> std::io::Result<()> {
        // Tests click through the page, and must not reach a real home folder.
        if cfg!(test) {
            return Ok(());
        }
        let file = Self::user_file().ok_or(std::io::ErrorKind::NotFound)?;
        Self::store_in(&file, key, value)
    }

    fn store_in(file: &Path, key: &str, value: serde_json::Value) -> std::io::Result<()> {
        let mut settings = crate::integrations::read_settings(file)?;
        settings.insert(key.to_string(), value);
        crate::integrations::write_settings(file, &settings)
    }

    pub fn load(root: &Path) -> Self {
        let mut settings = Self::default();
        if let Some(found) = Self::user_file().as_deref().and_then(read) {
            settings.merge(found);
        }
        if let Some(found) = read(&root.join(".cellix/settings.json")) {
            settings.merge(found);
        }
        settings
    }
}

/// The server for a language when settings don't name one.
pub fn default_server(language: &str) -> Option<Vec<String>> {
    let command: &[&str] = match language {
        "Rust" => &["rust-analyzer"],
        "Python" => &["pyright-langserver", "--stdio"],
        "Go" => &["gopls"],
        "TypeScript" | "TSX" | "JavaScript" => &["typescript-language-server", "--stdio"],
        "Terraform" => &["terraform-ls", "serve"],
        "YAML" => &["yaml-language-server", "--stdio"],
        "TOML" => &["taplo", "lsp", "stdio"],
        "Shell" => &["bash-language-server", "start"],
        "C" | "C++" => &["clangd"],
        _ => return None,
    };
    Some(command.iter().map(|s| s.to_string()).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_change_keeps_the_rest_of_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("cellix/settings.json");
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(&file, r#"{"keymap":"vim"}"#).unwrap();
        Settings::store_in(&file, "theme", "light".into()).unwrap();
        let saved = read(&file).unwrap();
        assert_eq!(saved.theme.as_deref(), Some("light"));
        assert_eq!(saved.keymap.as_deref(), Some("vim"));
    }
}
