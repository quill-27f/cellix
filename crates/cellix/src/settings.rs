use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

/// Read from `~/.config/cellix/settings.json`, then `.cellix/settings.json` in the
/// workspace, with later files replacing whole keys.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct Settings {
    pub theme: Option<String>,
    /// `direct`, `placeholder` or `cells`.
    pub tier: Option<String>,
    /// Language name to server command, overriding the built-in table.
    pub language_servers: HashMap<String, Vec<String>>,
}

fn read(path: &Path) -> Option<Settings> {
    serde_json::from_str(&fs::read_to_string(path).ok()?).ok()
}

impl Settings {
    fn merge(&mut self, other: Self) {
        self.theme = other.theme.or(self.theme.take());
        self.tier = other.tier.or(self.tier.take());
        self.language_servers.extend(other.language_servers);
    }

    pub fn load(root: &Path) -> Self {
        let mut settings = Self::default();
        if let Some(home) = std::env::var_os("HOME") {
            let user = PathBuf::from(home).join(".config/cellix/settings.json");
            if let Some(found) = read(&user) {
                settings.merge(found);
            }
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
        _ => return None,
    };
    Some(command.iter().map(|s| s.to_string()).collect())
}
