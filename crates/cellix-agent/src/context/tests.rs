//! Every test builds its own home and project in a temporary folder; none reads the real
//! `~/.claude`.

use std::fs;
use std::path::{Path, PathBuf};

use tempfile::TempDir;

use super::*;

struct Fixture {
    _dir: TempDir,
    home: PathBuf,
    project: PathBuf,
    homes: Homes,
}

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

/// A home with Claude Code, Codex and Gemini CLI set up, and a project like Cellix's own.
fn fixture() -> Fixture {
    let dir = TempDir::new().unwrap();
    let root = dunce::canonicalize(dir.path()).unwrap();
    let home = root.join("home");
    let project = root.join("work").join("app");
    fs::create_dir_all(project.join(".git")).unwrap();
    let claude = home.join(".claude");
    write(
        &claude.join("CLAUDE.md"),
        "# How I like to work\n\n- Ask before adding a dependency.\n- Run tests with `cargo test` before saying it's done.\n",
    );
    write(
        &claude.join("skills/browser/SKILL.md"),
        "---\nname: terminal-browser\ndescription: A real browser in the terminal\n---\nBody\n",
    );
    write(
        &claude.join("skills/bare/SKILL.md"),
        "---\nname: pdf-tools\n---\nNo description here.\n",
    );
    write(
        &claude.join("commands/git/ship.md"),
        "---\ndescription: Commit and push\n---\nDo it.\n",
    );
    write(
        &claude.join("settings.json"),
        "{\n  \"hooks\": {\n    \"Stop\": [{\"hooks\": [{\"type\": \"command\", \"command\": \"say done\"}]}]\n  }\n}\n",
    );
    write(
        &home.join(".claude.json"),
        &format!(
            "{{\"mcpServers\": {{\"postwren\": {{\"command\": \"postwren\", \"args\": [\"mcp\"]}}}},\n \"projects\": {{\"{}\": {{\"mcpServers\": {{\"local-db\": {{\"url\": \"http://localhost:9\"}}}}}}}}}}",
            project.display()
        ),
    );
    let memory = claude_project_dir(&claude, &project).join("memory/MEMORY.md");
    let long: String = (1..=212).map(|i| format!("- note {i}\n")).collect();
    write(&memory, &long);

    write(
        &project.join("CLAUDE.md"),
        "@AGENTS.md\n@docs/STYLE.md\nSee me@example.com and @types/node.\n",
    );
    write(
        &project.join("AGENTS.md"),
        "# Working here\n\n## Rules\n\n**Never run `cargo build`, `test` directly.** Use `harness/x`.\n\n@docs/MORE.md\n",
    );
    write(&project.join("docs/MORE.md"), "More rules.\n");
    write(&project.join("crates/core/CLAUDE.md"), "Core only.\n");
    write(
        &project.join("target/debug/CLAUDE.md"),
        "Build output, never looked at.\n",
    );
    write(
        &project.join(".claude/agents/ui-expert.md"),
        "---\nname: ui-expert\ndescription: Reviews what it looks like\n---\n",
    );
    write(
        &project.join(".claude/settings.json"),
        "{\n  \"hooks\": {\n    \"PreToolUse\": [\n      {\n        \"matcher\": \"Bash\",\n        \"hooks\": [{\"type\": \"command\", \"command\": \"bash \\\"$CLAUDE_PROJECT_DIR\\\"/harness/hooks/guard.sh\"}]\n      }\n    ]\n  }\n}\n",
    );
    write(
        &project.join(".mcp.json"),
        "{\"mcpServers\": {\"docs\": {\"command\": \"docs-server\"}}}",
    );

    write(
        &home.join(".codex/config.toml"),
        "model = \"x\"\n\n[mcp_servers.context7]\ncommand = \"npx\"\n\n[mcp_servers.context7.env]\nA = \"b\"\n",
    );
    write(&home.join(".codex/prompts/review.md"), "Review the diff.\n");
    write(
        &home.join(".gemini/GEMINI.md"),
        "## Gemini Added Memories\n- likes tea\n",
    );
    write(
        &home.join(".gemini/extensions/ext/gemini-extension.json"),
        "{\"name\": \"helper\", \"mcpServers\": {\"h\": {\"command\": \"h\"}}}",
    );

    let homes = Homes {
        claude: Some(claude),
        claude_json: Some(home.join(".claude.json")),
        codex: Some(home.join(".codex")),
        gemini: Some(home.join(".gemini")),
        managed: Some(root.join("managed/CLAUDE.md")),
    };
    Fixture {
        _dir: dir,
        home,
        project,
        homes,
    }
}

fn names(context: &Context, project: &Path, home: &Path) -> Vec<String> {
    context
        .instructions
        .iter()
        .map(|i| {
            let path = i
                .path
                .strip_prefix(project)
                .map(|p| p.display().to_string());
            let path = path.unwrap_or_else(|_| {
                format!(
                    "~/{}",
                    i.path.strip_prefix(home).unwrap_or(&i.path).display()
                )
            });
            let mut name = format!("{}{path}", "  ".repeat(i.depth));
            if !i.exists() {
                name.push_str(" (missing)");
            }
            if i.later.is_some() {
                name.push_str(" (later)");
            }
            name
        })
        .collect()
}

#[test]
fn claude_code_instructions_come_in_its_load_order_with_imports() {
    let f = fixture();
    let context = discover(Agent::Claude, &f.project, &f.homes);
    let got = names(&context, &f.project, &f.home);
    let memory =
        claude_project_dir(f.homes.claude.as_ref().unwrap(), &f.project).join("memory/MEMORY.md");
    let memory = format!("~/{}", memory.strip_prefix(&f.home).unwrap().display());
    assert_eq!(
        got,
        [
            "~/.claude/CLAUDE.md",
            "CLAUDE.md",
            "  AGENTS.md",
            "    docs/MORE.md",
            "  docs/STYLE.md (missing)",
            memory.as_str(),
            "crates/core/CLAUDE.md (later)",
        ]
    );
    let agents = &context.instructions[2];
    assert_eq!(agents.imported_by, Some((f.project.join("CLAUDE.md"), 1)));
    let memory = context
        .instructions
        .iter()
        .find(|i| i.scope == Scope::Memory)
        .unwrap();
    assert_eq!(memory.lines(), 212);
    assert_eq!(memory.loaded().lines().count(), MEMORY_LINES);
}

#[test]
fn claude_code_items_say_what_each_is_for_and_where_it_is() {
    let f = fixture();
    let context = discover(Agent::Claude, &f.project, &f.homes);
    let got: Vec<(ItemKind, &str, &str, Scope)> = context
        .items
        .iter()
        .map(|i| (i.kind, i.name.as_str(), i.about.as_str(), i.scope))
        .collect();
    assert!(got.contains(&(
        ItemKind::Skill,
        "terminal-browser",
        "A real browser in the terminal",
        Scope::User
    )));
    assert!(got.contains(&(ItemKind::Skill, "pdf-tools", "", Scope::User)));
    assert!(got.contains(&(
        ItemKind::Subagent,
        "ui-expert",
        "Reviews what it looks like",
        Scope::Project
    )));
    assert!(got.contains(&(
        ItemKind::Command,
        "git:ship",
        "Commit and push",
        Scope::User
    )));
    assert!(got.contains(&(ItemKind::Hook, "Stop", "say done", Scope::User)));
    assert!(got.contains(&(ItemKind::Mcp, "postwren", "postwren mcp", Scope::User)));
    assert!(got.contains(&(
        ItemKind::Mcp,
        "local-db",
        "http://localhost:9",
        Scope::Local
    )));
    assert!(got.contains(&(ItemKind::Mcp, "docs", "docs-server", Scope::Project)));
    let hook = context
        .items
        .iter()
        .find(|i| i.name == "PreToolUse · Bash")
        .unwrap();
    assert_eq!(hook.line, Some(6), "opens at the hook's own line");
}

#[test]
fn codex_and_gemini_read_their_own_files() {
    let f = fixture();
    let codex = discover(Agent::Codex, &f.project, &f.homes);
    assert_eq!(
        names(&codex, &f.project, &f.home),
        ["~/.codex/AGENTS.md (missing)", "AGENTS.md"]
    );
    let mcp: Vec<_> = codex
        .items
        .iter()
        .filter(|i| i.kind == ItemKind::Mcp)
        .map(|i| (i.name.as_str(), i.line))
        .collect();
    assert_eq!(mcp, [("context7", Some(3))]);
    assert!(codex.items.iter().any(|i| i.kind == ItemKind::Command
        && i.name == "review"
        && i.about == "Review the diff."));

    let gemini = discover(Agent::Gemini, &f.project, &f.homes);
    assert_eq!(names(&gemini, &f.project, &f.home), ["~/.gemini/GEMINI.md"]);
    assert!(
        gemini
            .items
            .iter()
            .any(|i| i.kind == ItemKind::Extension && i.name == "helper")
    );
    assert!(
        gemini
            .items
            .iter()
            .any(|i| i.kind == ItemKind::Mcp && i.name == "h" && i.scope == Scope::Plugin)
    );

    // Told to read AGENTS.md too, it does.
    write(
        &f.project.join(".gemini/settings.json"),
        "{\"context\": {\"fileName\": [\"GEMINI.md\", \"AGENTS.md\"]}}",
    );
    let gemini = discover(Agent::Gemini, &f.project, &f.homes);
    assert_eq!(
        names(&gemini, &f.project, &f.home),
        ["~/.gemini/GEMINI.md", "AGENTS.md", "  docs/MORE.md"]
    );
}

#[test]
fn checks_find_what_the_files_alone_can_tell() {
    let f = fixture();
    let context = discover(Agent::Claude, &f.project, &f.homes);
    let found = checks(Agent::Claude, &f.project, &context);
    let kinds: Vec<CheckKind> = found.iter().map(|c| c.kind).collect();
    let opposite = found
        .iter()
        .find(|c| c.kind == CheckKind::Opposite)
        .expect("cargo test, both ways");
    assert!(
        opposite.message.contains("cargo test"),
        "{}",
        opposite.message
    );
    assert_eq!(opposite.line, Some(4));
    assert_eq!(opposite.other, Some((f.project.join("AGENTS.md"), 5)));
    let missing = found
        .iter()
        .find(|c| c.kind == CheckKind::MissingImport)
        .unwrap();
    assert_eq!(
        (missing.path.clone(), missing.line),
        (f.project.join("CLAUDE.md"), Some(2))
    );
    assert!(missing.message.contains("docs/STYLE.md"));
    let cut = found.iter().find(|c| c.kind == CheckKind::CutOff).unwrap();
    assert!(
        cut.message.contains("212 lines") && cut.message.contains("first 200"),
        "{}",
        cut.message
    );
    assert!(
        found
            .iter()
            .any(|c| c.kind == CheckKind::NoDescription && c.message.contains("pdf-tools"))
    );
    let hook = found
        .iter()
        .find(|c| c.kind == CheckKind::MissingFile)
        .unwrap();
    assert!(
        hook.message.contains("harness/hooks/guard.sh"),
        "{}",
        hook.message
    );
    assert!(!kinds.contains(&CheckKind::NotRead));

    // With the script there, the hook is fine.
    write(&f.project.join("harness/hooks/guard.sh"), "#!/bin/sh\n");
    let found = checks(Agent::Claude, &f.project, &context);
    assert!(!found.iter().any(|c| c.kind == CheckKind::MissingFile));

    let gemini = discover(Agent::Gemini, &f.project, &f.homes);
    let found = checks(Agent::Gemini, &f.project, &gemini);
    assert!(
        found
            .iter()
            .any(|c| c.kind == CheckKind::NotRead && c.message.contains("AGENTS.md"))
    );
}

/// A transcript in the shape Claude Code 2.1 writes, cut down to what's read here.
fn transcript(f: &Fixture) -> PathBuf {
    let app = f.project.join("src/app.rs");
    write(&app, &"fn main() {}\n".repeat(40));
    let lines = [
        serde_json::json!({"type": "attachment", "timestamp": "2026-11-07T10:00:00.000Z", "attachment": {"type": "instructions", "files": [
            {"path": f.project.join("CLAUDE.md"), "type": "Project", "content": "@AGENTS.md"},
            {"path": f.project.join("AGENTS.md"), "type": "Project", "content": "Rules, rules."},
            {"path": "/m/MEMORY.md", "type": "AutoMem", "content": "- a note"}
        ]}}),
        serde_json::json!({"type": "attachment", "attachment": {"type": "skill_listing", "content": "- terminal-browser: A real browser"}}),
        serde_json::json!({"type": "attachment", "attachment": {"type": "agent_listing_delta", "addedLines": ["- ui-expert: Reviews"]}}),
        serde_json::json!({"type": "attachment", "attachment": {"type": "mcp_instructions_delta", "addedNames": ["postwren"], "addedBlocks": ["## postwren\nMail."]}}),
        serde_json::json!({"type": "api-request-shape", "shape": {"system": [{"type": "text", "text": "You are Claude Code. ".repeat(20)}], "tools": [{"name": "Read"}]}}),
        serde_json::json!({"type": "user", "timestamp": "2026-11-07T10:00:01.000Z", "message": {"role": "user", "content": "Add a context view"}}),
        serde_json::json!({"type": "assistant", "timestamp": "2026-11-07T10:00:02.000Z", "message": {"model": "claude-opus-5-5", "content": [
            {"type": "tool_use", "id": "t1", "name": "Read", "input": {"file_path": app}},
            {"type": "tool_use", "id": "t2", "name": "Grep", "input": {"pattern": "fn draw", "path": "crates/"}}
        ], "usage": {"input_tokens": 2, "cache_creation_input_tokens": 1000, "cache_read_input_tokens": 3000}}}),
        serde_json::json!({"type": "user", "message": {"role": "user", "content": [
            {"type": "tool_result", "tool_use_id": "t1", "content": "x".repeat(4000)},
            {"type": "tool_result", "tool_use_id": "t2", "content": "y".repeat(400)}
        ]}}),
        serde_json::json!({"type": "assistant", "isSidechain": true, "message": {"content": [
            {"type": "tool_use", "id": "s1", "name": "Read", "input": {"file_path": "/not/mine.rs"}}
        ], "usage": {"input_tokens": 999999}}}),
        serde_json::json!({"type": "user", "timestamp": "2026-11-07T10:05:00.000Z", "message": {"role": "user", "content": [{"type": "text", "text": "And again"}]}}),
        serde_json::json!({"type": "assistant", "timestamp": "2026-11-07T10:05:01.000Z", "message": {"model": "claude-opus-5-5", "content": [
            {"type": "tool_use", "id": "t3", "name": "Read", "input": {"file_path": app, "offset": 10, "limit": 5}}
        ], "usage": {"input_tokens": 5, "cache_creation_input_tokens": 2000, "cache_read_input_tokens": 8000}}}),
        serde_json::json!({"type": "user", "message": {"role": "user", "content": [{"type": "tool_result", "tool_use_id": "t3", "content": "z".repeat(800)}]}}),
    ];
    let mut text: String = lines.iter().map(|l| format!("{l}\n")).collect();
    text.push_str("not json at all\n");
    let path = transcript_path(f.homes.claude.as_ref().unwrap(), &f.project, "s-1");
    write(&path, &text);
    path
}

#[test]
fn a_transcript_says_what_the_session_loaded_and_read() {
    let f = fixture();
    let path = transcript(&f);
    let session = read_session(&path).unwrap();
    let loaded: Vec<(&Path, Scope)> = session
        .loaded
        .iter()
        .map(|l| (l.path.as_path(), l.scope))
        .collect();
    assert_eq!(
        loaded,
        [
            (f.project.join("CLAUDE.md").as_path(), Scope::Project),
            (f.project.join("AGENTS.md").as_path(), Scope::Project),
            (Path::new("/m/MEMORY.md"), Scope::Memory)
        ]
    );
    assert_eq!(session.skills, "- terminal-browser: A real browser");
    assert_eq!(session.subagents, ["- ui-expert: Reviews"]);
    assert_eq!(session.mcp[0].0, "postwren");
    assert_eq!(session.turns, 2);
    assert_eq!(session.model.as_deref(), Some("claude-opus-5-5"));
    assert_eq!(session.window(), 200_000);

    assert_eq!(
        session.files.len(),
        1,
        "the subagent's read isn't this session's"
    );
    let app = &session.files[0];
    assert_eq!(
        (app.reads, app.turns.clone(), app.tokens),
        (2, vec![1, 2], 1200)
    );
    assert_eq!(
        app.last_read,
        transcript::unix_seconds("2026-11-07T10:05:01.000Z")
    );
    assert_eq!(session.searches.len(), 1);
    assert_eq!(
        (
            session.searches[0].pattern.as_str(),
            session.searches[0].tokens
        ),
        ("fn draw", 100)
    );

    let b = session.breakdown;
    assert_eq!(
        b.total, 10_005,
        "the last reply's input, cache written and cache read"
    );
    assert_eq!(b.files, 1200);
    assert_eq!(b.output, 100);
    assert!(b.system > 0 && b.tools > 0 && b.instructions > 0 && b.listings > 0 && b.mcp > 0);
    assert_eq!(b.before() + b.conversation(), b.total);

    // The file was written well after the session last read it, so its copy is old.
    assert_eq!(session.stale().len(), 1);
}

#[test]
fn rfc3339_times_become_seconds() {
    assert_eq!(transcript::unix_seconds("1970-01-01T00:00:00Z"), Some(0));
    assert_eq!(
        transcript::unix_seconds("2026-11-07T10:00:01.500Z"),
        Some(1_794_045_601)
    );
}
