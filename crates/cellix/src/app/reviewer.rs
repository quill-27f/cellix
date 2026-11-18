//! The reviewer: an agent started on the changes Review shows, told to read them and leave
//! what it finds as suggested comments, which Review shows under their lines to keep or
//! dismiss. It starts from Review's "Review with…", from the palette, or as the Reviewer
//! role on the side pane's +.

use super::*;

use crate::agents::AGENTS;
use crate::review::{Review, Suggested};
use crate::roles;

/// What the reviewer is called among the agents here, while the name is free.
const REVIEWER: &str = "reviewer";

impl App {
    /// The menu Review with… drops at `at`: every agent Cellix starts, those not installed
    /// dimmed, saying so.
    pub(super) fn show_review_with_menu(&mut self, at: (u16, u16)) {
        self.agents.look_for_agents();
        let installed = self.agents.installed_programs();
        let theme = &self.theme;
        let items = AGENTS
            .iter()
            .map(|(name, program)| {
                let (icon, color) = crate::agents::mark(program, theme);
                let item = MenuItem::new(*name, MenuAction::ReviewWith(program));
                if installed.contains(program) {
                    item.icon(icon, color)
                } else {
                    item.icon(icon, theme.disabled)
                        .note("not installed")
                        .when(false)
                }
            })
            .collect();
        let screen = Rect::new(0, 0, self.size.0.saturating_sub(1), self.size.1);
        self.overlay = Overlay::Menu(Menu::new(at, items, &*self.measure, screen));
    }

    /// Review with… from the palette: Review opens on the changes, and the menu drops from
    /// its button, or from the top of Review where it has no list.
    pub(super) fn review_with_menu(&mut self) {
        if self.review.is_none() {
            self.review = Some(self.new_review(self.workspace.root().to_path_buf()));
        }
        let area = self.main_area();
        let at = self.review.as_ref().and_then(|r| r.review_with_at(area));
        self.show_review_with_menu(at.unwrap_or((area.x + 2, area.y + 1)));
    }

    /// Starts `program` reviewing the changes Review compares, or those since the last
    /// commit with Review closed: the Reviewer role's prompt, its first line saying which
    /// changes, as the agent called reviewer.
    pub(super) fn review_with(&mut self, program: &'static str) {
        let home = if cfg!(test) {
            None
        } else {
            std::env::var_os("HOME").map(PathBuf::from)
        };
        let roles = roles::find(self.workspace.root(), home.as_deref());
        let Some(role) = roles.iter().find(|r| r.name.eq_ignore_ascii_case(REVIEWER)) else {
            return;
        };
        let range = self.review.as_ref().map(Review::agent_range);
        let prompt = with_range(&role.prompt, range.as_deref());
        self.team.wanted.replace(Some(REVIEWER.to_string()));
        self.start_role(program, &prompt);
        self.team.wanted.replace(None);
        let name = crate::agents::display_name(program);
        self.say(format!(
            "{name} is reviewing: what it finds shows in Review, to keep or dismiss"
        ));
    }

    /// What agents have suggested that nobody has kept or dismissed yet, for Review.
    pub(super) fn suggestions(&self) -> Vec<Suggested> {
        let open = self.threads.iter();
        open.filter(|t| !t.resolved && !t.suggested_by.is_empty())
            .map(|t| Suggested {
                id: t.id,
                file: t.file.clone(),
                line: t.line,
                by: t.suggested_by.clone(),
                kind: t.kind.clone(),
                text: t.text.clone(),
            })
            .collect()
    }

    /// An agent's suggestion kept: it's the person's comment now, and one on code goes with
    /// the review, as the person's own do.
    pub(super) fn keep_suggestion(&mut self, id: u32) {
        let Some(thread) = self.threads.iter_mut().find(|t| t.id == id) else {
            return;
        };
        thread.suggested_by.clear();
        let code = Dialect::of(Path::new(&thread.file)).is_none();
        let comment = Comment {
            file: thread.file.clone(),
            line: thread.line,
            text: thread.text.clone(),
            quote: thread.quote.clone(),
            removed: false,
        };
        if code && let Ok(mut comments) = self.comments.lock() {
            comments.push(comment);
        }
        self.save_threads();
        self.say("Kept as your comment");
    }

    pub(super) fn dismiss_suggestion(&mut self, id: u32) {
        if let Some(thread) = self.threads.iter_mut().find(|t| t.id == id) {
            thread.resolved = true;
        }
        self.save_threads();
    }
}

/// The reviewer's prompt with its first line saying which changes: `range` in place of the
/// line that names them, or before the rest where a project's own Reviewer has none.
fn with_range(prompt: &str, range: Option<&str>) -> String {
    let Some(range) = range else {
        return prompt.to_string();
    };
    match prompt.split_once('\n') {
        Some((first, rest)) if first.starts_with("Review the changes") => {
            format!("{range}\n{rest}")
        }
        _ if prompt.starts_with("Review the changes") => range.to_string(),
        _ => format!("{range}\n\n{prompt}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cellix_input::{Mods, MouseEvent};
    use std::fs;

    fn git(dir: &Path, args: &[&str]) {
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .unwrap()
            .status;
        assert!(status.success(), "git {args:?}");
    }

    fn screen(app: &App) -> Grid {
        let mut grid = Grid::new(app.size().0, app.size().1, app.background());
        app.draw(&mut grid);
        grid
    }

    fn press(app: &mut App, (x, y): (u16, u16)) {
        for kind in [MouseKind::Left, MouseKind::Up] {
            app.handle(Event::Mouse(MouseEvent {
                kind,
                x,
                y,
                fine_x: f32::from(x) + 0.5,
                fine_y: f32::from(y) + 0.5,
                mods: Mods::default(),
            }));
        }
    }

    /// Where `words` start on screen, a column into them.
    fn place_of(grid: &Grid, words: &str) -> (u16, u16) {
        (0..grid.height())
            .find_map(|y| {
                let row = grid.row_text(y);
                let at = row.find(words)?;
                Some((row[..at].chars().count() as u16 + 1, y))
            })
            .unwrap_or_else(|| panic!("no {words}: {}", grid.text()))
    }

    /// A committed project with a line added to review, and an app that counts Codex and
    /// Gemini CLI as installed, whatever this machine has, with a stand-in for them that
    /// writes down what it was started with, in a folder of its own. Claude Code isn't, so
    /// it's never started here: starting it writes its plugin into the real home folder.
    fn project() -> (tempfile::TempDir, tempfile::TempDir, App, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("queue.txt"), "one\ntwo\n").unwrap();
        git(dir.path(), &["init", "-q"]);
        git(dir.path(), &["config", "user.email", "t@example.com"]);
        git(dir.path(), &["config", "user.name", "Tester"]);
        git(dir.path(), &["add", "."]);
        git(dir.path(), &["commit", "-q", "-m", "base"]);
        fs::write(dir.path().join("queue.txt"), "one\ntwo\nthree\n").unwrap();
        let aside = tempfile::tempdir().unwrap();
        let heard = aside.path().join("heard.txt");
        let fake = aside.path().join("stand-in");
        // Written aside and moved into place, so the test never reads it half written: a
        // long prompt goes out in more than one write, and on a loaded machine the test can
        // look between them.
        let script = format!(
            "#!/bin/sh\nprintf '%s\\n' started \"$@\" > '{0}.part'\nmv '{0}.part' '{0}'\nsleep 9\n",
            heard.display()
        );
        fs::write(&fake, script).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&fake, fs::Permissions::from_mode(0o755)).unwrap();
        }
        let mut app = App::new(dir.path().to_path_buf(), (150, 40));
        app.agents.pretend_installed(&["codex", "gemini"]);
        app.agent_program = Some(fake.display().to_string());
        (dir, aside, app, heard)
    }

    /// What the stand-in was started with, once it has said.
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

    fn settle(app: &mut App) {
        while app.review.as_ref().is_some_and(Review::is_reading) {
            app.poll();
        }
    }

    /// The row of Review's cards with a suggestion on it, its Keep and Dismiss with it: the
    /// Comments list beside says who suggested it on a row of its own.
    fn suggestion_row(grid: &Grid) -> u16 {
        (0..grid.height())
            .find(|y| {
                let row = grid.row_text(*y);
                row.find("Suggested by")
                    .is_some_and(|at| row.find("Dismiss").is_some_and(|dismiss| dismiss > at))
            })
            .unwrap_or_else(|| panic!("no suggestion in Review:\n{}", grid.text()))
    }

    fn suggest(app: &mut App, line: usize, text: &str) -> u32 {
        let (made, number) = std::sync::mpsc::channel();
        app.serve(Request::Suggest {
            file: "queue.txt".into(),
            line,
            kind: "Correctness".into(),
            quote: "three".into(),
            text: text.into(),
            author: "Codex".into(),
            made,
        });
        number.recv().unwrap()
    }

    #[cfg(unix)]
    #[test]
    fn an_agent_reviews_the_changes_and_what_it_finds_shows_in_review() {
        let (dir, _aside, mut app, file) = project();
        app.review = Some(app.new_review(dir.path().to_path_buf()));
        settle(&mut app);

        // Review's list has Review with…, which drops a menu of the agents.
        let at = place_of(&screen(&app), "Review with");
        press(&mut app, at);
        assert!(matches!(app.overlay, Overlay::Menu(_)), "a menu drops");
        let text = screen(&app).text();
        assert!(text.contains("Gemini CLI"), "{text}");
        assert!(text.contains("not installed"), "Claude Code isn't: {text}");

        // Codex starts as the reviewer, told the Reviewer role's prompt as it starts: which
        // changes and how to see them, that it's read-only, and how to leave what it finds.
        let at = place_of(&screen(&app), "Codex");
        press(&mut app, at);
        assert_eq!(app.dock.sessions.len(), 1);
        assert_eq!(app.dock.sessions[0].called(), Some("reviewer"));
        let said = heard(&mut app, &file);
        assert!(said.starts_with("started\n"), "{said}");
        assert!(said.contains("since the last commit"), "{said}");
        assert!(said.contains("`git diff HEAD`"), "{said}");
        assert!(said.contains("read-only"), "{said}");
        assert!(
            said.contains("cellix suggest <file> <line> <kind>"),
            "{said}"
        );

        // What it suggests shows in Review under its line: who, what kind, what, and Keep
        // and Dismiss.
        let kept = suggest(&mut app, 3, "Past the limit.");
        let grid = screen(&app);
        let row = suggestion_row(&grid);
        let line = grid.row_text(row);
        assert!(line.contains("Suggested by Codex · Correctness"), "{line}");
        assert!(line.contains("Past the limit."), "{line}");
        assert!(line.contains("Keep") && line.contains("Dismiss"), "{line}");
        let (_, three) = place_of(&grid, "three");
        assert!(three < row, "under its line:\n{}", grid.text());

        // Kept, it's the person's comment, which goes with the review.
        let x = line.find("Keep").unwrap();
        press(&mut app, (line[..x].chars().count() as u16 + 1, row));
        let thread = app.threads.iter().find(|t| t.id == kept).unwrap();
        assert!(thread.suggested_by.is_empty(), "the person's own now");
        let comments = app.comments.lock().unwrap().clone();
        assert!(
            comments
                .iter()
                .any(|c| c.file == "queue.txt" && c.line == 3 && c.text == "Past the limit."),
            "{comments:?}"
        );
        let text = screen(&app).text();
        assert!(!text.contains("Suggested by Codex"), "{text}");
        assert!(text.contains("Your comment"), "{text}");

        // Dismissed, it's gone.
        let dropped = suggest(&mut app, 3, "Name it.");
        let grid = screen(&app);
        let row = suggestion_row(&grid);
        let line = grid.row_text(row);
        let x = line.find("Dismiss").unwrap();
        press(&mut app, (line[..x].chars().count() as u16 + 1, row));
        let thread = app.threads.iter().find(|t| t.id == dropped).unwrap();
        assert!(thread.resolved);
        assert!(!screen(&app).text().contains("Name it."));
    }

    /// From the palette, with Review closed: it opens on the changes and the menu drops
    /// from its button.
    #[test]
    fn the_palette_reviews_with_an_agent_from_review() {
        let (_dir, _aside, mut app, _) = project();
        app.run(Command::ReviewWith);
        assert!(app.review.is_some());
        assert!(matches!(app.overlay, Overlay::Menu(_)), "a menu drops");
    }

    /// A turn's changes are named by its checkpoints, in place of the last commit.
    #[test]
    fn the_reviewer_is_told_which_changes() {
        let role = "Review the changes since the last commit: `git diff HEAD`.\n\nRead-only.";
        let turn = "Review the changes one agent turn made: `git diff a b`.";
        assert_eq!(
            with_range(role, Some(turn)),
            format!("{turn}\n\nRead-only.")
        );
        assert_eq!(with_range(role, None), role);
        // A project's own Reviewer that doesn't say is told before the rest.
        assert_eq!(
            with_range("Be brief.", Some(turn)),
            format!("{turn}\n\nBe brief.")
        );
    }
}
