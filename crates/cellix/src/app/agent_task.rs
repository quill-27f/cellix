//! The Task page in the app: opening it, drawing it where the file would be, and doing what
//! its buttons, rows and menus ask.

use std::path::PathBuf;

use cellix_core::Buffer;
use cellix_input::{Key, KeyEvent, MouseEvent, MouseKind};
use cellix_render::Grid;
use cellix_review::tasks::{self, Event as TaskEvent, Step};

use super::{App, Choice, Command, Confirm, Front, Overlay, Prompt, PromptAction};
use crate::agent_task::{Context, Hit, TaskPage};
use crate::editor::Editor;
use crate::menu::{Action as MenuAction, Item as MenuItem};
use crate::theme::Theme;

/// What a row of a Task page menu does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Act {
    /// Shows another task, by its place in the page's list.
    Show(usize),
    ReviewTurn,
    UndoTurn,
    Split,
    Rename,
}

impl App {
    pub(super) fn task_shown(&self) -> bool {
        self.task_shown && self.task_page.is_some()
    }

    /// Opens the Task page on the work an agent did last here, or where it was left.
    pub(super) fn show_task_page(&mut self) {
        let root = self.workspace.root().to_path_buf();
        match &mut self.task_page {
            Some(page) if page.root() == root => page.reload(),
            _ => self.task_page = Some(TaskPage::new(root)),
        }
        self.review = None;
        self.settings_page = None;
        self.task_shown = true;
    }

    fn task_context(&self) -> Context<'_> {
        Context {
            threads: &self.threads,
            running: &self.agents.running,
            pending_plan: self.plan.as_ref().map(|p| p.path.as_path()),
            measure: &*self.measure,
        }
    }

    /// Draws the page if it's in view, and says whether it was.
    pub(super) fn draw_task_page(&self, grid: &mut Grid, theme: &Theme) -> bool {
        let Some(page) = self.task_page.as_ref().filter(|_| self.task_shown) else {
            return false;
        };
        page.title_room.set(self.actions_over_main());
        page.draw(
            grid,
            self.main_area(),
            self.hover,
            &self.task_context(),
            theme,
        );
        true
    }

    /// Takes in what changed since the page was drawn. Says whether to draw it again.
    pub(super) fn poll_task_page(&mut self) -> bool {
        if !self.task_shown {
            return false;
        }
        self.task_page.as_mut().is_some_and(TaskPage::poll)
    }

    /// A turn that is still going changes the tree, so what it changed is read again now
    /// and then.
    pub(super) fn refresh_task_page(&mut self) {
        if let Some(page) = self.task_page.as_mut().filter(|p| !p.is_reading())
            && self.task_shown
        {
            page.read_changes_later();
        }
    }

    pub(super) fn task_key(&mut self, key: KeyEvent) -> bool {
        if !self.task_shown() || self.review.is_some() {
            return false;
        }
        let Some(page) = &mut self.task_page else {
            return false;
        };
        match key.key {
            Key::Esc => self.task_shown = false,
            Key::Down | Key::Right => page.step_by(1),
            Key::Up | Key::Left => page.step_by(-1),
            Key::Enter => match page.chosen() {
                Some(Step::Turn(_)) => self.use_task_hit(Hit::ReviewTurn),
                Some(Step::Plan(_)) => self.use_task_hit(Hit::OpenPlan),
                None => {}
            },
            _ => return false,
        }
        true
    }

    /// Handles the pointer over the page. Says whether it was the page's.
    pub(super) fn task_mouse(&mut self, mouse: MouseEvent) -> bool {
        let area = self.main_area();
        if !self.task_shown() || self.review.is_some() || !area.contains(mouse.x, mouse.y) {
            return false;
        }
        let hit = {
            let context = self.task_context();
            let page = self.task_page.as_ref();
            page.and_then(|p| p.hit(area, mouse.x, mouse.y, &context, &self.theme))
        };
        match mouse.kind {
            MouseKind::Left => {
                if let Some(hit) = hit {
                    self.use_task_hit(hit);
                }
            }
            MouseKind::Right => {
                if let Some(Hit::Step(index)) = hit {
                    if let Some(page) = &mut self.task_page {
                        page.choose(index);
                    }
                    self.task_step_menu(mouse.x, mouse.y);
                }
            }
            MouseKind::ScrollUp | MouseKind::ScrollDown => {
                let delta = if mouse.kind == MouseKind::ScrollUp {
                    -self.wheel_lines
                } else {
                    self.wheel_lines
                };
                if let Some(page) = &mut self.task_page {
                    page.scroll_by(area, mouse.x, delta);
                }
            }
            _ => {}
        }
        true
    }

    fn task_step_menu(&mut self, x: u16, y: u16) {
        let Some(page) = &self.task_page else {
            return;
        };
        let mut items = Vec::new();
        if let Some(turn) = page.chosen_turn() {
            let undo = if page.turns_after() == 0 {
                "Undo Turn"
            } else {
                "Undo From Here"
            };
            items.push(MenuItem::new(
                "Review Turn",
                MenuAction::Task(Act::ReviewTurn),
            ));
            items.push(
                MenuItem::new(undo, MenuAction::Task(Act::UndoTurn)).when(turn.commit.is_some()),
            );
            items.push(
                MenuItem::new("Start a New Task Here", MenuAction::Task(Act::Split))
                    .when(turn.number > 1)
                    .after_rule(),
            );
        }
        items.push(MenuItem::new("Rename Task…", MenuAction::Task(Act::Rename)).after_rule());
        self.show_menu((x, y.saturating_add(1)), items);
    }

    /// Lists the tasks here, newest first, to switch to one.
    fn task_switch_menu(&mut self) {
        let Some(page) = &self.task_page else {
            return;
        };
        let mut order: Vec<usize> = (0..page.tasks.len()).collect();
        order.sort_by_key(|&i| std::cmp::Reverse(page.tasks[i].latest()));
        let mut items: Vec<MenuItem> = order
            .into_iter()
            .map(|i| {
                let task = &page.tasks[i];
                let turns = task.turns().count();
                let state = if task.waiting_plan().is_some() {
                    "plan waiting".to_string()
                } else {
                    format!("{turns} turn{}", if turns == 1 { "" } else { "s" })
                };
                let label = format!("{}  ·  {}, {state}", task.name, task.agent);
                MenuItem::new(label, MenuAction::Task(Act::Show(i))).when(i != page.shown)
            })
            .collect();
        items.push(MenuItem::new("Rename This Task…", MenuAction::Task(Act::Rename)).after_rule());
        let area = self.main_area();
        self.show_menu((area.x + 2, area.y + 1), items);
    }

    pub(super) fn run_task_act(&mut self, act: Act) {
        match act {
            Act::Show(index) => {
                if let Some(page) = &mut self.task_page {
                    page.show_task(index);
                }
            }
            Act::ReviewTurn => self.use_task_hit(Hit::ReviewTurn),
            Act::UndoTurn => self.use_task_hit(Hit::UndoTurn),
            Act::Split => {
                let event = self.task_page.as_ref().and_then(TaskPage::split_here);
                if let Some(TaskEvent::Split { at, session }) = &event {
                    let id = format!("{session}@{at}");
                    self.log_task_event(event.clone().into_iter().collect());
                    if let Some(page) = &mut self.task_page
                        && let Some(index) = page.tasks.iter().position(|t| t.id == id)
                    {
                        page.show_task(index);
                        page.choose(0);
                    }
                }
            }
            Act::Rename => {
                let name = self
                    .task_page
                    .as_ref()
                    .and_then(|p| p.task())
                    .map(|t| t.name.clone())
                    .unwrap_or_default();
                self.overlay = Overlay::Prompt(Prompt {
                    label: "Name this task".to_string(),
                    input: name,
                    action: PromptAction::RenameTask,
                });
            }
        }
    }

    pub(super) fn rename_task(&mut self, name: &str) {
        let event = self.task_page.as_ref().and_then(|p| p.rename(name));
        self.log_task_event(event.into_iter().collect());
    }

    /// Adds the reviewer's own events to the log, and shows what they did.
    fn log_task_event(&mut self, events: Vec<TaskEvent>) {
        let Some(page) = &mut self.task_page else {
            return;
        };
        for event in &events {
            if let Err(e) = tasks::record(page.root(), event) {
                self.say(format!("Could not save the task: {e}"));
                return;
            }
        }
        page.reload();
    }

    fn use_task_hit(&mut self, hit: Hit) {
        let Some(page) = &mut self.task_page else {
            return;
        };
        match hit {
            Hit::Step(index) => page.choose(index),
            Hit::Previous => page.step_by(-1),
            Hit::Next => page.step_by(1),
            Hit::Switch => self.task_switch_menu(),
            Hit::ReviewTurn => self.review_task_turn(None),
            Hit::File(path) => self.review_task_turn(Some(path)),
            Hit::ReviewAll => {
                let Some(first) = page.first_commit() else {
                    return;
                };
                let root = page.root().to_path_buf();
                let mut review = self.new_review(root);
                if review.show_turn(&first, None) {
                    self.review = Some(review);
                } else {
                    self.say("The task's first checkpoint is gone");
                }
            }
            Hit::UndoTurn => {
                let after = page.turns_after();
                let Some(turn) = page.chosen_turn().cloned() else {
                    return;
                };
                let Some(commit) = turn.commit.clone() else {
                    self.say(
                        "This folder isn't in git, so the turn has no checkpoint to go back to",
                    );
                    return;
                };
                let title = match after {
                    0 => format!("Undo turn {}?", turn.number),
                    1 => format!("Undo turn {} and the one after it?", turn.number),
                    n => format!("Undo turn {} and the {n} after it?", turn.number),
                };
                let detail = format!(
                    "Files go back to how they were before \u{201c}{}\u{201d}. A checkpoint is kept, so you can redo.",
                    tasks::name_from(&turn.text)
                );
                self.overlay = Overlay::Confirm(Confirm {
                    title,
                    detail,
                    buttons: vec![("Undo", Choice::Undo(commit)), ("Cancel", Choice::Cancel)],
                    selected: 0,
                });
            }
            Hit::Test(index) => {
                let Some(run) = page.chosen_turn().and_then(|t| t.tests.get(index)) else {
                    return;
                };
                let text = format!("$ {}\n\n{}", run.command, run.output);
                let editor = Editor::new(
                    Buffer::from_text(&text),
                    self.measure.clone(),
                    self.registry.clone(),
                );
                self.editors.push(editor);
                self.set_active(self.editors.len() - 1);
            }
            Hit::Thread(id) => {
                let Some(thread) = self.threads.iter().find(|t| t.id == id) else {
                    return;
                };
                let (file, line) = (thread.file.clone(), thread.line);
                let path = page.root().join(&file);
                match self.open(&path) {
                    Ok(()) => {
                        let area = self.focused_area();
                        if let Some(editor) = self.editor_mut() {
                            editor.go_to_line(line, area);
                        }
                        if !self.dock.showing(Front::Comments) {
                            self.run(Command::ShowComments);
                        }
                    }
                    Err(e) => self.say(format!("Could not open {file}: {e}")),
                }
            }
            Hit::Transcript => {
                let Some(task) = page.task() else {
                    return;
                };
                let (agent, session) = (task.agent.clone(), task.session.clone());
                match crate::transcript::find(&agent, &session) {
                    Some(source) => self.open_transcript_file(&agent, &session, source),
                    None => self.say(format!("Can't find {agent}'s transcript for this task")),
                }
            }
            Hit::Approve => self.run(Command::ApprovePlan),
            Hit::SendBack => self.run(Command::SendBackPlan),
            Hit::ComparePlan => self.run(Command::ComparePlanVersions),
            Hit::OpenPlan => {
                let path = page
                    .task()
                    .zip(page.chosen_plan())
                    .and_then(|(task, plan)| page.plan_path(task, plan));
                if let Some(path) = path
                    && let Err(e) = self.open(&path)
                {
                    self.say(format!("Could not open the plan: {e}"));
                }
            }
        }
    }

    /// Opens Review on what the chosen turn changed, at a file if one is given.
    fn review_task_turn(&mut self, path: Option<PathBuf>) {
        let Some(page) = &self.task_page else {
            return;
        };
        let Some(turn) = page.chosen_turn().cloned() else {
            return;
        };
        let Some(commit) = turn.commit else {
            self.say("This folder isn't in git, so the turn has no checkpoint to compare with");
            return;
        };
        let root = page.root().to_path_buf();
        let mut review = self.new_review(root);
        if !review.show_turn(&commit, turn.until) {
            self.say("That turn's checkpoint is gone");
            return;
        }
        if let Some(path) = path {
            review.show_path(path);
        }
        self.review = Some(review);
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;
    use std::process::Command as Process;
    use std::time::{Duration, Instant};

    use cellix_input::{Event as Input, Key, KeyEvent, Mods, MouseEvent, MouseKind};
    use cellix_render::Grid;
    use cellix_review::tasks::{self, Event};
    use tempfile::TempDir;

    use super::super::{App, Command};

    const BACKGROUND: Duration = Duration::from_secs(30);
    const MINUTE: u64 = 60_000;

    fn git(dir: &Path, args: &[&str]) {
        let status = Process::new("git").arg("-C").arg(dir).args(args).status();
        assert!(status.unwrap().success(), "git {args:?}");
    }

    fn prompt(at: u64, text: &str) -> Event {
        Event::Prompt {
            at,
            session: "s1".into(),
            agent: "Claude Code".into(),
            text: text.into(),
            commit: None,
            threads: 0,
        }
    }

    /// A project where an agent planned, was sent back, was approved, worked and tested, and
    /// then took a second prompt, as its hooks log it.
    fn worked_on() -> (TempDir, App) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        git(&root, &["init", "-q"]);
        git(&root, &["config", "user.email", "t@example.com"]);
        git(&root, &["config", "user.name", "T"]);
        fs::write(root.join("orders.rs"), "fn refund() {}\n").unwrap();
        git(&root, &["add", "."]);
        git(&root, &["commit", "-qm", "start"]);
        let start = tasks::now() - 30 * MINUTE;
        let first = prompt(start, "Add a refund route");
        tasks::checkpoint_prompt(&root, "Add a refund route", Some((&root, first)));
        // The plan's versions where the hook keeps them, out of the tree.
        let plans = root.join(".git/plans");
        fs::create_dir_all(&plans).unwrap();
        fs::write(plans.join("Plan v1.md"), "# Refunds\n\nRefund any order.\n").unwrap();
        fs::write(
            plans.join("Plan.md"),
            "# Refunds\n\nRefund paid orders only.\n",
        )
        .unwrap();
        let file = plans.join("Plan.md").display().to_string();
        let plan = |at| Event::Plan {
            at,
            session: "s1".into(),
            agent: "Claude Code".into(),
            file: file.clone(),
        };
        let answer = |at, approved: bool| Event::Answer {
            at,
            file: file.clone(),
            approved,
            note: if approved {
                String::new()
            } else {
                "Only paid orders".into()
            },
            kept: approved.then(|| plans.display().to_string()),
        };
        let test = |at, passed| Event::Test {
            at,
            session: "s1".into(),
            command: "cargo test".into(),
            passed,
            output: if passed {
                "ok".into()
            } else {
                "test refund ... FAILED".into()
            },
        };
        for event in [
            plan(start + MINUTE),
            answer(start + 2 * MINUTE, false),
            plan(start + 3 * MINUTE),
            answer(start + 4 * MINUTE, true),
        ] {
            tasks::record(&root, &event).unwrap();
        }
        fs::write(
            root.join("orders.rs"),
            "fn refund() {}\nfn refund_paid() {}\n",
        )
        .unwrap();
        tasks::record(&root, &test(start + 5 * MINUTE, false)).unwrap();
        tasks::record(&root, &test(start + 6 * MINUTE, true)).unwrap();
        let second = prompt(start + 10 * MINUTE, "Reject a refund on an unpaid order");
        tasks::checkpoint_prompt(&root, "Reject unpaid", Some((&root, second)));
        fs::write(root.join("unpaid.rs"), "fn unpaid() {}\n").unwrap();
        let app = App::new(root, (150, 40));
        (dir, app)
    }

    fn screen(app: &App) -> Grid {
        let mut grid = Grid::new(app.size().0, app.size().1, app.background());
        app.draw(&mut grid);
        grid
    }

    /// Waits for background reads until the screen shows `text`, and gives the screen.
    fn wait_for(app: &mut App, text: &str) -> String {
        let started = Instant::now();
        loop {
            app.poll();
            let shown = screen(app).text();
            if shown.contains(text) {
                return shown;
            }
            assert!(
                started.elapsed() < BACKGROUND,
                "never showed {text:?}:\n{shown}"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// Where `text` starts on the screen.
    fn spot(app: &App, text: &str) -> (u16, u16) {
        let grid = screen(app);
        for y in 0..grid.height() {
            let row = grid.row_text(y);
            if let Some(at) = row.find(text) {
                return (row[..at].chars().count() as u16, y);
            }
        }
        panic!("{text:?} isn't on screen:\n{}", grid.text());
    }

    fn click(app: &mut App, kind: MouseKind, (x, y): (u16, u16)) {
        app.handle(Input::Mouse(MouseEvent {
            kind,
            x: x + 1,
            y,
            fine_x: x as f32 + 1.0,
            fine_y: y as f32 + 0.5,
            mods: Mods::default(),
        }));
    }

    fn click_on(app: &mut App, kind: MouseKind, text: &str) {
        let at = spot(app, text);
        click(app, kind, at);
    }

    fn press(app: &mut App, key: Key) {
        app.handle(Input::Key(KeyEvent {
            key,
            mods: Mods::default(),
        }));
    }

    #[test]
    fn the_task_page_shows_a_plan_its_versions_turns_and_tests() {
        let (_dir, mut app) = worked_on();
        app.run(Command::ShowAgentTask);
        let text = wait_for(&mut app, "1 file +1 -0");
        assert!(text.contains("Add a refund route"), "{text}");
        assert!(text.contains("Plan v1  sent back"), "{text}");
        assert!(text.contains("Plan v2  approved"), "{text}");
        assert!(text.contains("tests passed"), "{text}");
        assert!(text.contains("2 turns  ·  2 test runs"), "{text}");
        // The latest step is chosen, with what it changed so far.
        assert!(text.contains("Turn 2"), "{text}");
        assert!(text.contains("unpaid.rs"), "{text}");

        // A plan version shows what was said about it and its own text.
        click_on(&mut app, MouseKind::Left, "Plan v1  sent back");
        let text = screen(&app).text();
        assert!(
            text.contains("You ") && text.contains("Only paid orders"),
            "{text}"
        );
        assert!(text.contains("Refund any order."), "{text}");
        press(&mut app, Key::Down);
        assert!(screen(&app).text().contains("Refund paid orders only."));

        // The first turn's tests, newest last, each opening its output.
        click_on(&mut app, MouseKind::Left, "1  Add a refund route");
        let text = screen(&app).text();
        assert!(text.contains("TESTS  2 runs"), "{text}");
        click_on(&mut app, MouseKind::Left, "✗ cargo test");
        assert!(screen(&app).text().contains("test refund ... FAILED"));
        // The output stands in front of the page; the palette brings the page back as it was.
        app.run(Command::ShowAgentTask);
        assert!(screen(&app).text().contains("TESTS  2 runs"));
    }

    #[test]
    fn a_turn_is_reviewed_alone_and_undone_from_the_task_page() {
        let (dir, mut app) = worked_on();
        let root = dir.path().canonicalize().unwrap();
        app.run(Command::ShowAgentTask);
        wait_for(&mut app, "1 file +1 -0");
        click_on(&mut app, MouseKind::Left, "1  Add a refund route");
        click_on(&mut app, MouseKind::Left, "[Review Turn]");
        let text = screen(&app).text();
        assert!(text.contains("in the turn"), "{text}");
        assert!(text.contains("orders.rs"), "{text}");
        assert!(
            !text.contains("unpaid.rs"),
            "only what that turn changed:\n{text}"
        );
        assert!(
            !text.contains("[Accept]"),
            "a past turn is only to read:\n{text}"
        );

        press(&mut app, Key::Esc);
        assert!(screen(&app).text().contains("Turn 1"), "back on the page");
        click_on(&mut app, MouseKind::Left, "[Undo From Here]");
        let text = screen(&app).text();
        assert!(text.contains("Undo turn 1 and the one after it?"), "{text}");
        press(&mut app, Key::Enter);
        assert_eq!(
            fs::read_to_string(root.join("orders.rs")).unwrap(),
            "fn refund() {}\n"
        );
        assert!(!root.join("unpaid.rs").exists());
    }

    #[test]
    fn a_task_can_be_split_at_a_turn_and_renamed() {
        let (dir, mut app) = worked_on();
        let root = dir.path().canonicalize().unwrap();
        app.run(Command::ShowAgentTask);
        wait_for(&mut app, "1 file +1 -0");
        click_on(&mut app, MouseKind::Right, "2  Reject a refund");
        click_on(&mut app, MouseKind::Left, "Start a New Task Here");
        let text = screen(&app).text();
        assert!(
            text.contains("Reject a refund on an unpaid order ▾"),
            "{text}"
        );
        assert!(
            !text.contains("Plan v1"),
            "the plan stays with the first task:\n{text}"
        );

        // The arrow lists the tasks here, to go back to the first.
        let (x, y) = spot(&app, "unpaid order ▾");
        click(
            &mut app,
            MouseKind::Left,
            (x + "unpaid order ".len() as u16, y),
        );
        click_on(&mut app, MouseKind::Left, "Add a refund route  ·");
        assert!(screen(&app).text().contains("Plan v1"));

        click_on(&mut app, MouseKind::Right, "Plan v1");
        click_on(&mut app, MouseKind::Left, "Rename Task…");
        for _ in 0.."Add a refund route".len() {
            press(&mut app, Key::Backspace);
        }
        for c in "Refunds".chars() {
            press(&mut app, Key::Char(c));
        }
        press(&mut app, Key::Enter);
        assert!(screen(&app).text().contains("Refunds ▾"));
        // Both are kept in the log, so they hold the next time.
        let log = tasks::load(&root);
        assert!(log.iter().any(|e| matches!(e, Event::Split { .. })));
        assert!(
            log.iter()
                .any(|e| matches!(e, Event::Rename { name, .. } if name == "Refunds"))
        );
    }

    #[test]
    fn with_no_agent_work_the_page_says_how_a_task_starts() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = App::new(dir.path().to_path_buf(), (150, 40));
        app.run(Command::ShowAgentTask);
        let text = screen(&app).text();
        assert!(text.contains("No agent work in this folder yet"), "{text}");
        assert!(text.contains("first prompt to an agent"), "{text}");
    }
}
