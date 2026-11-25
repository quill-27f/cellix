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
        self.agents.look_for_agents(
            &self.settings.agents,
            &self.settings.disabled_agents,
            &self.side_path(),
        );
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
        // Under the tab row it hangs 8 px clear of its line, as a tab's menu does: it stood
        // on the line.
        if menu.top() == at.1 {
            menu.hang(&*self.measure, screen);
        }
        // The + it drops from stands lit while it's open: at its row, a column into it.
        menu.from = Some((at.0 + 1, at.1.saturating_sub(1)));
        self.overlay = Overlay::Menu(menu);
    }

    /// The menu's rows: Terminal; every agent Cellix starts, those not installed dimmed, the
    /// person's own runners among them by name; the roles; and a new worktree.
    fn start_items(&self, roles: &[Role]) -> Vec<MenuItem> {
        let theme = &self.theme;
        let installed = self.agents.installed_agents();
        let has = |program: &str| installed.iter().any(|o| o.program == program);
        let (icon, color) = crate::agents::mark(SHELL.1, theme);
        let mut items = vec![
            MenuItem::new(SHELL.0, MenuAction::Start(SHELL.1.to_string()))
                .icon(icon, color)
                .key(TERMINAL_KEY),
        ];
        let mut keys = AGENT_KEYS.iter().zip("123456789".chars());
        for (index, (name, program)) in AGENTS
            .iter()
            .filter(|(_, program)| !self.settings.disabled_agents.iter().any(|d| d == *program))
            .enumerate()
        {
            let (icon, color) = crate::agents::mark(program, theme);
            let item = MenuItem::new(*name, MenuAction::Start(program.to_string()));
            let item = if has(program) {
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
        // The person's own runners, after the built-ins, each as its like agent's mark.
        for offered in installed.iter().filter(|o| o.custom) {
            let (icon, color) = crate::agents::mark(&offered.like, theme);
            let item = MenuItem::new(
                offered.name.clone(),
                MenuAction::Start(offered.program.clone()),
            )
            .icon(icon, color);
            items.push(match keys.next() {
                Some((shown, key)) => item.key((key, *shown)),
                None => item,
            });
        }
        if !roles.is_empty() {
            items.push(MenuItem::heading("Roles").after_rule());
        }
        // Roles are for the agents the role names, which are Cellix's own.
        let programs: Vec<&'static str> = installed
            .iter()
            .filter(|o| !o.custom)
            .filter_map(|o| {
                AGENTS
                    .iter()
                    .find(|(_, p)| *p == o.program)
                    .map(|(_, p)| *p)
            })
            .collect();
        for role in roles {
            let program = role.program(&AGENTS, &programs);
            let usable = program.is_some_and(has);
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
    /// which is the person's to answer. A runner takes its `like` agent's way of a prompt.
    pub(super) fn start_role(&mut self, program: &str, prompt: &str) {
        let root = self.workspace.root().to_path_buf();
        let (env, mut launch) = self.agent_launch(program);
        let offered = self
            .agents
            .installed_agents()
            .iter()
            .find(|o| o.program == program)
            .cloned();
        let name = offered
            .as_ref()
            .map_or_else(|| crate::agents::display_name(program), |o| o.name.as_str());
        let like = offered.as_ref().map_or(program, crate::agents::like_of);
        match crate::agents::first_prompt(like, prompt) {
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
        plus(app);
    }

    /// Presses the + after the pane's tabs.
    fn plus(app: &mut App) {
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

    /// The Agents list's START says what the + menu says, in its order: Terminal first, a
    /// rule, then each installed agent with its mark, one row each with what it runs at the
    /// row's end. It had a generic + before each, "Starts here, working on this project" under
    /// every agent, and Terminal last after a row of air.
    #[test]
    fn the_start_list_lists_as_the_plus_menu_does() {
        let (_dir, mut app) = project();
        app.run(Command::ListAgents);
        let grid = screen(&app);
        let rows: Vec<u16> = ["> Terminal", "⬡ Codex", "✦ Gemini CLI"]
            .iter()
            .map(|words| place_of(&grid, words).1)
            .collect();
        assert_eq!(rows[1], rows[0] + 2, "a rule's row after Terminal");
        assert_eq!(rows[2], rows[1] + 1, "one row each");
        assert!(grid.row_text(rows[0]).trim_end().ends_with("shell"));
        assert!(grid.row_text(rows[1]).trim_end().ends_with("codex"));
        assert!(!grid.text().contains("Starts here"), "{}", grid.text());
        // The menu, in the same order.
        plus(&mut app);
        let grid = screen(&app);
        let menu: Vec<u16> = ["Terminal", "Codex", "Gemini CLI"]
            .iter()
            .map(|words| place_of(&grid, words).1)
            .collect();
        assert!(menu.windows(2).all(|pair| pair[0] < pair[1]), "{menu:?}");
    }

    /// With a menu open, the pointer lights nothing under it: over the + menu's rows, the
    /// START list's row under them stayed lit as the pointer crossed it.
    #[test]
    fn a_menu_takes_the_pointer_from_what_is_under_it() {
        let (_dir, mut app) = project();
        app.set_measure(std::rc::Rc::new(cellix_render::FontMeasure::new(17, 37)));
        app.run(Command::ListAgents);
        let hover = app.theme.row_hover;
        // The washes on the list's rows, which stand a step under the pane's tabs.
        let top = (app.dock_area().unwrap().y + 2) as i32 * 64;
        let washes = |grid: &Grid| {
            let panels = grid.paint.iter().filter(|p| {
                matches!(p, Paint::Panel { fill: Some(fill), bounds, .. }
                    if *fill == hover && bounds.y >= top)
            });
            panels.count()
        };
        // A place on one of the list's rows that the menu will stand over, on a row of the
        // menu that can be chosen: a dimmed one, an agent not installed, lights nothing. On
        // the list's Gemini CLI row, the menu's row there became one when Kiro CLI joined it,
        // so every agent but Claude Code counts as installed here, each a row of both.
        let others = [
            "codex",
            "gemini",
            "opencode",
            "copilot",
            "cursor-agent",
            "kiro-cli",
        ];
        app.agents.pretend_installed(&others);
        let list = screen(&app);
        plus(&mut app);
        let Overlay::Menu(menu) = &app.overlay else {
            unreachable!()
        };
        let x = menu.area().x + 2;
        let y = others
            .iter()
            .map(|program| place_of(&list, crate::agents::display_name(program)).1)
            .find(|y| {
                let row = menu.row_at(x, f32::from(*y) + 0.5);
                menu.contains(x, *y) && row.is_some_and(|row| menu.action(row).is_some())
            })
            .expect("a row of the list under one of the menu's that can be chosen");
        app.handle(Event::Key(KeyEvent::plain(Key::Esc)));
        let moved = |app: &mut App, x: u16, y: u16| {
            app.handle(Event::Mouse(MouseEvent {
                kind: MouseKind::Move,
                x,
                y,
                fine_x: f32::from(x),
                fine_y: f32::from(y) + 0.5,
                mods: Mods::default(),
            }));
        };
        // Without a menu, the pointer there lights the list's row.
        moved(&mut app, x, y);
        assert_eq!(washes(&screen(&app)), 1, "lit under the pointer");
        // With the menu open over it, the same place lights a row of the menu, not the list's.
        plus(&mut app);
        let Overlay::Menu(menu) = &app.overlay else {
            unreachable!()
        };
        assert!(menu.area().contains(x, y), "the menu stands over that row");
        moved(&mut app, x, y);
        let grid = screen(&app);
        assert_eq!(washes(&grid), 0, "nothing under the menu is lit");
        let lit = grid.paint.iter().any(|p| {
            matches!(p, Paint::Panel { fill: Some(fill), .. } if *fill == app.theme.popup_hover)
        });
        assert!(lit, "the menu's own row is");
    }

    /// A stand-in for an agent's program, which writes the arguments it was started with,
    /// one a line, to the file this returns.
    #[cfg(unix)]
    fn stand_in(app: &mut App, dir: &Path, name: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let heard = dir.join(format!("{name}.txt"));
        let fake = dir.join(name);
        // Written whole and then moved into place: `printf` may write a line at a time, and
        // a look between them read "started" alone, which failed a test under load.
        let script = format!(
            "#!/bin/sh\nprintf '%s\\n' started \"$@\" > '{0}.part'\nmv '{0}.part' '{0}'\nsleep 9\n",
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

    /// Kiro CLI is offered where `kiro-cli` is installed: under START, and in the + menu with
    /// its key. Where it isn't, START leaves it out and the menu dims it, saying so, as any
    /// agent not installed. Started, it runs its chat, `kiro-cli chat`, under its own mark.
    #[cfg(unix)]
    #[test]
    fn kiro_cli_is_offered_where_it_is_installed_and_starts_its_chat() {
        let (dir, mut app) = project();
        let kiro = crate::agents::mark("kiro-cli", &app.theme);
        assert_eq!(kiro.0, Icon::AgentKiro);
        let marks = |grid: &Grid| -> Vec<(Icon, Rgb)> {
            let icons = grid.paint.iter().filter_map(|p| match p {
                Paint::Icon { icon, color, .. } => Some((*icon, *color)),
                _ => None,
            });
            icons.collect()
        };
        // Not installed: not under START, and dimmed in the menu, where a press does nothing.
        app.run(Command::ListAgents);
        assert!(!screen(&app).text().contains("Kiro CLI"));
        plus(&mut app);
        let grid = screen(&app);
        let row = grid.row_text(place_of(&grid, "Kiro CLI").1);
        assert!(row.trim_end().ends_with("not installed"), "{row}");
        assert!(marks(&grid).contains(&(kiro.0, app.theme.disabled)));
        app.handle(Event::Key(KeyEvent::plain(Key::Esc)));

        // Installed: under START with what it runs, and in the menu with its key.
        app.agents
            .pretend_installed(&["codex", "gemini", "kiro-cli"]);
        let grid = screen(&app);
        let (_, y) = place_of(&grid, "K Kiro CLI");
        assert!(grid.row_text(y).trim_end().ends_with("kiro-cli"));
        assert!(marks(&grid).contains(&kiro), "its own mark, in its colour");
        plus(&mut app);
        let grid = screen(&app);
        let row = grid.row_text(place_of(&grid, "Kiro CLI").1);
        assert!(row.trim_end().ends_with('3'), "{row}");
        let file = stand_in(&mut app, dir.path(), "kiro");
        app.handle(Event::Key(KeyEvent::plain(Key::Char('3'))));
        assert_eq!(app.dock.sessions.len(), 1);
        assert_eq!(app.dock.sessions[0].name, "Kiro CLI");
        assert_eq!(app.dock.sessions[0].icon(), Icon::AgentKiro);
        assert_eq!(heard(&mut app, &file), "started\nchat\n");
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
