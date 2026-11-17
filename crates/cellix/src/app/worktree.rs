//! A worktree for an agent: a branch checked out in a folder beside the project, opened as a
//! workspace of its own, with an agent started in it; and taking one away again. Kept apart
//! from `app.rs`, which reaches here through a handful of calls.

use super::*;

use crate::agents::AGENTS;

/// The dialog's measures: its width in columns and its rows (an edge, the title, the box,
/// where it goes, which agent, a gap, the buttons, an edge). A row of air before the agents
/// as well left four pieces floating, the inner gaps as wide as the edges.
const WIDTH: u16 = 62;
const HEIGHT: u16 = 8;
/// The row of each part, from the dialog's top.
const TITLE_ROW: u16 = 1;
const FIELD_ROW: u16 = 2;
const NOTE_ROW: u16 = 3;
const AGENT_ROW: u16 = 4;
const BUTTON_ROW: u16 = 6;
/// What the last choice of agent says: start nothing.
const NOTHING: &str = "Nothing";
const BUTTONS: [&str; 2] = ["Cancel", "Create"];

/// New Worktree, as it's being filled in.
pub(super) struct NewWorktree {
    /// The branch's name, and the caret in it.
    pub branch: String,
    pub caret: usize,
    /// Where it starts from: the branch in view, and its commit, short.
    from: (String, String),
    /// The project's own checkout, beside which the folder goes.
    main: PathBuf,
    /// The agents that can be started in it, by program; the one after the last is
    /// Nothing.
    agents: Vec<&'static str>,
    pub picked: usize,
    /// What went wrong the last time Create was pressed.
    error: Option<String>,
}

/// What's asked before a worktree is taken away.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum WorktreeChoice {
    /// Take it away, if git finds nothing in it that would be lost.
    Remove(PathBuf),
    /// Take it away, changes and all: asked a second time.
    RemoveAnyway(PathBuf),
}

/// Where a worktree of `branch` goes: in a folder beside the project's own checkout, named
/// after it, `queue.worktrees/fix-pop-order` beside `queue`. A `/` in the branch's name is a
/// `-` in the folder's, so each is one folder there.
pub(super) fn folder_for(main: &Path, branch: &str) -> PathBuf {
    let name = main
        .file_name()
        .map_or_else(|| "project".into(), |n| n.to_string_lossy().into_owned());
    let slug: String = branch
        .trim()
        .chars()
        .map(|c| if c == '/' || c == '\\' { '-' } else { c })
        .collect();
    main.with_file_name(format!("{name}.worktrees")).join(slug)
}

impl NewWorktree {
    /// Where it goes, as the dialog says it: from the folder the project is in.
    fn place(&self) -> String {
        let folder = folder_for(&self.main, &self.branch);
        let parent = self.main.parent().unwrap_or(&self.main);
        let shown = folder.strip_prefix(parent).unwrap_or(&folder);
        shown.display().to_string()
    }

    fn choices(&self) -> Vec<&'static str> {
        let mut names: Vec<&'static str> = self
            .agents
            .iter()
            .copied()
            .map(crate::agents::display_name)
            .collect();
        names.push(NOTHING);
        names
    }

    /// The agent to start in it, by program.
    pub fn agent(&self) -> Option<&'static str> {
        self.agents.get(self.picked).copied()
    }
}

impl App {
    /// Asks for a branch's name and the agent to start in it.
    pub(super) fn ask_new_worktree(&mut self) {
        let root = self.workspace.root().to_path_buf();
        let Some(main) = main_checkout(&root) else {
            self.say("A worktree needs a git repository");
            return;
        };
        self.agents.look_for_agents();
        let installed = self.agents.installed_programs();
        let agents: Vec<&'static str> = AGENTS
            .iter()
            .map(|(_, program)| *program)
            .filter(|program| installed.contains(program))
            .collect();
        let branch = cellix_review::current_branch(&root).unwrap_or_else(|| "HEAD".into());
        let commit = cellix_review::head(&root).unwrap_or_default();
        let short = commit.chars().take(7).collect();
        self.overlay = Overlay::NewWorktree(NewWorktree {
            branch: String::new(),
            caret: 0,
            from: (branch, short),
            main,
            agents,
            picked: 0,
            error: None,
        });
    }

    /// Makes the worktree, opens it as a workspace and starts the agent picked there. What
    /// goes wrong is said under the box, which stays.
    fn create_worktree(&mut self, mut form: NewWorktree) {
        let branch = form.branch.trim().to_string();
        let root = self.workspace.root().to_path_buf();
        let folder = folder_for(&form.main, &branch);
        let refused = if branch.is_empty() {
            Some("Name the branch".to_string())
        } else if !cellix_review::valid_branch(&root, &branch) {
            Some(format!("{branch} can't be a branch's name"))
        } else if folder.exists() {
            Some(format!("{} is already there", form.place()))
        } else {
            None
        };
        let made = match refused {
            Some(why) => Err(why),
            None => {
                if let Some(parent) = folder.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                cellix_review::add_worktree(&root, &branch, &folder)
            }
        };
        if let Err(why) = made {
            form.error = Some(why);
            self.overlay = Overlay::NewWorktree(form);
            return;
        }
        self.refresh_agents();
        self.say(format!("Made a worktree of {branch} beside the project"));
        self.workspace_request = Some(match form.agent() {
            Some(program) => WorkspaceRequest::OpenAndStart(folder, program),
            None => WorkspaceRequest::Open(folder),
        });
    }

    pub(super) fn new_worktree_key(&mut self, mut form: NewWorktree, key: KeyEvent) {
        let count = form.agents.len() + 1;
        match key.key {
            Key::Esc => {}
            Key::Enter => self.create_worktree(form),
            // The box has the keyboard; Tab moves along the agents to start in it.
            Key::Tab | Key::BackTab => {
                let step = if key.key == Key::Tab { 1 } else { count - 1 };
                form.picked = (form.picked + step) % count;
                self.overlay = Overlay::NewWorktree(form);
            }
            _ => {
                if crate::field::key(&mut form.branch, &mut form.caret, key) {
                    form.error = None;
                }
                self.overlay = Overlay::NewWorktree(form);
            }
        }
    }

    pub(super) fn new_worktree_paste(&mut self, text: &str) {
        if let Overlay::NewWorktree(form) = &mut self.overlay {
            let line = text.lines().next().unwrap_or_default().trim();
            crate::field::insert(&mut form.branch, &mut form.caret, line);
            form.error = None;
        }
    }

    pub(super) fn new_worktree_area(&self) -> Rect {
        let screen = Rect::new(0, 0, self.size.0, self.size.1);
        let width = WIDTH.min(screen.w);
        let y = (screen.h / 4).min(screen.h.saturating_sub(HEIGHT));
        Rect::new(screen.x + (screen.w - width) / 2, y, width, HEIGHT)
    }

    /// The choices of agent, each as the columns it takes on its row, at the dialog's right:
    /// all as wide as the widest, as Settings' are.
    fn worktree_segments(&self, form: &NewWorktree) -> Vec<(Rect, &'static str)> {
        let area = self.new_worktree_area();
        let choices = form.choices();
        let cells = |c: &&str| measures::button_cells(c, &*self.measure).chars().count() as u16;
        let width = choices.iter().map(cells).max().unwrap_or(0);
        let count = choices.len() as u16;
        let start = area.right().saturating_sub(2 + width * count);
        let y = area.y + AGENT_ROW;
        let place = |(i, name): (usize, &'static str)| {
            (Rect::new(start + width * i as u16, y, width, 1), name)
        };
        choices.into_iter().enumerate().map(place).collect()
    }

    /// Cancel and Create, each as (cells, x), from the dialog's right edge.
    fn worktree_buttons(&self) -> Vec<(String, u16)> {
        let area = self.new_worktree_area();
        let mut x = area.right().saturating_sub(2);
        let mut placed = Vec::new();
        for label in BUTTONS.iter().rev() {
            let cells = measures::button_cells(label, &*self.measure);
            x = x.saturating_sub(cells.chars().count() as u16);
            placed.push((cells, x));
            x = x.saturating_sub(BUTTON_GAP);
        }
        placed.reverse();
        placed
    }

    pub(super) fn new_worktree_click(&mut self, mut form: NewWorktree, at: (f32, f32)) {
        let area = self.new_worktree_area();
        let y = area.y + BUTTON_ROW;
        let width = |cells: &String| cells.chars().count() as u16;
        let buttons = self.worktree_buttons();
        let pressed = buttons
            .iter()
            .position(|(c, x)| on_large_button((*x, y), width(c), at));
        let at = (at.0 as u16, at.1 as u16);
        let segment = self
            .worktree_segments(&form)
            .iter()
            .position(|(rect, _)| rect.contains(at.0, at.1));
        match (pressed, segment) {
            (Some(0), _) => {}
            (Some(_), _) => self.create_worktree(form),
            (None, Some(index)) => {
                form.picked = index;
                self.overlay = Overlay::NewWorktree(form);
            }
            // A press elsewhere in it leaves it be, and one outside it leaves, as Escape does.
            (None, None) if area.contains(at.0, at.1) => {
                self.overlay = Overlay::NewWorktree(form);
            }
            (None, None) => {}
        }
    }

    pub(super) fn draw_new_worktree(&self, grid: &mut Grid, form: &NewWorktree) {
        let theme = &self.theme;
        let area = self.new_worktree_area();
        popup(grid, area, theme);
        let base = Style {
            fg: theme.text.fg,
            bg: theme.popup,
            ..theme.bar
        };
        let strong = Style {
            bold: true,
            fg: theme.heading,
            ..base
        };
        let dim = Style {
            fg: theme.popup_dim,
            ..base
        };
        let (left, right) = (area.x + 2, area.right().saturating_sub(2));
        let title = "New Worktree";
        let end = grid.label(left, area.y + TITLE_ROW, title, strong, right);
        let (branch, commit) = &form.from;
        let from = format!("from {branch} at {commit}");
        // A column after the title as it's painted, and after its cells where it isn't.
        let bold = Font {
            bold: true,
            ..Font::default()
        };
        let after = f32::from(left) + self.measure.width(title, bold) + 1.0;
        grid.label_at(after, end + 1, area.y + TITLE_ROW, &from, dim, right, 90);
        let line = Rect::new(left, area.y + FIELD_ROW, right - left, 1);
        let field = Input {
            text: &form.branch,
            hint: "Branch name",
            focused: true,
            behind: theme.popup,
        };
        input(grid, line, &field, &*self.measure, theme);
        let note = match &form.error {
            Some(error) => (
                error.clone(),
                Style {
                    fg: theme.error,
                    ..base
                },
            ),
            None if form.branch.trim().is_empty() => (
                "A new branch from here, in a folder beside the project".to_string(),
                dim,
            ),
            None => (format!("Beside the project, in {}", form.place()), dim),
        };
        grid.label_at(
            left as f32,
            left,
            area.y + NOTE_ROW,
            &note.0,
            note.1,
            right,
            90,
        );
        grid.label(left, area.y + AGENT_ROW, "Start in it", base, right);
        for (index, (rect, name)) in self.worktree_segments(form).into_iter().enumerate() {
            crate::settings_page::draw_segment(grid, rect, name, index == form.picked, theme);
        }
        let y = area.y + BUTTON_ROW;
        for (i, (cells, x)) in self.worktree_buttons().iter().enumerate() {
            let primary = i == 1;
            let width = cells.chars().count() as u16;
            let hovered = self.over_button((*x, y), width);
            let quiet = theme.quiet_button(theme.popup);
            let fill = match (primary, hovered) {
                (true, _) => theme.button,
                (false, true) => theme.quiet_button(quiet),
                (false, false) => quiet,
            };
            let style = Style {
                fg: if primary {
                    theme.button_text
                } else {
                    theme.text.fg
                },
                bg: fill,
                ..base
            };
            grid.large_button(*x, y, cells, BUTTONS[i], fill, style);
        }
    }

    /// The menu on a worktree's row in the Agents list.
    pub(super) fn open_worktree_menu(&mut self, path: PathBuf, at: (u16, u16)) {
        let root = self.workspace.root();
        let main = main_checkout(root);
        // The project's own checkout, and the one open here, aren't to be taken away from
        // under it.
        let removable = main.as_deref() != Some(path.as_path()) && path != root;
        let items = vec![
            MenuItem::new("Review Changes", MenuAction::ReviewWorktree(path.clone())),
            MenuItem::new("Remove Worktree…", MenuAction::RemoveWorktree(path))
                .after_rule()
                .when(removable),
        ];
        self.show_menu(at, items);
    }

    /// Asks before taking a worktree away. Its branch stays whatever's answered.
    pub(super) fn ask_remove_worktree(&mut self, path: PathBuf) {
        let name = worktree_name(&path);
        self.overlay = Overlay::Confirm(Confirm {
            title: format!("Remove the worktree {name}?"),
            detail: "Its folder goes. Its branch stays, with every commit on it.".to_string(),
            buttons: vec![
                ("Cancel", Choice::Cancel),
                ("Remove", Choice::Worktree(WorktreeChoice::Remove(path))),
            ],
            selected: 0,
        });
    }

    /// Tests start a stand-in for an agent's program, never the real one.
    #[cfg(test)]
    pub(crate) fn stand_in_for_agents(&mut self, program: &str) {
        self.agent_program = Some(program.to_string());
    }

    pub(super) fn choose_worktree(&mut self, choice: WorktreeChoice) {
        let root = self.workspace.root().to_path_buf();
        let (path, force) = match choice {
            WorktreeChoice::Remove(path) => (path, false),
            WorktreeChoice::RemoveAnyway(path) => (path, true),
        };
        let name = worktree_name(&path);
        // Changes not committed there would go with it: asked again, and only then forced.
        if !force && !cellix_review::changed_files(&path).is_empty() {
            self.overlay = Overlay::Confirm(Confirm {
                title: format!("{name} has changes that aren't committed"),
                detail: "Removing it loses them. Its branch stays.".to_string(),
                buttons: vec![
                    ("Cancel", Choice::Cancel),
                    (
                        "Remove Anyway",
                        Choice::Worktree(WorktreeChoice::RemoveAnyway(path)),
                    ),
                ],
                selected: 0,
            });
            return;
        }
        match cellix_review::remove_worktree(&root, &path, force) {
            Ok(()) => self.say(format!("Removed the worktree {name}; its branch stays")),
            Err(e) => self.say(format!("Could not remove {name}: {e}")),
        }
        self.refresh_agents();
    }
}

/// The project's own checkout, as the disk names it.
fn main_checkout(root: &Path) -> Option<PathBuf> {
    let main = cellix_review::main_checkout(root)?;
    Some(dunce::canonicalize(&main).unwrap_or(main))
}

/// What a worktree is called in what's said of it: its folder's name.
fn worktree_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use cellix_input::{Mods, MouseEvent};
    use std::fs;
    use std::process::Command as Git;

    fn screen(app: &App) -> Grid {
        let mut grid = Grid::new(app.size().0, app.size().1, app.background());
        app.draw(&mut grid);
        grid
    }

    fn git(root: &Path, args: &[&str]) -> String {
        let done = Git::new("git")
            .arg("-C")
            .arg(root)
            .args(["-c", "user.name=t", "-c", "user.email=t@example.com"])
            .args(args)
            .output()
            .unwrap();
        assert!(
            done.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&done.stderr)
        );
        String::from_utf8_lossy(&done.stdout).trim().to_string()
    }

    /// A repository called `queue` with one commit on `main`, in a folder of its own, so
    /// what's made beside it is in the temporary folder too.
    fn repository() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("queue");
        fs::create_dir(&root).unwrap();
        fs::write(root.join("queue.rs"), "pub struct Queue;\n").unwrap();
        git(&root, &["init", "-q", "-b", "main"]);
        git(&root, &["add", "-A"]);
        git(&root, &["commit", "-qm", "First"]);
        (dir, dunce::canonicalize(&root).unwrap())
    }

    fn typed(app: &mut App, text: &str) {
        for c in text.chars() {
            app.handle(Event::Key(KeyEvent::plain(Key::Char(c))));
        }
    }

    fn enter(app: &mut App) {
        app.handle(Event::Key(KeyEvent::plain(Key::Enter)));
    }

    /// Cellix listed worktrees and couldn't make one. New Worktree asks for a branch, says
    /// where the folder goes, offers the installed agents to start in it, and makes the
    /// worktree from the HEAD in view beside the project; the folder opens as a workspace
    /// with the agent picked.
    #[test]
    fn a_new_worktree_is_made_beside_the_project_and_opened_with_its_agent() {
        let (dir, root) = repository();
        let mut app = App::new(root.clone(), (150, 40));
        app.agents.pretend_installed(&["codex", "gemini"]);
        assert!(
            app.can_run(Command::NewWorktree),
            "in the palette in a repository"
        );
        // The + in the side pane offers it too, last of what it starts.
        app.run(Command::ListAgents);
        let title = app.title_of(app.dock_area().expect("the pane is open"));
        let plus = app.dock.plus_x(title) + 1;
        for kind in [MouseKind::Left, MouseKind::Up] {
            app.handle(Event::Mouse(MouseEvent {
                kind,
                x: plus,
                y: title.y,
                fine_x: f32::from(plus),
                fine_y: f32::from(title.y) + 0.5,
                mods: Mods::default(),
            }));
        }
        let Overlay::Menu(menu) = &mut app.overlay else {
            panic!("no menu from the +");
        };
        menu.step(-1);
        assert_eq!(menu.selected_label(), Some("New Worktree…"), "the last row");
        enter(&mut app);
        let head = git(&root, &["rev-parse", "--short=7", "HEAD"]);
        let text = screen(&app).text();
        assert!(text.contains("New Worktree"), "{text}");
        assert!(text.contains(&format!("from main at {head}")), "{text}");
        for choice in ["Codex", "Gemini CLI", "Nothing"] {
            assert!(text.contains(choice), "{choice} offered: {text}");
        }
        assert!(
            !text.contains("Claude Code"),
            "only what's installed: {text}"
        );
        typed(&mut app, "fix/pop-order");
        let text = screen(&app).text();
        assert!(
            text.contains("in queue.worktrees/fix-pop-order"),
            "where it goes: {text}"
        );
        // Tab moves along the agents: Gemini CLI.
        app.handle(Event::Key(KeyEvent::plain(Key::Tab)));
        enter(&mut app);
        assert!(
            matches!(app.overlay, Overlay::None),
            "{}",
            screen(&app).text()
        );
        let folder = dir.path().join("queue.worktrees/fix-pop-order");
        let folder = dunce::canonicalize(&folder).expect("the folder is made beside the project");
        assert_eq!(git(&folder, &["branch", "--show-current"]), "fix/pop-order");
        assert_eq!(
            git(&folder, &["rev-parse", "--short=7", "HEAD"]),
            head,
            "from HEAD"
        );
        assert!(git(&root, &["worktree", "list"]).contains("fix-pop-order"));
        assert_eq!(
            app.workspace_request,
            Some(WorkspaceRequest::OpenAndStart(folder.clone(), "gemini")),
            "opened as a workspace, with the agent picked"
        );
        // The same name again is refused under the box, which stays.
        app.workspace_request = None;
        app.run(Command::NewWorktree);
        typed(&mut app, "fix/pop-order");
        enter(&mut app);
        assert!(matches!(app.overlay, Overlay::NewWorktree(_)), "it stays");
        assert!(screen(&app).text().contains("is already there"));
        // Nothing picked, it opens with nothing started; a branch that's there is checked out.
        app.handle(Event::Key(KeyEvent::plain(Key::Esc)));
        git(&root, &["branch", "kept"]);
        app.run(Command::NewWorktree);
        typed(&mut app, "kept");
        app.handle(Event::Key(KeyEvent::plain(Key::BackTab)));
        enter(&mut app);
        let kept = dunce::canonicalize(dir.path().join("queue.worktrees/kept")).unwrap();
        assert_eq!(
            app.workspace_request,
            Some(WorkspaceRequest::Open(kept.clone()))
        );
        assert_eq!(git(&kept, &["branch", "--show-current"]), "kept");
    }

    /// A worktree's row in the Agents list takes it away on a right click: asked first,
    /// asked again where it has changes not committed, and its branch is never deleted.
    #[test]
    fn a_worktree_is_removed_after_asking_and_keeps_its_branch() {
        let (dir, root) = repository();
        let clean = dir.path().join("queue.worktrees/clean");
        let dirty = dir.path().join("queue.worktrees/dirty");
        for (path, branch) in [(&clean, "clean"), (&dirty, "dirty")] {
            git(
                &root,
                &[
                    "worktree",
                    "add",
                    "-q",
                    "-b",
                    branch,
                    &path.display().to_string(),
                ],
            );
        }
        fs::write(dirty.join("queue.rs"), "pub struct Changed;\n").unwrap();
        let mut app = App::new(root.clone(), (150, 40));
        app.run(Command::ListAgents);
        // A right click on the words in the pane, under its tabs.
        let right = |app: &mut App, words: &str| {
            let grid = screen(app);
            let pane = app.dock_area().expect("the pane is open");
            let (x, y) = (pane.y + 1..grid.height())
                .find_map(|y| {
                    let row: String = grid.row_text(y).chars().skip(pane.x as usize).collect();
                    let at = row.find(words)?;
                    Some((pane.x + row[..at].chars().count() as u16, y))
                })
                .unwrap_or_else(|| panic!("no {words}: {}", grid.text()));
            app.handle(Event::Mouse(MouseEvent {
                kind: MouseKind::Right,
                x,
                y,
                fine_x: f32::from(x),
                fine_y: f32::from(y) + 0.5,
                mods: Mods::default(),
            }));
        };
        let choose = |app: &mut App, label: &str| {
            let Overlay::Menu(menu) = &mut app.overlay else {
                panic!("no menu");
            };
            while menu.selected_label() != Some(label) {
                menu.step(1);
            }
            app.handle(Event::Key(KeyEvent::plain(Key::Enter)));
        };
        right(&mut app, "clean");
        choose(&mut app, "Remove Worktree…");
        assert!(screen(&app).text().contains("Remove the worktree clean?"));
        assert!(clean.exists(), "nothing yet");
        app.handle(Event::Key(KeyEvent::plain(Key::Char('r'))));
        assert!(!clean.exists(), "removed");
        assert!(
            git(&root, &["branch", "--list", "clean"]).contains("clean"),
            "the branch stays"
        );
        // With changes, once isn't enough.
        right(&mut app, "dirty");
        choose(&mut app, "Remove Worktree…");
        app.handle(Event::Key(KeyEvent::plain(Key::Char('r'))));
        let text = screen(&app).text();
        assert!(
            text.contains("dirty has changes that aren't committed"),
            "{text}"
        );
        assert!(dirty.exists(), "refused");
        app.handle(Event::Key(KeyEvent::plain(Key::Char('r'))));
        assert!(!dirty.exists(), "asked twice, it goes");
        assert!(
            git(&root, &["branch", "--list", "dirty"]).contains("dirty"),
            "the branch stays"
        );
        // The project's own checkout can't be taken away from under it.
        right(&mut app, "main");
        let Overlay::Menu(menu) = &app.overlay else {
            panic!("no menu on the project's row");
        };
        assert_eq!(menu.action(1), None, "greyed");
    }
}
