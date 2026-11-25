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
    stand_in_for(dir, "claude");
}

/// The same stand-in under another command line's name, `codex` or `gemini`, keeping its own
/// record of each run.
fn stand_in_for(dir: &Path, program: &str) {
    let bin = dir.join("bin");
    let record = bin.join(program);
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
    answers_for(dir, "claude", run, lines);
}

fn answers_for(dir: &Path, program: &str, run: usize, lines: &[String]) {
    let out = dir.join("bin").join(format!("{program}.out.{run}"));
    fs::write(out, lines.join("\n") + "\n").unwrap();
}

fn given(dir: &Path, run: usize) -> (Vec<String>, String, String) {
    given_for(dir, "claude", run)
}

fn given_for(dir: &Path, program: &str, run: usize) -> (Vec<String>, String, String) {
    let base = dir.join("bin").join(program);
    let read = |what: &str| {
        fs::read_to_string(format!("{}.{what}.{run}", base.display())).unwrap_or_default()
    };
    let args = read("args").lines().map(str::to_string).collect();
    (args, read("prompt"), read("side"))
}

fn runs(dir: &Path) -> usize {
    runs_of(dir, "claude")
}

fn runs_of(dir: &Path, program: &str) -> usize {
    let file = dir.join("bin").join(format!("{program}.runs"));
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

/// Polish has the agent, with no tools at all, make a draft short, clear and kind, told its
/// words, its quote and the paragraph round them. The suggestion shows under the draft with
/// the words counted, after and before; Use This takes it as the draft's words, still a draft,
/// and Keep Mine leaves the draft as it was. Polish posts nothing.
#[test]
fn polish_suggests_a_shorter_draft_to_use_or_not() {
    let (dir, mut app) = project();
    stand_in(dir.path());
    read_doc(dir.path(), &mut app);
    let long = "I think this is a problem because if one consumer gets slow for whatever reason \
                then all of the events behind it are going to wait, and the doc doesn't say \
                what happens then or how we'd notice.";
    comment(&mut app, "slow consumer", long);
    let short = "One slow consumer stalls its partition. How is that detected, and does the \
                 two-second shed cover it?";
    answers(dir.path(), 2, &printed(short));
    press_on_card(&mut app, "Draft, not posted yet", "Polish");
    let text = shows(&mut app, "Use This").text();
    let (args, prompt, side) = given(dir.path(), 2);
    assert_eq!(side, "1");
    assert_eq!(values(&args, "--tools"), [""]);
    assert!(
        args.contains(&"--strict-mcp-config".to_string()),
        "{args:?}"
    );
    assert!(values(&args, "--allowedTools").is_empty(), "{args:?}");
    assert!(prompt.contains("short, clear and kind"), "{prompt}");
    assert!(prompt.contains(long), "{prompt}");
    assert!(
        prompt.contains("Each partition is drained by one consumer"),
        "{prompt}"
    );
    assert!(text.contains("Polished"), "{text}");
    assert!(text.contains("Claude Code · 17 words, from 38"), "{text}");
    assert!(text.contains("Keep Mine"), "{text}");
    assert_eq!(thread_saying(&app, long).text, long);

    // Keep Mine leaves the draft; asked again, Use This takes the suggestion.
    press_on_card(&mut app, "Polished", "Keep Mine");
    assert!(!screen(&app).text().contains("Use This"));
    assert_eq!(thread_saying(&app, long).text, long);
    answers(dir.path(), 3, &printed(short));
    press_on_card(&mut app, "Draft, not posted yet", "Polish");
    shows(&mut app, "Use This");
    press_on_card(&mut app, "Polished", "Use This");
    assert_eq!(state_of(&app, short), Some(Sent::Draft), "still a draft");
    assert_eq!(runs(dir.path()), 3, "nothing posted");
}

/// A reply the reviewer writes on any thread, someone else's or their own posted one, is a
/// draft with its own Post Reply, posted one by one: a press posts that reply alone, under the
/// doc's id of its comment. End Review asks, then takes the doc's folder, its tab and its
/// comments off this machine.
#[test]
fn replies_post_one_by_one_and_end_review_takes_the_doc_away() {
    let (dir, mut app) = project();
    stand_in(dir.path());
    read_doc(dir.path(), &mut app);
    press_beside(&mut app, "Where does 3x come from?", "Ask Agent", "Reply");
    typed(&mut app, "Can we link the growth numbers?");
    key(&mut app, Key::Enter);
    press_beside(&mut app, "Where does 3x come from?", "Ask Agent", "Reply");
    typed(&mut app, "And last year's peak itself?");
    key(&mut app, Key::Enter);
    let thread = thread_saying(&app, "Where does 3x come from?");
    let drafts: Vec<_> = thread
        .replies
        .iter()
        .filter(|r| r.author == "You")
        .collect();
    assert_eq!(drafts.len(), 2);
    assert!(drafts.iter().all(|r| r.remote == Some(Remote::draft())));
    let text = screen(&app).text();
    assert_eq!(text.matches("Post Reply").count(), 2, "{text}");
    assert_eq!(runs(dir.path()), 1, "nothing posted by writing");

    answers(dir.path(), 2, &printed(r#"{"id": "r-9"}"#));
    press_on_card(&mut app, "Can we link the growth numbers?", "Post Reply");
    until(&mut app, "posted the reply", |app| {
        let thread = thread_saying(app, "Where does 3x come from?");
        thread.replies.iter().any(|r| {
            r.text == "Can we link the growth numbers?"
                && matches!(
                    r.remote.as_ref().map(|r| &r.state),
                    Some(Sent::Posted { .. })
                )
        })
    });
    let (_, prompt, _) = given(dir.path(), 2);
    assert!(prompt.contains("comment whose id is c1"), "{prompt}");
    assert!(
        prompt.contains("Can we link the growth numbers?"),
        "{prompt}"
    );
    assert!(!prompt.contains("last year's peak itself"), "{prompt}");
    let thread = thread_saying(&app, "Where does 3x come from?");
    let other = thread
        .replies
        .iter()
        .find(|r| r.text == "And last year's peak itself?")
        .unwrap();
    assert_eq!(
        other.remote,
        Some(Remote::draft()),
        "the other is still a draft"
    );
    assert_eq!(screen(&app).text().matches("Post Reply").count(), 1);
    assert_eq!(runs(dir.path()), 2);

    // End Review asks first, then takes it all away.
    let slug = remote::slug(LINK);
    let folder = dir.path().join(remote::FOLDER).join(&slug);
    let strip = screen(&app).row_text(1);
    let at = strip
        .find('\u{22ef}')
        .unwrap_or_else(|| panic!("no menu in {strip}"));
    click(&mut app, strip[..at].chars().count() as u16, 1);
    let (x, y) = find(&screen(&app), "End Review");
    click(&mut app, x + 1, y);
    let grid = screen(&app);
    assert!(grid.text().contains("End this review?"), "{}", grid.text());
    assert!(folder.is_dir(), "nothing gone before it's agreed to");
    let (x, y) = find(&grid, "End Review");
    click(&mut app, x + 1, y);
    assert!(!folder.exists());
    let file = remote::doc_file(&slug);
    assert!(app.threads.iter().all(|t| t.file != file));
    assert!(!screen(&app).row_text(0).contains("Queue service launch"));
    assert_eq!(runs(dir.path()), 2);
}

/// As the board has them: someone's name on a remote card stands bold in the words' colour,
/// theirs and their reply's, while the reviewer's "You" doesn't; the menu that picks the MCP
/// server has a row lit for Enter, the one in use or the first; and Post to Doc and Copy carry
/// their marks, a plane and two sheets, before their words, which still press the same.
#[test]
fn a_remote_cards_names_menu_and_marks_stand_as_the_board_has_them() {
    use cellix_render::{Icon, Paint};
    let (dir, mut app) = project();
    stand_in(dir.path());
    // The menu, before anything is picked: its first server lit.
    answers(dir.path(), 1, &printed(&read_answer(json!([]))));
    app.run(Command::ReviewRemoteDoc);
    typed(&mut app, LINK);
    key(&mut app, Key::Enter);
    let lit = match &app.overlay {
        super::Overlay::Menu(menu) => menu.selected,
        _ => None,
    };
    assert_eq!(lit, Some(1), "the first server, under the heading");
    // Enter takes it: the row is lit for the keys as well as shown.
    key(&mut app, Key::Enter);
    assert_eq!(app.settings.remote_docs_mcp.as_deref(), Some(SERVER));
    shows(&mut app, "From team-docs, read by Claude Code");
    comment(
        &mut app,
        "slow consumer",
        "What happens when one consumer is slow?",
    );

    app.set_measure(std::rc::Rc::new(cellix_render::FontMeasure::new(17, 37)));
    let grid = screen(&app);
    let texts: Vec<(&str, bool)> = grid
        .paint
        .iter()
        .filter_map(|p| match p {
            Paint::Text { text, font, .. } => Some((text.as_str(), font.bold)),
            _ => None,
        })
        .collect();
    assert!(texts.contains(&("Priya", true)), "{texts:?}");
    assert!(texts.contains(&("Sam", true)), "{texts:?}");
    assert!(
        !texts.iter().any(|(t, bold)| t.starts_with("You") && *bold),
        "{texts:?}"
    );
    let icons: Vec<Icon> = grid
        .paint
        .iter()
        .filter_map(|p| match p {
            Paint::Icon { icon, .. } => Some(*icon),
            _ => None,
        })
        .collect();
    assert!(icons.contains(&Icon::Send), "{icons:?}");
    assert!(icons.contains(&Icon::Copy), "{icons:?}");
    assert!(texts.contains(&("Post to Doc", false)), "{texts:?}");

    // On cells, no marks and the same words, and a press on Copy still copies.
    app.set_measure(std::rc::Rc::new(cellix_render::CellMeasure));
    press_on_card(&mut app, "What happens when one consumer is slow?", "Copy");
    assert_eq!(app.clipboard, "What happens when one consumer is slow?");
}

/// What Codex prints for a run that answers `answer`: its thread, its message, its turn's end.
fn codex_printed(answer: &str) -> Vec<String> {
    vec![
        json!({"type": "thread.started", "thread_id": "codex-1"}).to_string(),
        json!({"type": "item.completed", "item": {"type": "agent_message", "text": answer}})
            .to_string(),
        json!({"type": "turn.completed", "usage": {}}).to_string(),
    ]
}

/// The row of the screen that says `words`, and the column `then` starts at on it.
fn on_row_of(grid: &Grid, words: &str, then: &str) -> (u16, u16) {
    let (_, y) = find(grid, words);
    let row = grid.row_text(y);
    let at = row
        .find(then)
        .unwrap_or_else(|| panic!("no {then:?} beside {words:?}: {row}"));
    (row[..at].chars().count() as u16, y)
}

/// Settings › Integrations has a Default agent, Auto, Claude, Codex or Gemini, kept in the
/// `ask_agent` setting: a press on Codex keeps it, and every card under it still shows. Polish
/// then runs Codex, not Claude Code, read-only with nothing but its prompt, and says it's
/// Codex's; the doc is still read through Claude Code, whatever the Default agent is.
#[test]
fn the_default_agent_is_picked_in_settings_and_polish_runs_it() {
    let (dir, mut app) = project();
    stand_in(dir.path());
    stand_in_for(dir.path(), "codex");
    app.run(Command::OpenSettings);
    let integrations = {
        let page = app.settings_page.as_ref().unwrap();
        let spots = page.controls(app.sheet_area());
        let section = crate::settings_page::Section::Integrations;
        let tab = spots
            .iter()
            .find(|(_, c)| *c == crate::settings_page::Control::Section(section));
        tab.unwrap().0
    };
    click(&mut app, integrations.x + 1, integrations.y);
    let grid = screen(&app);
    let text = grid.text();
    for words in [
        "Default agent",
        "Answers Ask, Polish and Validate",
        "Auto",
        "Gemini",
    ] {
        assert!(text.contains(words), "{words:?} in\n{text}");
    }
    // Every card stands under it. (Which agents this machine has is the page's own look, so
    // the cards that depend on it are `settings_page`'s tests'.)
    for card in ["Claude Code", "Codex", "Gemini CLI", "Browser"] {
        assert!(text.contains(card), "{card:?} in\n{text}");
    }
    let (x, y) = on_row_of(&grid, "Default agent", "Codex");
    click(&mut app, x + 1, y);
    assert_eq!(app.settings.ask_agent.as_deref(), Some("codex"));
    assert_eq!(
        crate::settings::default_agent_name(app.settings.ask_agent.as_deref()),
        "codex"
    );
    app.run(Command::CloseTab);
    assert!(app.settings_page.is_none());

    // The doc is read through Claude Code still.
    read_doc(dir.path(), &mut app);
    assert_eq!(runs(dir.path()), 1);
    assert_eq!(runs_of(dir.path(), "codex"), 0);
    let long = "I think this is a problem because if one consumer gets slow for whatever reason \
                then all of the events behind it are going to wait.";
    comment(&mut app, "slow consumer", long);
    let short = "One slow consumer stalls its partition. How is that detected?";
    answers_for(dir.path(), "codex", 1, &codex_printed(short));
    press_on_card(&mut app, "Draft, not posted yet", "Polish");
    let text = shows(&mut app, "Use This").text();
    assert_eq!(runs_of(dir.path(), "codex"), 1, "Codex polished it");
    assert_eq!(runs(dir.path()), 1, "and Claude Code didn't");
    let (args, prompt, side) = given_for(dir.path(), "codex", 1);
    assert_eq!(side, "1", "a side run, which no hook takes for an agent's");
    assert_eq!(args.first().map(String::as_str), Some("exec"));
    assert_eq!(values(&args, "--sandbox"), ["read-only"]);
    assert!(prompt.contains("short, clear and kind"), "{prompt}");
    assert!(prompt.contains(long), "{prompt}");
    assert!(text.contains("Codex · 10 words, from 26"), "{text}");
    assert_eq!(thread_saying(&app, long).text, long);
}

/// Validate, on one of the reviewer's comments on a page of the project: the Default agent
/// runs with no tools, given the whole document, the words the comment is on and the comment,
/// and its one line shows under the comment, a verdict on a chip with the kind and who said it,
/// and why. The comment's words and kind stay as they were. What it said is kept with the
/// comment, so a new Cellix shows it, until the comment is edited.
#[test]
fn validate_says_whether_a_comment_holds_up_and_never_changes_it() {
    let (dir, mut app) = project();
    stand_in(dir.path());
    let path = dir.path().join("plan.md");
    fs::write(&path, DOC).unwrap();
    app.open(&path).unwrap();
    let words = "Where is a slow consumer noticed?";
    comment(&mut app, "slow consumer", words);
    app.dock.open = true;
    app.dock.show_comments();
    answers(
        dir.path(),
        1,
        &printed(
            "**Already answered (Evidence):** Traffic and capacity says each partition is \
             drained by one consumer.",
        ),
    );
    press_on_card(&mut app, words, "Validate");
    let text = shows(&mut app, "Already answered").text();
    assert_eq!(runs(dir.path()), 1);
    let (args, prompt, side) = given(dir.path(), 1);
    assert_eq!(side, "1");
    assert_eq!(values(&args, "--tools"), [""], "no tools at all");
    assert!(
        args.contains(&"--strict-mcp-config".to_string()),
        "{args:?}"
    );
    assert!(values(&args, "--allowedTools").is_empty(), "{args:?}");
    assert!(prompt.contains(DOC), "the whole document: {prompt}");
    assert!(
        prompt.contains("The words the comment is on: slow consumer"),
        "{prompt}"
    );
    assert!(prompt.contains(words), "{prompt}");
    assert!(
        prompt.contains("Holds up, Doesn't hold, Already answered"),
        "{prompt}"
    );
    for said in [
        "Evidence · Claude Code",
        "drained by one consumer",
        "Dismiss",
    ] {
        assert!(text.contains(said), "{said:?} in\n{text}");
    }
    let thread = thread_saying(&app, words);
    assert_eq!(thread.text, words, "the comment is as it was");
    assert!(thread.kind.is_empty(), "its kind too");
    let validation = thread.validation.clone().expect("kept with the comment");
    assert_eq!(
        validation.verdict,
        cellix_review::threads::Verdict::AlreadyAnswered
    );
    assert_eq!(validation.kind, "Evidence");

    // A new Cellix shows it from the comments file.
    let mut again = App::new(dir.path().to_path_buf(), (210, 46));
    again.open(&path).unwrap();
    again.dock.open = true;
    again.dock.show_comments();
    assert!(screen(&again).text().contains("Already answered"));
    assert_eq!(runs(dir.path()), 1, "nothing ran again");

    // Edited, the comment's words are new, and what was said of the old ones goes.
    let id = thread_saying(&app, words).id;
    app.thread_hit(crate::threads::Hit::Edit(id));
    typed(&mut app, " And who is told?");
    key(&mut app, Key::Enter);
    let text = screen(&app).text();
    assert!(text.contains("And who is told?"), "{text}");
    assert!(!text.contains("Already answered"), "{text}");
}

/// On a remote doc's draft, Validate is the same press, and posts nothing: what it said stands
/// under the draft, green where it holds up, beside Polish's words, both at once. It's kept
/// through a Sync and a new Cellix. Dismiss takes it away and leaves the draft and Polish's
/// words as they were.
#[test]
fn validate_on_a_draft_is_kept_through_sync_and_reopening_till_dismissed() {
    use cellix_render::Paint;
    let (dir, mut app) = project();
    stand_in(dir.path());
    read_doc(dir.path(), &mut app);
    let words = "How is a stalled partition noticed?";
    comment(&mut app, "slow consumer", words);
    answers(
        dir.path(),
        2,
        &printed("Holds up (Correctness): Nothing in the doc says how a stall is noticed."),
    );
    press_on_card(&mut app, "Draft, not posted yet", "Validate");
    let text = shows(&mut app, "Holds up").text();
    assert!(text.contains("Correctness · Claude Code"), "{text}");
    let (args, prompt, _) = given(dir.path(), 2);
    assert!(values(&args, "--allowedTools").is_empty(), "{args:?}");
    assert!(prompt.contains(DOC), "{prompt}");
    assert!(!prompt.contains("add this one comment"), "{prompt}");
    assert_eq!(state_of(&app, words), Some(Sent::Draft), "nothing posted");
    // Its chip in green, the colour a comment that holds up has.
    app.set_measure(std::rc::Rc::new(cellix_render::FontMeasure::new(17, 37)));
    let grid = screen(&app);
    let green = app.theme.git_added;
    let chip = grid.paint.iter().any(
        |p| matches!(p, Paint::Text { text, color, .. } if text == "Holds up" && *color == green),
    );
    assert!(chip, "the verdict in green");
    app.set_measure(std::rc::Rc::new(cellix_render::CellMeasure));

    // Polish too: both stand at once.
    answers(
        dir.path(),
        3,
        &printed("How is a stalled partition noticed?"),
    );
    press_on_card(&mut app, "Draft, not posted yet", "Polish");
    let text = shows(&mut app, "Use This").text();
    assert!(text.contains("Holds up"), "{text}");

    // Sync, and a new Cellix: still there.
    answers(dir.path(), 4, &printed(&read_answer(json!([]))));
    let (x, y) = find(&screen(&app), "Sync");
    click(&mut app, x + 1, y);
    until(&mut app, "synced", |app| {
        runs(dir.path()) == 4 && app.remote.reading.is_none()
    });
    assert!(screen(&app).text().contains("Holds up"));
    let mut again = App::new(dir.path().to_path_buf(), (210, 46));
    again.reopen_remote_docs();
    again.dock.open = true;
    again.dock.show_comments();
    let text = screen(&again).text();
    assert!(text.contains("Holds up"), "{text}");
    assert!(text.contains("Use This"), "{text}");

    press_on_card(&mut app, "Holds up", "Dismiss");
    let text = screen(&app).text();
    assert!(!text.contains("Holds up"), "{text}");
    assert!(text.contains("Use This"), "Polish's words stay: {text}");
    let thread = thread_saying(&app, words);
    assert!(thread.validation.is_none());
    assert_eq!(state_of(&app, words), Some(Sent::Draft));
    assert_eq!(runs(dir.path()), 4);
}

/// The words of a card's buttons, in the order they stand, from the row under its words to the
/// card's end: what's on the screen's rows from `card` on, up to `till`.
fn buttons_after(grid: &Grid, card: &str, till: &str) -> String {
    let (_, top) = find(grid, card);
    let mut said = String::new();
    for y in top + 1..grid.height() {
        let row = grid.row_text(y);
        if row.contains(till) {
            break;
        }
        said.push_str(&row);
        said.push('\n');
    }
    said
}

/// Whether `words` stand in `said` in this order.
fn in_order(said: &str, words: &[&str]) -> bool {
    let mut from = 0;
    words.iter().all(|word| match said[from..].find(word) {
        Some(at) => {
            from += at + word.len();
            true
        }
        None => false,
    })
}

/// A card reads the same on a page of the project and on a remote doc: the reviewer's own has
/// Validate, Polish, Copy, Ask Agent and Reply, in that order, a remote draft Post to Doc
/// before them and a page's Resolve after them; someone else's Reply and Ask Agent. Polish
/// works on a page's comment as on a draft: Use This takes its words as Edit's Save does, so a
/// comment an agent was told of is an edited one, not told again; Copy copies its words.
#[test]
fn the_same_buttons_stand_on_both_and_polish_works_on_a_page() {
    let (dir, mut app) = project();
    stand_in(dir.path());
    let path = dir.path().join("plan.md");
    fs::write(&path, DOC).unwrap();
    app.open(&path).unwrap();
    let words = "I wonder if one slow consumer could hold back everything behind it here?";
    comment(&mut app, "slow consumer", words);
    app.dock.open = true;
    app.dock.show_comments();
    let grid = screen(&app);
    let said = buttons_after(&grid, "I wonder if one", "\u{2003}");
    assert!(
        in_order(
            &said,
            &[
                "Validate",
                "Polish",
                "Copy",
                "Ask Agent",
                "Reply",
                "Resolve"
            ]
        ),
        "{said}"
    );
    press_on_card(&mut app, words, "Copy");
    assert_eq!(app.clipboard, words);

    // Told to an agent, then polished and used: an edited comment, its words new.
    let id = thread_saying(&app, words).id;
    if let Some(thread) = app.threads.iter_mut().find(|t| t.id == id) {
        thread.told = Some(0);
    }
    let short = "Can one slow consumer hold back the rest?";
    answers(dir.path(), 1, &printed(short));
    press_on_card(&mut app, words, "Polish");
    let text = shows(&mut app, "Use This").text();
    assert!(text.contains("Claude Code · 8 words, from 13"), "{text}");
    let (args, prompt, _) = given(dir.path(), 1);
    assert_eq!(values(&args, "--tools"), [""]);
    assert!(prompt.contains(words), "{prompt}");
    assert_eq!(thread_saying(&app, words).text, words, "not till Use This");
    press_on_card(&mut app, "Polished", "Use This");
    let thread = thread_saying(&app, short);
    assert_eq!(thread.told, Some(0), "an edited comment, as Edit leaves it");
    assert!(thread.polished.is_none());
    assert!(thread.remote.is_none());
    assert!(!screen(&app).text().contains("Use This"));

    // A remote doc's draft, and someone else's comment, read the same way.
    app.settings.remote_docs_mcp = Some(SERVER.to_string());
    answers(dir.path(), 2, &printed(&read_answer(json!([]))));
    app.run(Command::ReviewRemoteDoc);
    typed(&mut app, LINK);
    key(&mut app, Key::Enter);
    shows(&mut app, "From team-docs, read by Claude Code");
    comment(&mut app, "holiday week", "Which week exactly?");
    let grid = screen(&app);
    let said = buttons_after(&grid, "Which week exactly?", "\u{2003}");
    assert!(
        in_order(
            &said,
            &[
                "Post to Doc",
                "Validate",
                "Polish",
                "Copy",
                "Ask Agent",
                "Reply"
            ]
        ),
        "{said}"
    );
    assert!(
        !said.contains("Resolve"),
        "a doc's comment is the doc's to resolve: {said}"
    );
    let said = buttons_after(&grid, "Where does 3x come from?", "You \u{b7}");
    assert!(
        in_order(&said, &["Reply", "Ask Agent", "Open in Doc"]),
        "{said}"
    );
    assert!(
        !said.contains("Validate") && !said.contains("Polish"),
        "{said}"
    );
    assert_eq!(runs(dir.path()), 2, "one polish, one read: nothing posted");
}

/// A stand-in `claude` that holds every run till a file named `go` is there, and keeps count
/// of how many ran at once, the most in `claude.most`. Each run answers `answer`.
fn holding_stand_in(dir: &Path, answer: &str) {
    let bin = dir.join("bin");
    let record = bin.join("claude");
    let live = bin.join("live");
    fs::create_dir_all(&live).unwrap();
    let out = bin.join("claude.held");
    fs::write(&out, printed(answer).join("\n") + "\n").unwrap();
    let script = format!(
        "#!/bin/sh\n\
         n=1; while ! /bin/mkdir '{rec}.run.'$n 2>/dev/null; do n=$((n + 1)); done\n\
         /bin/cat > '{rec}.prompt.'$n\n\
         : > '{live}/'$n\n\
         now=$(/bin/ls '{live}' | /usr/bin/wc -l | /usr/bin/tr -d ' ')\n\
         most=$(/bin/cat '{rec}.most' 2>/dev/null || echo 0)\n\
         if [ \"$now\" -gt \"$most\" ]; then echo $now > '{rec}.most'; fi\n\
         while [ ! -e '{go}' ]; do /bin/sleep 0.05; done\n\
         /bin/rm -f '{live}/'$n\n\
         while IFS= read -r line; do printf '%s\\n' \"$line\"; done < '{out}'\n",
        rec = record.display(),
        live = live.display(),
        go = dir.join("go").display(),
        out = out.display(),
    );
    fs::write(&record, script).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&record, fs::Permissions::from_mode(0o755)).unwrap();
    }
}

/// How many runs the holding stand-in has had: a folder each, made whole or not at all, as
/// two runs at once would both read one count and write the same next one.
fn held_runs(dir: &Path) -> usize {
    fs::read_dir(dir.join("bin"))
        .unwrap()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with("claude.run."))
        .count()
}

fn most_at_once(dir: &Path) -> usize {
    fs::read_to_string(dir.join("bin").join("claude.most"))
        .ok()
        .and_then(|n| n.trim().parse().ok())
        .unwrap_or(0)
}

/// Validate All, at the top of the Comments pane, validates every comment of the reviewer's
/// that has nothing from Validate yet, two at a time, the pane's head saying "Validating 2 of
/// 5" with a Stop; Stop lets the two under way finish and starts none of the rest. Pressed
/// again it takes the rest, never more than two at once, and none twice.
#[test]
fn validate_all_runs_two_at_a_time_and_stop_cancels_the_rest() {
    let (dir, mut app) = project();
    let path = dir.path().join("plan.md");
    fs::write(&path, DOC).unwrap();
    app.open(&path).unwrap();
    for (words, text) in [
        ("At launch", "Which launch?"),
        ("order events", "All of them?"),
        ("three times", "Three times what?"),
        ("holiday week", "Which week?"),
        ("slow consumer", "How slow?"),
    ] {
        comment(&mut app, words, text);
    }
    app.dock.open = true;
    app.dock.show_comments();
    holding_stand_in(dir.path(), "Holds up (Clarity): It's vague.");
    let (x, y) = find(&screen(&app), "Validate All");
    click(&mut app, x + 1, y);
    until(&mut app, "two started", |_| held_runs(dir.path()) == 2);
    for _ in 0..20 {
        app.poll();
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(held_runs(dir.path()), 2, "two at a time");
    let text = screen(&app).text();
    assert!(text.contains("Validating 2 of 5"), "{text}");
    let (x, y) = find(&screen(&app), "Stop");
    click(&mut app, x + 1, y);
    fs::write(dir.path().join("go"), "").unwrap();
    until(&mut app, "the two finished", |app| {
        app.threads
            .iter()
            .filter(|t| t.validation.is_some())
            .count()
            == 2
    });
    for _ in 0..20 {
        app.poll();
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(held_runs(dir.path()), 2, "Stop started none of the rest");
    let text = screen(&app).text();
    assert!(!text.contains("Validating"), "{text}");
    assert!(
        text.contains("Validate All"),
        "three still have none: {text}"
    );

    // Again: the three left, each once, never more than two at once.
    fs::remove_file(dir.path().join("go")).unwrap();
    let (x, y) = find(&screen(&app), "Validate All");
    click(&mut app, x + 1, y);
    until(&mut app, "two started again", |_| {
        held_runs(dir.path()) == 4
    });
    assert!(screen(&app).text().contains("Validating 2 of 3"));
    fs::write(dir.path().join("go"), "").unwrap();
    until(&mut app, "all validated", |app| {
        app.threads.iter().all(|t| t.validation.is_some())
    });
    assert_eq!(held_runs(dir.path()), 5, "none twice");
    assert_eq!(most_at_once(dir.path()), 2);
    let prompts: Vec<String> = (1..=5).map(|n| given(dir.path(), n).1).collect();
    for text in [
        "Which launch?",
        "All of them?",
        "Three times what?",
        "Which week?",
        "How slow?",
    ] {
        let told = prompts.iter().filter(|p| p.contains(text)).count();
        assert_eq!(told, 1, "{text:?} validated once");
    }
    assert!(
        !screen(&app).text().contains("Validate All"),
        "none left to validate"
    );
}

/// Post All Drafts asks first, saying how many, and posts nothing till Post is pressed; then
/// posts each draft once, one at a time, each card saying where it stands. One the doc tool
/// refuses says why on its card, and the rest still go.
#[test]
fn post_all_drafts_asks_first_and_posts_each_once_one_at_a_time() {
    let (dir, mut app) = project();
    stand_in(dir.path());
    read_doc(dir.path(), &mut app);
    let drafts = [
        ("slow consumer", "How slow is slow?"),
        ("holiday week", "Which week exactly?"),
        ("order events", "All the order events?"),
    ];
    for (words, text) in drafts {
        comment(&mut app, words, text);
    }
    let (x, y) = find(&screen(&app), "Post All Drafts");
    click(&mut app, x + 1, y);
    let text = screen(&app).text();
    assert!(text.contains("Post 3 comments to the doc?"), "{text}");
    assert_eq!(runs(dir.path()), 1, "nothing posted while it asks");
    let (x, y) = find(&screen(&app), "Cancel");
    click(&mut app, x + 1, y);
    for _ in 0..10 {
        app.poll();
    }
    assert_eq!(runs(dir.path()), 1, "Cancel posts nothing");

    answers(dir.path(), 2, &printed(r#"{"id": "c-1"}"#));
    answers(
        dir.path(),
        3,
        &printed(r#"{"error": "The doc is locked for comments."}"#),
    );
    answers(dir.path(), 4, &printed(r#"{"id": "c-3"}"#));
    let (x, y) = find(&screen(&app), "Post All Drafts");
    click(&mut app, x + 1, y);
    // The question's buttons, on the row under it that has Cancel.
    let grid = screen(&app);
    let (_, asked) = find(&grid, "Post 3 comments to the doc?");
    let y = (asked + 1..grid.height())
        .find(|y| grid.row_text(*y).contains("Cancel"))
        .expect("the question's buttons");
    let row = grid.row_text(y);
    let x = row[..row.find("Post").expect("its Post")].chars().count() as u16;
    click(&mut app, x + 1, y);
    // One at a time: never two posts under way, each card saying where it stands.
    let deadline = Instant::now() + PATIENCE;
    loop {
        assert!(app.remote.posts.len() <= 1, "one post at a time");
        let tried = drafts.iter().all(|(_, text)| {
            matches!(
                state_of(&app, text),
                Some(Sent::Posted { .. } | Sent::Failed { .. })
            )
        });
        if tried && app.remote.posts.is_empty() {
            break;
        }
        assert!(Instant::now() < deadline, "never all tried");
        app.poll();
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(runs(dir.path()), 4, "each posted once");
    let prompts: Vec<String> = (2..=4).map(|run| given(dir.path(), run).1).collect();
    for (_, text) in drafts {
        let told = prompts.iter().filter(|p| p.contains(text)).count();
        assert_eq!(told, 1, "{text:?} posted once");
    }
    for prompt in &prompts {
        assert!(prompt.contains("Add only this comment, once"), "{prompt}");
    }
    // The second post failed, and the third went all the same.
    let second = drafts
        .iter()
        .find(|(_, text)| prompts[1].contains(text))
        .map(|(_, text)| *text)
        .unwrap();
    for (_, text) in drafts {
        let state = state_of(&app, text);
        if text == second {
            assert!(matches!(state, Some(Sent::Failed { .. })), "{state:?}");
        } else {
            assert!(
                matches!(state, Some(Sent::Posted { .. })),
                "{text}: {state:?}"
            );
        }
    }
    let text = screen(&app).text();
    assert!(text.contains("locked"), "why on its card: {text}");
    for _ in 0..20 {
        app.poll();
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(runs(dir.path()), 4);
}

/// The bulk row's buttons flow to another row where the pane's head has no room, as a
/// card's buttons do, each whole, never cut short; a press on one that flowed works as
/// one that fit would.
#[test]
fn bulk_buttons_flow_where_the_narrow_pane_has_no_room() {
    let dir = tempfile::tempdir().unwrap();
    let bin = dir.path().join("bin");
    fs::create_dir(&bin).unwrap();
    let home = dir.path().join("home");
    fs::create_dir(&home).unwrap();
    fs::write(
        home.join(".claude.json"),
        r#"{"mcpServers":{"team-docs":{"command":"docs-mcp"}}}"#,
    )
    .unwrap();
    let mut app = App::new(dir.path().to_path_buf(), (64, 40));
    app.side_path = Some(bin.into_os_string());
    app.remote.env = Some(HashMap::from([(
        "HOME".to_string(),
        home.display().to_string(),
    )]));
    // The doc read, as the read would leave it (the read's own wait can't run in a window
    // this narrow, whose strip has no room for its words): its files written, its pane open.
    let slug = "queue-launch";
    let folder = remote::folder(dir.path(), slug);
    fs::create_dir_all(&folder).unwrap();
    fs::write(folder.join("doc.md"), DOC).unwrap();
    let doc_file = format!(".cellix/remote/{slug}/doc.md");
    app.remote.docs.insert(
        slug.to_string(),
        remote::Meta {
            title: "Queue service launch".to_string(),
            link: LINK.to_string(),
            version: "12".to_string(),
            server: SERVER.to_string(),
            synced: cellix_review::threads::now(),
            others: 0,
        },
    );
    // The drafts, made as the doc read's comments are: at 64 columns the doc page's words
    // can't be selected, so they're put in place of a comment made on them.
    let at = cellix_review::threads::now();
    for (n, (quote, text)) in [
        ("slow consumer", "How slow is slow?"),
        ("holiday week", "Which week exactly?"),
        ("order events", "All of them?"),
    ]
    .into_iter()
    .enumerate()
    {
        app.threads.push(cellix_review::Thread {
            id: 6 + n as u32,
            file: doc_file.clone(),
            line: 5,
            quote: quote.to_string(),
            text: text.to_string(),
            placed: true,
            at: at + n as u64,
            remote: Some(cellix_review::threads::Remote::draft()),
            ..Default::default()
        });
    }
    app.open(&dir.path().join(&doc_file)).unwrap();
    app.dock.open = true;
    app.dock.show_comments();
    let text = screen(&app).text();
    assert!(text.contains("Validate All"), "{text}");
    assert!(text.contains("Polish All"), "{text}");
    assert!(
        text.contains("Post All Drafts"),
        "whole, never cut short: {text}"
    );
    // A press on one that flowed asks as the wide pane's does, and posts nothing till Post.
    let (x, y) = find(&screen(&app), "Post All Drafts");
    click(&mut app, x + 1, y);
    let text = screen(&app).text();
    assert!(text.contains("Post 3 comments to the doc?"), "{text}");
    assert_eq!(runs(dir.path()), 0, "nothing posted while it asks");
    let (x, y) = find(&screen(&app), "Cancel");
    click(&mut app, x + 1, y);
    for _ in 0..10 {
        app.poll();
    }
    assert_eq!(runs(dir.path()), 0, "Cancel posts nothing");
    for text in ["How slow is slow?", "Which week exactly?", "All of them?"] {
        assert!(matches!(state_of(&app, text), Some(Sent::Draft)), "{text}");
    }
}

/// A press on a kind's pill lists only that kind, and Validate All then takes only what's
/// listed: the pane's row acts on what the pane shows.
#[test]
fn validate_all_takes_only_the_kind_the_pill_lists() {
    let (dir, mut app) = project();
    let path = dir.path().join("plan.md");
    fs::write(&path, DOC).unwrap();
    app.open(&path).unwrap();
    for (words, text) in [
        ("At launch", "Which launch?"),
        ("order events", "All of them?"),
        ("three times", "Three times what?"),
    ] {
        comment(&mut app, words, text);
    }
    app.threads[0].kind = "Clarity".into();
    app.threads[1].kind = "Evidence".into();
    app.threads[2].kind = "Clarity".into();
    app.dock.open = true;
    app.dock.show_comments();
    holding_stand_in(dir.path(), "Holds up (Clarity): It's vague.");
    let (x, y) = find(&screen(&app), "Clarity 2");
    click(&mut app, x + 1, y);
    let text = screen(&app).text();
    assert!(
        !text.contains("All of them?"),
        "the other kind isn't listed: {text}"
    );
    let (x, y) = find(&screen(&app), "Validate All");
    click(&mut app, x + 1, y);
    until(&mut app, "the two listed started", |_| {
        held_runs(dir.path()) == 2
    });
    fs::write(dir.path().join("go"), "").unwrap();
    until(&mut app, "both validated", |app| {
        app.threads
            .iter()
            .filter(|t| t.validation.is_some())
            .count()
            == 2
    });
    for _ in 0..20 {
        app.poll();
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(held_runs(dir.path()), 2, "only the kind listed");
    let prompts: Vec<String> = (1..=2).map(|n| given(dir.path(), n).1).collect();
    for text in ["Which launch?", "Three times what?"] {
        let told = prompts.iter().filter(|p| p.contains(text)).count();
        assert_eq!(told, 1, "{text:?} told once");
    }
    assert!(
        !prompts.iter().any(|p| p.contains("All of them?")),
        "not the kind not listed: {prompts:?}"
    );
    let text = screen(&app).text();
    assert_eq!(text.matches("Holds up").count(), 2, "{text}");
}

/// A runner that works stands in the Default agent row after the built-ins, and a press
/// picks it for Ask, Polish and Validate: `ask_agent` keeps its name, as it keeps a
/// built-in's command.
#[test]
fn the_default_agent_row_offers_a_runner_and_keeps_its_name() {
    let (dir, mut app) = project();
    let script = "#!/bin/sh\nexec sleep 30\n";
    fs::write(dir.path().join("bin/claude-ds"), script).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(
            dir.path().join("bin/claude-ds"),
            fs::Permissions::from_mode(0o755),
        )
        .unwrap();
    }
    app.settings.agents.push(crate::settings::CustomAgent {
        name: "Claude (DeepSeek)".to_string(),
        command: "claude-ds".to_string(),
        like: "claude".to_string(),
    });
    app.run(Command::OpenSettings);
    let integrations = {
        let page = app.settings_page.as_ref().unwrap();
        let spots = page.controls(app.sheet_area());
        let section = crate::settings_page::Section::Integrations;
        let tab = spots
            .iter()
            .find(|(_, c)| *c == crate::settings_page::Control::Section(section));
        tab.unwrap().0
    };
    click(&mut app, integrations.x + 1, integrations.y);
    let grid = screen(&app);
    let text = grid.text();
    assert!(text.contains("Claude (DeepSeek)"), "{text}");
    // A press on it picks it.
    let (x, y) = on_row_of(&grid, "Default agent", "Claude (DeepSeek)");
    click(&mut app, x + 1, y);
    assert_eq!(app.settings.ask_agent.as_deref(), Some("Claude (DeepSeek)"));
}

/// Settings › Integrations has a Your agents card: each of the person's runners with Edit
/// and Remove, each runner found on the PATH with Add as its agent, and Add Agent, which
/// opens a form. A press on Add as Claude Code adds the found runner; Edit opens the form
/// filled; Remove asks first and takes it out of every offer.
#[test]
fn the_your_agents_card_lists_adds_edits_and_removes_runners() {
    let (dir, mut app) = project();
    for name in ["claude-ds", "codex-fast"] {
        fs::write(
            dir.path().join(format!("bin/{name}")),
            "#!/bin/sh\nexec sleep 30\n",
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(
                dir.path().join(format!("bin/{name}")),
                fs::Permissions::from_mode(0o755),
            )
            .unwrap();
        }
    }
    app.settings.agents.push(crate::settings::CustomAgent {
        name: "Claude (DeepSeek)".to_string(),
        command: "claude-ds".to_string(),
        like: "claude".to_string(),
    });
    app.run(Command::OpenSettings);
    let integrations = {
        let page = app.settings_page.as_ref().unwrap();
        let spots = page.controls(app.sheet_area());
        let section = crate::settings_page::Section::Integrations;
        let tab = spots
            .iter()
            .find(|(_, c)| *c == crate::settings_page::Control::Section(section));
        tab.unwrap().0
    };
    click(&mut app, integrations.x + 1, integrations.y);
    // The card is below the agents' cards: the page scrolls whole, and its rows come into
    // view.
    let area = app.sheet_area();
    if let Some(page) = app.settings_page.as_mut() {
        page.scroll_by(50, area);
    }
    let grid = screen(&app);
    let text = grid.text();
    // The added runner, its command and what it works like; the found one, offered, added
    // by nothing yet.
    assert!(text.contains("Claude (DeepSeek)"), "{text}");
    assert!(text.contains("works like Claude Code"), "{text}");
    assert!(text.contains("claude-ds"), "{text}");
    assert!(text.contains("codex-fast"), "found, never added: {text}");
    assert_eq!(app.settings.agents.len(), 1, "nothing adds itself");
    // A press on Add as Codex adds it, as the agent its name's prefix says.
    let (x, y) = on_row_of(&grid, "codex-fast", "Add as Codex");
    click(&mut app, x + 1, y);
    assert_eq!(app.settings.agents.len(), 2);
    assert_eq!(app.settings.agents[1].command, "codex-fast");
    assert_eq!(app.settings.agents[1].like, "codex");
    // Edit opens the form filled with what it is.
    let grid = screen(&app);
    let (x, y) = on_row_of(&grid, "codex-fast", "Edit");
    click(&mut app, x + 1, y);
    let text = screen(&app).text();
    assert!(text.contains("Edit Agent"), "{text}");
    assert!(text.contains("codex-fast"), "{text}");
    key(&mut app, Key::Esc);
    // Remove asks first, and takes it out.
    let grid = screen(&app);
    let (x, y) = on_row_of(&grid, "codex-fast", "Remove");
    click(&mut app, x + 1, y);
    assert!(screen(&app).text().contains("Remove codex-fast?"));
    // The dialog's own Remove, on its buttons' row.
    let grid = screen(&app);
    let (_, asked) = find(&grid, "Remove codex-fast?");
    let buttons = (asked + 1..grid.height())
        .find(|y| grid.row_text(*y).contains("Cancel"))
        .expect("the dialog's buttons");
    let row = grid.row_text(buttons);
    let at = row.find("Remove").expect("the dialog's Remove");
    let x = row[..at].chars().count() as u16 + 1;
    click(&mut app, x, buttons);
    assert_eq!(app.settings.agents.len(), 1);
    assert!(
        !app.settings
            .agents
            .iter()
            .any(|a| a.command == "codex-fast")
    );
}

/// Polish runs the Default agent: where that's one of the person's runners, the runner's own
/// command runs, with its like agent's treatment and its own name on the card; what the
/// command printed on failure reaches the card, as any agent's does.
#[test]
fn polish_runs_a_runner_as_the_default_agent() {
    let (dir, mut app) = project();
    // A stand-in runner that behaves as the stand-in claude does: each run answers what it's
    // told, and it records how it was run; the doc's read goes through the stand-in claude.
    stand_in(dir.path());
    stand_in_for(dir.path(), "claude-ds");
    app.settings.agents.push(crate::settings::CustomAgent {
        name: "Claude (DeepSeek)".to_string(),
        command: "claude-ds".to_string(),
        like: "claude".to_string(),
    });
    app.settings.ask_agent = Some("Claude (DeepSeek)".to_string());
    // The doc's read runs the runner too (it works like Claude Code): its first run answers
    // the read, its second the polish.
    answers_for(
        dir.path(),
        "claude-ds",
        1,
        &printed(&read_answer(json!([]))),
    );
    read_doc(dir.path(), &mut app);
    let long = "I think this is a problem because if one consumer gets slow for whatever reason \
                then all of the events behind it are going to wait.";
    comment(&mut app, "slow consumer", long);
    let short = "One slow consumer stalls its partition. How is that detected?";
    answers_for(dir.path(), "claude-ds", 2, &printed(short));
    press_on_card(&mut app, "Draft, not posted yet", "Polish");
    let text = shows(&mut app, "Use This").text();
    assert_eq!(
        runs_of(dir.path(), "claude-ds"),
        2,
        "the runner read and polished"
    );
    assert_eq!(runs(dir.path()), 0, "and plain claude didn't");
    let (_args, prompt, side) = given_for(dir.path(), "claude-ds", 2);
    assert_eq!(side, "1", "a side run, which no hook takes for an agent's");
    assert!(prompt.contains("short, clear and kind"), "{prompt}");
    assert!(
        text.contains("Claude (DeepSeek) · 10 words, from 26"),
        "the card says who: {text}"
    );
    // The doc is still read through Claude Code, not the runner, until the Default agent
    // names a runner that works like Claude Code.
    assert_eq!(thread_saying(&app, long).text, long);
}

/// Agents turned off are offered nowhere new: not the + menu, Review with…, New Worktree or
/// the Default agent row, which Auto answers for; what's already running keeps its tab and
/// can still be resumed; turned back on, they're offered again.
#[test]
fn agents_turned_off_are_offered_nowhere_new_and_back_when_on() {
    let (dir, mut app) = project();
    app.agents.pretend_installed(&["claude", "codex", "gemini"]);
    // A Codex session already running, as one going keeps its tab however the switch stands.
    stand_in(dir.path());
    app.agent_program = Some(dir.path().join("bin/claude").display().to_string());
    app.open_agent("codex");
    let sessions = app.dock.sessions.len();
    assert_eq!(sessions, 1);
    // Turn Codex and Gemini off, on the Your agents card's switches.
    app.run(Command::OpenSettings);
    let integrations = {
        let page = app.settings_page.as_ref().unwrap();
        let spots = page.controls(app.sheet_area());
        let section = crate::settings_page::Section::Integrations;
        let tab = spots
            .iter()
            .find(|(_, c)| *c == crate::settings_page::Control::Section(section));
        tab.unwrap().0
    };
    click(&mut app, integrations.x + 1, integrations.y);
    let area = app.sheet_area();
    if let Some(page) = app.settings_page.as_mut() {
        page.scroll_by(20, area);
    }
    let off = |app: &mut App, key: &str| {
        let page = app.settings_page.as_ref().unwrap();
        let spot = page
            .controls(app.sheet_area())
            .into_iter()
            .find(|(_, c)| matches!(c, crate::settings_page::Control::ToggleAgent(k) if k == key))
            .map(|(rect, _)| rect)
            .unwrap_or_else(|| panic!("no switch for {key}"));
        click(app, spot.x + 1, spot.y);
    };
    off(&mut app, "codex");
    off(&mut app, "gemini");
    assert_eq!(
        app.settings.disabled_agents,
        vec!["codex".to_string(), "gemini".to_string()]
    );
    app.run(Command::CloseTab);
    // Offered nowhere new: not the + menu, Review with…, nor New Worktree's choice. (The
    // running Codex tab says "Codex" on the tab row; the menus' own rows are what's read.)
    let labels = |app: &App| match &app.overlay {
        super::Overlay::Menu(menu) => menu.labels(),
        _ => panic!("no menu"),
    };
    app.show_start_menu((2, 2));
    let menu = labels(&app);
    assert!(
        !menu.iter().any(|l| l == "Codex" || l == "Gemini CLI"),
        "{menu:?}"
    );
    key(&mut app, Key::Esc);
    app.run(Command::ReviewWith);
    let menu = labels(&app);
    assert!(
        !menu.iter().any(|l| l == "Codex" || l == "Gemini CLI"),
        "{menu:?}"
    );
    key(&mut app, Key::Esc);
    app.run(Command::NewWorktree);
    let shown = screen(&app).text();
    assert!(
        !shown.contains("Gemini CLI") && !shown.contains("  Codex  "),
        "not a choice in New Worktree: {shown}"
    );
    key(&mut app, Key::Esc);
    // The running Codex tab is untouched, and it can still be resumed.
    assert_eq!(app.dock.sessions.len(), sessions);
    // The Default agent row offers neither, and Auto answers.
    app.run(Command::OpenSettings);
    let integrations = {
        let page = app.settings_page.as_ref().unwrap();
        let spots = page.controls(app.sheet_area());
        let section = crate::settings_page::Section::Integrations;
        let tab = spots
            .iter()
            .find(|(_, c)| *c == crate::settings_page::Control::Section(section));
        tab.unwrap().0
    };
    click(&mut app, integrations.x + 1, integrations.y);
    let has = |app: &App, name: &str| {
        app.settings_page
            .as_ref()
            .unwrap()
            .controls(app.sheet_area())
            .iter()
            .any(|(_, c)| matches!(c, crate::settings_page::Control::DefaultAgent(n) if *n == name))
    };
    assert!(has(&app, "auto") && has(&app, "claude"));
    assert!(
        !has(&app, "codex") && !has(&app, "gemini"),
        "turned off: not in the row"
    );
    // Turned back on, they're offered again.
    let area = app.sheet_area();
    if let Some(page) = app.settings_page.as_mut() {
        page.scroll_by(20, area);
    }
    off(&mut app, "codex");
    off(&mut app, "gemini");
    assert!(app.settings.disabled_agents.is_empty());
    app.run(Command::CloseTab);
    app.show_start_menu((2, 2));
    let menu = labels(&app);
    assert!(
        menu.iter().any(|l| l == "Codex") && menu.iter().any(|l| l == "Gemini CLI"),
        "{menu:?}"
    );
    key(&mut app, Key::Esc);
}
