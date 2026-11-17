//! Roles: small prompt files that start an agent on one job. A role is Markdown, its front
//! matter saying which agent runs it (`agent`) and what it's for in a few words
//! (`description`), and its body the first prompt that agent is given. The side pane's +
//! lists them under ROLES: the project's own in `.cellix/roles`, the person's in
//! `~/.config/cellix/roles`, and Cellix's built in, a name found sooner hiding the same name
//! later.

use std::path::{Path, PathBuf};

use cellix_agent::context::front_matter;

/// Cellix's own roles, each as the file it would be: its name, and its text. Another is a
/// file under `roles/` and a line here.
const BUILT_IN: &[(&str, &str)] = &[("tester", include_str!("../roles/tester.md"))];

/// Where a role was found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Project,
    Person,
    BuiltIn,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Role {
    /// What the menu calls it: `name` from its front matter, or its file's name.
    pub name: String,
    /// The agent that runs it, as its front matter names it: a program (`claude`) or what
    /// Cellix calls the agent (`Claude Code`). None for the first agent installed.
    pub agent: Option<String>,
    pub description: String,
    /// What the agent is told first.
    pub prompt: String,
    pub source: Source,
}

impl Role {
    /// Reads a role from its file's text. `stem` is the file's name without `.md`.
    pub fn parse(stem: &str, text: &str, source: Source) -> Self {
        let front = front_matter(text);
        let field = |key: &str| {
            front
                .iter()
                .find(|(k, v)| k == key && !v.is_empty())
                .map(|(_, v)| v.clone())
        };
        Self {
            name: field("name").unwrap_or_else(|| title(stem)),
            agent: field("agent"),
            description: field("description").unwrap_or_default(),
            prompt: body(text).trim().to_string(),
            source,
        }
    }

    /// The program of the agent that runs it, among `installed` (the programs of the agents
    /// that are): the one it names, or with none named, the first installed. None when the
    /// one it names isn't one Cellix starts.
    pub fn program(
        &self,
        known: &[(&'static str, &'static str)],
        installed: &[&'static str],
    ) -> Option<&'static str> {
        let Some(wanted) = self.agent.as_deref() else {
            return installed.first().copied();
        };
        let wanted = wanted.trim();
        let same = |a: &str| a.eq_ignore_ascii_case(wanted);
        known
            .iter()
            .find(|(name, program)| {
                same(name) || same(program) || same(name.split(' ').next().unwrap_or(name))
            })
            .map(|(_, program)| *program)
    }
}

/// The text after a role's front matter.
fn body(text: &str) -> &str {
    let Some(rest) = text.trim_start().strip_prefix("---") else {
        return text;
    };
    match rest.split_once("\n---") {
        Some((_, after)) => after.split_once('\n').map_or("", |(_, b)| b),
        None => text,
    }
}

/// A file's name as a role's: words parted by `-` or `_`, the first capitalised.
fn title(stem: &str) -> String {
    let words = stem.replace(['-', '_'], " ");
    let mut chars = words.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(chars).collect()
    })
}

/// Every role, the project's first, then the person's, then Cellix's own, each set by name.
/// `home` is the person's home folder, or None to leave theirs out, as tests do.
pub fn find(project: &Path, home: Option<&Path>) -> Vec<Role> {
    let mut roles: Vec<Role> = Vec::new();
    let mut add = |role: Role| {
        if !roles
            .iter()
            .any(|r| r.name.eq_ignore_ascii_case(&role.name))
        {
            roles.push(role);
        }
    };
    let mut folders = vec![(project.join(".cellix/roles"), Source::Project)];
    if let Some(home) = home {
        folders.push((home.join(".config/cellix/roles"), Source::Person));
    }
    for (folder, source) in folders {
        for (stem, text) in files_in(&folder) {
            add(Role::parse(&stem, &text, source));
        }
    }
    for (stem, text) in BUILT_IN {
        add(Role::parse(stem, text, Source::BuiltIn));
    }
    roles
}

/// The Markdown files in a folder, by name, with their text.
fn files_in(folder: &Path) -> Vec<(String, String)> {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return Vec::new();
    };
    let mut paths: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "md") && p.is_file())
        .collect();
    paths.sort();
    paths
        .into_iter()
        .filter_map(|path| {
            let stem = path.file_stem()?.to_string_lossy().into_owned();
            let text = std::fs::read_to_string(&path).ok()?;
            Some((stem, text))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const AGENTS: [(&str, &str); 3] = [
        ("Claude Code", "claude"),
        ("Codex", "codex"),
        ("Gemini CLI", "gemini"),
    ];

    #[test]
    fn a_role_is_front_matter_and_a_first_prompt() {
        let text = "---\nagent: codex\ndescription: reads the changes\n---\nReview what changed.\n\nSay what's wrong.\n";
        let role = Role::parse("code-reviewer", text, Source::Project);
        assert_eq!(role.name, "Code reviewer", "named after its file");
        assert_eq!(role.agent.as_deref(), Some("codex"));
        assert_eq!(role.description, "reads the changes");
        assert_eq!(role.prompt, "Review what changed.\n\nSay what's wrong.");
        assert_eq!(role.program(&AGENTS, &["claude", "codex"]), Some("codex"));
        // An agent is named as Cellix calls it too, or not at all: then the first installed.
        let named = Role::parse(
            "x",
            "---\nagent: Gemini CLI\nname: Docs\n---\nGo.",
            Source::Person,
        );
        assert_eq!(named.name, "Docs");
        assert_eq!(named.program(&AGENTS, &["claude"]), Some("gemini"));
        let any = Role::parse("x", "Just go.", Source::Person);
        assert_eq!(any.prompt, "Just go.");
        assert_eq!(any.program(&AGENTS, &["codex", "claude"]), Some("codex"));
        assert_eq!(any.program(&AGENTS, &[]), None);
        let unknown = Role::parse("x", "---\nagent: aider\n---\nGo.", Source::Person);
        assert_eq!(unknown.program(&AGENTS, &["claude"]), None);
    }

    #[test]
    fn the_projects_roles_come_first_and_hide_the_same_name_later() {
        let project = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        let ours = project.path().join(".cellix/roles");
        let theirs = home.path().join(".config/cellix/roles");
        std::fs::create_dir_all(&ours).unwrap();
        std::fs::create_dir_all(&theirs).unwrap();
        std::fs::write(
            ours.join("tester.md"),
            "---\ndescription: ours\n---\nRun make check.",
        )
        .unwrap();
        std::fs::write(ours.join("notes.txt"), "not a role").unwrap();
        std::fs::write(
            theirs.join("planner.md"),
            "---\nagent: claude\n---\nPlan it.",
        )
        .unwrap();
        let roles = find(project.path(), Some(home.path()));
        let names: Vec<(&str, Source)> =
            roles.iter().map(|r| (r.name.as_str(), r.source)).collect();
        assert_eq!(
            names,
            [("Tester", Source::Project), ("Planner", Source::Person)]
        );
        assert_eq!(roles[0].prompt, "Run make check.");
        // Without those, Cellix's own.
        let empty = tempfile::tempdir().unwrap();
        let built: Vec<String> = find(empty.path(), None)
            .into_iter()
            .map(|r| r.name)
            .collect();
        assert_eq!(built, ["Tester"]);
    }

    #[test]
    fn the_built_in_tester_runs_the_tests_and_changes_nothing() {
        let empty = tempfile::tempdir().unwrap();
        let tester = find(empty.path(), None).remove(0);
        assert_eq!(tester.source, Source::BuiltIn);
        assert!(!tester.description.is_empty());
        assert!(tester.prompt.contains("test"), "{}", tester.prompt);
        assert!(tester.prompt.contains("Don't change"), "{}", tester.prompt);
    }
}
