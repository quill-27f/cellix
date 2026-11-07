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
    /// `true` wraps long lines of code at the pane's width. Alt+Z changes one file.
    pub word_wrap: Option<bool>,
    /// `false` lets the headers of the scopes scrolled into go by with the rest.
    pub sticky_scroll: Option<bool>,
    /// How many of those headers may stay at the top: 5 unless set, 0 for none.
    pub sticky_scroll_lines: Option<usize>,
    /// `false` leaves code's brackets in the text colour.
    pub bracket_pairs: Option<bool>,
    /// `false` turns off spelling and grammar on pages.
    pub spell_check: Option<bool>,
    /// What opens web pages, before the system's own browser. `terminal-browser` works here.
    pub browser: Option<Vec<String>>,
    /// Language name to server command, overriding the built-in table.
    pub language_servers: HashMap<String, Vec<String>>,
    /// Languages whose support the user said not to offer again.
    pub declined_languages: Vec<String>,
    /// What builds a paper, taking the file last. Without it, the first engine found.
    pub latex_engine: Option<Vec<String>>,
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
        self.spell_check = other.spell_check.or(self.spell_check.take());
        self.bracket_pairs = other.bracket_pairs.or(self.bracket_pairs.take());
        self.word_wrap = other.word_wrap.or(self.word_wrap.take());
        self.sticky_scroll = other.sticky_scroll.or(self.sticky_scroll.take());
        self.sticky_scroll_lines = other
            .sticky_scroll_lines
            .or(self.sticky_scroll_lines.take());
        self.language_servers.extend(other.language_servers);
        self.declined_languages.extend(other.declined_languages);
        self.latex_engine = other.latex_engine.or(self.latex_engine.take());
    }

    /// The words the user has said are right.
    pub fn words_file() -> Option<PathBuf> {
        if cfg!(test) {
            return None;
        }
        let home = PathBuf::from(std::env::var_os("HOME")?);
        Some(home.join(".config/cellix/words.txt"))
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

/// A language server: the languages it covers, how it starts, and how to install it.
pub struct Server {
    pub languages: &'static [&'static str],
    pub command: &'static [&'static str],
    pub install: &'static str,
}

const fn server(
    languages: &'static [&'static str],
    command: &'static [&'static str],
    install: &'static str,
) -> Server {
    Server {
        languages,
        command,
        install,
    }
}

/// The popular servers. A language without one still has colour and word completion.
pub const SERVERS: [Server; 27] = [
    server(
        &["Rust"],
        &["rust-analyzer"],
        "rustup component add rust-analyzer",
    ),
    server(
        &["Python"],
        &["pyright-langserver", "--stdio"],
        "npm install -g pyright",
    ),
    server(
        &["TypeScript", "TSX", "JavaScript"],
        &["typescript-language-server", "--stdio"],
        "npm install -g typescript typescript-language-server",
    ),
    server(
        &["Go"],
        &["gopls"],
        "go install golang.org/x/tools/gopls@latest",
    ),
    server(&["Java"], &["jdtls"], "brew install jdtls"),
    server(
        &["C#"],
        &["csharp-ls"],
        "dotnet tool install --global csharp-ls",
    ),
    server(&["C", "C++"], &["clangd"], "brew install llvm"),
    server(
        &["Kotlin"],
        &["kotlin-language-server"],
        "brew install kotlin-language-server",
    ),
    server(&["Swift"], &["sourcekit-lsp"], "xcode-select --install"),
    server(
        &["PHP"],
        &["intelephense", "--stdio"],
        "npm install -g intelephense",
    ),
    server(&["Ruby"], &["ruby-lsp"], "gem install ruby-lsp"),
    server(
        &["Lua"],
        &["lua-language-server"],
        "brew install lua-language-server",
    ),
    server(&["Scala"], &["metals"], "brew install metals"),
    server(&["Elixir"], &["elixir-ls"], "brew install elixir-ls"),
    server(
        &["Haskell"],
        &["haskell-language-server-wrapper", "--lsp"],
        "brew install haskell-language-server",
    ),
    server(&["Zig"], &["zls"], "brew install zls"),
    server(
        &["Dart"],
        &["dart", "language-server", "--protocol=lsp"],
        "brew install dart",
    ),
    server(
        &["Shell"],
        &["bash-language-server", "start"],
        "npm install -g bash-language-server",
    ),
    server(
        &["Terraform"],
        &["terraform-ls", "serve"],
        "brew install hashicorp/tap/terraform-ls",
    ),
    server(
        &["YAML"],
        &["yaml-language-server", "--stdio"],
        "npm install -g yaml-language-server",
    ),
    server(&["TOML"], &["taplo", "lsp", "stdio"], "brew install taplo"),
    server(
        &["HTML"],
        &["vscode-html-language-server", "--stdio"],
        "npm install -g vscode-langservers-extracted",
    ),
    server(
        &["CSS"],
        &["vscode-css-language-server", "--stdio"],
        "npm install -g vscode-langservers-extracted",
    ),
    server(
        &["JSON"],
        &["vscode-json-language-server", "--stdio"],
        "npm install -g vscode-langservers-extracted",
    ),
    server(
        &["Dockerfile"],
        &["docker-langserver", "--stdio"],
        "npm install -g dockerfile-language-server-nodejs",
    ),
    server(&["LaTeX"], &["texlab"], "brew install texlab tectonic"),
    server(
        &["Markdown"],
        &["marksman", "server"],
        "brew install marksman",
    ),
];

/// The server for a language when settings don't name one.
pub fn default_server(language: &str) -> Option<Vec<String>> {
    let found = SERVERS.iter().find(|s| s.languages.contains(&language))?;
    Some(found.command.iter().map(|s| s.to_string()).collect())
}

/// A formatter that reads a file on stdin and writes it to stdout, for when no language
/// server offers formatting. The file's name goes on the end where a tool needs it.
pub fn formatter(language: &str) -> Option<&'static [&'static str]> {
    Some(match language {
        "Rust" => &["rustfmt", "--edition", "2024"],
        "Go" => &["gofmt"],
        "Python" => &["ruff", "format", "-"],
        "C" | "C++" => &["clang-format"],
        "Shell" => &["shfmt"],
        "Terraform" => &["terraform", "fmt", "-"],
        "Swift" => &["swift-format"],
        "Lua" => &["stylua", "-"],
        "Zig" => &["zig", "fmt", "--stdin"],
        "JavaScript" | "TypeScript" | "TSX" | "JSON" | "HTML" | "CSS" | "YAML" | "Markdown" => {
            &["prettier", "--stdin-filepath"]
        }
        _ => return None,
    })
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
