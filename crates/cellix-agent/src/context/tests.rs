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
        // Built as JSON, so a Windows path's backslashes are escaped.
        &serde_json::json!({
            "mcpServers": {"postwren": {"command": "postwren", "args": ["mcp"]}},
            "projects": {project.to_string_lossy(): {"mcpServers": {"local-db": {"url": "http://localhost:9"}}}},
        })
        .to_string(),
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
            // Compared with `/` on every system.
            let path = path.replace('\\', "/");
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
    let memory =
        format!("~/{}", memory.strip_prefix(&f.home).unwrap().display()).replace('\\', "/");
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
    assert!(missing.message.replace('\\', "/").contains("docs/STYLE.md"));
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
    assert_eq!(session.window(), 1_000_000);

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

#[test]
fn every_current_claude_model_has_a_million_and_only_old_ones_200k() {
    for model in [
        "claude-opus-5-5",
        "claude-sonnet-5-5",
        "claude-haiku-5-5",
        "claude-fable-5-1",
        "claude-opus-4-6",
        "claude-sonnet-4-6",
        "us.anthropic.claude-opus-5-5",
        "some-model-from-next-year",
    ] {
        assert_eq!(claude_window(model, 170_000), 1_000_000, "{model}");
    }
    for model in [
        "claude-haiku-4-5",
        "claude-haiku-4-5-20251001",
        "claude-sonnet-4-5-20250929",
        "claude-opus-4-5-20251101",
        "claude-opus-4-1-20250805",
        "claude-opus-4-20250514",
        "claude-sonnet-4-20250514",
        "claude-3-7-sonnet-20250219",
        "anthropic.claude-haiku-4-5-20251001-v1:0",
    ] {
        assert_eq!(claude_window(model, 170_000), 200_000, "{model}");
    }
    // An old model says it has the long window by its name, or by using it.
    assert_eq!(claude_window("claude-sonnet-4-5[1m]", 10), 1_000_000);
    assert_eq!(claude_window("claude-sonnet-4-5", 250_000), 1_000_000);
    // A later 4.x isn't taken for an old one by its prefix.
    assert_eq!(claude_window("claude-opus-4-6-20260101", 10), 1_000_000);
}

#[test]
fn codex_cuts_a_long_file_between_characters_and_skips_an_empty_override() {
    let f = fixture();
    // A dash of three bytes across the 32 KiB cut.
    let mut long = "a".repeat(CODEX_BYTES - 1);
    long.push_str("—\nmore\n");
    write(&f.project.join("AGENTS.md"), &long);
    write(&f.project.join("AGENTS.override.md"), "\n  \n");
    let codex = discover(Agent::Codex, &f.project, &f.homes);
    let file = codex
        .instructions
        .iter()
        .find(|i| i.path.ends_with("AGENTS.md") && i.scope == Scope::Project)
        .expect("AGENTS.md, since the override is empty");
    assert_eq!(file.limit, Some(1));
}

#[test]
fn claude_code_reads_agents_md_when_the_project_has_no_claude_md() {
    let f = fixture();
    fs::remove_file(f.project.join("CLAUDE.md")).unwrap();
    write(&f.project.join("web/AGENTS.md"), "Web only.\n");
    write(
        &f.project.join("crates/core/AGENTS.md"),
        "Not read: it has a CLAUDE.md.\n",
    );
    let context = discover(Agent::Claude, &f.project, &f.homes);
    let got = names(&context, &f.project, &f.home);
    assert!(got.contains(&"AGENTS.md".to_string()), "{got:?}");
    assert!(
        got.contains(&"  docs/MORE.md".to_string()),
        "its imports: {got:?}"
    );
    assert!(
        got.contains(&"web/AGENTS.md (later)".to_string()),
        "{got:?}"
    );
    assert!(
        !got.iter().any(|n| n.starts_with("crates/core/AGENTS.md")),
        "{got:?}"
    );
    // So there's nothing to say about instructions Claude Code doesn't read.
    let found = checks(Agent::Claude, &f.project, &context);
    assert!(!found.iter().any(|c| c.kind == CheckKind::NotRead));

    // A CLAUDE.local.md counts as a CLAUDE.md, and then AGENTS.md isn't read...
    write(&f.project.join("CLAUDE.local.md"), "Mine.\n");
    let context = discover(Agent::Claude, &f.project, &f.homes);
    let got = names(&context, &f.project, &f.home);
    assert!(!got.contains(&"AGENTS.md".to_string()), "{got:?}");
    // ...unless the user set Claude Code to read both.
    write(
        &f.homes.claude.as_ref().unwrap().join("settings.json"),
        r#"{"pluginConfigs": {"cc-plugin-agents-md@builtin": {"options": {"instructionFiles": "claude-md-and-agents-md"}}}}"#,
    );
    let context = discover(Agent::Claude, &f.project, &f.homes);
    let got = names(&context, &f.project, &f.home);
    assert!(got.contains(&"AGENTS.md".to_string()), "{got:?}");
}

#[test]
fn claude_code_rules_are_found_in_subfolders_and_path_rules_load_later() {
    let f = fixture();
    write(&f.project.join(".claude/rules/testing.md"), "Test first.\n");
    write(
        &f.project.join(".claude/rules/frontend/react.md"),
        "---\npaths:\n  - \"src/**/*.tsx\"\n---\nHooks at the top.\n",
    );
    write(
        &f.project.join("crates/core/CLAUDE.local.md"),
        "Mine, for core.\n",
    );
    let context = discover(Agent::Claude, &f.project, &f.homes);
    let got = names(&context, &f.project, &f.home);
    assert!(
        got.contains(&".claude/rules/testing.md".to_string()),
        "{got:?}"
    );
    assert!(
        got.contains(&".claude/rules/frontend/react.md (later)".to_string()),
        "a rule for some paths loads when they're worked on: {got:?}"
    );
    assert!(
        got.contains(&"crates/core/CLAUDE.local.md (later)".to_string()),
        "{got:?}"
    );
}

#[test]
fn claude_code_memory_is_the_repositorys_and_cut_at_25_kb() {
    let f = fixture();
    // Opened in a subfolder of the repository, it's still the repository's memory.
    let web = f.project.join("web");
    fs::create_dir_all(&web).unwrap();
    let context = discover(Agent::Claude, &web, &f.homes);
    let memory = context
        .instructions
        .iter()
        .find(|i| i.scope == Scope::Memory)
        .expect("the repository's memory");
    assert_eq!(memory.limit, Some(MEMORY_LINES));
    // 150 long lines are over 25 KB long before 200 lines.
    let claude = f.homes.claude.as_ref().unwrap();
    let path = claude_project_dir(claude, &f.project).join("memory/MEMORY.md");
    let line = format!("- {}\n", "x".repeat(270));
    write(&path, &line.repeat(150));
    let context = discover(Agent::Claude, &f.project, &f.homes);
    let memory = context
        .instructions
        .iter()
        .find(|i| i.scope == Scope::Memory)
        .unwrap();
    assert_eq!(memory.limit, Some(MEMORY_BYTES / line.len()));
    let found = checks(Agent::Claude, &f.project, &context);
    assert!(
        found.iter().any(|c| c.kind == CheckKind::CutOff),
        "{found:?}"
    );
}

#[test]
fn imports_go_four_hops_deep() {
    let f = fixture();
    write(&f.project.join("CLAUDE.md"), "@one.md\n");
    for (file, next) in [
        ("one", "two"),
        ("two", "three"),
        ("three", "four"),
        ("four", "five"),
    ] {
        write(
            &f.project.join(format!("{file}.md")),
            &format!("@{next}.md\n"),
        );
    }
    write(&f.project.join("five.md"), "Too deep.\n");
    let context = discover(Agent::Claude, &f.project, &f.homes);
    let got = names(&context, &f.project, &f.home);
    assert!(got.contains(&"        four.md".to_string()), "{got:?}");
    assert!(!got.iter().any(|n| n.contains("five.md")), "{got:?}");
}
