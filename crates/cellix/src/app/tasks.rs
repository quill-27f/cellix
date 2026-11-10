//! Run Task: a task runs in a terminal tab of the side pane named for it, and once it ends,
//! the errors it printed join the Problems list beside the language servers' own.

use std::collections::HashMap;
use std::path::PathBuf;

use cellix_lang::lsp::Diagnostic;

use super::{App, Overlay};
use crate::palette::{Item, Palette};
use crate::tasks::{self, Task};

/// What the last line of a task's tab says, which is also how its exit code is read back,
/// since a task can run under a host that only says it ended.
const EXITED: &str = "exited with code ";

/// Tasks run and running, and what they found wrong.
#[derive(Default)]
pub struct State {
    /// Each task started, by its label, and whether its end was taken in.
    runs: Vec<(Task, bool)>,
    /// Labels run, newest first, for the top of the picker.
    pub recently: Vec<String>,
    /// The problems each task printed, by its label.
    problems: Vec<(String, PathBuf, Diagnostic)>,
}

impl App {
    /// Every task in the workspace, as the picker offers them.
    pub(super) fn found_tasks(&self) -> Vec<Task> {
        tasks::find(self.workspace.root())
    }

    /// Run Task: the picker.
    pub(super) fn pick_task(&mut self) {
        let found = self.found_tasks();
        if found.is_empty() {
            self.say(
                "No tasks here: they come from package.json, Cargo.toml, a Makefile, a \
                 justfile, go.mod, pytest or .vscode/tasks.json",
            );
            return;
        }
        let items = self.task_items(found);
        self.overlay = Overlay::Palette(Palette::asking(items, "Select the task to run"));
    }

    /// The picker's rows: what ran lately first, then each source's tasks under its name.
    fn task_items(&self, mut found: Vec<Task>) -> Vec<Item> {
        // Ctrl+Shift+B's task says so, wherever it's listed.
        let default = tasks::default_build(&found).map(|t| t.label.clone());
        for task in &mut found {
            task.default_build = Some(&task.label) == default.as_ref();
        }
        let recently = &self.workflow.tasks.recently;
        let ran: Vec<&Task> = recently
            .iter()
            .filter_map(|label| found.iter().find(|t| &t.label == label))
            .collect();
        let mut items = Vec::new();
        if !ran.is_empty() {
            items.push(Item::Header("RECENTLY RUN".into()));
            items.extend(ran.iter().map(|t| Item::Task((*t).clone())));
        }
        let mut group = None;
        for task in &found {
            if ran.iter().any(|t| t.label == task.label) {
                continue;
            }
            let name = self.group_name(task);
            if group.as_ref() != Some(&name) {
                items.push(Item::Header(name.clone()));
                group = Some(name);
            }
            items.push(Item::Task(task.clone()));
        }
        items
    }

    /// A source's name over its group, with the folder when it isn't the workspace's own.
    fn group_name(&self, task: &Task) -> String {
        let name = task.source.name().to_uppercase();
        match task.folder.strip_prefix(self.workspace.root()) {
            Ok(inside)
                if !inside.as_os_str().is_empty() && task.source != tasks::Source::Configured =>
            {
                format!("{name} · {}", inside.display())
            }
            _ => name,
        }
    }

    /// Run Build Task, Ctrl+Shift+B: the default build task, or a choice of build tasks.
    pub(super) fn run_build_task(&mut self) {
        let found = self.found_tasks();
        if let Some(task) = tasks::default_build(&found) {
            let task = task.clone();
            self.run_task(task);
            return;
        }
        let builds: Vec<Item> = found
            .into_iter()
            .filter(|t| t.build)
            .map(Item::Task)
            .collect();
        if builds.is_empty() {
            self.pick_task();
            return;
        }
        self.overlay = Overlay::Palette(Palette::asking(builds, "Select the build task to run"));
    }

    /// Rerun Last Task.
    pub(super) fn rerun_last_task(&mut self) {
        let last = self
            .workflow
            .tasks
            .runs
            .last()
            .map(|(task, _)| task.clone());
        match last {
            Some(task) => self.run_task(task),
            None => self.pick_task(),
        }
    }

    /// Runs a task in a terminal tab named for it. Running it again replaces its tab.
    pub(super) fn run_task(&mut self, task: Task) {
        if let Some(index) = self.dock.sessions.iter().position(|s| s.name == task.label) {
            self.dock.close(index);
        }
        let argv = task_argv(&task);
        self.start_session(&task.label, &argv, &[], &task.folder);
        let Some(session) = self
            .dock
            .sessions
            .last_mut()
            .filter(|s| s.name == task.label)
        else {
            return;
        };
        session.printed = Some(Vec::new());
        let state = &mut self.workflow.tasks;
        state.runs.retain(|(t, _)| t.label != task.label);
        state.recently.retain(|l| *l != task.label);
        state.recently.insert(0, task.label.clone());
        state.problems.retain(|(label, ..)| *label != task.label);
        state.runs.push((task, false));
        // The keys stay where they were: a task is watched, not typed into.
        self.dock.focused = false;
    }

    /// Takes in tasks that ended: their problems, and a word on how they went.
    pub(super) fn watch_tasks(&mut self) {
        let mut ended = Vec::new();
        for (task, taken) in &mut self.workflow.tasks.runs {
            if *taken {
                continue;
            }
            let session = self.dock.sessions.iter().find(|s| s.name == task.label);
            match session {
                Some(session) if session.exited => {
                    *taken = true;
                    let said = session.printed.as_deref().unwrap_or_default();
                    ended.push((task.clone(), String::from_utf8_lossy(said).into_owned()));
                }
                // Closed before it ended: nothing to read.
                None => *taken = true,
                Some(_) => {}
            }
        }
        for (task, said) in ended {
            self.task_ended(&task, &said);
        }
    }

    fn task_ended(&mut self, task: &Task, said: &str) {
        let found = tasks::problems(said, &task.folder);
        let code = said
            .rfind(EXITED)
            .and_then(|at| said[at + EXITED.len()..].split_whitespace().next())
            .and_then(|n| n.parse::<i32>().ok());
        let count = found.len();
        for problem in found {
            let diagnostic = Diagnostic {
                line: problem.line.saturating_sub(1),
                column: problem.column.saturating_sub(1),
                message: problem.message,
                severity: problem.severity,
            };
            let row = (task.label.clone(), problem.file, diagnostic);
            self.workflow.tasks.problems.push(row);
        }
        let label = &task.label;
        let problems = match count {
            1 => "1 problem".to_string(),
            n => format!("{n} problems"),
        };
        let note = match code {
            Some(0) if count == 0 => format!("{label} finished"),
            Some(0) => format!("{label} finished, with {problems} in Problems"),
            _ if count > 0 => format!("{label} failed, with {problems} in Problems"),
            Some(code) => format!("{label} failed with code {code}"),
            None => format!("{label} ended"),
        };
        if count > 0 {
            // Problems gets a tab beside the task's, which stays in front.
            self.dock.problems = true;
            self.dock.problems_top = 0;
        }
        self.say(note);
    }

    /// Every problem known, from the language servers and from tasks, by file.
    pub(super) fn all_problems(&self) -> HashMap<PathBuf, Vec<Diagnostic>> {
        let mut all = self.diagnostics.clone();
        for (_, file, problem) in &self.workflow.tasks.problems {
            let list = all.entry(file.clone()).or_default();
            // A server and a build often say the same thing, quoted or not: once is enough.
            let plain = |m: &str| m.replace(['\'', '`', '"'], "").to_lowercase();
            let same = |d: &Diagnostic| {
                d.line == problem.line && plain(&d.message) == plain(&problem.message)
            };
            if !list.iter().any(same) {
                list.push(problem.clone());
            }
        }
        all
    }

    /// Whether the Problems tab is in front, for tests.
    #[cfg(test)]
    pub(super) fn problems_in_front(&self) -> bool {
        self.dock.front == super::Front::Problems
    }
}

/// What runs a task: the shell with its command, then a line saying how it ended.
fn task_argv(task: &Task) -> Vec<String> {
    if cfg!(windows) {
        return vec!["cmd".into(), "/C".into(), task.command.clone()];
    }
    let label = task.label.replace('\'', r"'\''");
    let command = task.command.replace('\'', r"'\''");
    // What runs is said first, dimmed, as VS Code's "Executing task".
    let script = format!(
        "printf '\\033[2m> %s\\033[0m\\n\\n' '{command}'\n{}\ncode=$?\nprintf '\\n\\033[2m%s {EXITED}%s\\033[0m\\n' '{label}' \"$code\"\nexit $code",
        task.command
    );
    vec!["sh".into(), "-c".into(), script]
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::{Duration, Instant};

    use cellix_input::{Command, Event};

    use super::*;

    fn wait_for(app: &mut App, done: impl Fn(&App) -> bool) {
        let start = Instant::now();
        while !done(app) {
            assert!(start.elapsed() < Duration::from_secs(30), "timed out");
            std::thread::sleep(Duration::from_millis(20));
            app.poll();
            app.handle(Event::Tick);
        }
    }

    fn screen(app: &App) -> cellix_render::Grid {
        let mut grid = cellix_render::Grid::new(app.size().0, app.size().1, app.background());
        app.draw(&mut grid);
        grid
    }

    #[test]
    fn a_task_runs_in_its_own_tab_and_its_errors_join_problems() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("src")).unwrap();
        fs::write(dir.path().join("src/main.c"), "int main() { return x; }\n").unwrap();
        // A stand-in compiler that prints as gcc does, and fails.
        fs::write(
            dir.path().join("Makefile"),
            "build:\n\t@echo \"src/main.c:1:21: error: 'x' undeclared\"; exit 2\n",
        )
        .unwrap();
        let mut app = App::new(dir.path().to_path_buf(), (120, 36));
        app.run(Command::RunTask);
        let text = screen(&app).text();
        assert!(text.contains("make build"), "the picker lists it");
        app.handle(Event::Key(cellix_input::KeyEvent::plain(
            cellix_input::Key::Enter,
        )));
        assert!(app.dock.sessions.iter().any(|s| s.name == "make build"));
        // A real build takes longer than the few seconds in which an agent that ends is
        // taken to have failed to start; its tab must stay all the same.
        let task = app
            .dock
            .sessions
            .iter_mut()
            .find(|s| s.name == "make build");
        let born = &mut task.unwrap().born;
        *born = born.checked_sub(Duration::from_secs(60)).unwrap();
        wait_for(&mut app, |app| !app.workflow.tasks.problems.is_empty());
        assert!(
            app.dock.sessions.iter().any(|s| s.name == "make build"),
            "the task's tab stays once it ends"
        );
        let rows = app.all_problems();
        let file = dir.path().join("src/main.c");
        assert_eq!(rows[&file][0].line, 0);
        assert_eq!(rows[&file][0].message, "'x' undeclared");
        let text = screen(&app).text();
        assert!(text.contains("make build failed, with 1 problem in Problems"));
        assert!(!app.problems_in_front(), "the task's tab stays in front");
        // Running it again replaces its tab rather than adding one, and Ctrl+Shift+B finds it
        // as the only build task.
        app.run(Command::RunBuildTask);
        wait_for(&mut app, |app| {
            app.workflow.tasks.runs.iter().all(|(_, taken)| *taken)
        });
        let named = app.dock.sessions.iter().filter(|s| s.name == "make build");
        assert_eq!(named.count(), 1);
        assert_eq!(app.all_problems()[&file].len(), 1, "not twice");
    }
}
