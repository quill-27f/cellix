//! What an agent works from: the instruction files it loads and in what order, its skills,
//! subagents, commands, hooks and MCP servers, what a session of it read, and checks on all of
//! that. Every file here is read as data; nothing in one is ever followed. Nothing here writes.
//!
//! Each agent's rules are its own, so each has its own discovery. With a transcript, what the
//! agent says it loaded wins over what the files suggest (`transcript`).

mod checks;
mod imports;
mod transcript;

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

pub use checks::{Check, CheckKind, checks};
pub use imports::{IMPORT_DEPTH, imports};
pub use transcript::{
    Breakdown, FileRead, Loaded, Search, Session, claude_window, read_session, transcript_path,
};

/// Claude Code loads only this many lines of its auto memory's `MEMORY.md`.
pub const MEMORY_LINES: usize = 200;
/// Codex stops reading project instructions after this many bytes (`project_doc_max_bytes`).
pub const CODEX_BYTES: usize = 32 * 1024;
/// Claude Code warns of an instruction file past this many characters.
pub const LARGE_CHARS: usize = 40_000;
/// How deep below the project to look for files an agent reads only when it works there.
const NESTED_DEPTH: usize = 4;
/// Folders never looked in for nested instructions: build output and tools' own.
const SKIPPED: [&str; 7] = [
    "target",
    "node_modules",
    ".git",
    "dist",
    "build",
    ".venv",
    "vendor",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Agent {
    Claude,
    Codex,
    Gemini,
}

impl Agent {
    pub const ALL: [Agent; 3] = [Agent::Claude, Agent::Codex, Agent::Gemini];

    pub fn name(self) -> &'static str {
        match self {
            Agent::Claude => "Claude Code",
            Agent::Codex => "Codex",
            Agent::Gemini => "Gemini CLI",
        }
    }

    /// The agent Cellix names `name`, as its tabs do.
    pub fn named(name: &str) -> Option<Agent> {
        Agent::ALL.into_iter().find(|a| a.name() == name)
    }
}

/// Where a file or setting comes from, as the agent ranks them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    /// Set for the machine by an administrator.
    Managed,
    /// The user's, for every project.
    User,
    /// Checked in with the project.
    Project,
    /// The user's own for this project, not checked in.
    Local,
    /// The agent's own notes, written by it.
    Memory,
    /// Brought in by a plugin or extension.
    Plugin,
    /// Set for the user's account rather than in a file.
    Account,
}

impl Scope {
    pub fn label(self) -> &'static str {
        match self {
            Scope::Managed => "managed",
            Scope::User => "user",
            Scope::Project => "project",
            Scope::Local => "local",
            Scope::Memory => "auto memory",
            Scope::Plugin => "plugin",
            Scope::Account => "account",
        }
    }

    /// The scope Claude Code's transcript gives a loaded file.
    pub fn from_claude(kind: &str) -> Scope {
        match kind {
            "Managed" | "Policy" | "Enterprise" => Scope::Managed,
            "User" => Scope::User,
            "Local" => Scope::Local,
            "AutoMem" | "Memory" | "AutoMemory" => Scope::Memory,
            _ => Scope::Project,
        }
    }
}

/// Where each agent keeps the user's own settings. Tests point these at fixtures.
#[derive(Clone, Debug, Default)]
pub struct Homes {
    /// `~/.claude`, or `CLAUDE_CONFIG_DIR`.
    pub claude: Option<PathBuf>,
    /// Where `.claude.json` is: the home folder, or `CLAUDE_CONFIG_DIR` when that's set.
    pub claude_json: Option<PathBuf>,
    /// `~/.codex`, or `CODEX_HOME`.
    pub codex: Option<PathBuf>,
    /// `~/.gemini`.
    pub gemini: Option<PathBuf>,
    /// The machine's managed instructions for Claude Code.
    pub managed: Option<PathBuf>,
}

impl Homes {
    pub fn from_env() -> Homes {
        let var = |name: &str| {
            std::env::var_os(name)
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
        };
        let home = var("HOME").or_else(|| var("USERPROFILE"));
        let claude_dir = var("CLAUDE_CONFIG_DIR");
        let managed = if cfg!(target_os = "macos") {
            Some(PathBuf::from(
                "/Library/Application Support/ClaudeCode/CLAUDE.md",
            ))
        } else if cfg!(windows) {
            Some(PathBuf::from(r"C:\Program Files\ClaudeCode\CLAUDE.md"))
        } else {
            Some(PathBuf::from("/etc/claude-code/CLAUDE.md"))
        };
        Homes {
            claude: claude_dir
                .clone()
                .or_else(|| home.as_ref().map(|h| h.join(".claude"))),
            claude_json: claude_dir
                .or_else(|| home.clone())
                .map(|d| d.join(".claude.json")),
            codex: var("CODEX_HOME").or_else(|| home.as_ref().map(|h| h.join(".codex"))),
            gemini: home.as_ref().map(|h| h.join(".gemini")),
            managed,
        }
    }
}

/// One instruction file, in the order the agent reads it.
#[derive(Clone, Debug, PartialEq)]
pub struct Instruction {
    pub path: PathBuf,
    pub scope: Scope,
    /// The file and line that imported it, for a file brought in by `@path`.
    pub imported_by: Option<(PathBuf, usize)>,
    /// How many imports deep it is: 0 for a file the agent finds itself.
    pub depth: usize,
    /// Loaded only once the agent works in this folder, not at the start.
    pub later: Option<PathBuf>,
    /// The agent reads only this many lines of it.
    pub limit: Option<usize>,
    pub text: Option<String>,
}

impl Instruction {
    fn new(path: PathBuf, scope: Scope) -> Instruction {
        let text = read_text(&path);
        Instruction {
            path,
            scope,
            imported_by: None,
            depth: 0,
            later: None,
            limit: None,
            text,
        }
    }

    pub fn exists(&self) -> bool {
        self.text.is_some()
    }

    pub fn lines(&self) -> usize {
        self.text.as_deref().map_or(0, |t| t.lines().count())
    }

    /// What the agent actually takes in: all of it, or the lines it reads.
    pub fn loaded(&self) -> &str {
        let text = self.text.as_deref().unwrap_or("");
        match self.limit {
            Some(limit) => {
                let end = text.split_inclusive('\n').take(limit).map(str::len).sum();
                &text[..end]
            }
            None => text,
        }
    }

    pub fn tokens(&self) -> usize {
        estimate_tokens(self.loaded())
    }
}

/// Text is about four characters a token in English and code; good enough to compare parts.
pub fn estimate_tokens(text: &str) -> usize {
    text.chars().count().div_ceil(4)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemKind {
    Skill,
    Subagent,
    Command,
    Hook,
    Mcp,
    Extension,
}

impl ItemKind {
    pub fn label(self) -> &'static str {
        match self {
            ItemKind::Skill => "skill",
            ItemKind::Subagent => "subagent",
            ItemKind::Command => "command",
            ItemKind::Hook => "hook",
            ItemKind::Mcp => "MCP server",
            ItemKind::Extension => "extension",
        }
    }
}

/// A skill, subagent, command, hook or MCP server, and the file that defines it.
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub kind: ItemKind,
    pub name: String,
    /// What it's for, as the agent is told: a description, or the hook's command.
    pub about: String,
    pub scope: Scope,
    pub path: PathBuf,
    /// The line it starts on, 1-based, where that isn't the top.
    pub line: Option<usize>,
}

/// Everything an agent works from in one project, before any session.
#[derive(Clone, Debug, Default)]
pub struct Context {
    pub instructions: Vec<Instruction>,
    pub items: Vec<Item>,
}

/// What `agent` would load working in `project`, worked out from the files by its rules.
pub fn discover(agent: Agent, project: &Path, homes: &Homes) -> Context {
    match agent {
        Agent::Claude => Context {
            instructions: claude_instructions(project, homes),
            items: claude_items(project, homes),
        },
        Agent::Codex => Context {
            instructions: codex_instructions(project, homes),
            items: codex_items(homes),
        },
        Agent::Gemini => Context {
            instructions: gemini_instructions(project, homes),
            items: gemini_items(project, homes),
        },
    }
}

fn read_text(path: &Path) -> Option<String> {
    let meta = fs::metadata(path).ok()?;
    if !meta.is_file() {
        return None;
    }
    let bytes = fs::read(path).ok()?;
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

/// Each instruction file followed at once by what it imports, depth first, as Claude Code and
/// Gemini CLI read them. A file already in the list isn't read twice.
fn with_imports(found: Vec<Instruction>) -> Vec<Instruction> {
    let mut out: Vec<Instruction> = Vec::new();
    for file in found {
        push_with_imports(file, &mut out);
    }
    out
}

fn push_with_imports(file: Instruction, out: &mut Vec<Instruction>) {
    if out.iter().any(|i| i.path == file.path) {
        return;
    }
    let refs = file.text.as_deref().map(imports).unwrap_or_default();
    let (scope, depth, later, dir) = (
        file.scope,
        file.depth,
        file.later.clone(),
        file.path.parent().map(Path::to_path_buf),
    );
    let from = file.path.clone();
    out.push(file);
    if depth >= IMPORT_DEPTH {
        return;
    }
    for import in refs {
        let path = imports::resolve(&import.target, dir.as_deref().unwrap_or(Path::new("")));
        let mut child = Instruction::new(path, scope);
        child.imported_by = Some((from.clone(), import.line));
        child.depth = depth + 1;
        child.later = later.clone();
        push_with_imports(child, out);
    }
}

/// The folders from the top of the file system down to `project`, the order an agent reads
/// instructions in, so the closest comes last and counts most.
fn ancestors_down(project: &Path) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = project.ancestors().map(Path::to_path_buf).collect();
    dirs.reverse();
    dirs
}

/// The top of the git repository `project` is in, or `project` itself.
fn repo_root(project: &Path) -> PathBuf {
    project
        .ancestors()
        .find(|d| d.join(".git").exists())
        .unwrap_or(project)
        .to_path_buf()
}

/// The main checkout of the repository a worktree belongs to, from its `.git` file.
fn main_checkout(project: &Path) -> Option<PathBuf> {
    let root = repo_root(project);
    let text = fs::read_to_string(root.join(".git")).ok()?;
    let gitdir = PathBuf::from(text.strip_prefix("gitdir:")?.trim());
    // <main>/.git/worktrees/<name>
    let dot_git = gitdir.parent()?.parent()?;
    (dot_git.file_name()? == ".git").then(|| dot_git.parent().map(Path::to_path_buf))?
}

/// The folder Claude Code keeps a project's sessions in: its path with every character that
/// isn't a letter or digit made a dash.
pub fn claude_project_dir(claude: &Path, project: &Path) -> PathBuf {
    let name: String = project
        .to_string_lossy()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    claude.join("projects").join(name)
}

/// Claude Code's own notes for the project. A worktree shares its main checkout's.
fn claude_memory(project: &Path, claude: &Path) -> PathBuf {
    let index = |p: &Path| {
        claude_project_dir(claude, p)
            .join("memory")
            .join("MEMORY.md")
    };
    let own = index(project);
    if own.exists() {
        return own;
    }
    main_checkout(project)
        .map(|m| index(&m))
        .filter(|p| p.exists())
        .unwrap_or(own)
}

fn markdown_files(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "md") && p.is_file())
        .collect();
    files.sort();
    files
}

fn claude_instructions(project: &Path, homes: &Homes) -> Vec<Instruction> {
    let mut found = Vec::new();
    if let Some(managed) = &homes.managed {
        found.push(Instruction::new(managed.clone(), Scope::Managed));
    }
    if let Some(claude) = &homes.claude {
        found.push(Instruction::new(claude.join("CLAUDE.md"), Scope::User));
        found.extend(
            markdown_files(&claude.join("rules"))
                .into_iter()
                .map(|p| Instruction::new(p, Scope::User)),
        );
    }
    let user_file = homes.claude.as_ref().map(|c| c.join("CLAUDE.md"));
    for dir in ancestors_down(project) {
        for name in ["CLAUDE.md", ".claude/CLAUDE.md"] {
            let path = dir.join(name);
            if Some(&path) != user_file.as_ref() {
                found.push(Instruction::new(path, Scope::Project));
            }
        }
        if dir == project {
            found.extend(
                markdown_files(&dir.join(".claude").join("rules"))
                    .into_iter()
                    .map(|p| Instruction::new(p, Scope::Project)),
            );
        }
        found.push(Instruction::new(dir.join("CLAUDE.local.md"), Scope::Local));
    }
    // Only what's there, but the user's file stays so its absence can be shown.
    found.retain(|i| {
        i.exists()
            || (i.scope == Scope::User && i.path.file_name().is_some_and(|n| n == "CLAUDE.md"))
    });
    if let Some(claude) = &homes.claude {
        let mut memory = Instruction::new(claude_memory(project, claude), Scope::Memory);
        memory.limit = Some(MEMORY_LINES);
        if memory.exists() {
            found.push(memory);
        }
    }
    for path in nested(project, &["CLAUDE.md"]) {
        let mut later = Instruction::new(path.clone(), Scope::Project);
        later.later = path.parent().map(Path::to_path_buf);
        found.push(later);
    }
    with_imports(found)
}

/// Instruction files in folders under `project`, which an agent loads only when it reads a file
/// there.
fn nested(project: &Path, names: &[&str]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![(project.to_path_buf(), 0)];
    let mut seen = 0;
    while let Some((dir, depth)) = stack.pop() {
        seen += 1;
        if seen > 4000 {
            break;
        }
        let mut entries: Vec<PathBuf> = fs::read_dir(&dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .collect();
        entries.sort();
        for path in entries.into_iter().rev() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if path.is_dir() {
                if depth < NESTED_DEPTH && !name.starts_with('.') && !SKIPPED.contains(&name) {
                    stack.push((path, depth + 1));
                }
            } else if dir != project && names.contains(&name) {
                out.push(path);
            }
        }
    }
    out.sort();
    out
}

/// A Markdown file's front matter, as plain `key: value` pairs, quotes taken off.
pub fn front_matter(text: &str) -> Vec<(String, String)> {
    let mut lines = text.lines();
    if lines.next().map(str::trim) != Some("---") {
        return Vec::new();
    }
    let mut out = Vec::new();
    for line in lines {
        if line.trim() == "---" {
            break;
        }
        if line.starts_with([' ', '\t']) {
            continue;
        }
        if let Some((key, value)) = line.split_once(':') {
            let value = value
                .trim()
                .trim_matches(|c| c == '"' || c == '\'')
                .to_string();
            out.push((key.trim().to_string(), value));
        }
    }
    out
}

fn field(front: &[(String, String)], key: &str) -> Option<String> {
    front
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.clone())
        .filter(|v| !v.is_empty())
}

/// The first line of prose after the front matter, for a file that says what it's for there.
fn first_line(text: &str) -> String {
    let body = match text.strip_prefix("---") {
        Some(rest) => rest.split_once("\n---").map_or("", |(_, b)| b),
        None => text,
    };
    body.lines()
        .map(|l| l.trim().trim_start_matches('#').trim())
        .find(|l| !l.is_empty())
        .unwrap_or("")
        .to_string()
}

/// One item per Markdown file in `dir` (skills: one per folder holding `SKILL.md`).
fn markdown_items(kind: ItemKind, dir: &Path, scope: Scope) -> Vec<Item> {
    let files: Vec<PathBuf> = if kind == ItemKind::Skill {
        let mut dirs: Vec<PathBuf> = fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path().join("SKILL.md"))
            .filter(|p| p.is_file())
            .collect();
        dirs.sort();
        dirs
    } else {
        walk_files(dir, "md")
    };
    files
        .into_iter()
        .map(|path| {
            let text = read_text(&path).unwrap_or_default();
            let front = front_matter(&text);
            let fallback = if kind == ItemKind::Skill {
                path.parent().and_then(Path::file_name)
            } else {
                path.file_stem()
            };
            let mut name = field(&front, "name").unwrap_or_else(|| {
                fallback
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default()
            });
            if kind == ItemKind::Command {
                // A command is named by its path under the folder, `a/b.md` being `/a:b`.
                let rel = path.strip_prefix(dir).unwrap_or(&path).with_extension("");
                name = rel
                    .to_string_lossy()
                    .replace(std::path::MAIN_SEPARATOR, ":");
            }
            let about = match kind {
                ItemKind::Command => {
                    field(&front, "description").unwrap_or_else(|| first_line(&text))
                }
                _ => field(&front, "description").unwrap_or_default(),
            };
            Item {
                kind,
                name,
                about,
                scope,
                path,
                line: None,
            }
        })
        .collect()
}

/// Every file with `ext` under `dir`, sorted, a few folders deep.
fn walk_files(dir: &Path, ext: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![(dir.to_path_buf(), 0)];
    while let Some((d, depth)) = stack.pop() {
        for entry in fs::read_dir(&d).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() && depth < 3 {
                stack.push((path, depth + 1));
            } else if path.extension().is_some_and(|e| e == ext) {
                out.push(path);
            }
        }
    }
    out.sort();
    out
}

fn read_json(path: &Path) -> Option<Map<String, Value>> {
    match serde_json::from_str(&read_text(path)?).ok()? {
        Value::Object(map) => Some(map),
        _ => None,
    }
}

/// The 1-based line where `needle` first shows in `text`, to open a setting where it is.
fn line_of(text: &str, needle: &str) -> Option<usize> {
    text.lines().position(|l| l.contains(needle)).map(|i| i + 1)
}

/// Claude Code's hooks in one settings file: one item per command, named by its event and,
/// where it has one, what it matches.
fn claude_hooks(path: &Path, scope: Scope) -> Vec<Item> {
    let Some(settings) = read_json(path) else {
        return Vec::new();
    };
    let text = read_text(path).unwrap_or_default();
    let Some(Value::Object(events)) = settings.get("hooks") else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (event, groups) in events {
        for group in groups.as_array().into_iter().flatten() {
            let matcher = group["matcher"]
                .as_str()
                .filter(|m| !m.is_empty() && *m != "*");
            for hook in group["hooks"].as_array().into_iter().flatten() {
                let Some(command) = hook["command"].as_str() else {
                    continue;
                };
                let name = match matcher {
                    Some(m) => format!("{event} · {m}"),
                    None => event.clone(),
                };
                let needle = serde_json::to_string(command).unwrap_or_default();
                let needle = needle.trim_matches('"');
                out.push(Item {
                    kind: ItemKind::Hook,
                    name,
                    about: command.to_string(),
                    scope,
                    path: path.to_path_buf(),
                    line: line_of(&text, needle),
                });
            }
        }
    }
    out
}

/// MCP servers under `key` in a JSON settings file.
fn json_servers(path: &Path, servers: Option<&Value>, scope: Scope) -> Vec<Item> {
    let Some(Value::Object(servers)) = servers else {
        return Vec::new();
    };
    let text = read_text(path).unwrap_or_default();
    servers
        .iter()
        .map(|(name, server)| {
            let about = server["url"]
                .as_str()
                .map(str::to_string)
                .or_else(|| {
                    let command = server["command"].as_str()?;
                    let args: Vec<&str> = server["args"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                        .collect();
                    Some(
                        std::iter::once(command)
                            .chain(args)
                            .collect::<Vec<_>>()
                            .join(" "),
                    )
                })
                .unwrap_or_default();
            Item {
                kind: ItemKind::Mcp,
                name: name.clone(),
                about,
                scope,
                path: path.to_path_buf(),
                line: line_of(&text, &format!("\"{name}\"")),
            }
        })
        .collect()
}

fn claude_items(project: &Path, homes: &Homes) -> Vec<Item> {
    let mut items = Vec::new();
    let local = project.join(".claude");
    if let Some(user) = &homes.claude {
        items.extend(markdown_items(
            ItemKind::Skill,
            &user.join("skills"),
            Scope::User,
        ));
    }
    items.extend(markdown_items(
        ItemKind::Skill,
        &local.join("skills"),
        Scope::Project,
    ));
    if let Some(user) = &homes.claude {
        items.extend(markdown_items(
            ItemKind::Subagent,
            &user.join("agents"),
            Scope::User,
        ));
    }
    items.extend(markdown_items(
        ItemKind::Subagent,
        &local.join("agents"),
        Scope::Project,
    ));
    if let Some(user) = &homes.claude {
        items.extend(markdown_items(
            ItemKind::Command,
            &user.join("commands"),
            Scope::User,
        ));
    }
    items.extend(markdown_items(
        ItemKind::Command,
        &local.join("commands"),
        Scope::Project,
    ));
    if let Some(user) = &homes.claude {
        items.extend(claude_hooks(&user.join("settings.json"), Scope::User));
    }
    items.extend(claude_hooks(&local.join("settings.json"), Scope::Project));
    items.extend(claude_hooks(
        &local.join("settings.local.json"),
        Scope::Local,
    ));
    if let Some(path) = &homes.claude_json
        && let Some(config) = read_json(path)
    {
        items.extend(json_servers(path, config.get("mcpServers"), Scope::User));
        let key = project.to_string_lossy();
        let mine = config
            .get("projects")
            .and_then(|p| p.get(key.as_ref()))
            .and_then(|p| p.get("mcpServers"));
        items.extend(json_servers(path, mine, Scope::Local));
    }
    let shared = project.join(".mcp.json");
    if let Some(config) = read_json(&shared) {
        items.extend(json_servers(
            &shared,
            config.get("mcpServers"),
            Scope::Project,
        ));
    }
    items
}

/// Codex reads its own `AGENTS.md`, then one per folder from the repository's top down to
/// where it runs, an `AGENTS.override.md` in place of the `AGENTS.md` beside it, stopping once
/// they come to `CODEX_BYTES` in all.
fn codex_instructions(project: &Path, homes: &Homes) -> Vec<Instruction> {
    let mut found = Vec::new();
    if let Some(codex) = &homes.codex {
        let over = codex.join("AGENTS.override.md");
        let path = if over.is_file() {
            over
        } else {
            codex.join("AGENTS.md")
        };
        found.push(Instruction::new(path, Scope::User));
    }
    let root = repo_root(project);
    let mut budget = CODEX_BYTES;
    for dir in ancestors_down(project)
        .into_iter()
        .filter(|d| d.starts_with(&root))
    {
        // Codex reads the first of the two that has something in it.
        let over = dir.join("AGENTS.override.md");
        let path = if fs::read_to_string(&over).is_ok_and(|t| !t.trim().is_empty()) {
            over
        } else {
            dir.join("AGENTS.md")
        };
        let mut file = Instruction::new(path, Scope::Project);
        if let Some(text) = &file.text {
            if budget == 0 {
                continue;
            }
            if text.len() > budget {
                // Cut where a character ends, so a dash or a word in another script that the
                // cut falls inside doesn't stop the reading.
                let mut cut = budget.min(text.len());
                while !text.is_char_boundary(cut) {
                    cut -= 1;
                }
                file.limit = Some(text[..cut].lines().count());
            }
            budget = budget.saturating_sub(text.len());
            found.push(file);
        }
    }
    found.retain(|i| i.exists() || i.scope == Scope::User);
    found
}

/// Names of tables under `[prefix.<name>]` in a TOML file, with their lines.
fn toml_tables(text: &str, prefix: &str) -> Vec<(String, usize)> {
    text.lines()
        .enumerate()
        .filter_map(|(i, line)| {
            let inner = line.trim().strip_prefix('[')?.strip_suffix(']')?;
            let name = inner.strip_prefix(prefix)?.strip_prefix('.')?;
            (!name.contains('.')).then(|| (name.trim_matches('"').to_string(), i + 1))
        })
        .collect()
}

fn codex_items(homes: &Homes) -> Vec<Item> {
    let Some(codex) = &homes.codex else {
        return Vec::new();
    };
    let mut items = markdown_items(ItemKind::Skill, &codex.join("skills"), Scope::User);
    items.extend(markdown_items(
        ItemKind::Command,
        &codex.join("prompts"),
        Scope::User,
    ));
    let config = codex.join("config.toml");
    if let Some(text) = read_text(&config) {
        items.extend(
            toml_tables(&text, "mcp_servers")
                .into_iter()
                .map(|(name, line)| Item {
                    kind: ItemKind::Mcp,
                    name,
                    about: String::new(),
                    scope: Scope::User,
                    path: config.clone(),
                    line: Some(line),
                }),
        );
    }
    items
}

/// The names Gemini CLI reads its instructions from: `context.fileName` in its settings,
/// the project's over the user's, or `GEMINI.md`.
pub fn gemini_names(project: &Path, homes: &Homes) -> Vec<String> {
    let names = |path: PathBuf| -> Option<Vec<String>> {
        let settings = read_json(&path)?;
        match settings.get("context")?.get("fileName")? {
            Value::String(name) => Some(vec![name.clone()]),
            Value::Array(list) => Some(
                list.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect(),
            ),
            _ => None,
        }
    };
    names(project.join(".gemini").join("settings.json"))
        .or_else(|| {
            homes
                .gemini
                .as_ref()
                .and_then(|g| names(g.join("settings.json")))
        })
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| vec!["GEMINI.md".to_string()])
}

/// Gemini CLI reads its own file, then each folder's from the repository's top down to where
/// it runs, then those in folders below, each followed by what it imports.
fn gemini_instructions(project: &Path, homes: &Homes) -> Vec<Instruction> {
    let names = gemini_names(project, homes);
    let mut found = Vec::new();
    if let Some(gemini) = &homes.gemini {
        found.push(Instruction::new(gemini.join(&names[0]), Scope::User));
        for name in &names[1..] {
            found.push(Instruction::new(gemini.join(name), Scope::User));
        }
    }
    let root = repo_root(project);
    for dir in ancestors_down(project)
        .into_iter()
        .filter(|d| d.starts_with(&root))
    {
        for name in &names {
            found.push(Instruction::new(dir.join(name), Scope::Project));
        }
    }
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    found.extend(
        nested(project, &refs)
            .into_iter()
            .map(|p| Instruction::new(p, Scope::Project)),
    );
    found.retain(|i| {
        i.exists()
            || (i.scope == Scope::User && i.path.file_name().is_some_and(|n| *n == *names[0]))
    });
    with_imports(found)
}

fn gemini_items(project: &Path, homes: &Homes) -> Vec<Item> {
    let mut items = Vec::new();
    let local = project.join(".gemini");
    for (dir, scope) in [
        (homes.gemini.clone(), Scope::User),
        (Some(local.clone()), Scope::Project),
    ] {
        let Some(dir) = dir else { continue };
        for path in walk_files(&dir.join("commands"), "toml") {
            let text = read_text(&path).unwrap_or_default();
            let about = text
                .lines()
                .find_map(|l| {
                    l.trim()
                        .strip_prefix("description")?
                        .trim_start()
                        .strip_prefix('=')
                        .map(|v| v.trim().trim_matches('"').to_string())
                })
                .unwrap_or_default();
            let rel = path
                .strip_prefix(dir.join("commands"))
                .unwrap_or(&path)
                .with_extension("");
            items.push(Item {
                kind: ItemKind::Command,
                name: rel
                    .to_string_lossy()
                    .replace(std::path::MAIN_SEPARATOR, ":"),
                about,
                scope,
                path,
                line: None,
            });
        }
        let settings = dir.join("settings.json");
        if let Some(config) = read_json(&settings) {
            items.extend(json_servers(&settings, config.get("mcpServers"), scope));
        }
    }
    if let Some(gemini) = &homes.gemini {
        let mut extensions: Vec<PathBuf> = fs::read_dir(gemini.join("extensions"))
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path().join("gemini-extension.json"))
            .filter(|p| p.is_file())
            .collect();
        extensions.sort();
        for path in extensions {
            let config = read_json(&path).unwrap_or_default();
            let name = config
                .get("name")
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| {
                    path.parent()
                        .and_then(Path::file_name)
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default()
                });
            items.push(Item {
                kind: ItemKind::Extension,
                name,
                about: config
                    .get("description")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                scope: Scope::Plugin,
                path: path.clone(),
                line: None,
            });
            items.extend(json_servers(&path, config.get("mcpServers"), Scope::Plugin));
        }
    }
    items
}

#[cfg(test)]
mod tests;
