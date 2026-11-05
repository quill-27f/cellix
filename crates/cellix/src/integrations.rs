//! Putting Cellix into the tools around it. Everything here changes a file the user owns,
//! so it only runs when they ask, keeps whatever else is in that file, and refuses a file
//! it can't read rather than replace it.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use cellix_term::Bleed;
use serde_json::{Map, Value, json};

/// Loads a folder as a plugin in every session, which `--plugin-dir` does for one.
const PLUGIN_DIRS: &str = "CLAUDE_CODE_PLUGIN_DIRS";
/// What separates the folders in it, as in PATH: Windows paths have a `:` of their own.
const LIST: &str = if cfg!(windows) { ";" } else { ":" };
/// Connects to a running Cellix on its own, which `--ide` does for one session.
const AUTO_CONNECT: &str = "CLAUDE_CODE_AUTO_CONNECT_IDE";
/// Where Cellix is, for the plugin to start it by when Claude Code's PATH doesn't have it, as
/// when Claude Code is started from an IDE, a service or by its full path.
const CELLIX_BIN: &str = "CELLIX_BIN";
/// What Claude Code asks of tmux before it shows pictures inside it, read as it starts. In the
/// user's tmux config, so every tmux server has them.
const TMUX_LINES: [&str; 2] = ["set -g allow-passthrough on", "set -g focus-events on"];
const TMUX_NOTE: &str = "# Added by Cellix: Claude Code shows pictures inside tmux with these.";
/// Ghostty keeps Cmd+A (select the terminal's text) and Cmd+D (split) for itself, so an editor
/// inside never sees Select All or Select Next Match.
const GHOSTTY_FREE: [&str; 2] = ["keybind = super+d=unbind", "keybind = super+a=unbind"];
const CODEX_PLUGIN: &str = "cellix@cellix";

fn config_dir(var: &impl Fn(&str) -> Option<String>) -> Option<PathBuf> {
    let chosen = var("CLAUDE_CONFIG_DIR").filter(|d| !d.is_empty());
    chosen
        .map(PathBuf::from)
        .or_else(|| Some(PathBuf::from(var("HOME")?).join(".claude")))
}

pub(crate) fn read_settings(path: &Path) -> io::Result<Map<String, Value>> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Map::new()),
        Err(e) => return Err(e),
    };
    if text.trim().is_empty() {
        return Ok(Map::new());
    }
    match serde_json::from_str(&text) {
        Ok(Value::Object(settings)) => Ok(settings),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{} isn't valid JSON", path.display()),
        )),
    }
}

/// Written beside the file and moved over it, so a failure can't leave half a file.
pub(crate) fn write_settings(path: &Path, settings: &Map<String, Value>) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let text = serde_json::to_string_pretty(settings).map_err(io::Error::other)?;
    let draft = path.with_extension("cellix-new");
    fs::write(&draft, text + "\n")?;
    fs::rename(draft, path)
}

fn folders(env: &Map<String, Value>) -> Vec<String> {
    let listed = env.get(PLUGIN_DIRS).and_then(Value::as_str).unwrap_or("");
    let listed = listed.split(LIST).filter(|d| !d.is_empty());
    listed.map(str::to_string).collect()
}

/// Whether Claude Code loads Cellix without being told to.
pub fn claude_installed(var: impl Fn(&str) -> Option<String>) -> bool {
    let Some(file) = config_dir(&var).map(|d| d.join("settings.json")) else {
        return false;
    };
    let Some(folder) = crate::plugin::folder(&var) else {
        return false;
    };
    let settings = read_settings(&file).unwrap_or_default();
    let env = settings.get("env").and_then(Value::as_object);
    env.is_some_and(|env| folders(env).contains(&folder.display().to_string()))
}

/// Writes the plugin and adds it to the `env` block of Claude Code's user settings.
pub fn install_claude(var: impl Fn(&str) -> Option<String>) -> io::Result<()> {
    let missing = || io::Error::new(io::ErrorKind::NotFound, "no home folder");
    let file = config_dir(&var).ok_or_else(missing)?.join("settings.json");
    let folder = crate::plugin::folder(&var).ok_or_else(missing)?;
    // Before the plugin is written, so a settings file that can't be read stops it all.
    let mut settings = read_settings(&file)?;
    crate::plugin::write_to(&folder)?;
    let env = settings.entry("env").or_insert_with(|| json!({}));
    let Value::Object(env) = env else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "the env block of Claude Code's settings isn't an object",
        ));
    };
    let mut listed = folders(env);
    let ours = folder.display().to_string();
    if !listed.contains(&ours) {
        listed.push(ours);
    }
    env.insert(PLUGIN_DIRS.to_string(), json!(listed.join(LIST)));
    env.insert(AUTO_CONNECT.to_string(), json!("1"));
    if let Some(bin) = cellix_bin(&var) {
        env.insert(CELLIX_BIN.to_string(), json!(bin.display().to_string()));
    }
    write_settings(&file, &settings)?;
    if on_path("tmux", &var) {
        add_tmux_lines(&var)?;
    }
    Ok(())
}

/// The tmux config tmux reads: the one that exists, or the first place it looks.
fn tmux_config(var: &impl Fn(&str) -> Option<String>) -> Option<PathBuf> {
    let home = PathBuf::from(var("HOME")?);
    let classic = home.join(".tmux.conf");
    let xdg = var("XDG_CONFIG_HOME")
        .filter(|d| !d.is_empty())
        .map_or_else(|| home.join(".config"), PathBuf::from)
        .join("tmux/tmux.conf");
    Some(if !classic.exists() && xdg.exists() {
        xdg
    } else {
        classic
    })
}

/// Adds what Claude Code needs to show pictures in tmux, keeping the rest of the config, and
/// only the lines it lacks.
fn add_tmux_lines(var: &impl Fn(&str) -> Option<String>) -> io::Result<()> {
    let Some(path) = tmux_config(var) else {
        return Ok(());
    };
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e),
    };
    let has = |line: &str| text.lines().any(|l| l.trim() == line);
    let missing: Vec<&str> = TMUX_LINES.into_iter().filter(|line| !has(line)).collect();
    if missing.is_empty() {
        return Ok(());
    }
    let mut added = text.clone();
    if !added.is_empty() && !added.ends_with('\n') {
        added.push('\n');
    }
    added.push_str(TMUX_NOTE);
    added.push('\n');
    for line in missing {
        added.push_str(line);
        added.push('\n');
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, added)
}

/// Takes back exactly the lines `add_tmux_lines` wrote, under its note.
fn remove_tmux_lines(var: &impl Fn(&str) -> Option<String>) -> io::Result<()> {
    let Some(path) = tmux_config(var) else {
        return Ok(());
    };
    let Ok(text) = fs::read_to_string(&path) else {
        return Ok(());
    };
    let mut kept = Vec::new();
    let mut ours = false;
    for line in text.lines() {
        if line == TMUX_NOTE {
            ours = true;
            continue;
        }
        if ours && TMUX_LINES.contains(&line) {
            continue;
        }
        ours = false;
        kept.push(line);
    }
    if kept.len() == text.lines().count() {
        return Ok(());
    }
    let mut left = kept.join("\n");
    if !left.is_empty() {
        left.push('\n');
    }
    fs::write(path, left)
}

/// This Cellix, by the path the PATH finds it at when that's the same file, since that path
/// stays put when Homebrew moves the real one on an upgrade.
fn cellix_bin(var: &impl Fn(&str) -> Option<String>) -> Option<PathBuf> {
    let running = std::env::current_exe().ok()?;
    let same = |path: &Path| fs::canonicalize(path).ok() == fs::canonicalize(&running).ok();
    let name = if cfg!(windows) {
        "cellix.exe"
    } else {
        "cellix"
    };
    let path = var("PATH").unwrap_or_default();
    let found = std::env::split_paths(&path)
        .filter(|d| !d.as_os_str().is_empty())
        .map(|d| d.join(name))
        .find(|candidate| candidate.is_file());
    Some(found.filter(|f| same(f)).unwrap_or(running))
}

/// Takes back exactly what `install_claude` added, and leaves the plugin's files.
pub fn remove_claude(var: impl Fn(&str) -> Option<String>) -> io::Result<()> {
    let missing = || io::Error::new(io::ErrorKind::NotFound, "no home folder");
    let file = config_dir(&var).ok_or_else(missing)?.join("settings.json");
    let folder = crate::plugin::folder(&var).ok_or_else(missing)?;
    let mut settings = read_settings(&file)?;
    let Some(Value::Object(env)) = settings.get_mut("env") else {
        return Ok(());
    };
    let ours = folder.display().to_string();
    let left: Vec<String> = folders(env).into_iter().filter(|d| *d != ours).collect();
    if left.is_empty() {
        env.remove(PLUGIN_DIRS);
    } else {
        env.insert(PLUGIN_DIRS.to_string(), json!(left.join(LIST)));
    }
    env.remove(AUTO_CONNECT);
    env.remove(CELLIX_BIN);
    if env.is_empty() {
        settings.remove("env");
    }
    write_settings(&file, &settings)?;
    remove_tmux_lines(&var)
}

/// Runs an agent's own command, so its config stays its business, and returns what went
/// wrong in its words.
fn run(program: &str, args: &[&str]) -> io::Result<()> {
    let done = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .output()?;
    if done.status.success() {
        return Ok(());
    }
    let said = String::from_utf8_lossy(&done.stderr);
    let first = said
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("it refused");
    Err(io::Error::other(first.trim().to_string()))
}

fn codex_home(var: &impl Fn(&str) -> Option<String>) -> Option<PathBuf> {
    let chosen = var("CODEX_HOME").filter(|d| !d.is_empty());
    chosen
        .map(PathBuf::from)
        .or_else(|| Some(PathBuf::from(var("HOME")?).join(".codex")))
}

/// Whether Codex has Cellix's plugin, which it records in its own config.
pub fn codex_installed(var: impl Fn(&str) -> Option<String>) -> bool {
    let config = codex_home(&var).map(|home| home.join("config.toml"));
    let text = config.and_then(|c| fs::read_to_string(c).ok());
    text.is_some_and(|t| t.contains(CODEX_PLUGIN))
}

/// Adds Cellix as a local marketplace and installs its plugin, with Codex's own commands.
pub fn install_codex(var: impl Fn(&str) -> Option<String>) -> io::Result<()> {
    let folder = crate::plugin::write_codex(var)?;
    let source = folder.display().to_string();
    run("codex", &["plugin", "marketplace", "add", &source])?;
    run("codex", &["plugin", "add", CODEX_PLUGIN])
}

pub fn remove_codex() -> io::Result<()> {
    run("codex", &["plugin", "remove", CODEX_PLUGIN])?;
    run("codex", &["plugin", "marketplace", "remove", "cellix"])
}

/// Whether Gemini CLI has Cellix's extension, which it keeps in a folder of its own.
pub fn gemini_installed(var: impl Fn(&str) -> Option<String>) -> bool {
    let home = var("HOME").map(PathBuf::from);
    home.is_some_and(|h| h.join(".gemini/extensions/cellix").exists())
}

pub fn install_gemini(var: impl Fn(&str) -> Option<String>) -> io::Result<()> {
    let folder = crate::plugin::write_gemini(var)?;
    let source = folder.display().to_string();
    // `--consent` doesn't cover its folder trust question. The user's click answered that.
    let mut child = Command::new("gemini")
        .args(["extensions", "install", &source, "--consent"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(b"y\n")?;
    }
    let done = child.wait_with_output()?;
    if done.status.success() {
        return Ok(());
    }
    let said = String::from_utf8_lossy(&done.stderr);
    let last = said.lines().rfind(|l| !l.trim().is_empty());
    Err(io::Error::other(
        last.unwrap_or("it refused").trim().to_string(),
    ))
}

pub fn remove_gemini() -> io::Result<()> {
    run("gemini", &["extensions", "uninstall", "cellix"])
}

/// Whether a program can be run by name.
pub fn on_path(program: &str, var: impl Fn(&str) -> Option<String>) -> bool {
    let path = var("PATH").unwrap_or_default();
    // Windows runs `codex` as `codex.exe`, or `codex.cmd` where npm installed it.
    let names: Vec<String> = if cfg!(windows) {
        ["", ".exe", ".cmd", ".bat"]
            .iter()
            .map(|ending| format!("{program}{ending}"))
            .collect()
    } else {
        vec![program.to_string()]
    };
    std::env::split_paths(&path)
        .filter(|d| !d.as_os_str().is_empty())
        .any(|d| names.iter().any(|name| d.join(name).is_file()))
}

/// Ghostty's config file: the one that exists, or the first place it looks.
fn ghostty_config(var: &impl Fn(&str) -> Option<String>) -> Option<PathBuf> {
    let home = PathBuf::from(var("HOME")?);
    let xdg = var("XDG_CONFIG_HOME")
        .filter(|d| !d.is_empty())
        .map_or_else(|| home.join(".config"), PathBuf::from)
        .join("ghostty/config");
    let mac = home.join("Library/Application Support/com.mitchellh.ghostty/config");
    Some(if !xdg.exists() && mac.exists() {
        mac
    } else {
        xdg
    })
}

/// Whether this is Ghostty, and it still keeps Cmd+D for itself.
pub fn ghostty_keeps_cmd_d(var: impl Fn(&str) -> Option<String>) -> Option<bool> {
    let ghostty = var("TERM").is_some_and(|t| t.contains("ghostty"));
    let config = ghostty_config(&var).filter(|_| ghostty)?;
    let text = fs::read_to_string(config).unwrap_or_default();
    let has = |wanted: &str| {
        text.lines()
            .any(|line| line.replace(' ', "") == wanted.replace(' ', ""))
    };
    Some(!GHOSTTY_FREE.iter().all(|line| has(line)))
}

/// How far tiles should reach past the text area. Only Ghostty is known to draw images over
/// its padding. The bottom padding isn't reported, so the band is taken as the row, the
/// configured padding, and half a row of what's left over, which is the average.
pub fn window_bleed(var: impl Fn(&str) -> Option<String>, cell_height: usize) -> Bleed {
    if !var("TERM").is_some_and(|t| t.contains("ghostty")) {
        return Bleed::default();
    }
    let text = ghostty_config(&var)
        .and_then(|path| fs::read_to_string(path).ok())
        .unwrap_or_default();
    let padding = text
        .lines()
        .filter_map(|line| line.split_once('='))
        .filter(|(key, _)| key.trim() == "window-padding-y")
        .filter_map(|(_, value)| value.rsplit(',').next()?.trim().parse::<usize>().ok())
        .next_back()
        .unwrap_or(2);
    // Points in the config, and pixels here. Retina screens have tall cells.
    let scale = if cell_height >= 24 { 2 } else { 1 };
    Bleed {
        cols: 8,
        rows: 3,
        drop: (padding * scale + cell_height / 2) / 2,
    }
}

fn append_to_ghostty(lines: &[&str], var: &impl Fn(&str) -> Option<String>) -> io::Result<()> {
    let missing = || io::Error::new(io::ErrorKind::NotFound, "no home folder");
    let config = ghostty_config(var).ok_or_else(missing)?;
    if let Some(parent) = config.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut text = match fs::read_to_string(&config) {
        Ok(text) => text,
        Err(e) if e.kind() == io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e),
    };
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    for line in lines {
        text.push_str(line);
        text.push('\n');
    }
    fs::write(config, text)
}

/// Adds the line that lets Cmd+D through. Ghostty picks it up when its config reloads.
pub fn free_cmd_d(var: impl Fn(&str) -> Option<String>) -> io::Result<()> {
    let config = ghostty_config(&var).and_then(|path| fs::read_to_string(path).ok());
    let text = config.unwrap_or_default();
    let has = |wanted: &str| {
        text.lines()
            .any(|line| line.replace(' ', "") == wanted.replace(' ', ""))
    };
    let missing: Vec<&str> = GHOSTTY_FREE.iter().copied().filter(|l| !has(l)).collect();
    append_to_ghostty(&missing, &var)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn home() -> (TempDir, impl Fn(&str) -> Option<String>) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().display().to_string();
        let var = move |name: &str| match name {
            "HOME" => Some(root.clone()),
            "TERM" => Some("xterm-ghostty".to_string()),
            _ => None,
        };
        (dir, var)
    }

    fn claude_file(dir: &TempDir) -> PathBuf {
        dir.path().join(".claude/settings.json")
    }

    #[test]
    fn installing_leaves_the_rest_of_claudes_settings_alone() {
        let (dir, var) = home();
        fs::create_dir_all(dir.path().join(".claude")).unwrap();
        let before = r#"{"model":"opus","env":{"FOO":"1","CLAUDE_CODE_PLUGIN_DIRS":"/other"}}"#;
        fs::write(claude_file(&dir), before).unwrap();
        assert!(!claude_installed(&var));
        install_claude(&var).unwrap();
        assert!(claude_installed(&var));

        let after: Value =
            serde_json::from_str(&fs::read_to_string(claude_file(&dir)).unwrap()).unwrap();
        assert_eq!(after["model"], "opus");
        assert_eq!(after["env"]["FOO"], "1");
        assert_eq!(after["env"][AUTO_CONNECT], "1");
        let bin = after["env"][CELLIX_BIN].as_str().unwrap();
        assert!(
            Path::new(bin).is_file(),
            "where the plugin finds Cellix off the PATH: {bin}"
        );
        let dirs = after["env"][PLUGIN_DIRS].as_str().unwrap();
        assert!(dirs.starts_with(&format!("/other{LIST}")), "{dirs}");
        let plugin = crate::plugin::folder(&var).unwrap();
        assert!(plugin.join(".claude-plugin/plugin.json").exists());

        install_claude(&var).unwrap();
        let again = fs::read_to_string(claude_file(&dir)).unwrap();
        assert_eq!(again.matches("cellix/claude-code").count(), 1, "only once");

        remove_claude(&var).unwrap();
        assert!(!claude_installed(&var));
        let removed: Value =
            serde_json::from_str(&fs::read_to_string(claude_file(&dir)).unwrap()).unwrap();
        assert_eq!(
            removed,
            json!({"model": "opus", "env": {"FOO": "1", PLUGIN_DIRS: "/other"}})
        );
    }

    #[test]
    fn a_settings_file_it_cant_read_is_never_replaced() {
        let (dir, var) = home();
        fs::create_dir_all(dir.path().join(".claude")).unwrap();
        fs::write(claude_file(&dir), "{ not json").unwrap();
        assert!(install_claude(&var).is_err());
        assert!(remove_claude(&var).is_err());
        assert_eq!(fs::read_to_string(claude_file(&dir)).unwrap(), "{ not json");
        assert!(
            !crate::plugin::folder(&var).unwrap().exists(),
            "nothing half done"
        );
    }

    /// Claude Code shows pictures inside tmux only with these on as it starts, so they go in
    /// the user's tmux config, under a note, keeping what was there; removing takes back
    /// just those.
    #[test]
    fn tmux_gets_what_claude_code_needs_for_pictures() {
        let (dir, home_var) = home();
        let bin = dir.path().join("bin");
        fs::create_dir_all(&bin).unwrap();
        fs::write(bin.join("tmux"), "").unwrap();
        let path = bin.display().to_string();
        let var = |name: &str| {
            if name == "PATH" {
                Some(path.clone())
            } else {
                home_var(name)
            }
        };
        let conf = dir.path().join(".tmux.conf");
        fs::write(&conf, "set -g mouse on\nset -g focus-events on\n").unwrap();
        install_claude(var).unwrap();
        let text = fs::read_to_string(&conf).unwrap();
        assert_eq!(
            text,
            format!(
                "set -g mouse on\nset -g focus-events on\n{TMUX_NOTE}\nset -g allow-passthrough on\n"
            ),
            "only what was missing"
        );
        install_claude(var).unwrap();
        assert_eq!(fs::read_to_string(&conf).unwrap(), text, "once");
        remove_claude(var).unwrap();
        assert_eq!(
            fs::read_to_string(&conf).unwrap(),
            "set -g mouse on\nset -g focus-events on\n",
            "and the user's own lines stay"
        );
        // Without tmux, its config isn't touched.
        let (dir, var) = home();
        install_claude(var).unwrap();
        assert!(!dir.path().join(".tmux.conf").exists());
    }

    #[test]
    fn installing_works_with_no_settings_yet() {
        let (_dir, var) = home();
        install_claude(&var).unwrap();
        assert!(claude_installed(&var));
        remove_claude(&var).unwrap();
        assert!(!claude_installed(&var));
    }

    #[test]
    fn cmd_d_is_freed_once_and_keeps_the_config() {
        let (dir, var) = home();
        let config = dir.path().join(".config/ghostty/config");
        fs::create_dir_all(config.parent().unwrap()).unwrap();
        fs::write(&config, "font-size = 14").unwrap();
        assert_eq!(ghostty_keeps_cmd_d(&var), Some(true));
        free_cmd_d(&var).unwrap();
        assert_eq!(ghostty_keeps_cmd_d(&var), Some(false));
        assert_eq!(
            fs::read_to_string(&config).unwrap(),
            format!("font-size = 14\n{}\n{}\n", GHOSTTY_FREE[0], GHOSTTY_FREE[1]),
            "Cmd+D and Cmd+A, each once"
        );
        free_cmd_d(&var).unwrap();
        assert_eq!(
            fs::read_to_string(&config)
                .unwrap()
                .matches("unbind")
                .count(),
            2
        );
    }

    #[test]
    fn tiles_only_leave_the_text_area_on_ghostty() {
        let (dir, var) = home();
        fs::create_dir_all(dir.path().join(".config/ghostty")).unwrap();
        fs::write(
            dir.path().join(".config/ghostty/config"),
            "font-size = 14\nwindow-padding-y = 8\n",
        )
        .unwrap();
        let bleed = window_bleed(&var, 36);
        assert!(bleed.cols > 0 && bleed.rows > 0);
        assert_eq!(bleed.drop, (16 + 18) / 2);
        let other = |name: &str| (name == "TERM").then(|| "xterm-kitty".to_string());
        assert_eq!(window_bleed(other, 36), Bleed::default());
    }

    #[test]
    fn a_codex_install_is_read_from_its_config() {
        let (dir, var) = home();
        assert!(!codex_installed(&var));
        fs::create_dir_all(dir.path().join(".codex")).unwrap();
        let config = "[plugins.\"cellix@cellix\"]\nenabled = true\n";
        fs::write(dir.path().join(".codex/config.toml"), config).unwrap();
        assert!(codex_installed(&var));
    }

    #[test]
    fn other_terminals_have_nothing_to_free() {
        let (_dir, var) = home();
        let elsewhere = |name: &str| (name != "TERM").then(|| var(name)).flatten();
        assert_eq!(ghostty_keeps_cmd_d(elsewhere), None);
    }

    #[test]
    fn programs_are_found_on_the_path() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("codex"), "").unwrap();
        let root = dir.path().display().to_string();
        let path = std::env::join_paths(["/nowhere", &root]).unwrap();
        let var = move |_: &str| Some(path.to_string_lossy().into_owned());
        assert!(on_path("codex", &var));
        assert!(!on_path("claude", &var));
    }
}
