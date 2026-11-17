//! What the + after the side pane's tabs starts, in a menu dropping from it: a terminal, an
//! agent, a role, or a worktree for an agent. Kept apart from `app.rs`, which reaches here
//! through a handful of calls.

use super::*;

use crate::agents::{AGENTS, SHELL};
use crate::roles::{self, Role};

/// What picks a row of the menu while it's open: a letter for Terminal, and a digit for each
/// installed agent in turn, as they're shown at the row's end.
const TERMINAL_KEY: (char, &str) = ('t', "T");
const AGENT_KEYS: [&str; 9] = ["1", "2", "3", "4", "5", "6", "7", "8", "9"];
/// What a row says of an agent or a role that can't be started here.
const NOT_INSTALLED: &str = "not installed";

impl App {
    /// Opens what the + starts, dropping from it at `at`.
    pub(super) fn show_start_menu(&mut self, at: (u16, u16)) {
        self.agents.look_for_agents();
        let home = if cfg!(test) {
            None
        } else {
            std::env::var_os("HOME").map(PathBuf::from)
        };
        let roles = roles::find(self.workspace.root(), home.as_deref());
        let items = self.start_items(&roles);
        // A column clear of the window's edge, where the + near it pushes the menu.
        let screen = Rect::new(0, 0, self.size.0.saturating_sub(1), self.size.1);
        let mut menu = Menu::new(at, items, &*self.measure, screen);
        // The + it drops from stands lit while it's open: at its row, a column into it.
        menu.from = Some((at.0 + 1, at.1.saturating_sub(1)));
        self.overlay = Overlay::Menu(menu);
    }

    /// The menu's rows: Terminal; every agent Cellix starts, those not installed dimmed;
    /// the roles; and a new worktree.
    fn start_items(&self, roles: &[Role]) -> Vec<MenuItem> {
        let theme = &self.theme;
        let installed = self.agents.installed_programs();
        let (icon, color) = crate::agents::mark(SHELL.1, theme);
        let mut items = vec![
            MenuItem::new(SHELL.0, MenuAction::Start(SHELL.1))
                .icon(icon, color)
                .key(TERMINAL_KEY),
        ];
        let mut keys = AGENT_KEYS.iter().zip("123456789".chars());
        for (index, (name, program)) in AGENTS.iter().enumerate() {
            let (icon, color) = crate::agents::mark(program, theme);
            let item = MenuItem::new(*name, MenuAction::Start(program));
            let item = if installed.contains(program) {
                let item = item.icon(icon, color);
                match keys.next() {
                    Some((shown, key)) => item.key((key, *shown)),
                    None => item,
                }
            } else {
                item.icon(icon, theme.disabled)
                    .note(NOT_INSTALLED)
                    .when(false)
            };
            items.push(if index == 0 { item.after_rule() } else { item });
        }
        if !roles.is_empty() {
            items.push(MenuItem::heading("Roles").after_rule());
        }
        for role in roles {
            let program = role.program(&AGENTS, &installed);
            let usable = program.is_some_and(|p| installed.contains(&p));
            let (icon, color) = program.map_or((Icon::File, theme.disabled), |p| {
                crate::agents::mark(p, theme)
            });
            let action = MenuAction::StartRole(program.unwrap_or_default(), role.prompt.clone());
            let item = MenuItem::new(role.name.clone(), action).detail(role.description.clone());
            items.push(if usable {
                item.icon(icon, color)
            } else {
                item.icon(icon, theme.disabled)
                    .note(NOT_INSTALLED)
                    .when(false)
            });
        }
        let worktree = MenuItem::run("New Worktree…", Command::NewWorktree)
            .icon(Icon::Git, theme.activity.fg)
            .after_rule()
            .when(self.can_run(Command::NewWorktree));
        items.push(worktree);
        items
    }

    /// Starts a role: its agent, given the role's prompt as its first. The agent takes it
    /// once it's ready, after whatever it asks first, such as whether to trust the folder,
    /// which is the person's to answer.
    pub(super) fn start_role(&mut self, program: &'static str, prompt: &str) {
        let root = self.workspace.root().to_path_buf();
        let (env, mut launch) = self.agent_launch(program);
        let name = crate::agents::display_name(program);
        match crate::agents::first_prompt(program, prompt) {
            Some(args) => launch.extend(args),
            None => {
                self.clipboard = prompt.to_string();
                self.say(format!(
                    "{name} can't be given a first prompt: it's on the clipboard"
                ));
            }
        }
        self.start_session(name, &launch, &env, &root);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cellix_input::{Mods, MouseEvent};
    use cellix_render::Paint;
    use std::fs;

    fn screen(app: &App) -> Grid {
        let mut grid = Grid::new(app.size().0, app.size().1, app.background());
        app.draw(&mut grid);
        grid
    }

    fn press(app: &mut App, x: u16, y: u16) {
        for kind in [MouseKind::Left, MouseKind::Up] {
            app.handle(Event::Mouse(MouseEvent {
                kind,
                x,
                y,
                fine_x: f32::from(x),
                fine_y: f32::from(y) + 0.5,
                mods: Mods::default(),
            }));
        }
    }

    /// A project with a role of its own, and an app that counts Codex and Gemini CLI as
    /// installed, whatever this machine has. Claude Code isn't, so it's never started here:
    /// starting it writes its plugin into the real home folder.
    fn project() -> (tempfile::TempDir, App) {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("notes.txt"), "notes\n").unwrap();
        let roles = dir.path().join(".cellix/roles");
        fs::create_dir_all(&roles).unwrap();
        let docs = "---\nagent: gemini\ndescription: keeps the docs true\n---\n- Read the docs.\n";
        fs::write(roles.join("docs-keeper.md"), docs).unwrap();
        let mut app = App::new(dir.path().to_path_buf(), (150, 40));
        app.agents.pretend_installed(&["codex", "gemini"]);
        (dir, app)
    }

    /// Opens the pane on the Agents list, and presses the + after its tabs.
    fn open_menu(app: &mut App) {
        app.run(Command::ListAgents);
        let title = app.title_of(app.dock_area().expect("the pane is open"));
        press(app, app.dock.plus_x(title) + 1, title.y);
        assert!(
            matches!(app.overlay, Overlay::Menu(_)),
            "a menu drops from the +"
        );
    }

    /// The row of the screen that says `words`, and the column they start at.
    fn place_of(grid: &Grid, words: &str) -> (u16, u16) {
        (0..grid.height())
            .find_map(|y| {
                let row = grid.row_text(y);
                let at = row.find(words)?;
                Some((row[..at].chars().count() as u16, y))
            })
            .unwrap_or_else(|| panic!("no {words}: {}", grid.text()))
    }

    /// The + took the pane over with the Agents list to start one thing. It drops a menu:
    /// Terminal, every agent with its key (one not installed dimmed, saying so, and doing
    /// nothing), the roles under their heading, each with its agent's mark and what it's for.
    #[test]
    fn the_plus_drops_a_menu_of_what_it_starts() {
        let (_dir, mut app) = project();
        open_menu(&mut app);
        let grid = screen(&app);
        let rows: Vec<u16> = ["Terminal", "Claude Code", "Codex", "Gemini CLI", "ROLES"]
            .iter()
            .map(|words| place_of(&grid, words).1)
            .collect();
        assert!(rows.windows(2).all(|pair| pair[0] < pair[1]), "{rows:?}");
        let Overlay::Menu(menu) = &app.overlay else {
            unreachable!()
        };
        let area = menu.area();
        // What a row of the menu says, within its edges.
        let said = |y: u16| -> String {
            let row = grid.row_text(y);
            let row = row
                .chars()
                .skip(usize::from(area.x))
                .take(usize::from(area.w));
            row.collect::<String>().trim_end().to_string()
        };
        let ends = |words: &str, end: &str| {
            let row = said(place_of(&grid, words).1);
            assert!(row.ends_with(end), "{words} ends with {end}: {row}");
        };
        ends("Terminal", "T");
        ends("Codex", "1");
        ends("Gemini CLI", "2");
        ends("Claude Code", "not installed");
        // Roles: the project's, then Cellix's own Tester, each with what it's for.
        let docs = place_of(&grid, "Docs keeper").1;
        assert!(said(docs).contains("keeps the docs true"), "{}", said(docs));
        assert!(place_of(&grid, "Tester").1 > docs);
        // Each has its agent's mark in the agent's colour; one not installed is dimmed, mark
        // and all.
        let marks: Vec<(Icon, Rgb)> = grid
            .paint
            .iter()
            .filter_map(|p| match p {
                Paint::Icon { icon, color, .. } => Some((*icon, *color)),
                _ => None,
            })
            .collect();
        let gemini = crate::agents::mark("gemini", &app.theme);
        assert!(marks.contains(&gemini), "{marks:?}");
        let claude = crate::agents::mark("claude", &app.theme).0;
        assert!(marks.contains(&(claude, app.theme.disabled)), "{marks:?}");
        // A press on the dimmed row leaves the menu open, and starts nothing.
        let (x, y) = place_of(&grid, "Claude Code");
        press(&mut app, x + 2, y);
        assert!(matches!(app.overlay, Overlay::Menu(_)), "still open");
        assert!(app.dock.sessions.is_empty(), "nothing started");
        app.handle(Event::Key(KeyEvent::plain(Key::Esc)));
        assert!(matches!(app.overlay, Overlay::None), "Escape leaves it");
    }

    /// A stand-in for an agent's program, which writes the arguments it was started with,
    /// one a line, to the file this returns.
    #[cfg(unix)]
    fn stand_in(app: &mut App, dir: &Path, name: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let heard = dir.join(format!("{name}.txt"));
        let fake = dir.join(name);
        let script = format!(
            "#!/bin/sh\nprintf '%s\\n' started \"$@\" > '{}'\nsleep 9\n",
            heard.display()
        );
        fs::write(&fake, script).unwrap();
        fs::set_permissions(&fake, fs::Permissions::from_mode(0o755)).unwrap();
        app.agent_program = Some(fake.display().to_string());
        heard
    }

    /// What the stand-in was started with, once it has said.
    #[cfg(unix)]
    fn heard(app: &mut App, file: &Path) -> String {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        loop {
            app.poll();
            if let Ok(text) = fs::read_to_string(file)
                && text.ends_with('\n')
            {
                return text;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "the stand-in never ran"
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }

    /// While the menu is open, an installed agent's digit starts it, with nothing more.
    #[cfg(unix)]
    #[test]
    fn a_digit_starts_its_agent() {
        let (dir, mut app) = project();
        let file = stand_in(&mut app, dir.path(), "plain");
        open_menu(&mut app);
        app.handle(Event::Key(KeyEvent::plain(Key::Char('1'))));
        assert!(matches!(app.overlay, Overlay::None), "the menu goes");
        assert_eq!(app.dock.sessions.len(), 1);
        assert_eq!(
            app.dock.sessions[0].name, "Codex",
            "1 is the first installed"
        );
        assert_eq!(heard(&mut app, &file), "started\n", "and plainly");
    }

    /// A role starts its agent with the role's body as its first prompt, given the way the
    /// agent takes one as it starts: it asks what it asks first, such as whether to trust
    /// the folder, and works on the prompt after. Nothing is typed into its screen, where it
    /// could answer that question. A body that starts with a `-`, as a list does, isn't
    /// taken for an option.
    #[cfg(unix)]
    #[test]
    fn a_role_starts_its_agent_with_its_prompt() {
        let (dir, mut app) = project();
        let file = stand_in(&mut app, dir.path(), "tester");
        open_menu(&mut app);
        let (x, y) = place_of(&screen(&app), "Tester");
        press(&mut app, x, y);
        assert_eq!(app.dock.sessions.len(), 1);
        assert_eq!(
            app.dock.sessions[0].name, "Codex",
            "no agent named: the first installed"
        );
        let roles = roles::find(dir.path(), None);
        let tester = roles.iter().find(|role| role.name == "Tester").unwrap();
        assert_eq!(
            heard(&mut app, &file),
            format!("started\n--\n{}\n", tester.prompt)
        );
        // The project's own role names Gemini CLI, which is told its prompt as its option.
        let file = stand_in(&mut app, dir.path(), "keeper");
        open_menu(&mut app);
        let (x, y) = place_of(&screen(&app), "Docs keeper");
        press(&mut app, x, y);
        assert_eq!(app.dock.sessions.len(), 2);
        assert_eq!(app.dock.sessions[1].name, "Gemini CLI");
        assert_eq!(
            heard(&mut app, &file),
            "started\n--prompt-interactive=- Read the docs.\n"
        );
    }
}
