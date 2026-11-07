//! Tasks: the unit of agent work Cellix shows. A task is one agent session's turns, split where
//! the session sends a new plan after one was approved, or where the reviewer starts a new task
//! by hand. Agents' hooks append what happened to a log in the project's `.cellix` folder, and
//! the tasks are worked out from it each time it's read, so nothing else needs to agree on them.

use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::history::{Turn, checkpoint};

/// The log, one event a line, under the project.
const FILE: &str = ".cellix/tasks.jsonl";
/// What `.cellix/.gitignore` holds, so nothing Cellix keeps there shows as a change.
pub const IGNORED: &str = "threads.json\nthreads.json.tmp\nplans/\ntasks.jsonl\n.gitignore\n";
/// Longest prompt kept. A pasted log can be megabytes; the start says what was asked.
const PROMPT_LENGTH: usize = 4000;
/// Longest test output kept, from the end, where failures are.
const OUTPUT_LENGTH: usize = 8000;
/// Longest name made from a prompt.
const NAME_LENGTH: usize = 70;
/// Set by the prompt hook for `cellix checkpoint`, which logs the prompt with its checkpoint:
/// the prompt, and the project whose log it goes in.
pub const EVENT_VAR: &str = "CELLIX_TASK_EVENT";
pub const ROOT_VAR: &str = "CELLIX_TASK_ROOT";

/// One thing that happened, as an agent's hooks or the reviewer report it. Times are
/// milliseconds since the epoch.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Event {
    /// The user prompted an agent: a turn begins.
    Prompt {
        at: u64,
        session: String,
        agent: String,
        text: String,
        /// The checkpoint taken as it began, where there's git.
        #[serde(default)]
        commit: Option<String>,
        /// The highest thread id then, so the threads made during the turn can be told.
        #[serde(default)]
        threads: u32,
    },
    /// An agent sent a plan for review. `file` is where its text is.
    Plan {
        at: u64,
        session: String,
        agent: String,
        file: String,
    },
    /// The reviewer answered the plan at `file`.
    Answer {
        at: u64,
        file: String,
        approved: bool,
        /// What was said when sending it back.
        #[serde(default)]
        note: String,
        /// Where its versions went once approved.
        #[serde(default)]
        kept: Option<String>,
    },
    /// Tests ran during a turn.
    Test {
        at: u64,
        session: String,
        command: String,
        passed: bool,
        #[serde(default)]
        output: String,
    },
    /// The reviewer started a new task at the turn that began at `at`.
    Split { at: u64, session: String },
    /// The reviewer named a task.
    Rename { at: u64, task: String, name: String },
}

impl Event {
    pub fn at(&self) -> u64 {
        match self {
            Event::Prompt { at, .. }
            | Event::Plan { at, .. }
            | Event::Answer { at, .. }
            | Event::Test { at, .. }
            | Event::Split { at, .. }
            | Event::Rename { at, .. } => *at,
        }
    }

    /// Splits sort before the prompt they split at, and renames after everything.
    fn rank(&self) -> u8 {
        match self {
            Event::Split { .. } => 0,
            Event::Rename { .. } => 2,
            _ => 1,
        }
    }
}

/// Milliseconds since the epoch, now.
pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

fn file(root: &Path) -> PathBuf {
    root.join(FILE)
}

/// Makes sure `.cellix` ignores what Cellix keeps in it, including a `.gitignore` written
/// before the task log existed.
pub fn ignore_own_files(folder: &Path) -> io::Result<()> {
    let ignore = folder.join(".gitignore");
    match fs::read_to_string(&ignore) {
        Ok(text) if text.lines().any(|l| l == "tasks.jsonl") => Ok(()),
        Ok(text) => {
            let mut text = text;
            if !text.ends_with('\n') && !text.is_empty() {
                text.push('\n');
            }
            text.push_str("tasks.jsonl\n");
            fs::write(ignore, text)
        }
        Err(_) => fs::write(ignore, IGNORED),
    }
}

/// Adds an event to the project's log. One short write in append mode, so hooks running at
/// once don't tear each other's lines.
pub fn record(root: &Path, event: &Event) -> io::Result<()> {
    let path = file(root);
    if let Some(folder) = path.parent() {
        fs::create_dir_all(folder)?;
        ignore_own_files(folder)?;
    }
    let mut line = serde_json::to_string(event)?;
    line.push('\n');
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?
        .write_all(line.as_bytes())
}

/// Every event logged for the project, in the order written. A line that can't be read, as
/// one cut short by a full disk, is skipped.
pub fn load(root: &Path) -> Vec<Event> {
    fs::read_to_string(file(root))
        .unwrap_or_default()
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}

/// When the log last changed, to know when to read it again.
pub fn changed(root: &Path) -> Option<SystemTime> {
    fs::metadata(file(root)).and_then(|m| m.modified()).ok()
}

/// The start of a prompt, as much as the log keeps.
pub fn clip_prompt(text: &str) -> String {
    match text.char_indices().nth(PROMPT_LENGTH) {
        Some((at, _)) => format!("{}…", &text[..at]),
        None => text.to_string(),
    }
}

/// The end of a test's output, as much as the log keeps.
pub fn clip_output(text: &str) -> String {
    let start = text
        .char_indices()
        .rev()
        .nth(OUTPUT_LENGTH)
        .map_or(0, |(at, _)| at);
    text[start..].to_string()
}

/// Checkpoints the tree for a prompt and logs the prompt with it in the log at `log`. The
/// prompt is logged even where there's no git to checkpoint.
pub fn checkpoint_prompt(root: &Path, label: &str, prompt: Option<(&Path, Event)>) -> Option<Turn> {
    let turn = checkpoint(root, label);
    if let Some((
        log,
        Event::Prompt {
            at,
            session,
            agent,
            text,
            threads,
            ..
        },
    )) = prompt
    {
        let event = Event::Prompt {
            at,
            session,
            agent,
            text,
            commit: turn.as_ref().map(|t| t.commit.clone()),
            threads,
        };
        let _ = record(log, &event);
    }
    turn
}

/// How a plan's review ended, or that it hasn't.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    Waiting,
    SentBack(String),
    /// With the folder its versions were put away in.
    Approved(Option<String>),
}

/// One version of a plan.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanStep {
    pub at: u64,
    /// Counting from one, within the task.
    pub version: usize,
    pub file: String,
    pub outcome: Outcome,
    /// When it was answered.
    pub answered: Option<u64>,
}

/// A test run during a turn.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TestStep {
    pub at: u64,
    pub command: String,
    pub passed: bool,
    pub output: String,
}

/// One prompt and what the agent did with it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TurnStep {
    pub at: u64,
    /// Counting from one, within the task.
    pub number: usize,
    pub text: String,
    /// The checkpoint taken as it began.
    pub commit: Option<String>,
    /// The next checkpoint, as the next turn in the project began, or None while this is the
    /// last: then what it changed runs up to the tree as it is.
    pub until: Option<String>,
    /// When the next turn in the project began.
    pub ended: Option<u64>,
    pub tests: Vec<TestStep>,
    /// Thread ids made while it was the latest turn: above the first, up to the second.
    pub threads: (u32, Option<u32>),
}

impl TurnStep {
    /// Whether a thread was made while this turn was the latest.
    pub fn holds_thread(&self, id: u32) -> bool {
        id > self.threads.0 && self.threads.1.is_none_or(|last| id <= last)
    }

    /// Whether any test run in it failed and none passed after.
    pub fn tests_failing(&self) -> bool {
        self.tests.last().is_some_and(|t| !t.passed)
    }
}

/// Something in a task's timeline.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    Plan(PlanStep),
    Turn(TurnStep),
}

impl Step {
    pub fn at(&self) -> u64 {
        match self {
            Step::Plan(plan) => plan.at,
            Step::Turn(turn) => turn.at,
        }
    }
}

/// One piece of agent work.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Task {
    /// The session and when the task began, which no later event changes.
    pub id: String,
    pub name: String,
    /// Whether the reviewer named it.
    pub named: bool,
    pub agent: String,
    pub session: String,
    pub steps: Vec<Step>,
}

impl Task {
    pub fn started(&self) -> u64 {
        self.steps.first().map_or(0, Step::at)
    }

    /// When the last thing in it happened.
    pub fn latest(&self) -> u64 {
        self.steps
            .iter()
            .map(|s| match s {
                Step::Plan(p) => p.answered.unwrap_or(p.at),
                Step::Turn(t) => t.tests.last().map_or(t.at, |r| r.at),
            })
            .max()
            .unwrap_or(0)
    }

    pub fn turns(&self) -> impl DoubleEndedIterator<Item = &TurnStep> {
        self.steps.iter().filter_map(|s| match s {
            Step::Turn(t) => Some(t),
            Step::Plan(_) => None,
        })
    }

    pub fn plans(&self) -> impl DoubleEndedIterator<Item = &PlanStep> {
        self.steps.iter().filter_map(|s| match s {
            Step::Plan(p) => Some(p),
            Step::Turn(_) => None,
        })
    }

    /// The plan waiting for the reviewer, if one is.
    pub fn waiting_plan(&self) -> Option<&PlanStep> {
        self.plans().rev().find(|p| p.outcome == Outcome::Waiting)
    }

    fn approved(&self) -> bool {
        self.plans()
            .any(|p| matches!(p.outcome, Outcome::Approved(_)))
    }
}

/// A name from what was asked: its first line, cut to fit.
pub fn name_from(text: &str) -> String {
    let line = text
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("");
    match line.char_indices().nth(NAME_LENGTH) {
        Some((at, _)) => format!("{}…", line[..at].trim_end()),
        None => line.to_string(),
    }
}

/// The tasks in a log, oldest first. `plan_title` gives a plan's heading, to name a task that
/// began with a plan and has no prompt to be named from.
pub fn tasks(events: &[Event], plan_title: impl Fn(&str) -> Option<String>) -> Vec<Task> {
    let mut events: Vec<&Event> = events.iter().collect();
    events.sort_by_key(|e| (e.at(), e.rank()));
    let mut tasks: Vec<Task> = Vec::new();
    // Each session's task in progress, by index into `tasks`.
    let mut current: HashMap<&str, usize> = HashMap::new();
    let mut splits: HashMap<&str, Vec<u64>> = HashMap::new();
    let mut names: HashMap<&str, &str> = HashMap::new();
    // Every turn in the project, as (task, step), to know where each one ended.
    let mut order: Vec<(usize, usize)> = Vec::new();

    let begin = |tasks: &mut Vec<Task>, session: &str, agent: &str, at: u64| {
        tasks.push(Task {
            id: format!("{session}@{at}"),
            name: String::new(),
            named: false,
            agent: agent.to_string(),
            session: session.to_string(),
            steps: Vec::new(),
        });
        tasks.len() - 1
    };

    for event in &events {
        match event {
            Event::Split { at, session } => splits.entry(session.as_str()).or_default().push(*at),
            Event::Rename { task, name, .. } => {
                names.insert(task.as_str(), name.as_str());
            }
            Event::Prompt {
                at,
                session,
                agent,
                text,
                commit,
                threads,
            } => {
                let split = splits
                    .get_mut(session.as_str())
                    .and_then(|s| s.iter().position(|t| t <= at).map(|i| s.remove(i)))
                    .is_some();
                let index = match current.get(session.as_str()) {
                    Some(&index) if !split => index,
                    _ => begin(&mut tasks, session, agent, *at),
                };
                current.insert(session.as_str(), index);
                let task = &mut tasks[index];
                let number = task.turns().count() + 1;
                task.steps.push(Step::Turn(TurnStep {
                    at: *at,
                    number,
                    text: text.clone(),
                    commit: commit.clone(),
                    until: None,
                    ended: None,
                    tests: Vec::new(),
                    threads: (*threads, None),
                }));
                if let Some(&(before, step)) = order.last()
                    && let Step::Turn(previous) = &mut tasks[before].steps[step]
                {
                    previous.until.clone_from(commit);
                    previous.ended = Some(*at);
                    previous.threads.1 = Some(*threads);
                }
                order.push((index, tasks[index].steps.len() - 1));
            }
            Event::Plan {
                at,
                session,
                agent,
                file,
            } => {
                let index = match current.get(session.as_str()) {
                    Some(&index) if tasks[index].approved() => {
                        // The turn that led to this plan began the new task, unless it was the
                        // one that carried out the plan approved before.
                        let approved_at = tasks[index]
                            .plans()
                            .filter_map(|p| p.answered)
                            .max()
                            .unwrap_or(0);
                        let moved = match tasks[index].steps.last() {
                            Some(Step::Turn(turn)) if turn.at > approved_at => {
                                tasks[index].steps.pop()
                            }
                            _ => None,
                        };
                        let start = moved.as_ref().map_or(*at, Step::at);
                        let new = begin(&mut tasks, session, agent, start);
                        if let Some(Step::Turn(mut turn)) = moved {
                            turn.number = 1;
                            if let Some(slot) = order.iter_mut().rev().find(|o| o.0 == index) {
                                *slot = (new, 0);
                            }
                            tasks[new].steps.push(Step::Turn(turn));
                        }
                        new
                    }
                    Some(&index) => index,
                    None => begin(&mut tasks, session, agent, *at),
                };
                current.insert(session.as_str(), index);
                let task = &mut tasks[index];
                let version = task.plans().count() + 1;
                task.steps.push(Step::Plan(PlanStep {
                    at: *at,
                    version,
                    file: file.clone(),
                    outcome: Outcome::Waiting,
                    answered: None,
                }));
            }
            Event::Answer {
                at,
                file,
                approved,
                note,
                kept,
            } => {
                let waiting = tasks.iter_mut().rev().find_map(|task| {
                    task.steps.iter_mut().rev().find_map(|step| match step {
                        Step::Plan(plan)
                            if plan.file == *file && plan.outcome == Outcome::Waiting =>
                        {
                            Some(plan)
                        }
                        _ => None,
                    })
                });
                if let Some(plan) = waiting {
                    plan.answered = Some(*at);
                    plan.outcome = if *approved {
                        Outcome::Approved(kept.clone())
                    } else {
                        Outcome::SentBack(note.clone())
                    };
                }
            }
            Event::Test {
                at,
                session,
                command,
                passed,
                output,
            } => {
                let turn = current.get(session.as_str()).and_then(|&index| {
                    tasks[index].steps.iter_mut().rev().find_map(|s| match s {
                        Step::Turn(t) => Some(t),
                        Step::Plan(_) => None,
                    })
                });
                if let Some(turn) = turn {
                    turn.tests.push(TestStep {
                        at: *at,
                        command: command.clone(),
                        passed: *passed,
                        output: output.clone(),
                    });
                }
            }
        }
    }

    tasks.retain(|t| !t.steps.is_empty());
    for task in &mut tasks {
        if let Some(name) = names.get(task.id.as_str()) {
            task.name = (*name).to_string();
            task.named = true;
            continue;
        }
        let asked = task
            .turns()
            .map(|t| name_from(&t.text))
            .find(|n| !n.is_empty());
        let planned = task.plans().next().and_then(|p| plan_title(&p.file));
        task.name = asked
            .or(planned)
            .unwrap_or_else(|| "Untitled task".to_string());
    }
    tasks
}

/// The heading a plan starts with, as a name for its task.
pub fn plan_title(file: &str) -> Option<String> {
    let text = fs::read_to_string(file).ok()?;
    text.lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .map(|l| name_from(l.trim_start_matches('#')))
}

/// A plan's versions as files, oldest first, the one at `step.file` last. While the plan is in
/// review its earlier versions sit beside it; once approved they're in the folder it was put
/// away in.
pub fn plan_versions(task: &Task, step: &PlanStep) -> Vec<PathBuf> {
    let path = PathBuf::from(&step.file);
    let folder = task
        .plans()
        .filter(|p| p.file == step.file)
        .find_map(|p| match &p.outcome {
            Outcome::Approved(Some(kept)) => Some(PathBuf::from(kept)),
            _ => None,
        })
        .or_else(|| path.parent().map(Path::to_path_buf))
        .unwrap_or_default();
    let count = task.plans().filter(|p| p.file == step.file).count();
    (1..=count)
        .map(|version| {
            if version == count {
                folder.join("Plan.md")
            } else {
                folder.join(format!("Plan v{version}.md"))
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prompt(at: u64, session: &str, text: &str, threads: u32) -> Event {
        Event::Prompt {
            at,
            session: session.to_string(),
            agent: "Claude Code".to_string(),
            text: text.to_string(),
            commit: Some(format!("c{at}")),
            threads,
        }
    }

    fn plan(at: u64, session: &str) -> Event {
        Event::Plan {
            at,
            session: session.to_string(),
            agent: "Claude Code".to_string(),
            file: format!("/plans/{session}/Plan.md"),
        }
    }

    fn answer(at: u64, session: &str, approved: bool) -> Event {
        Event::Answer {
            at,
            file: format!("/plans/{session}/Plan.md"),
            approved,
            note: if approved {
                String::new()
            } else {
                "Only paid orders".to_string()
            },
            kept: approved.then(|| format!("/plans/{session}/Approved 1")),
        }
    }

    fn test(at: u64, session: &str, passed: bool) -> Event {
        Event::Test {
            at,
            session: session.to_string(),
            command: "cargo test".to_string(),
            passed,
            output: String::new(),
        }
    }

    #[test]
    fn a_session_is_one_task_and_a_plan_after_an_approved_one_starts_another() {
        let events = [
            prompt(10, "s1", "Add a refund route", 0),
            plan(12, "s1"),
            answer(14, "s1", false),
            plan(16, "s1"),
            answer(18, "s1", true),
            test(19, "s1", false),
            test(20, "s1", true),
            prompt(30, "s1", "Also reject unpaid orders", 2),
            test(31, "s1", true),
            // A new plan after the approved one: the prompt that led to it begins a new task.
            prompt(40, "s1", "Split refunds into partial and full", 3),
            plan(42, "s1"),
        ];
        let tasks = tasks(&events, |_| None);
        assert_eq!(tasks.len(), 2);
        let first = &tasks[0];
        assert_eq!(first.name, "Add a refund route");
        let kinds: Vec<String> = first
            .steps
            .iter()
            .map(|s| match s {
                Step::Turn(t) => format!("turn {}", t.number),
                Step::Plan(p) => format!("plan v{} {:?}", p.version, p.outcome),
            })
            .collect();
        assert_eq!(
            kinds,
            [
                "turn 1",
                "plan v1 SentBack(\"Only paid orders\")",
                "plan v2 Approved(Some(\"/plans/s1/Approved 1\"))",
                "turn 2"
            ]
        );
        let turns: Vec<&TurnStep> = first.turns().collect();
        assert_eq!(turns[0].tests.len(), 2);
        assert!(!turns[0].tests_failing());
        assert_eq!(turns[0].until.as_deref(), Some("c30"));
        assert_eq!(turns[1].until.as_deref(), Some("c40"));
        assert!(turns[1].holds_thread(3) && !turns[1].holds_thread(2) && !turns[1].holds_thread(4));

        let second = &tasks[1];
        assert_eq!(second.name, "Split refunds into partial and full");
        assert_eq!(second.id, "s1@40");
        let turn = second.turns().next().unwrap();
        assert_eq!(turn.number, 1);
        assert_eq!(turn.until, None);
        assert!(turn.holds_thread(99));
        assert_eq!(second.waiting_plan().map(|p| p.version), Some(1));
    }

    #[test]
    fn sessions_are_tasks_of_their_own_and_turns_end_at_the_next_in_the_project() {
        let events = [
            prompt(10, "a", "Fix the queue", 0),
            prompt(20, "b", "Write docs", 0),
            test(25, "a", false),
            prompt(30, "a", "Try again", 0),
        ];
        let tasks = tasks(&events, |_| None);
        assert_eq!(tasks.len(), 2);
        let a: Vec<&TurnStep> = tasks[0].turns().collect();
        assert_eq!(a.len(), 2);
        // Checkpoints are the project's, so a turn's changes end where any next turn began.
        assert_eq!(a[0].until.as_deref(), Some("c20"));
        assert!(a[0].tests_failing());
        assert_eq!(
            tasks[1].turns().next().unwrap().until.as_deref(),
            Some("c30")
        );
    }

    #[test]
    fn the_reviewer_can_split_and_rename_a_task() {
        let mut events = vec![
            prompt(10, "s", "One", 0),
            prompt(20, "s", "Two", 0),
            prompt(30, "s", "Three", 0),
        ];
        events.push(Event::Split {
            at: 20,
            session: "s".to_string(),
        });
        events.push(Event::Rename {
            at: 50,
            task: "s@20".to_string(),
            name: "The second half".to_string(),
        });
        let tasks = tasks(&events, |_| None);
        assert_eq!(tasks.len(), 2);
        assert_eq!(tasks[0].name, "One");
        assert_eq!(tasks[0].turns().count(), 1);
        assert_eq!(tasks[1].name, "The second half");
        assert!(tasks[1].named);
        let numbers: Vec<usize> = tasks[1].turns().map(|t| t.number).collect();
        assert_eq!(numbers, [1, 2]);
    }

    #[test]
    fn a_plan_with_no_prompt_names_its_task_from_its_heading() {
        let tasks = tasks(&[plan(5, "s")], |_| Some("Partial refunds".to_string()));
        assert_eq!(tasks[0].name, "Partial refunds");
    }

    #[test]
    fn the_log_keeps_its_events_and_stays_out_of_review() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join(".cellix");
        fs::create_dir_all(&folder).unwrap();
        // A .gitignore from before the log existed learns of it.
        fs::write(folder.join(".gitignore"), "threads.json\n").unwrap();
        record(dir.path(), &prompt(1, "s", "Hello", 0)).unwrap();
        record(dir.path(), &test(2, "s", true)).unwrap();
        fs::write(
            file(dir.path()),
            fs::read_to_string(file(dir.path())).unwrap() + "{\"kind\":\"pro",
        )
        .unwrap();
        assert_eq!(load(dir.path()).len(), 2);
        let ignore = fs::read_to_string(folder.join(".gitignore")).unwrap();
        assert!(ignore.lines().any(|l| l == "tasks.jsonl"));
    }

    #[test]
    fn long_prompts_and_outputs_are_clipped() {
        let long = "x".repeat(PROMPT_LENGTH + 10);
        assert!(clip_prompt(&long).ends_with('…'));
        assert_eq!(clip_prompt("short"), "short");
        let output = format!("{}FAILED", "y".repeat(OUTPUT_LENGTH * 2));
        assert!(clip_output(&output).ends_with("FAILED"));
        assert!(clip_output(&output).len() <= OUTPUT_LENGTH + 1);
        assert_eq!(name_from("\n  Add a route\nmore"), "Add a route");
    }
}
