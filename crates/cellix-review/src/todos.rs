//! To-dos: the workspace's shared list. Agents' hooks and `cellix todo` append what happened
//! to `.cellix/todos.jsonl`, and the list is worked out from it whenever it's read, so no two
//! writers need to agree. One level of sub-tasks: a parent's state follows its children's.
//! Times are milliseconds since the epoch.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// The log, one event a line, under the project.
const FILE: &str = ".cellix/todos.jsonl";
/// Longest item text kept. A pasted log can be megabytes; the start says what it's for. The
/// hook caps an agent's own words with it too.
pub const TEXT_LENGTH: usize = 2000;
/// The person's own name, as `owner` and `source`.
pub const YOU: &str = "you";

/// A to-do's state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Todo,
    Doing,
    Done,
}

impl State {
    /// The state an agent's tool name gives: Claude Code's to-dos are pending, in_progress or
    /// completed; anything else is still to do.
    pub fn from_tool(name: &str) -> Self {
        match name {
            "in_progress" | "doing" => Self::Doing,
            "completed" | "done" => Self::Done,
            _ => Self::Todo,
        }
    }

    /// The state the CLI writes: todo, doing or done.
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "todo" => Some(Self::Todo),
            "doing" => Some(Self::Doing),
            "done" => Some(Self::Done),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Todo => "todo",
            Self::Doing => "doing",
            Self::Done => "done",
        }
    }
}

/// One to-do after the log is replayed. `cellix todo list --json` prints them in this shape.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Item {
    pub id: String,
    pub text: String,
    /// The agent's active form while doing ("Running the tests").
    pub active: String,
    pub state: State,
    /// The tab's name, or "you".
    pub owner: String,
    /// The merge key: "you", or an agent's session ("claude:<session>").
    pub source: String,
    /// One level only.
    pub parent: Option<String>,
    /// When it first appeared.
    pub at: u64,
}

/// One item in a `replace`, as an agent's `TodoWrite` gives it. The id is Cellix's own: a
/// mirror writes the ids it matched or minted, so every reader of the log agrees on them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mirrored {
    #[serde(default)]
    pub id: Option<String>,
    pub text: String,
    pub state: State,
    #[serde(default)]
    pub active: String,
}

/// One thing that happened.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Event {
    /// A to-do was added. `parent` names a top-level item, and only one level is kept.
    Add {
        at: u64,
        id: String,
        text: String,
        #[serde(default)]
        owner: String,
        #[serde(default)]
        source: String,
        #[serde(default)]
        parent: Option<String>,
        #[serde(default)]
        active: String,
    },
    /// One item changed state.
    Set { at: u64, id: String, state: State },
    /// A parent and every child of it changed state at once: the parent's box.
    Group {
        at: u64,
        parent: String,
        state: State,
    },
    /// An item's words changed.
    Edit { at: u64, id: String, text: String },
    /// An item's active form changed.
    Active { at: u64, id: String, text: String },
    /// An item was removed, and a parent takes its children with it.
    Remove { at: u64, id: String },
    /// Every finished top-level group went, as Clear done does.
    Clear { at: u64 },
    /// An agent's whole list, as its `TodoWrite` gives it.
    Replace {
        at: u64,
        source: String,
        owner: String,
        items: Vec<Mirrored>,
    },
}

impl Event {
    pub fn at(&self) -> u64 {
        match self {
            Event::Add { at, .. }
            | Event::Set { at, .. }
            | Event::Group { at, .. }
            | Event::Edit { at, .. }
            | Event::Active { at, .. }
            | Event::Remove { at, .. }
            | Event::Clear { at }
            | Event::Replace { at, .. } => *at,
        }
    }
}

/// Milliseconds since the epoch, now.
pub fn now() -> u64 {
    crate::tasks::now()
}

/// A short id, from the clock's nanoseconds, the process and a counter, so writers at once
/// can't mint the same one. The CLI takes a prefix.
pub fn mint_id() -> String {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos() as u64);
    let seq = NEXT.fetch_add(1, Ordering::Relaxed);
    format!(
        "{:x}{:02x}{:x}",
        nanos & 0xffff_ffff,
        std::process::id() & 0xff,
        seq & 0xfff
    )
}

/// The most of a to-do's words kept.
pub fn clip(text: &str) -> String {
    match text.char_indices().nth(TEXT_LENGTH) {
        Some((at, _)) => format!("{}…", &text[..at]),
        None => text.to_string(),
    }
}

fn file(root: &Path) -> PathBuf {
    root.join(FILE)
}

/// Adds an event to the project's log. One short write in append mode, so agents' hooks, the
/// CLI and the app at once don't tear each other's lines.
pub fn record(root: &Path, event: &Event) -> io::Result<()> {
    let path = file(root);
    if let Some(folder) = path.parent() {
        fs::create_dir_all(folder)?;
        crate::tasks::ignore_own_files(folder)?;
    }
    let mut line = serde_json::to_string(event)?;
    line.push('\n');
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?
        .write_all(line.as_bytes())
}

/// Every event in the project's log, in the order written. A line that can't be read, as one
/// cut short by a full disk in the middle of a character, is skipped, and only that line.
pub fn load(root: &Path) -> Vec<Event> {
    fs::read(file(root))
        .unwrap_or_default()
        .split(|byte| *byte == b'\n')
        .filter_map(|line| serde_json::from_slice(line).ok())
        .collect()
}

/// When and how big the log was last seen, to know when to read it again. The length catches
/// a write in the same clock tick as the last.
pub fn changed(root: &Path) -> Option<(SystemTime, u64)> {
    let meta = fs::metadata(file(root)).ok()?;
    Some((meta.modified().ok()?, meta.len()))
}

/// The list the log makes, in the order its items first appeared.
pub fn list(root: &Path) -> Vec<Item> {
    list_from(&load(root))
}

/// The list a run of events makes.
pub fn list_from(events: &[Event]) -> Vec<Item> {
    let mut items = Vec::new();
    for event in events {
        apply(&mut items, event);
    }
    items
}

fn apply(items: &mut Vec<Item>, event: &Event) {
    match event {
        Event::Add {
            at,
            id,
            text,
            owner,
            source,
            parent,
            active,
        } => {
            if items.iter().any(|item| item.id == *id) {
                return;
            }
            // One level: a parent has to be a top-level item itself.
            let parent = parent.as_ref().and_then(|wanted| {
                items
                    .iter()
                    .find(|item| item.id == *wanted && item.parent.is_none())
                    .map(|item| item.id.clone())
            });
            items.push(Item {
                id: id.clone(),
                text: clip(text),
                active: clip(active),
                state: State::Todo,
                owner: owner.clone(),
                source: source.clone(),
                parent,
                at: *at,
            });
        }
        Event::Set { id, state, .. } => {
            if let Some(item) = items.iter_mut().find(|item| item.id == *id) {
                item.state = *state;
            }
        }
        Event::Group { parent, state, .. } => {
            for item in items.iter_mut() {
                if item.id == *parent || item.parent.as_deref() == Some(parent.as_str()) {
                    item.state = *state;
                }
            }
        }
        Event::Edit { id, text, .. } => {
            if let Some(item) = items.iter_mut().find(|item| item.id == *id) {
                item.text = clip(text);
            }
        }
        Event::Active { id, text, .. } => {
            if let Some(item) = items.iter_mut().find(|item| item.id == *id) {
                item.active = clip(text);
            }
        }
        Event::Remove { id, .. } => drop_item(items, id),
        Event::Clear { .. } => {
            // Only finished top-level groups go; a done child under an unfinished parent is
            // part of a group still under way and stays.
            let done: Vec<String> = items
                .iter()
                .filter(|item| item.parent.is_none() && effective(item, items) == State::Done)
                .map(|item| item.id.clone())
                .collect();
            for id in done {
                drop_item(items, &id);
            }
        }
        Event::Replace {
            source,
            owner,
            items: new,
            at,
        } => apply_replace(items, source, owner, new, *at),
    }
}

/// A parent's state as the list shows it: its children's, while it has any.
pub fn effective(item: &Item, items: &[Item]) -> State {
    let children: Vec<&Item> = items
        .iter()
        .filter(|child| child.parent.as_deref() == Some(item.id.as_str()))
        .collect();
    if children.is_empty() {
        return item.state;
    }
    if children.iter().all(|child| child.state == State::Done) {
        State::Done
    } else if children.iter().any(|child| child.state != State::Todo) {
        State::Doing
    } else {
        State::Todo
    }
}

/// The top-level items, in the order they appeared.
pub fn top_level(items: &[Item]) -> impl Iterator<Item = &Item> {
    items.iter().filter(|item| item.parent.is_none())
}

/// The children of `parent`, in the order they appeared.
pub fn children<'a>(items: &'a [Item], parent: &'a str) -> impl Iterator<Item = &'a Item> + 'a {
    items
        .iter()
        .filter(move |item| item.parent.as_deref() == Some(parent))
}

/// Removes an item and its children.
fn drop_item(items: &mut Vec<Item>, id: &str) {
    items.retain(|item| item.id != id && item.parent.as_deref() != Some(id));
}

fn apply_replace(items: &mut Vec<Item>, source: &str, owner: &str, new: &[Mirrored], at: u64) {
    let old: Vec<Item> = items
        .iter()
        .filter(|item| item.source == source && item.parent.is_none())
        .cloned()
        .collect();
    let old_ids: Vec<String> = old.iter().map(|item| item.id.clone()).collect();
    let merged = if new.iter().any(|item| item.id.is_some()) {
        merge_with_ids(&old, new, source, owner, at)
    } else {
        let fresh: Vec<(String, State, String)> = new
            .iter()
            .map(|item| (item.text.clone(), item.state, item.active.clone()))
            .collect();
        merge(&old, &fresh, source, owner, at)
    };
    for id in old_ids {
        if !merged.iter().any(|item| item.id == id) {
            drop_item(items, &id);
        }
    }
    for item in merged {
        if let Some(existing) = items.iter_mut().find(|existing| existing.id == item.id) {
            *existing = item;
        } else {
            items.push(item);
        }
    }
}

/// The merge a `TodoWrite` makes: text first, then the same list index for what's left. A
/// matched item keeps its id, its time and its place, and any children the person added under
/// it; a new one mints an id and comes after; an old one that's gone is removed.
fn merge(
    old: &[Item],
    new: &[(String, State, String)],
    source: &str,
    owner: &str,
    at: u64,
) -> Vec<Item> {
    let mut taken = vec![false; old.len()];
    let mut matched: Vec<Option<usize>> = vec![None; new.len()];
    for (ni, (text, _, _)) in new.iter().enumerate() {
        if let Some(oi) = (0..old.len()).find(|oi| !taken[*oi] && old[*oi].text == *text) {
            taken[oi] = true;
            matched[ni] = Some(oi);
        }
    }
    for (ni, chosen) in matched.iter_mut().enumerate() {
        if chosen.is_none() && ni < old.len() && !taken[ni] {
            taken[ni] = true;
            *chosen = Some(ni);
        }
    }
    let mut out = Vec::new();
    for (oi, item) in old.iter().enumerate() {
        if let Some(ni) = matched.iter().position(|chosen| *chosen == Some(oi)) {
            let (text, state, active) = &new[ni];
            out.push(Item {
                id: item.id.clone(),
                text: clip(text),
                active: clip(active),
                state: *state,
                owner: owner.to_string(),
                source: source.to_string(),
                parent: None,
                at: item.at,
            });
        }
    }
    for (ni, (text, state, active)) in new.iter().enumerate() {
        if matched[ni].is_none() {
            out.push(Item {
                id: mint_id(),
                text: clip(text),
                active: clip(active),
                state: *state,
                owner: owner.to_string(),
                source: source.to_string(),
                parent: None,
                at,
            });
        }
    }
    out
}

/// The merge when the event carries Cellix's ids, which every mirror writes: a matched id
/// keeps its place and children, an id not there is new, and the rest of the old goes.
fn merge_with_ids(old: &[Item], new: &[Mirrored], source: &str, owner: &str, at: u64) -> Vec<Item> {
    let mut out = Vec::new();
    for item in old {
        if let Some(given) = new
            .iter()
            .find(|given| given.id.as_deref() == Some(item.id.as_str()))
        {
            out.push(Item {
                id: item.id.clone(),
                text: clip(&given.text),
                active: clip(&given.active),
                state: given.state,
                owner: owner.to_string(),
                source: source.to_string(),
                parent: None,
                at: item.at,
            });
        }
    }
    for given in new {
        let matched = given
            .id
            .as_deref()
            .is_some_and(|id| old.iter().any(|item| item.id == id));
        if !matched {
            out.push(Item {
                id: given.id.clone().unwrap_or_else(mint_id),
                text: clip(&given.text),
                active: clip(&given.active),
                state: given.state,
                owner: owner.to_string(),
                source: source.to_string(),
                parent: None,
                at,
            });
        }
    }
    out
}

/// Mirrors an agent's whole list for `source`. A list that changes nothing in text, state,
/// active or order writes nothing, since `TodoWrite` is called again and again. Returns
/// whether a line was written.
pub fn mirror(
    root: &Path,
    source: &str,
    owner: &str,
    new: &[Mirrored],
    at: u64,
) -> io::Result<bool> {
    let items = list(root);
    let old: Vec<Item> = items
        .into_iter()
        .filter(|item| item.source == source && item.parent.is_none())
        .collect();
    let fresh: Vec<(String, State, String)> = new
        .iter()
        .map(|item| (item.text.clone(), item.state, item.active.clone()))
        .collect();
    let merged = merge(&old, &fresh, source, owner, at);
    let same = merged.len() == old.len()
        && merged.iter().zip(&old).all(|(now, before)| {
            now.text == before.text
                && now.state == before.state
                && now.active == before.active
                && now.owner == before.owner
        });
    if same {
        return Ok(false);
    }
    let items = merged
        .into_iter()
        .map(|item| Mirrored {
            id: Some(item.id),
            text: item.text,
            state: item.state,
            active: item.active,
        })
        .collect();
    record(
        root,
        &Event::Replace {
            at,
            source: source.to_string(),
            owner: owner.to_string(),
            items,
        },
    )?;
    Ok(true)
}

/// Adds a to-do and returns its id. `parent` names a top-level item.
pub fn add(
    root: &Path,
    text: &str,
    owner: &str,
    source: &str,
    parent: Option<&str>,
    active: &str,
) -> io::Result<String> {
    let id = mint_id();
    record(
        root,
        &Event::Add {
            at: now(),
            id: id.clone(),
            text: clip(text),
            owner: owner.to_string(),
            source: source.to_string(),
            parent: parent.map(str::to_string),
            active: clip(active),
        },
    )?;
    Ok(id)
}

/// Sets a to-do's state. A parent with children sets its whole group, as its box does.
pub fn set(root: &Path, id: &str, state: State) -> io::Result<()> {
    let items = list(root);
    let grouped = items.iter().any(|item| item.parent.as_deref() == Some(id));
    if grouped {
        record(
            root,
            &Event::Group {
                at: now(),
                parent: id.to_string(),
                state,
            },
        )
    } else {
        record(
            root,
            &Event::Set {
                at: now(),
                id: id.to_string(),
                state,
            },
        )
    }
}

pub fn edit(root: &Path, id: &str, text: &str) -> io::Result<()> {
    record(
        root,
        &Event::Edit {
            at: now(),
            id: id.to_string(),
            text: clip(text),
        },
    )
}

pub fn active(root: &Path, id: &str, text: &str) -> io::Result<()> {
    record(
        root,
        &Event::Active {
            at: now(),
            id: id.to_string(),
            text: clip(text),
        },
    )
}

pub fn remove(root: &Path, id: &str) -> io::Result<()> {
    record(
        root,
        &Event::Remove {
            at: now(),
            id: id.to_string(),
        },
    )
}

pub fn clear_done(root: &Path) -> io::Result<()> {
    record(root, &Event::Clear { at: now() })
}

/// The item `key` names: its id, a unique prefix of one, or its words exactly. A key that
/// matches more than one thing names none.
pub fn find<'a>(items: &'a [Item], key: &str) -> Option<&'a Item> {
    if let Some(item) = items.iter().find(|item| item.id == key) {
        return Some(item);
    }
    let by_prefix: Vec<&Item> = items
        .iter()
        .filter(|item| item.id.starts_with(key))
        .collect();
    if by_prefix.len() == 1 {
        return Some(by_prefix[0]);
    }
    let by_text: Vec<&Item> = items.iter().filter(|item| item.text == key).collect();
    if by_text.len() == 1 {
        return Some(by_text[0]);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn add_event(id: &str, text: &str, owner: &str, source: &str, parent: Option<&str>) -> Event {
        Event::Add {
            at: 1,
            id: id.to_string(),
            text: text.to_string(),
            owner: owner.to_string(),
            source: source.to_string(),
            parent: parent.map(str::to_string),
            active: String::new(),
        }
    }

    fn set_event(id: &str, state: State) -> Event {
        Event::Set {
            at: 2,
            id: id.to_string(),
            state,
        }
    }

    #[test]
    fn to_dos_are_read_from_a_log_that_skips_what_it_cant_read() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let folder = root.join(".cellix");
        fs::create_dir_all(&folder).unwrap();
        // An ignore file from before the to-do log learns it, as it learns every name.
        fs::write(folder.join(".gitignore"), "threads.json\n").unwrap();
        record(root, &add_event("p", "Fix the sort", YOU, YOU, None)).unwrap();
        record(root, &add_event("c1", "Read the code", YOU, YOU, Some("p"))).unwrap();
        record(
            root,
            &add_event("c2", "Swap the lists", YOU, YOU, Some("p")),
        )
        .unwrap();
        record(root, &set_event("c1", State::Done)).unwrap();
        record(root, &set_event("c2", State::Done)).unwrap();
        // The parent's own state is ignored while it has children.
        record(root, &set_event("p", State::Todo)).unwrap();
        // A line cut short, as a full disk cuts one.
        let path = folder.join("todos.jsonl");
        let mut text = fs::read_to_string(&path).unwrap();
        text.push_str("{\"kind\":\"pro");
        fs::write(&path, text).unwrap();

        let items = list(root);
        assert_eq!(items.len(), 3, "{items:?}");
        assert_eq!(
            items.iter().find(|item| item.id == "c1").unwrap().state,
            State::Done
        );
        let parent = items.iter().find(|item| item.id == "p").unwrap();
        assert_eq!(effective(parent, &items), State::Done);
        assert_eq!(children(&items, "p").count(), 2);
        assert_eq!(top_level(&items).count(), 1);

        let ignore = fs::read_to_string(folder.join(".gitignore")).unwrap();
        assert!(ignore.lines().any(|line| line == "todos.jsonl"), "{ignore}");
        assert!(ignore.lines().any(|line| line == "tasks.jsonl"), "{ignore}");
        assert!(changed(root).is_some());
    }

    #[test]
    fn a_mirror_replace_keeps_ids_and_person_children() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let source = "claude:s";
        let mirrored = |text: &str, state: State| Mirrored {
            id: None,
            text: text.to_string(),
            state,
            active: String::new(),
        };
        assert!(
            mirror(
                root,
                source,
                "Claude Code",
                &[mirrored("One", State::Todo), mirrored("Two", State::Todo)],
                10
            )
            .unwrap()
        );
        let first = list(root);
        let one = first.iter().find(|item| item.text == "One").unwrap();
        let first_ids: Vec<String> = first
            .iter()
            .filter(|item| item.source == source)
            .map(|item| item.id.clone())
            .collect();
        // The person nests a detail under the agent's first item.
        add(root, "A detail", YOU, YOU, Some(&one.id), "").unwrap();

        // The agent keeps One, finishes Two and adds Three.
        assert!(
            mirror(
                root,
                source,
                "Claude Code",
                &[
                    mirrored("One", State::Todo),
                    mirrored("Two", State::Done),
                    mirrored("Three", State::Todo)
                ],
                20
            )
            .unwrap()
        );
        let items = list(root);
        assert!(items.iter().any(|item| item.text == "A detail"));
        let two = items.iter().find(|item| item.text == "Two").unwrap();
        assert_eq!(two.state, State::Done);
        assert_eq!(two.id, first_ids[1], "a matched item keeps its id");
        assert_eq!(items.iter().filter(|item| item.source == source).count(), 3);
        let detail = items.iter().find(|item| item.text == "A detail").unwrap();
        assert_eq!(detail.parent.as_deref(), Some(first_ids[0].as_str()));

        // The same list again writes nothing.
        let path = root.join(".cellix").join("todos.jsonl");
        let len = fs::metadata(&path).unwrap().len();
        assert!(
            !mirror(
                root,
                source,
                "Claude Code",
                &[
                    mirrored("One", State::Todo),
                    mirrored("Two", State::Done),
                    mirrored("Three", State::Todo)
                ],
                30
            )
            .unwrap()
        );
        assert_eq!(fs::metadata(&path).unwrap().len(), len);
    }

    #[test]
    fn a_parents_box_groups_its_children_and_clear_takes_only_finished_groups() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        record(root, &add_event("p", "Group", YOU, YOU, None)).unwrap();
        record(root, &add_event("c1", "One", YOU, YOU, Some("p"))).unwrap();
        record(root, &add_event("c2", "Two", YOU, YOU, Some("p"))).unwrap();
        record(root, &add_event("q", "Other", YOU, YOU, None)).unwrap();
        record(root, &add_event("q1", "Detail", YOU, YOU, Some("q"))).unwrap();
        record(root, &add_event("q2", "More", YOU, YOU, Some("q"))).unwrap();
        record(root, &set_event("q1", State::Done)).unwrap();
        set(root, "p", State::Done).unwrap();
        let items = list(root);
        let p = items.iter().find(|item| item.id == "p").unwrap();
        assert_eq!(effective(p, &items), State::Done);
        assert_eq!(children(&items, "p").count(), 2);
        assert!(children(&items, "p").all(|child| child.state == State::Done));

        clear_done(root).unwrap();
        let items = list(root);
        assert!(
            items
                .iter()
                .all(|item| item.id != "p" && item.id != "c1" && item.id != "c2")
        );
        assert!(
            items.iter().any(|item| item.id == "q"),
            "an unfinished group stays"
        );
        assert!(
            items.iter().any(|item| item.id == "q1"),
            "its done child stays"
        );
        assert!(items.iter().any(|item| item.id == "q2"));
    }

    #[test]
    fn two_threads_appending_keep_every_line_whole() {
        let dir = tempfile::tempdir().unwrap();
        let root = Arc::new(dir.path().to_path_buf());
        let mut threads = Vec::new();
        for thread in 0..2 {
            let root = Arc::clone(&root);
            threads.push(std::thread::spawn(move || {
                for number in 0..20 {
                    add(&root, &format!("{thread}-{number}"), YOU, YOU, None, "").unwrap();
                }
            }));
        }
        for thread in threads {
            thread.join().unwrap();
        }
        assert_eq!(load(&root).len(), 40);
        let items = list(&root);
        assert_eq!(items.len(), 40);
        let ids: std::collections::HashSet<&str> =
            items.iter().map(|item| item.id.as_str()).collect();
        assert_eq!(ids.len(), 40, "ids are not minted twice");
    }

    #[test]
    fn find_takes_an_id_a_unique_prefix_or_exact_words() {
        let items = vec![
            Item {
                id: "ab12".into(),
                text: "One".into(),
                active: String::new(),
                state: State::Todo,
                owner: YOU.into(),
                source: YOU.into(),
                parent: None,
                at: 1,
            },
            Item {
                id: "ab34".into(),
                text: "Two".into(),
                active: String::new(),
                state: State::Todo,
                owner: YOU.into(),
                source: YOU.into(),
                parent: None,
                at: 2,
            },
        ];
        assert_eq!(find(&items, "ab12").unwrap().text, "One");
        assert!(find(&items, "ab").is_none(), "a prefix of two names none");
        assert_eq!(find(&items, "Two").unwrap().id, "ab34");
        assert!(find(&items, "three").is_none());
    }
}
