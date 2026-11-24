//! Reviewing a remote doc, driven through the App's input and read off the Grid it draws. The
//! agent is a stand-in for Claude Code on a PATH of its own, which writes down its arguments,
//! its input and whether it was told it's a side run, and prints what the real one prints for
//! each run in turn. Claude Code's config is a made-up one in a folder of the test's: no test
//! here reads the person's own, runs a real agent or reaches a network.

use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::time::{Duration, Instant};

use cellix_input::{Command, Event, Key, KeyEvent, Mods, MouseEvent, MouseKind};
use cellix_render::Grid;
use cellix_review::threads::{Remote, Sent};
use serde_json::json;
use tempfile::TempDir;

use super::App;
use crate::remote;

/// How long a test waits for a stand-in, on a loaded machine.
const PATIENCE: Duration = Duration::from_secs(120);
const LINK: &str = "https://docs.example/d/queue-launch";
const SERVER: &str = "team-docs";
/// The doc as the doc tool has it.
const DOC: &str = "# Queue service launch\n\n## Traffic and capacity\n\nAt launch the service \
takes the order events. We expect three times last year's peak in the first holiday week.\n\n\
Each partition is drained by one consumer, so a slow consumer holds back every event behind \
it.\n";

/// What the doc tool says of the doc: its text, and Priya's comment with Sam's reply, with
/// what else `more` adds.
fn read_answer(more: serde_json::Value) -> String {
    read_with_replies(more, json!([]))
}

/// The same, with `replies` after Sam's under Priya's comment.
fn read_with_replies(more: serde_json::Value, replies: serde_json::Value) -> String {
    let mut under = vec![json!({"id": "r1", "author": "Sam",
        "text": "Last year's growth on the same week.", "at": "2026-11-24T10:30:00Z"})];
    if let Some(replies) = replies.as_array() {
        under.extend(replies.iter().cloned());
    }
    let mut comments = vec![json!({
        "id": "c1",
        "author": "Priya",
        "quote": "three times last year's peak",
        "text": "Where does 3x come from?",
        "at": "2026-11-24T09:30:00Z",
        "resolved": false,
        "replies": under,
    })];
    if let Some(more) = more.as_array() {
        comments.extend(more.iter().cloned());
    }
    json!({
        "title": "Queue service launch",
        "link": LINK,
        "version": 12,
        "markdown": DOC,
        "comments": comments,
    })
    .to_string()
}

/// What Claude Code prints for a run that answers `answer`: its session, a call of the doc
/// tool, and its result.
fn printed(answer: &str) -> Vec<String> {
    vec![
        json!({"type": "system", "subtype": "init", "session_id": "remote-1"}).to_string(),
        json!({"type": "assistant", "message": {"content": [
            {"type": "tool_use", "name": "mcp__team-docs__read", "input": {}}]}})
        .to_string(),
        json!({"type": "result", "subtype": "success", "is_error": false, "result": answer})
            .to_string(),
    ]
}

/// A project, with Claude Code's config in a home of its own naming two MCP servers, and the
/// App finding command lines only in its `bin` folder.
fn project() -> (TempDir, App) {
    let dir = tempfile::tempdir().unwrap();
    let bin = dir.path().join("bin");
    fs::create_dir(&bin).unwrap();
    let home = dir.path().join("home");
    fs::create_dir(&home).unwrap();
    fs::write(
        home.join(".claude.json"),
        r#"{"mcpServers":{"team-docs":{"command":"docs-mcp"},"tracker":{"command":"t"}}}"#,
    )
    .unwrap();
    let mut app = App::new(dir.path().to_path_buf(), (210, 46));
    app.side_path = Some(bin.into_os_string());
    let env = HashMap::from([("HOME".to_string(), home.display().to_string())]);
    app.remote.env = Some(env);
    (dir, app)
}

/// A stand-in for `claude`: each run writes its arguments, one a line, its input and whether
/// it was told it's a side run, then prints its run's lines, `claude.out.<run>`.
fn stand_in(dir: &Path) {
    let bin = dir.join("bin");
    let record = bin.join("claude");
    let script = format!(
        "#!/bin/sh\n\
         n=$(/bin/cat '{rec}.runs' 2>/dev/null || echo 0); n=$((n + 1)); echo $n > '{rec}.runs'\n\
         printf '%s\\n' \"$@\" > '{rec}.args.'$n\n\
         printf '%s' \"$CELLIX_SIDE_QUESTION\" > '{rec}.side.'$n\n\
         /bin/cat > '{rec}.prompt.'$n\n\
         while IFS= read -r line; do printf '%s\\n' \"$line\"; done < '{rec}.out.'$n\n",
        rec = record.display(),
    );
    fs::write(&record, script).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&record, fs::Permissions::from_mode(0o755)).unwrap();
    }
}

/// What the stand-in prints on its `run`th run.
fn answers(dir: &Path, run: usize, lines: &[String]) {
    let out = dir.join("bin").join(format!("claude.out.{run}"));
    fs::write(out, lines.join("\n") + "\n").unwrap();
}

fn given(dir: &Path, run: usize) -> (Vec<String>, String, String) {
    let base = dir.join("bin").join("claude");
    let read = |what: &str| {
        fs::read_to_string(format!("{}.{what}.{run}", base.display())).unwrap_or_default()
    };
    let args = read("args").lines().map(str::to_string).collect();
    (args, read("prompt"), read("side"))
}

fn runs(dir: &Path) -> usize {
    let file = dir.join("bin").join("claude.runs");
    fs::read_to_string(file)
        .ok()
        .and_then(|n| n.trim().parse().ok())
        .unwrap_or(0)
}

/// The values a run's arguments give a flag, each time it's given.
fn values<'a>(args: &'a [String], flag: &str) -> Vec<&'a str> {
    args.iter()
        .zip(args.iter().skip(1))
        .filter(|(a, _)| a.as_str() == flag)
        .map(|(_, v)| v.as_str())
        .collect()
}

fn screen(app: &App) -> Grid {
    let mut grid = Grid::new(app.size().0, app.size().1, app.background());
    app.draw(&mut grid);
    grid
}

fn click(app: &mut App, x: u16, y: u16) {
    for kind in [MouseKind::Left, MouseKind::Up] {
        app.handle(Event::Mouse(MouseEvent {
            kind,
            x,
            y,
            fine_x: x as f32,
            fine_y: y as f32 + 0.5,
            mods: Mods::default(),
        }));
    }
}

fn typed(app: &mut App, text: &str) {
    for c in text.chars() {
        app.handle(Event::Key(KeyEvent::plain(Key::Char(c))));
    }
}

fn key(app: &mut App, key: Key) {
    app.handle(Event::Key(KeyEvent::plain(key)));
}

fn find(grid: &Grid, text: &str) -> (u16, u16) {
    find_from(grid, text, 0).unwrap_or_else(|| panic!("{text:?} not on screen:\n{}", grid.text()))
}

fn find_from(grid: &Grid, text: &str, from: u16) -> Option<(u16, u16)> {
    (from..grid.height()).find_map(|y| {
        let row = grid.row_text(y);
        row.find(text).map(|i| (row[..i].chars().count() as u16, y))
    })
}

/// Presses the button that says `words` first under the card that says `card`.
fn press_on_card(app: &mut App, card: &str, words: &str) {
    let grid = screen(app);
    let (_, top) = find(&grid, card);
    let (x, y) = find_from(&grid, words, top)
        .unwrap_or_else(|| panic!("no {words:?} under {card:?}:\n{}", grid.text()));
    click(app, x + 1, y);
}

/// Presses `words` on the first row under the card that says `card` that also says `beside`:
/// a button among others whose words another's contain ("Reply" in "Post Reply").
fn press_beside(app: &mut App, card: &str, beside: &str, words: &str) {
    let grid = screen(app);
    let (_, top) = find(&grid, card);
    let (_, y) = find_from(&grid, beside, top)
        .unwrap_or_else(|| panic!("no {beside:?} under {card:?}:\n{}", grid.text()));
    let row = grid.row_text(y);
    let at = row
        .find(words)
        .unwrap_or_else(|| panic!("no {words:?} in {row}"));
    click(app, row[..at].chars().count() as u16 + 1, y);
}

/// Polls until `done` holds.
fn until(app: &mut App, what: &str, done: impl Fn(&App) -> bool) {
    let deadline = Instant::now() + PATIENCE;
    loop {
        app.poll();
        if done(app) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "never {what}:\n{}",
            screen(app).text()
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Polls until the screen shows `want`.
fn shows(app: &mut App, want: &str) -> Grid {
    until(app, &format!("showed {want:?}"), |app| {
        screen(app).text().contains(want)
    });
    screen(app)
}

/// Selects `words` in the doc in front, as a drag over them does.
fn select(app: &mut App, words: &str) {
    let editor = app.editor_mut().unwrap();
    let text = editor.buffer.text();
    let start = text[..text.find(words).unwrap()].chars().count();
    editor.selections.set_single(cellix_core::Selection {
        anchor: start,
        head: start + words.chars().count(),
        goal: None,
    });
}

/// A comment of the reviewer's on `words`, saved as Enter saves it.
fn comment(app: &mut App, words: &str, text: &str) {
    select(app, words);
    app.run(Command::AddComment);
    typed(app, text);
    key(app, Key::Enter);
}

fn thread_saying<'a>(app: &'a App, text: &str) -> &'a cellix_review::Thread {
    app.threads
        .iter()
        .find(|t| t.text == text)
        .unwrap_or_else(|| panic!("no comment {text:?}"))
}

fn state_of(app: &App, text: &str) -> Option<Sent> {
    thread_saying(app, text)
        .remote
        .as_ref()
        .map(|r| r.state.clone())
}

/// The doc read as the palette reads it, the server picked already.
fn read_doc(dir: &Path, app: &mut App) {
    app.settings.remote_docs_mcp = Some(SERVER.to_string());
    answers(dir, 1, &printed(&read_answer(json!([]))));
    app.run(Command::ReviewRemoteDoc);
    typed(app, LINK);
    key(app, Key::Enter);
    shows(app, "From team-docs, read by Claude Code");
}

/// The palette's Review Remote Doc… asks for a link, and the first time, which of Claude
/// Code's MCP servers the doc tool is: its user config's, read and never written. Claude Code
/// then runs once in the background as a side run, allowed that server's tools and no other's,
/// refused its own that change things, told the link and to read only. The doc is written under
/// `.cellix/remote` and opens as a page, its tab named by its title; the strip under the tabs
/// says where it's from, its version and how many comments are others', with Sync and Open in
/// Browser; Priya's comment is a thread under her name with Sam's reply, and nothing is posted.
#[test]
fn a_remote_doc_is_read_through_its_mcp_server_and_opens_as_a_page() {
    let (dir, mut app) = project();
    stand_in(dir.path());
    answers(dir.path(), 1, &printed(&read_answer(json!([]))));
    let config = dir.path().join("home/.claude.json");
    let before = fs::read_to_string(&config).unwrap();
    app.run(Command::ReviewRemoteDoc);
    typed(&mut app, LINK);
    key(&mut app, Key::Enter);
    // The first time, it asks which server the doc tool is.
    let grid = screen(&app);
    assert!(
        grid.text().contains("READ REMOTE DOCS THROUGH"),
        "{}",
        grid.text()
    );
    assert!(grid.text().contains("tracker"), "{}", grid.text());
    assert_eq!(runs(dir.path()), 0, "nothing runs till it's picked");
    let (x, y) = find(&grid, SERVER);
    click(&mut app, x + 1, y);
    assert_eq!(app.settings.remote_docs_mcp.as_deref(), Some(SERVER));
    let grid = shows(&mut app, "From team-docs, read by Claude Code");

    // The stand-in ran, once, as a side run allowed only the doc tool's server.
    assert_eq!(runs(dir.path()), 1);
    let (args, prompt, side) = given(dir.path(), 1);
    assert_eq!(side, "1");
    assert_eq!(values(&args, "--allowedTools"), ["mcp__team-docs__*"]);
    let refused = values(&args, "--disallowedTools").join(",");
    assert!(refused.contains("mcp__tracker__*"), "{refused}");
    assert!(
        refused.contains("Write") && refused.contains("Bash"),
        "{refused}"
    );
    assert_eq!(values(&args, "--tools"), [""]);
    assert!(
        !args
            .iter()
            .any(|a| a.contains("dangerously") || a.contains("bypass"))
    );
    assert!(prompt.contains(LINK), "{prompt}");
    assert!(prompt.contains("Use only the tools that read"), "{prompt}");
    assert!(prompt.contains("team-docs"), "{prompt}");
    assert_eq!(
        fs::read_to_string(&config).unwrap(),
        before,
        "read, never written"
    );

    // Written under .cellix/remote, kept out of git.
    let slug = remote::slug(LINK);
    let folder = dir.path().join(remote::FOLDER).join(&slug);
    assert_eq!(fs::read_to_string(folder.join("doc.md")).unwrap(), DOC);
    let meta = remote::load(dir.path(), &slug).unwrap();
    assert_eq!(
        (meta.title.as_str(), meta.version.as_str()),
        ("Queue service launch", "12")
    );
    assert_eq!((meta.server.as_str(), meta.others), (SERVER, 1));
    assert_eq!(
        fs::read_to_string(dir.path().join(remote::FOLDER).join(".gitignore")).unwrap(),
        "*\n"
    );

    // Its tab goes by its title, and the strip says where it's from.
    assert!(
        grid.row_text(0).contains("Queue service launch"),
        "{}",
        grid.row_text(0)
    );
    let strip = grid.row_text(1);
    for words in ["synced just now", "Sync", "Open in Browser"] {
        assert!(strip.contains(words), "{words:?} in {strip}");
    }
    // Beside the comments a narrow strip drops the version, then the count, whole; with the
    // pane closed it has room for all.
    assert!(
        !strip.contains("vers") || strip.contains("version 12"),
        "{strip}"
    );
    app.dock.open = false;
    let strip = screen(&app).row_text(1);
    for words in ["version 12", "synced just now", "1 comment from others"] {
        assert!(strip.contains(words), "{words:?} in {strip}");
    }
    app.dock.open = true;
    let editor = app.editors.get(app.active()).unwrap();
    assert!(editor.rendered && editor.buffer.read_only);

    // Priya's comment, under her name, with Sam's reply, to answer or ask about.
    let text = grid.text();
    for words in [
        "Priya · in the doc",
        "Where does 3x come from?",
        "Sam",
        "Others: 1",
    ] {
        assert!(text.contains(words), "{words:?} in\n{text}");
    }
    let priya = thread_saying(&app, "Where does 3x come from?");
    assert_eq!(priya.author, "Priya");
    assert_eq!(priya.remote, Some(Remote::posted("c1", 1_795_512_600)));
    assert_eq!(priya.replies[0].author, "Sam");
    assert!(
        !priya.waiting(),
        "someone's comment isn't handed to the agent here"
    );
    assert!(text.contains("Reply") && text.contains("Ask Agent") && text.contains("Open in Doc"));
    assert!(
        !text.contains("Post to Doc"),
        "nothing of the reviewer's yet"
    );

    // The next run reopens it from .cellix/remote, title and strip and all.
    let mut again = App::new(dir.path().to_path_buf(), (210, 46));
    again.reopen_remote_docs();
    let grid = screen(&again);
    assert!(
        grid.row_text(0).contains("Queue service launch"),
        "{}",
        grid.row_text(0)
    );
    assert!(
        grid.row_text(1).contains("From team-docs"),
        "{}",
        grid.row_text(1)
    );
    assert_eq!(runs(dir.path()), 1, "nothing ran again");
}

/// The reviewer's comments on a remote doc are drafts. Neither Enter nor Sync posts one: Sync
/// reads the doc again, bringing in a new comment and reply by id and leaving the drafts as
/// they were. One press on a card's Post to Doc posts that comment, and only it, once: the
/// agent is told its quote and words, allowed only the doc tool's server, and the card says
/// Posted with the doc's id kept; the other draft is still one. A later Sync finds the posted
/// one by its id and adds nothing twice.
#[test]
fn a_draft_is_posted_only_by_its_own_press_and_only_once() {
    let (dir, mut app) = project();
    stand_in(dir.path());
    read_doc(dir.path(), &mut app);
    comment(
        &mut app,
        "slow consumer",
        "What happens when one consumer is slow?",
    );
    comment(&mut app, "holiday week", "Which week exactly?");
    let grid = screen(&app);
    let text = grid.text();
    assert!(text.contains("Draft, not posted yet"), "{text}");
    assert!(
        text.contains("Save") || text.contains("Post to Doc"),
        "{text}"
    );
    assert_eq!(text.matches("Post to Doc").count(), 2, "{text}");
    assert_eq!(state_of(&app, "Which week exactly?"), Some(Sent::Draft));
    assert_eq!(runs(dir.path()), 1, "saving a draft posts nothing");

    // Sync reads again and posts nothing.
    let synced = read_with_replies(
        json!([
            {"id": "c2", "author": "Sam", "quote": "drained by one consumer",
             "text": "Is one consumer enough?", "at": 1_795_500_000},
        ]),
        json!([{"id": "r2", "author": "Priya", "text": "Thanks."}]),
    );
    answers(dir.path(), 2, &printed(&synced));
    let (x, y) = find(&screen(&app), "Sync");
    click(&mut app, x + 1, y);
    shows(&mut app, "Is one consumer enough?");
    until(&mut app, "synced", |app| app.remote.reading.is_none());
    assert_eq!(runs(dir.path()), 2);
    let (args, prompt, _) = given(dir.path(), 2);
    assert!(prompt.contains("read the document"), "{prompt}");
    assert_eq!(values(&args, "--allowedTools"), ["mcp__team-docs__*"]);
    assert_eq!(
        thread_saying(&app, "Where does 3x come from?")
            .replies
            .len(),
        2
    );
    for draft in [
        "What happens when one consumer is slow?",
        "Which week exactly?",
    ] {
        assert_eq!(state_of(&app, draft), Some(Sent::Draft), "{draft}");
    }

    // One press posts that comment alone.
    answers(dir.path(), 3, &printed(r#"{"id": "c-41"}"#));
    press_on_card(
        &mut app,
        "What happens when one consumer is slow?",
        "Post to Doc",
    );
    until(&mut app, "posted", |app| {
        matches!(
            state_of(app, "What happens when one consumer is slow?"),
            Some(Sent::Posted { .. })
        )
    });
    assert_eq!(runs(dir.path()), 3);
    let (args, prompt, _) = given(dir.path(), 3);
    assert_eq!(values(&args, "--allowedTools"), ["mcp__team-docs__*"]);
    assert!(
        prompt.contains("What happens when one consumer is slow?"),
        "{prompt}"
    );
    assert!(prompt.contains("Quoted words: slow consumer"), "{prompt}");
    assert!(prompt.contains("Add only this comment, once"), "{prompt}");
    assert!(!prompt.contains("Which week exactly?"), "{prompt}");
    let posted = thread_saying(&app, "What happens when one consumer is slow?");
    assert_eq!(posted.remote.as_ref().unwrap().id, "c-41");
    let text = screen(&app).text();
    assert!(text.contains("Posted just now"), "{text}");
    assert_eq!(text.matches("Post to Doc").count(), 1, "{text}");
    assert_eq!(state_of(&app, "Which week exactly?"), Some(Sent::Draft));
    for _ in 0..20 {
        app.poll();
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(runs(dir.path()), 3, "posted once");

    // A later Sync finds it by its id: nothing twice, the draft still a draft.
    let count = app.threads.len();
    let mine = json!([
        {"id": "c2", "author": "Sam", "quote": "drained by one consumer",
         "text": "Is one consumer enough?"},
        {"id": "c-41", "author": "Reviewer", "quote": "slow consumer",
         "text": "What happens when one consumer is slow?"},
    ]);
    answers(dir.path(), 4, &printed(&read_answer(mine)));
    let (x, y) = find(&screen(&app), "Sync");
    click(&mut app, x + 1, y);
    until(&mut app, "synced again", |app| {
        runs(dir.path()) == 4 && app.remote.reading.is_none()
    });
    assert_eq!(app.threads.len(), count);
    assert!(
        thread_saying(&app, "What happens when one consumer is slow?")
            .author
            .is_empty()
    );
    assert_eq!(state_of(&app, "Which week exactly?"), Some(Sent::Draft));
}

/// A post the doc tool refuses keeps the draft as it was and says why on its card, a sign-in
/// that ran out in words that say what to do, with Retry and Copy; Copy puts its words on the
/// clipboard, and Retry posts it.
#[test]
fn a_failed_post_keeps_the_draft_and_says_why() {
    let (dir, mut app) = project();
    stand_in(dir.path());
    read_doc(dir.path(), &mut app);
    let words = "What happens when one consumer is slow?";
    comment(&mut app, "slow consumer", words);
    answers(
        dir.path(),
        2,
        &printed(r#"{"error": "Your session has expired. Please sign in again."}"#),
    );
    press_on_card(&mut app, words, "Post to Doc");
    until(&mut app, "failed", |app| {
        matches!(state_of(app, words), Some(Sent::Failed { .. }))
    });
    assert_eq!(
        state_of(&app, words),
        Some(Sent::Failed {
            why: remote::SIGN_IN_AGAIN.to_string()
        })
    );
    assert_eq!(
        thread_saying(&app, words).text,
        words,
        "the draft is as it was"
    );
    let text = screen(&app).text();
    assert!(text.contains("Not posted"), "{text}");
    assert!(text.contains("asked to sign in again"), "{text}");
    assert!(text.contains("Retry"), "{text}");
    press_on_card(&mut app, words, "Copy");
    assert_eq!(app.clipboard, words);
    assert_eq!(runs(dir.path()), 2);
    answers(dir.path(), 3, &printed(r#"{"id": "c-7"}"#));
    press_beside(&mut app, words, "Copy", "Retry");
    until(&mut app, "posted", |app| {
        matches!(state_of(app, words), Some(Sent::Posted { .. }))
    });
    assert_eq!(runs(dir.path()), 3);
}
