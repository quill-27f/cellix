use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::install::{Release, Way};

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
    /// `false` keeps what shows something under way still: a dot where a spinner would turn.
    pub motion: Option<bool>,
    /// `false` keeps the caret still, in files, pages, text boxes and terminals. It blinks
    /// otherwise, as VS Code's does.
    pub cursor_blink: Option<bool>,
    /// How tall a row is where Cellix can set it: `airy` (the default), `comfortable`, or
    /// `terminal` for the terminal's own line, which is what a row is everywhere else.
    pub row_height: Option<String>,
    /// `false` hides the minimap, however wide the window.
    pub minimap: Option<bool>,
    /// `true` wraps long lines of code at the pane's width. Alt+Z changes one file.
    pub word_wrap: Option<bool>,
    /// `false` lets a page's text run the width of its pane, however wide. Otherwise it
    /// keeps to 88 columns, in the middle of a pane wider than that.
    pub reading_width: Option<bool>,
    /// `false` lets the headers of the scopes scrolled into go by with the rest.
    pub sticky_scroll: Option<bool>,
    /// How many of those headers may stay at the top: 5 unless set, 0 for none.
    pub sticky_scroll_lines: Option<usize>,
    /// `false` leaves code's brackets in the text colour.
    pub bracket_pairs: Option<bool>,
    /// `false` leaves out the types and parameter names the language server would show
    /// inside the lines.
    pub inlay_hints: Option<bool>,
    /// `false` turns off spelling and grammar on pages.
    pub spell_check: Option<bool>,
    /// `false` keeps Cellix from telling the terminal when something needs the person.
    pub notify: Option<bool>,
    /// What opens web pages, before the system's own browser. `terminal-browser` works here.
    pub browser: Option<Vec<String>>,
    /// Language name to server command, overriding the built-in table.
    pub language_servers: HashMap<String, Vec<String>>,
    /// Language name to a formatter that reads the file on stdin and writes it to stdout,
    /// overriding the built-in one.
    pub formatters: HashMap<String, Vec<String>>,
    /// Keys set in place of a command's own, by the command's name in the palette. An empty
    /// list takes its keys away.
    pub keybindings: HashMap<String, Vec<String>>,
    /// Languages whose support the user said not to offer again.
    pub declined_languages: Vec<String>,
    /// What builds a paper, taking the file last. Without it, the first engine found.
    pub latex_engine: Option<Vec<String>>,
    /// Formats a file as it's saved, with its language server or formatter.
    pub format_on_save: Option<bool>,
    /// `off` (the default), `after_delay`, or `on_focus_change`, which saves a file as its
    /// editor loses the keys and as the terminal loses focus.
    pub auto_save: Option<String>,
    /// How long after the last change `after_delay` saves, in milliseconds.
    pub auto_save_delay: Option<u64>,
    /// The order the Comments pane lists in: `document` (the default), `newest` or `oldest`.
    /// Picked from the pane's menu, it's kept here for every session on the machine.
    pub comment_sort: Option<String>,
    /// Which agent's command line answers Ask's side questions, Polish and Validate:
    /// `claude`, `codex` or `gemini`. Unset or `auto`, the kind of agent in the side pane,
    /// else the first installed. Settings › Integrations calls it the Default agent, and
    /// `default_agent` names it too. Remote docs are read and posted through Claude Code
    /// whatever it says.
    #[serde(alias = "default_agent")]
    pub ask_agent: Option<String>,
    /// The MCP server, among those Claude Code knows, that a remote doc is read and posted
    /// through: its tools are the only ones the agent is allowed. Unset, the first review asks.
    pub remote_docs_mcp: Option<String>,
    /// The person's own agent runners: a command like `claude-ds` that wraps an agent's CLI,
    /// treated everywhere as the agent `like` names.
    pub agents: Vec<CustomAgent>,
    /// Agents turned off, so they're offered nowhere: a built-in's by its command, a custom
    /// one's by the name given it.
    pub disabled_agents: Vec<String>,
}

/// One of the person's own agent runners, from the `agents` setting: `command` runs the
/// agent, and `like` says which of Cellix's agents it works like, so it gets that one's
/// marks, flags and treatment. The command is a name on the PATH or a path; Cellix never
/// reads the runner itself, only runs it.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default)]
pub struct CustomAgent {
    pub name: String,
    pub command: String,
    pub like: String,
}

/// The agent choices `ask_agent` can name, in the order Settings shows them.
pub const DEFAULT_AGENTS: [(&str, &str); 4] = [
    ("auto", "Auto"),
    ("claude", "Claude"),
    ("codex", "Codex"),
    ("gemini", "Gemini"),
];

/// Which default agent choice `ask_agent` names, whatever was written in it.
pub fn default_agent_name(setting: Option<&str>) -> &'static str {
    match setting.and_then(crate::ask::Cli::from_setting) {
        Some(crate::ask::Cli::Claude) => "claude",
        Some(crate::ask::Cli::Codex) => "codex",
        Some(crate::ask::Cli::Gemini) => "gemini",
        None => "auto",
    }
}

/// The row heights `row_height` can name, with how many of the terminal's lines each is
/// tall. The first is what an unset one means.
pub const ROW_HEIGHTS: [(&str, f32); 3] = [("airy", 1.4), ("comfortable", 1.2), ("terminal", 1.0)];

/// The row height a `row_height` setting names, whatever was written in it.
pub fn row_height_name(setting: Option<&str>) -> &'static str {
    let named = ROW_HEIGHTS.iter().find(|(name, _)| Some(*name) == setting);
    named.unwrap_or(&ROW_HEIGHTS[0]).0
}

/// How many of the terminal's lines a row is tall under that setting.
pub fn row_height_times(setting: Option<&str>) -> f32 {
    let name = row_height_name(setting);
    let named = ROW_HEIGHTS.iter().find(|(known, _)| *known == name);
    named.map_or(1.0, |(_, times)| *times)
}

fn read(path: &Path) -> Option<Settings> {
    serde_json::from_str(&fs::read_to_string(path).ok()?).ok()
}

impl Settings {
    fn merge(&mut self, other: Self) {
        self.theme = other.theme.or(self.theme.take());
        self.smooth_scroll = other.smooth_scroll.or(self.smooth_scroll.take());
        self.motion = other.motion.or(self.motion.take());
        self.cursor_blink = other.cursor_blink.or(self.cursor_blink.take());
        self.row_height = other.row_height.or(self.row_height.take());
        self.minimap = other.minimap.or(self.minimap.take());
        self.inlay_hints = other.inlay_hints.or(self.inlay_hints.take());
        self.keymap = other.keymap.or(self.keymap.take());
        self.tier = other.tier.or(self.tier.take());
        self.browser = other.browser.or(self.browser.take());
        self.spell_check = other.spell_check.or(self.spell_check.take());
        self.bracket_pairs = other.bracket_pairs.or(self.bracket_pairs.take());
        self.word_wrap = other.word_wrap.or(self.word_wrap.take());
        self.reading_width = other.reading_width.or(self.reading_width.take());
        self.sticky_scroll = other.sticky_scroll.or(self.sticky_scroll.take());
        self.sticky_scroll_lines = other
            .sticky_scroll_lines
            .or(self.sticky_scroll_lines.take());
        self.notify = other.notify.or(self.notify.take());
        self.language_servers.extend(other.language_servers);
        self.formatters.extend(other.formatters);
        self.keybindings.extend(other.keybindings);
        self.declined_languages.extend(other.declined_languages);
        self.latex_engine = other.latex_engine.or(self.latex_engine.take());
        self.format_on_save = other.format_on_save.or(self.format_on_save.take());
        self.auto_save = other.auto_save.or(self.auto_save.take());
        self.auto_save_delay = other.auto_save_delay.or(self.auto_save_delay.take());
        self.comment_sort = other.comment_sort.or(self.comment_sort.take());
        self.ask_agent = other.ask_agent.or(self.ask_agent.take());
        self.remote_docs_mcp = other.remote_docs_mcp.or(self.remote_docs_mcp.take());
        self.agents.extend(other.agents);
        self.disabled_agents.extend(other.disabled_agents);
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
    pub(crate) fn user_file() -> Option<PathBuf> {
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

    pub(crate) fn store_in(
        file: &Path,
        key: &str,
        value: serde_json::Value,
    ) -> std::io::Result<()> {
        let mut settings = crate::integrations::read_settings(file)?;
        settings.insert(key.to_string(), value);
        crate::integrations::write_settings(file, &settings)
    }

    pub fn load(root: &Path) -> Self {
        // Tests read nobody's own settings: an order or a keymap picked on the machine would
        // change what they see.
        let user = if cfg!(test) { None } else { Self::user_file() };
        Self::load_with(user.as_deref(), root)
    }

    /// The same, with the user's own file at `user`, if there's one.
    pub(crate) fn load_with(user: Option<&Path>, root: &Path) -> Self {
        let mut settings = Self::default();
        if let Some(found) = user.and_then(read) {
            settings.merge(found);
        }
        if let Some(found) = read(&root.join(".cellix/settings.json")) {
            settings.merge(found);
        }
        settings
    }
}

/// A language server: the languages it covers, how it starts, and the ways to install it,
/// in the order they're tried (see `install`).
pub struct Server {
    pub languages: &'static [&'static str],
    pub command: &'static [&'static str],
    pub ways: &'static [Way],
}

const fn server(
    languages: &'static [&'static str],
    command: &'static [&'static str],
    ways: &'static [Way],
) -> Server {
    Server {
        languages,
        command,
        ways,
    }
}

/// clangd as each Linux's packages name it.
const CLANGD: &[(&str, &str)] = &[
    ("dnf", "clang-tools-extra"),
    ("apt", "clangd"),
    ("pacman", "clang"),
    ("zypper", "clang-tools"),
    ("apk", "clang-extra-tools"),
];

/// The popular servers. A language without one still has colour and word completion.
pub const SERVERS: [Server; 27] = [
    server(
        &["Rust"],
        &["rust-analyzer"],
        &[
            Way::Tool("rustup component add rust-analyzer", "rustup"),
            Way::Brew("rust-analyzer"),
            Way::Release(Release::RustAnalyzer),
        ],
    ),
    server(
        &["Python"],
        &["pyright-langserver", "--stdio"],
        &[Way::Npm("pyright"), Way::Brew("pyright")],
    ),
    server(
        &["TypeScript", "TSX", "JavaScript"],
        &["typescript-language-server", "--stdio"],
        &[
            Way::Npm("typescript typescript-language-server"),
            Way::Brew("typescript-language-server"),
        ],
    ),
    server(
        &["Go"],
        &["gopls"],
        &[
            Way::Tool("go install golang.org/x/tools/gopls@latest", "Go"),
            Way::Brew("gopls"),
        ],
    ),
    server(&["Java"], &["jdtls"], &[Way::Brew("jdtls")]),
    server(
        &["C#"],
        &["csharp-ls"],
        &[Way::Tool(
            "dotnet tool install --global csharp-ls",
            "the .NET SDK",
        )],
    ),
    server(
        &["C", "C++"],
        &["clangd"],
        &[Way::Brew("llvm"), Way::System(CLANGD)],
    ),
    server(
        &["Kotlin"],
        &["kotlin-language-server"],
        &[Way::Brew("kotlin-language-server")],
    ),
    server(
        &["Swift"],
        &["sourcekit-lsp"],
        &[
            Way::Mac("xcode-select --install"),
            Way::Needs("Swift, from swift.org"),
        ],
    ),
    server(
        &["PHP"],
        &["intelephense", "--stdio"],
        &[Way::Npm("intelephense")],
    ),
    server(
        &["Ruby"],
        &["ruby-lsp"],
        &[Way::Tool(
            "gem install --user-install --bindir ~/.local/bin ruby-lsp",
            "Ruby",
        )],
    ),
    server(
        &["Lua"],
        &["lua-language-server"],
        &[Way::Brew("lua-language-server")],
    ),
    server(&["Scala"], &["metals"], &[Way::Brew("metals")]),
    server(&["Elixir"], &["elixir-ls"], &[Way::Brew("elixir-ls")]),
    server(
        &["Haskell"],
        &["haskell-language-server-wrapper", "--lsp"],
        &[Way::Brew("haskell-language-server")],
    ),
    server(
        &["Zig"],
        &["zls"],
        &[Way::Brew("zls"), Way::Release(Release::Zls)],
    ),
    server(
        &["Dart"],
        &["dart", "language-server", "--protocol=lsp"],
        &[Way::Brew("dart"), Way::Needs("the Dart SDK, from dart.dev")],
    ),
    server(
        &["Shell"],
        &["bash-language-server", "start"],
        &[
            Way::Npm("bash-language-server"),
            Way::Brew("bash-language-server"),
        ],
    ),
    server(
        &["Terraform"],
        &["terraform-ls", "serve"],
        &[
            Way::Brew("hashicorp/tap/terraform-ls"),
            Way::Release(Release::TerraformLs),
        ],
    ),
    server(
        &["YAML"],
        &["yaml-language-server", "--stdio"],
        &[
            Way::Npm("yaml-language-server"),
            Way::Brew("yaml-language-server"),
        ],
    ),
    server(
        &["TOML"],
        &["taplo", "lsp", "stdio"],
        &[Way::Brew("taplo"), Way::Release(Release::Taplo)],
    ),
    server(
        &["HTML"],
        &["vscode-html-language-server", "--stdio"],
        &[Way::Npm("vscode-langservers-extracted")],
    ),
    server(
        &["CSS"],
        &["vscode-css-language-server", "--stdio"],
        &[Way::Npm("vscode-langservers-extracted")],
    ),
    server(
        &["JSON"],
        &["vscode-json-language-server", "--stdio"],
        &[Way::Npm("vscode-langservers-extracted")],
    ),
    server(
        &["Dockerfile"],
        &["docker-langserver", "--stdio"],
        &[Way::Npm("dockerfile-language-server-nodejs")],
    ),
    server(
        &["LaTeX"],
        &["texlab"],
        &[Way::Brew("texlab tectonic"), Way::Release(Release::Texlab)],
    ),
    server(
        &["Markdown"],
        &["marksman", "server"],
        &[Way::Brew("marksman"), Way::Release(Release::Marksman)],
    ),
];

/// The server for a language when settings don't name one.
pub fn default_server(language: &str) -> Option<Vec<String>> {
    let found = SERVERS.iter().find(|s| s.languages.contains(&language))?;
    Some(found.command.iter().map(|s| s.to_string()).collect())
}

impl Settings {
    /// The formatter for a language: the one settings name, or the built-in one.
    pub fn formatter(&self, language: &str) -> Option<Vec<String>> {
        if let Some(own) = self.formatters.get(language) {
            return Some(own.clone());
        }
        formatter(language).map(|tool| tool.iter().map(|s| s.to_string()).collect())
    }
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

    /// The comment order is read from the user's file, and a workspace's own file overrides
    /// it, as every other key.
    #[test]
    fn a_workspaces_comment_order_overrides_the_users() {
        let home = tempfile::tempdir().unwrap();
        let user = home.path().join("settings.json");
        fs::write(&user, r#"{"comment_sort":"newest"}"#).unwrap();
        let root = tempfile::tempdir().unwrap();
        let read = Settings::load_with(Some(&user), root.path());
        assert_eq!(read.comment_sort.as_deref(), Some("newest"));
        fs::create_dir_all(root.path().join(".cellix")).unwrap();
        let own = r#"{"comment_sort":"oldest"}"#;
        fs::write(root.path().join(".cellix/settings.json"), own).unwrap();
        let read = Settings::load_with(Some(&user), root.path());
        assert_eq!(read.comment_sort.as_deref(), Some("oldest"));
    }
}
