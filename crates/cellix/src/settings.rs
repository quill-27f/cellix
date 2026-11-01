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
        self.keymap = other.keymap.or(self.keymap.take());
        self.tier = other.tier.or(self.tier.take());
        self.browser = other.browser.or(self.browser.take());
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
        "Terraform" => &["terraform-ls", "serve"],
        "YAML" => &["yaml-language-server", "--stdio"],
        "TOML" => &["taplo", "lsp", "stdio"],
        "Shell" => &["bash-language-server", "start"],
        "C" | "C++" => &["clangd"],
        _ => return None,
    };
    Some(command.iter().map(|s| s.to_string()).collect())
}
