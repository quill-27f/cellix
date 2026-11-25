//! Ask, on screen: a side question about what's selected, on a page, in code or in a
//! terminal's output, and its answer on a card under the words, as a comment's glance card
//! stands. A question isn't a comment. It's answered by the agent's own command line run once
//! in the background (`crate::ask`), never typed into the agent working in the side pane, and
//! its answer is never counted, sent or handed over with the comments. It goes when dismissed,
//! unless Make a Comment turns it into one.
//!
//! The card begins as a field, "Ask about 'the words'…", with the quick questions under it
//! (Explain, Define, Simplify): Enter with nothing typed explains, Esc closes. Asked, it shows
//! the question at its head and the answer as it streams in, with a spinner while it works,
//! and then Copy, Follow Up, Make a Comment and Dismiss.

use super::*;
use crate::ask::{self, Cli, News};

/// The most columns of words the card has: wider than a comment's glance card, which shows a
/// few rows of a comment, since an answer can run to a paragraph or two.
const WIDE: u16 = 56;
/// The fewest columns of words, under which there's no card.
const NARROWEST: u16 = 24;
/// Columns from the card's edge to its words, as every card's.
const INSET: u16 = 2;
/// The most rows of answer shown at once; past it the wheel scrolls them.
const ANSWER_ROWS: usize = 14;
/// What the buttons say.
pub(super) const ASK: &str = "Ask";
pub(super) const COPY: &str = "Copy";
pub(super) const FOLLOW_UP: &str = "Follow Up";
pub(super) const MAKE_COMMENT: &str = "Make a Comment";
pub(super) const DISMISS: &str = "Dismiss";
pub(super) const TRY_AGAIN: &str = "Try Again";
pub(super) const SET_UP: &str = "Open Integrations";
pub(super) const CANCEL: &str = "Cancel";
/// The choice that asks in a copy of the working agent's conversation.
pub(super) const WITH_AGENT: &str = "With the agent's conversation";

/// What a question is about.
#[derive(Clone, Debug)]
pub(super) enum About {
    /// Words in a file: its tab, the selection as letters from the start, its path in the
    /// project.
    Page {
        editor: usize,
        range: (usize, usize),
        file: String,
    },
    /// Output selected in a terminal tab: the tab, the command that printed it, the places in
    /// the project it names, the output, and the screen rows and column it was on.
    Output {
        tab: String,
        command: String,
        places: Vec<String>,
        text: String,
        rows: (u16, u16),
        column: u16,
    },
}

/// Where the card is in its life.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Phase {
    /// The field, waiting for a question.
    Asking,
    /// Asked; the answer is coming.
    Waiting,
    Answered,
    /// No answer, and why, in words.
    Failed(String),
    /// Nothing installed to ask: the one the setting names, or none.
    Missing(Option<Cli>),
}

/// The side question under way.
pub(super) struct Side {
    pub about: About,
    /// The words selected, on one line.
    pub quote: String,
    /// What's typed and the caret, while a field is open: the question, or a follow-up.
    pub field: Option<TextBox>,
    /// The question asked, as the card's head says it.
    pub asked: String,
    /// What it was asked, whole, to try again with.
    pub prompt: String,
    pub answer: String,
    /// What it's doing while no words have come.
    pub doing: String,
    pub phase: Phase,
    pub cli: Option<Cli>,
    /// The side conversation, once its command line says which.
    pub session: Option<String>,
    /// Asked in a copy of the working Claude Code agent's conversation (layer 4).
    pub copy: bool,
    /// That agent's session, where there's one to copy.
    pub main: Option<String>,
    pub running: Option<ask::Running>,
    /// How many rows of the answer are scrolled past.
    pub top: usize,
    /// A resumed conversation that failed before saying anything is tried once afresh.
    pub fresh: Option<ask::Question>,
    /// The document's version this question sent, kept with its conversation.
    pub version: u64,
}

/// How many lines of a long question Ask's field shows; past them it keeps the caret's in view.
const FIELD_ROWS: usize = 5;

/// What the card holds, row by row between its edges.
#[derive(Clone, Debug, PartialEq)]
enum Row {
    /// The question, or what the card asks for, bold; at its end, quietly, who answers.
    Head(String, String),
    /// A line of a field: its words, or its hint while empty, the caret's place in them, what's
    /// selected of them, and where the line starts in the field's text. A field longer than its
    /// box is several, each a line of its words wrapped as a comment's are, under one box.
    Field(String, bool, Option<usize>, Option<(usize, usize)>, usize),
    /// The choice of the agent's conversation, and whether it's chosen.
    Choice(bool),
    /// A row of the answer, or of what's wrong.
    Words(String),
    /// What it's doing, quietly, while nothing's come.
    Doing(String),
}

/// The card as it stands.
pub(super) struct Card {
    pub area: Rect,
    rows: Vec<Row>,
    /// The buttons, each as (label, cells, column), on their rows.
    buttons: Vec<Vec<(&'static str, String, u16)>>,
    /// Which button on the last row is the one to press, filled.
    filled: Option<&'static str>,
}

impl Card {
    /// The row of the card's first button row.
    fn button_row(&self) -> u16 {
        self.area
            .bottom()
            .saturating_sub(1 + self.buttons.len() as u16)
    }
}

/// An answer's Markdown as plain words, as a comment's are shown: its marks for bold, code and
/// headings left out, and a list's items each a bullet.
pub(super) fn plain(text: &str) -> String {
    let mut out = Vec::new();
    let mut fenced = false;
    for line in text.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") {
            fenced = !fenced;
            continue;
        }
        if fenced {
            out.push(line.to_string());
            continue;
        }
        let indent = &line[..line.len() - trimmed.len()];
        let body = trimmed.trim_start_matches('#').trim_start();
        let body = if trimmed.starts_with('#') {
            body
        } else {
            trimmed
        };
        let body = match body.strip_prefix("- ").or_else(|| body.strip_prefix("* ")) {
            Some(item) => format!("• {item}"),
            None => body.to_string(),
        };
        let body = body.replace("**", "").replace("__", "").replace('`', "");
        out.push(format!("{indent}{body}"));
    }
    out.join("\n")
}

impl App {
    /// The PATH the command lines are looked for on and run with. A test names its own, and
    /// without one finds nothing, so no test ever runs a real agent.
    pub(super) fn side_path(&self) -> std::ffi::OsString {
        if cfg!(test) {
            return self.side_path.clone().unwrap_or_default();
        }
        self.side_path
            .clone()
            .or_else(|| std::env::var_os("PATH"))
            .unwrap_or_default()
    }

    /// Which command line answers here, or what's missing.
    pub(super) fn side_cli(&self) -> Result<Cli, Option<Cli>> {
        let path = self.side_path();
        let pane = self
            .dock
            .agent_session()
            .and_then(|session| Cli::from_agent(&session.name));
        let setting = self.settings.ask_agent.as_deref();
        ask::choose(setting, pane, |cli| {
            ask::find(cli.program(), &path).is_some()
        })
    }

    /// The command line that answers here, and the command that runs it: the agent whose
    /// treatment the arguments take, and a runner's own command where the Default agent is
    /// one of the person's. What's missing is said as `side_cli` says it.
    pub(super) fn side_runner(&self) -> Result<(Cli, String), Option<Cli>> {
        let path = self.side_path();
        if let Some(set) = self.settings.ask_agent.as_deref()
            && let Some(agent) = self.settings.agents.iter().find(|a| a.name == set)
            && let Some(cli) = Cli::from_setting(&agent.like)
            && crate::ask::find(&agent.command, &path).is_some()
        {
            return Ok((cli, agent.command.clone()));
        }
        let cli = self.side_cli()?;
        Ok((cli, cli.program().to_string()))
    }

    /// The Claude Code session working here that a question can be asked in a copy of: the
    /// agent in the side pane, or inside Claude Code the one this pane is in, once its hooks
    /// have said its session.
    fn copyable_session(&self) -> Option<String> {
        let index = (0..self.dock.sessions.len())
            .filter(|i| self.dock.sessions[*i].agent())
            .find(|i| self.dock.sessions[*i].name == "Claude Code");
        if let Some((agent, id, _)) = index.and_then(|i| self.agent_word(i))
            && agent == "Claude Code"
            && !id.is_empty()
        {
            return Some(id);
        }
        if self.in_pane {
            return (self.agents.running.iter().rev())
                .find(|e| e.agent == "Claude Code" && !e.session.is_empty())
                .map(|e| e.session.clone());
        }
        None
    }

    /// Opens the Ask field under what's selected in the file in front, or the caret's line.
    pub(super) fn ask_about_selection(&mut self) {
        let Some(editor) = self.editors.get(self.active()) else {
            return;
        };
        let Some(path) = editor.buffer.path().map(Path::to_path_buf) else {
            self.say("Save the file first, so the agent can read it");
            return;
        };
        let primary = editor.selections.primary();
        let range = if primary.is_empty() {
            let rope = editor.buffer.rope();
            let line = rope.char_to_line(primary.head);
            let start = rope.line_to_char(line);
            let end = start + editor.line_text(line).chars().count();
            (start, end)
        } else {
            (primary.start(), primary.end())
        };
        let text = editor.buffer.text();
        let quote: String = text.chars().skip(range.0).take(range.1 - range.0).collect();
        let quote = quote.split_whitespace().collect::<Vec<_>>().join(" ");
        if quote.is_empty() {
            self.say("Select the words to ask about");
            return;
        }
        let about = About::Page {
            editor: self.active(),
            range,
            file: self.relative(&path),
        };
        self.open_side(about, quote);
    }

    /// Opens the Ask field under what's selected in the terminal tab in front.
    pub(super) fn ask_about_output(&mut self) {
        let Some(screen) = self.dock_screen() else {
            return;
        };
        let Some(session) = self.dock.session_mut() else {
            return;
        };
        let Some((text, from, to)) = session.selection() else {
            return;
        };
        let command = session.selection_command().unwrap_or_default();
        let tab = session.name.clone();
        session.clear_selection();
        let whole = text.trim().to_string();
        let places = self
            .places_named(&whole)
            .iter()
            .map(|(file, line)| format!("{file}:{line}"))
            .collect();
        let quote = whole.split_whitespace().collect::<Vec<_>>().join(" ");
        let about = About::Output {
            tab,
            command,
            places,
            text: whole,
            rows: (screen.y + from.1, screen.y + to.1),
            column: screen.x + from.0,
        };
        self.open_side(about, quote);
    }

    fn open_side(&mut self, about: About, quote: String) {
        let (phase, cli) = match self.side_cli() {
            Ok(cli) => (Phase::Asking, Some(cli)),
            Err(missing) => (Phase::Missing(missing), None),
        };
        let main = cli
            .filter(|cli| cli.copies())
            .and_then(|_| self.copyable_session());
        let field = (phase == Phase::Asking).then(TextBox::default);
        self.side = Some(Side {
            about,
            quote,
            field,
            asked: String::new(),
            prompt: String::new(),
            answer: String::new(),
            doing: String::new(),
            phase,
            cli,
            session: None,
            copy: false,
            main,
            running: None,
            top: 0,
            fresh: None,
            version: 0,
        });
        self.glance = None;
    }

    /// Asks the card's question: what was typed, or a quick one by its label.
    fn side_ask(&mut self, typed: &str) {
        let Some(side) = self.side.as_ref() else {
            return;
        };
        let follow = side.phase != Phase::Asking;
        let quick = ask::QUICK.iter().find(|(label, _)| *label == typed);
        let (label, question) = match quick {
            Some((label, question)) => (label.to_string(), question.to_string()),
            None if typed.trim().is_empty() => {
                (ask::QUICK[0].0.to_string(), ask::QUICK[0].1.to_string())
            }
            None => (typed.trim().to_string(), typed.trim().to_string()),
        };
        let question_words = question.clone();
        let Some(cli) = side.cli else {
            return;
        };
        let root = self.workspace.root().to_path_buf();
        let (prompt, resume, fork, version) = if follow {
            // On the same card: the conversation it's in, whatever the document did since.
            match side.session.clone() {
                Some(id) => (
                    ask::follow_up_prompt(&question),
                    Some(id),
                    false,
                    side.version,
                ),
                None => {
                    let (prompt, version) = self.side_first(&question, false);
                    (prompt, None, false, version)
                }
            }
        } else if side.copy && side.main.is_some() {
            let (prompt, version) = self.side_first(&question, true);
            (prompt, side.main.clone(), true, version)
        } else {
            self.side_question(cli, &question)
        };
        let question = ask::Question {
            cli,
            root,
            prompt,
            resume,
            fork,
        };
        // A resumed conversation that's gone (cleared by its command line, say) is tried
        // once afresh, with the whole document.
        let fresh =
            (question.resume.is_some() && !question.fork && !follow).then(|| ask::Question {
                prompt: self.side_first(&question_words, false).0,
                resume: None,
                ..question.clone()
            });
        let running = self.side_start(&question);
        if let Some(side) = self.side.as_mut() {
            side.asked = label;
            side.prompt = question.prompt.clone();
            side.answer.clear();
            side.doing.clear();
            side.field = None;
            side.phase = Phase::Waiting;
            side.top = 0;
            side.running = Some(running);
            side.fresh = fresh;
            side.version = version;
            if question.fork {
                // The copy is its own conversation: follow-ups go on in it, not in the
                // agent's.
                side.session = None;
            }
        }
    }

    /// The question a quick one's label stands for, or the label itself.
    fn side_question_words(&self, label: &str) -> String {
        ask::QUICK
            .iter()
            .find(|(quick, _)| *quick == label)
            .map_or_else(|| label.to_string(), |(_, words)| words.to_string())
    }

    /// The first thing a conversation about this is told, and the document's version in it.
    fn side_first(&self, question: &str, copy: bool) -> (String, u64) {
        let Some(side) = self.side.as_ref() else {
            return (String::new(), 0);
        };
        match &side.about {
            About::Page {
                editor,
                range,
                file,
            } => {
                let Some(editor) = self.editors.get(*editor) else {
                    return (String::new(), 0);
                };
                let text = editor.buffer.text();
                let path = editor.buffer.path();
                let doc = ask::Doc {
                    path: file,
                    folder: path.and_then(Path::parent),
                    text: &text,
                    dialect: path.and_then(crate::rendered::Dialect::of),
                    selection: *range,
                };
                (ask::first_prompt(&doc, question, copy), ask::version(&text))
            }
            About::Output {
                tab,
                command,
                places,
                text,
                ..
            } => (ask::output_prompt(tab, command, places, text, question), 0),
        }
    }

    /// What a new question on the card's words is told, and the document's version: in the
    /// document's own side conversation where there is one with this command line, the
    /// document again only where it changed since (layer 3).
    fn side_question(&self, cli: Cli, question: &str) -> (String, Option<String>, bool, u64) {
        let Some(side) = self.side.as_ref() else {
            return (String::new(), None, false, 0);
        };
        let About::Page {
            editor,
            range,
            file,
        } = &side.about
        else {
            let (prompt, version) = self.side_first(question, false);
            return (prompt, None, false, version);
        };
        let kept = self.side_kept.get(file).filter(|kept| kept.cli == cli);
        let (prompt, version) = self.side_first(question, false);
        let Some(kept) = kept else {
            return (prompt, None, false, version);
        };
        let Some(editor) = self.editors.get(*editor) else {
            return (prompt, None, false, version);
        };
        let text = editor.buffer.text();
        let path = editor.buffer.path();
        let doc = ask::Doc {
            path: file,
            folder: path.and_then(Path::parent),
            text: &text,
            dialect: path.and_then(crate::rendered::Dialect::of),
            selection: *range,
        };
        let prompt = if kept.version == version {
            ask::next_prompt(&doc, question)
        } else {
            format!(
                "The document has changed since the last question; here it is as it reads \
                 now.\n\n{prompt}"
            )
        };
        (prompt, Some(kept.session.clone()), false, version)
    }

    /// Starts the command line on a question.
    fn side_start(&self, question: &ask::Question) -> ask::Running {
        let path = self.side_path();
        let program = ask::find(question.cli.program(), &path)
            .unwrap_or_else(|| PathBuf::from(question.cli.program()));
        ask::start(&program, question, &path, self.side_limit)
    }

    /// Takes in what the command line said since. True when the card changed.
    pub(super) fn poll_side(&mut self) -> bool {
        let Some(side) = self.side.as_mut() else {
            return false;
        };
        let Some(running) = side.running.as_ref() else {
            return false;
        };
        let news: Vec<News> = running.news.try_iter().collect();
        if news.is_empty() {
            return false;
        }
        let mut keep = None;
        let mut again = None;
        for item in news {
            match item {
                News::Session(id) => {
                    side.session = Some(id.clone());
                    if !side.copy
                        && let (About::Page { file, .. }, Some(cli)) = (&side.about, side.cli)
                    {
                        keep = Some((file.clone(), cli, id, side.version));
                    }
                }
                News::Fresh => side.answer.clear(),
                News::Text(words) => {
                    side.answer.push_str(&words);
                    side.doing.clear();
                }
                News::Said(words) => {
                    side.answer = words;
                    side.doing.clear();
                }
                News::Doing(what) => side.doing = what,
                News::Done(words) => {
                    if !words.trim().is_empty() {
                        side.answer = words;
                    }
                    side.phase = Phase::Answered;
                    side.running = None;
                    break;
                }
                News::Failed(failure) => {
                    // A conversation its command line no longer has fails in its own words;
                    // one that timed out or isn't signed in would only fail again.
                    let gone = matches!(failure, ask::Failure::Said(_));
                    if side.answer.is_empty()
                        && gone
                        && let Some(fresh) = side.fresh.take()
                    {
                        again = Some(fresh);
                    } else {
                        let cli = side.cli.unwrap_or(Cli::Claude);
                        side.phase = Phase::Failed(failure.words(cli));
                    }
                    side.running = None;
                    break;
                }
            }
        }
        if let Some((file, cli, session, version)) = keep {
            self.side_kept.insert(
                file,
                ask::Kept {
                    cli,
                    session,
                    version,
                },
            );
            ask::save(self.workspace.root(), &self.side_kept);
        }
        if let Some(fresh) = again {
            // Its conversation is gone: forgotten, and the question asked afresh.
            if let Some(About::Page { file, .. }) = self.side.as_ref().map(|s| s.about.clone()) {
                self.side_kept.remove(&file);
            }
            let running = self.side_start(&fresh);
            if let Some(side) = self.side.as_mut() {
                side.prompt = fresh.prompt.clone();
                side.session = None;
                side.running = Some(running);
            }
        }
        true
    }

    /// Whether Ask's card is about `range` in the tab `editor`, so the buttons that would ask
    /// about it again give way to the card.
    pub(super) fn side_is_about(&self, editor: usize, range: (usize, usize)) -> bool {
        let about = self.side.as_ref().map(|side| &side.about);
        matches!(about, Some(About::Page { editor: e, range: r, .. }) if *e == editor && *r == range)
    }

    /// Whether Ask's field has the keyboard: the question's, or a follow-up's.
    pub(super) fn side_field_open(&self) -> bool {
        self.side.as_ref().is_some_and(|side| side.field.is_some())
    }

    /// The byte of the field's words a press or a drag at `mouse` stands at: on the field's
    /// row and across it, or with `clamp`, anywhere. Its words start where `draw_side` sets
    /// them.
    pub(super) fn side_field_place(&self, mouse: MouseEvent, clamp: bool) -> Option<usize> {
        let card = self.side_card()?;
        self.side.as_ref()?.field.as_ref()?;
        let area = card.area;
        // The field's lines, each with its row, its words and where they start.
        let lines: Vec<(u16, &str, usize)> = card
            .rows
            .iter()
            .enumerate()
            .filter_map(|(n, r)| match r {
                Row::Field(words, hint, _, _, start) => {
                    let words = if *hint { "" } else { words.as_str() };
                    Some((area.y + 1 + n as u16, words, *start))
                }
                _ => None,
            })
            .collect();
        let (top, bottom) = (lines.first()?.0, lines.last()?.0);
        let x = area.x + INSET;
        let right = area.right().saturating_sub(INSET);
        let on = (top..=bottom).contains(&mouse.y) && mouse.x + 1 >= x && mouse.x < right;
        if !clamp && !on {
            return None;
        }
        let row = mouse.y.clamp(top, bottom);
        let (_, text, line_start) = *lines.iter().find(|(y, _, _)| *y == row)?;
        let lead = self.card_lead(area);
        let inset = measures::columns(Self::card_room(), &*self.measure);
        let start = match inset {
            Some(inset) => {
                let left = f32::from(area.x) + lead + inset;
                let pad = measures::columns(COMMAND_PADDING, &*self.measure);
                left + pad.unwrap_or(0.0)
            }
            None => f32::from(x),
        };
        let mut edge = if self.measure.in_cells() {
            start.floor()
        } else {
            start
        };
        let advances = self.measure.advances(text, Font::default());
        let mut letters = 0;
        for advance in advances.iter() {
            if mouse.fine_x < edge + advance / 2.0 {
                break;
            }
            edge += advance;
            letters += 1;
        }
        // Past a wrapped line's last letter is the next line's start, which is the same place.
        Some(
            line_start
                + text
                    .char_indices()
                    .nth(letters)
                    .map_or(text.len(), |(at, _)| at),
        )
    }

    /// Keys while the card is up: a field's when one is open, Esc anyway. True when taken.
    pub(super) fn side_key(&mut self, key: KeyEvent) -> bool {
        let Some(side) = self.side.as_mut() else {
            return false;
        };
        if side.field.is_some() {
            match key.key {
                Key::Enter => {
                    let typed = side.field.as_ref().map(|field| field.text().to_string());
                    self.side_ask(&typed.unwrap_or_default());
                }
                Key::Esc => {
                    if side.phase == Phase::Asking {
                        self.side = None;
                    } else {
                        side.field = None;
                    }
                }
                _ => {
                    let command = self.workflow.keys.command(key);
                    let Some(field) = self.side.as_mut().and_then(|s| s.field.as_mut()) else {
                        return false;
                    };
                    // The field has the keyboard: what isn't its own does nothing, so no
                    // letter lands in the file under it, but a command's keys still work.
                    let taken = field.key_as(key, command);
                    let command = key.mods.ctrl || key.mods.alt;
                    return taken || !command;
                }
            }
            return true;
        }
        // Esc dismisses it, unless the keyboard is a terminal's in the side pane.
        if key.key == Key::Esc && !self.dock.focused {
            self.side = None;
            return true;
        }
        false
    }

    /// A press, the wheel or a move while the card is up. True when the card took it.
    pub(super) fn side_mouse(&mut self, mouse: MouseEvent) -> bool {
        let Some(card) = self.side_card() else {
            return false;
        };
        let inside = card.area.contains(mouse.x, mouse.y);
        match mouse.kind {
            MouseKind::ScrollUp | MouseKind::ScrollDown if inside => {
                if let Some(side) = self.side.as_mut() {
                    side.top = if mouse.kind == MouseKind::ScrollUp {
                        side.top.saturating_sub(1)
                    } else {
                        side.top + 1
                    };
                }
                true
            }
            MouseKind::Left if inside => {
                self.side_press(&card, mouse);
                true
            }
            // A press elsewhere closes the field, as it would a menu; an answer stays till
            // it's dismissed, so it can be read beside the words.
            MouseKind::Left => {
                if self.side.as_ref().is_some_and(|s| s.phase == Phase::Asking) {
                    self.side = None;
                }
                false
            }
            _ => inside,
        }
    }

    fn side_press(&mut self, card: &Card, mouse: MouseEvent) {
        let at = mouse.at();
        let pressed = card.buttons.iter().enumerate().find_map(|(n, row)| {
            let y = card.button_row() + n as u16;
            row.iter().find_map(|(label, cells, x)| {
                let width = cells.chars().count() as u16;
                on_large_button((*x, y), width, at).then_some(*label)
            })
        });
        if let Some(label) = pressed {
            self.side_button(label);
            return;
        }
        // The choice of the agent's conversation, anywhere on its row.
        let top = card.area.y + 1;
        let row = mouse.y.checked_sub(top).map(usize::from);
        if let Some(Row::Choice(_)) = row.and_then(|r| card.rows.get(r))
            && let Some(side) = self.side.as_mut()
        {
            side.copy = !side.copy;
        }
    }

    fn side_button(&mut self, label: &str) {
        match label {
            ASK => {
                let typed = self
                    .side
                    .as_ref()
                    .and_then(|s| s.field.as_ref())
                    .map(|field| field.text().to_string())
                    .unwrap_or_default();
                self.side_ask(&typed);
            }
            COPY => {
                if let Some(side) = self.side.as_ref() {
                    let text = side.answer.clone();
                    self.copy_text(text, "Copied the answer");
                }
            }
            FOLLOW_UP => {
                if let Some(side) = self.side.as_mut() {
                    side.field = Some(TextBox::default());
                }
            }
            MAKE_COMMENT => self.side_make_comment(),
            TRY_AGAIN => {
                let asked = self.side.as_ref().map(|s| s.asked.clone());
                if let Some(side) = self.side.as_mut() {
                    side.phase = Phase::Asking;
                    side.session = None;
                }
                if let Some(asked) = asked {
                    self.side_ask(&asked);
                }
            }
            SET_UP => {
                self.side = None;
                self.menu_action(MenuAction::SetUpAgents);
            }
            CANCEL => {
                if let Some(side) = self.side.as_mut() {
                    side.field = None;
                }
            }
            DISMISS => self.side = None,
            quick => {
                if ask::QUICK.iter().any(|(label, _)| *label == quick) {
                    self.side_ask(quick);
                }
            }
        }
    }

    /// Turns the question and its answer into a review comment on the same words, for when
    /// it was feedback after all: the question is the comment, Clarity its kind, and the
    /// answer the first reply, by the agent that gave it. It's the reviewer's to send on.
    fn side_make_comment(&mut self) {
        let Some(side) = self.side.take() else {
            return;
        };
        let author = side.cli.map_or("Agent", Cli::name).to_string();
        let question = self.side_question_words(&side.asked);
        let reply = Reply::new(author, side.answer.trim().to_string());
        let id = cellix_review::threads::next_id(&self.threads);
        let (file, line, quote, selection, about) = match &side.about {
            About::Page {
                editor,
                range,
                file,
            } => {
                let Some(editor) = self.editors.get(*editor) else {
                    return;
                };
                let rope = editor.buffer.rope();
                let line = rope.char_to_line(range.0.min(rope.len_chars())) + 1;
                let whole: String = rope
                    .slice(range.0.min(rope.len_chars())..range.1.min(rope.len_chars()))
                    .to_string();
                let whole = whole.trim().to_string();
                let selection = if whole.contains('\n') {
                    whole.clone()
                } else {
                    String::new()
                };
                (file.clone(), line, whole, selection, None)
            }
            About::Output {
                tab,
                command,
                places,
                text,
                ..
            } => {
                let quote = text.lines().next().unwrap_or_default().trim().to_string();
                let selection = if text.contains('\n') {
                    text.clone()
                } else {
                    String::new()
                };
                let about = cellix_review::About::Output {
                    tab: tab.clone(),
                    command: command.clone(),
                    places: places.clone(),
                };
                (String::new(), 0, quote, selection, Some(about))
            }
        };
        self.threads.push(Thread {
            id,
            file,
            line,
            quote,
            text: question,
            replies: vec![reply],
            resolved: false,
            // The answer is the last word: nothing waits for an agent until the reviewer
            // adds one.
            told: Some(1),
            asked: false,
            kind: "Clarity".to_string(),
            selection,
            about,
            at: cellix_review::threads::now(),
            ..Thread::default()
        });
        self.threads_seen = None;
        self.save_threads();
        self.dock.open = true;
        self.dock.show_comments();
        self.lit_thread = Some(id);
        self.show_lit_comment(id);
        self.say("Made a comment of the question and its answer. Reply to send it on");
    }

    /// Where the card stands and what it holds, while its words are on screen.
    pub(super) fn side_card(&self) -> Option<Card> {
        let side = self.side.as_ref()?;
        if !matches!(self.overlay, Overlay::None) || self.settings_page.is_some() {
            return None;
        }
        // The rows its words are on, the column they start at, and the area it stands in.
        let (area, first, last, starts) = match &side.about {
            About::Page { editor, range, .. } => {
                let pane = self.panes.iter().position(|i| i == editor)?;
                let area = self.pane_area(pane);
                let ed = self.editors.get(*editor)?;
                let (start, end) = *range;
                let first = ed.cell_of(area, start).map_or(area.y, |(_, row)| row);
                let last = ed
                    .cell_of(area, end.saturating_sub(1).max(start))
                    .map_or(area.bottom().saturating_sub(1), |(_, row)| row);
                let starts = ed
                    .cell_of(area, start)
                    .map_or(f32::from(area.x + INSET), |(x, _)| x);
                (area, first, last, starts)
            }
            About::Output { rows, column, .. } => {
                let area = self.dock_screen()?;
                (area, rows.0, rows.1, f32::from(*column))
            }
        };
        let room = area.w.saturating_sub(2 + INSET * 2);
        let words = WIDE.min(room);
        if words < NARROWEST {
            return None;
        }
        let wide = words + INSET * 2;
        let width = f32::from(words);
        let mut rows = self.side_rows(side, width);
        let buttons = self.side_labels(side);
        // The buttons on as few rows as they fit on, each row ending at the card's right.
        let mut lines: Vec<Vec<&'static str>> = vec![Vec::new()];
        let mut used = 0;
        for label in buttons {
            let cells = measures::button_cells(label, &*self.measure)
                .chars()
                .count() as u16;
            let more = if used == 0 { cells } else { cells + BUTTON_GAP };
            if used > 0 && used + more > words {
                lines.push(Vec::new());
                used = 0;
            }
            used += if used == 0 { cells } else { cells + BUTTON_GAP };
            if let Some(line) = lines.last_mut() {
                line.push(label);
            }
        }
        // Edges, rows, a gap where a row has no room, and the buttons.
        let fixed = 2 + self.card_gap() + lines.len() as u16;
        let wanted = rows.len() as u16 + fixed;
        let under = area.bottom().saturating_sub(last + 1);
        let over = first.saturating_sub(area.y);
        let (top, tall) = if wanted <= under {
            (last + 1, wanted)
        } else if wanted <= over {
            (first - wanted, wanted)
        } else if under >= over {
            (last + 1, under)
        } else {
            (area.y, over)
        };
        if tall < fixed + 1 {
            return None;
        }
        // Too tall for the room: the answer's rows scroll, the head and the field stay.
        let fits = usize::from(tall - fixed);
        if rows.len() > fits {
            let head = rows
                .iter()
                .take_while(|r| matches!(r, Row::Head(..)))
                .count();
            let tail = rows
                .iter()
                .rev()
                .take_while(|r| matches!(r, Row::Field(..)))
                .count();
            let body = rows.len() - head - tail;
            let shown = fits.saturating_sub(head + tail).max(1);
            let skip = side.top.min(body.saturating_sub(shown));
            let mut kept: Vec<Row> = rows[..head].to_vec();
            kept.extend(
                rows[head + skip..head + skip + shown.min(body)]
                    .iter()
                    .cloned(),
            );
            kept.extend(rows[rows.len() - tail..].iter().cloned());
            rows = kept;
        }
        let tall = rows.len() as u16 + fixed;
        let left = ((starts - f32::from(INSET)).round().max(0.0) as u16)
            .min(area.right().saturating_sub(wide + 1))
            .max(area.x + 1);
        let card_area = Rect::new(left, top, wide, tall);
        let first_buttons = card_area.bottom().saturating_sub(1 + lines.len() as u16);
        let placed = lines
            .iter()
            .enumerate()
            .map(|(n, labels)| {
                let row = Rect::new(card_area.x, first_buttons + n as u16, card_area.w, 1);
                let cells = self.card_buttons_right(row, labels);
                labels
                    .iter()
                    .zip(cells)
                    .map(|(label, (cells, x))| (*label, cells, x))
                    .collect()
            })
            .collect();
        let filled = match side.phase {
            Phase::Asking => Some(ASK),
            _ if side.field.is_some() => Some(ASK),
            Phase::Missing(_) => Some(SET_UP),
            _ => None,
        };
        Some(Card {
            area: card_area,
            rows,
            buttons: placed,
            filled,
        })
    }

    /// The card's buttons, in order, the one to press last.
    fn side_labels(&self, side: &Side) -> Vec<&'static str> {
        let quick = ask::QUICK.iter().map(|(label, _)| *label);
        match &side.phase {
            Phase::Asking => quick.chain([ASK]).collect(),
            _ if side.field.is_some() => vec![CANCEL, ASK],
            Phase::Waiting => vec![DISMISS],
            Phase::Answered => vec![COPY, FOLLOW_UP, MAKE_COMMENT, DISMISS],
            Phase::Failed(_) => vec![DISMISS, TRY_AGAIN],
            Phase::Missing(_) => vec![DISMISS, SET_UP],
        }
    }

    /// A field's rows: its words wrapped to its box, as a comment's are in theirs, at most
    /// `FIELD_ROWS` of them, the ones round the caret when there are more. Its words went on
    /// one row, past the box's right edge, where they couldn't be seen.
    fn field_rows(&self, field: &TextBox, hint: &str, width: f32) -> Vec<Row> {
        if field.is_empty() {
            return vec![Row::Field(hint.to_string(), true, Some(0), None, 0)];
        }
        let text = field.text();
        // Inside the box: its padding each side where there are pictures, and room at the
        // right for the caret and Vim's mode.
        let pad = measures::columns(COMMAND_PADDING, &*self.measure).unwrap_or(0.0);
        let mode = field
            .mode_word()
            .map_or(0.0, |word| self.measure.width(word, Font::default()) + 1.0);
        let inside = (width - pad * 2.0 - mode - 1.0).max(4.0);
        let spans = crate::threads::wrap_spans(text, inside, &*self.measure);
        let (caret, selected) = (field.caret(), field.selection());
        // At one line's end and the next one's start, it's at the start of the next.
        let on = spans
            .iter()
            .rposition(|(start, end)| *start <= caret && caret <= *end)
            .unwrap_or(0);
        let first = on
            .saturating_sub(FIELD_ROWS - 1)
            .min(spans.len().saturating_sub(FIELD_ROWS));
        spans
            .iter()
            .enumerate()
            .skip(first)
            .take(FIELD_ROWS)
            .map(|(index, &(start, end))| {
                let at = (index == on).then(|| caret - start);
                let in_line = selected
                    .map(|(a, b)| (a.clamp(start, end) - start, b.clamp(start, end) - start))
                    .filter(|(a, b)| a < b);
                Row::Field(text[start..end].to_string(), false, at, in_line, start)
            })
            .collect()
    }

    /// What the card says, row by row, its words set to `width` columns.
    fn side_rows(&self, side: &Side, width: f32) -> Vec<Row> {
        let measure = &*self.measure;
        let who = side.cli.map_or("", Cli::name).to_string();
        let shown_quote = {
            let words: Vec<&str> = side.quote.split_whitespace().collect();
            let mut shown = words.iter().take(6).copied().collect::<Vec<_>>().join(" ");
            if words.len() > 6 {
                shown.push('…');
            }
            shown
        };
        let mut rows = Vec::new();
        match &side.phase {
            Phase::Asking => {
                rows.push(Row::Head(format!("Ask {who}"), String::new()));
                let field = side.field.clone().unwrap_or_default();
                let hint = format!("Ask about ‘{shown_quote}’…");
                rows.extend(self.field_rows(&field, &hint, width));
                if side.main.is_some() {
                    rows.push(Row::Choice(side.copy));
                }
            }
            Phase::Missing(named) => {
                rows.push(Row::Head("Ask needs an agent".to_string(), String::new()));
                let why = match named {
                    Some(cli) => format!(
                        "The Default agent is {}, which isn't installed here. Install it, or \
                         pick another in Settings › Integrations.",
                        cli.name()
                    ),
                    None => "Ask runs Claude Code, Codex or Gemini CLI once in the background, \
                             and none of them is installed here."
                        .to_string(),
                };
                for line in threads::wrap(&why, width, measure) {
                    rows.push(Row::Words(line));
                }
            }
            phase => {
                rows.push(Row::Head(side.asked.clone(), who));
                let body = match phase {
                    Phase::Failed(why) => why.clone(),
                    _ => plain(&side.answer),
                };
                if body.trim().is_empty() {
                    let doing = if side.doing.is_empty() {
                        "Thinking…".to_string()
                    } else {
                        format!("{}…", side.doing)
                    };
                    rows.push(Row::Doing(doing));
                } else {
                    let lines = threads::wrap(body.trim(), width, measure);
                    let most = lines.len().min(ANSWER_ROWS.max(1));
                    let skip = side.top.min(lines.len().saturating_sub(most));
                    for line in lines.into_iter().skip(skip).take(most) {
                        rows.push(Row::Words(line));
                    }
                }
                if let Some(field) = &side.field {
                    rows.extend(self.field_rows(field, "Ask a follow-up…", width));
                }
            }
        }
        rows
    }

    /// The card, over the page and under any menu.
    pub(super) fn draw_side(&self, grid: &mut Grid) {
        let Some(card) = self.side_card() else {
            return;
        };
        let Some(side) = self.side.as_ref() else {
            return;
        };
        let theme = &self.theme;
        let area = card.area;
        // Words last, whatever's over its buttons: its panel and words stand the same as the
        // field gives way to the answer, where a field's card stood 7 px higher.
        let last = measures::DialogLast::Words;
        // Into its first column as a corner card's is (`card_lead`), so its words, two columns
        // into its cells, are 24 px in from its line, as its field and its buttons are.
        let lead = self.card_lead(area);
        self.draw_card_led(grid, area, (lead, last, 0.0));
        // What stands inside it, its field and a chip, `card_room` from its line, where there
        // are pictures: a column into its cells, its field stood 20 px from it (q1-ask-field).
        let inset = measures::columns(Self::card_room(), &*self.measure);
        let inside = inset.map(|inset| {
            let left = f32::from(area.x) + lead + inset;
            (left, f32::from(area.right()) - inset - left)
        });
        let words_from = grid.painted_so_far();
        let base = Style {
            fg: theme.text.fg,
            bg: theme.popup,
            ..theme.bar
        };
        let strong = Style { bold: true, ..base };
        let dim = Style {
            fg: theme.popup_dim,
            ..base
        };
        let x = area.x + INSET;
        let right = area.right().saturating_sub(INSET);
        let mut caret = None;
        for (n, row) in card.rows.iter().enumerate() {
            let y = area.y + 1 + n as u16;
            match row {
                Row::Head(text, who) => {
                    let running = side.running.is_some();
                    let who_wide = if who.is_empty() {
                        0
                    } else {
                        let font = Font::default();
                        let wide = self.measure.width(who, font).ceil() as u16;
                        wide + if running { 3 } else { 1 }
                    };
                    let end = right.saturating_sub(who_wide);
                    grid.label(x, y, text, strong, end);
                    if !who.is_empty() {
                        // Where its field and its buttons end, where there are pictures: two
                        // columns in, it ended 37 px inside the card's line where they end 25
                        // in (the release review of 0.12.0, q4-ask-follow-up).
                        let ends = inside.map_or(f32::from(right), |(left, wide)| left + wide);
                        grid.label_right_at(ends, right, y, who, dim, 100);
                        if running {
                            let font = Font::default();
                            let at = ends - self.measure.width(who, font) - 2.0;
                            grid.spinner(at, y, measures::WAIT_SPIN, theme.accent, base);
                        }
                    }
                }
                Row::Field(text, hint, at, selected, _) => {
                    // The field's lines stand under one box, drawn with its first.
                    let first = n == 0 || !matches!(card.rows[n - 1], Row::Field(..));
                    let lines = card.rows[n..]
                        .iter()
                        .take_while(|r| matches!(r, Row::Field(..)))
                        .count() as u16;
                    let last_line =
                        n + 1 == card.rows.len() || !matches!(card.rows[n + 1], Row::Field(..));
                    let wide_box = area.w.saturating_sub(INSET * 2 - 2);
                    let boxed = Rect::new(x - 1, y, wide_box, 1);
                    let well = Style {
                        fg: theme.text.fg,
                        bg: theme.popup_input(),
                        ..base
                    };
                    grid.fill(boxed, well);
                    // What's typed `COMMAND_PADDING` into it, as in a card's box for a command.
                    let start = match inside {
                        Some((left, wide)) => {
                            if first {
                                let air = cellix_render::FIELD_AIR;
                                let all = Rect::new(x - 1, y, wide_box, lines);
                                let ground = Bounds::inset(all, 0.0, 0.0);
                                grid.panel(ground, 0, Some(theme.popup), None);
                                // One line's box is shorter than its row, so it's drawn on the
                                // words' line's scale in the middle of the row; a taller one
                                // on the row's. Its edges stand where one line's would: at the
                                // row's scale they stood 2 px higher and 4 px nearer the last
                                // line's letters, and 4 px nearer the buttons (ui-expert).
                                let edge = match self.measure.cell_pixels() {
                                    Some((_, row)) if lines > 1 => {
                                        let row = row.max(1) as f32;
                                        let line = row / self.measure.row_air().max(1.0);
                                        let one = (
                                            cellix_render::units(air),
                                            cellix_render::units(1.0 - air * 2.0),
                                        );
                                        cellix_render::stood(one.0, one.1, row, line).0 / row
                                    }
                                    _ => air,
                                };
                                let tall = f32::from(lines) - edge * 2.0;
                                let bounds = Bounds::cells(left, f32::from(y) + edge, wide, tall);
                                let (fill, line) = (theme.popup_input(), theme.focus);
                                grid.panel(bounds, 22, Some(fill), Some(line));
                            }
                            let pad = measures::columns(COMMAND_PADDING, &*self.measure);
                            left + pad.unwrap_or(0.0)
                        }
                        None => {
                            if first {
                                let all = Rect::new(x - 1, y, wide_box, lines);
                                grid.field(all, theme.popup, theme.popup_input(), theme.focus);
                            }
                            f32::from(x)
                        }
                    };
                    let style = if *hint {
                        Style {
                            fg: theme.popup_dim,
                            ..well
                        }
                    } else {
                        well
                    };
                    let cell = start.floor() as u16;
                    // What's selected, as the file shows its selection.
                    let mark = |grid: &mut Grid, drawn| {
                        if let Some(range) = *selected {
                            let pair = (&*self.measure, theme);
                            let place = (start, cell, y);
                            let ground = theme.popup_input();
                            crate::palette::box_selection(
                                grid,
                                place,
                                text,
                                range,
                                (100, right, crate::palette::BOX_SELECTION_INSET),
                                ground,
                                pair,
                                drawn,
                            );
                        }
                    };
                    mark(grid, false);
                    grid.label_at(start, cell, y, text, style, right, 100);
                    mark(grid, true);
                    // Vim's mode at the field's right, on its last line, as the status bar
                    // says the file's.
                    if let Some(word) = side
                        .field
                        .as_ref()
                        .and_then(TextBox::mode_word)
                        .filter(|_| last_line)
                    {
                        let quiet = Style {
                            fg: theme.popup_dim,
                            ..well
                        };
                        let ends = inside.map_or(f32::from(right), |(left, wide)| left + wide);
                        grid.label_right_at(ends - 1.0, right, y, word, quiet, 80);
                    }
                    if let Some(at) = at {
                        let before = if *hint {
                            ""
                        } else {
                            &text[..(*at).min(text.len())]
                        };
                        let wide = self.measure.width(before, Font::default());
                        // Just before the hint, which starts where what's typed will, and just
                        // after the letter it follows, as the palette's field has it.
                        let past = if before.is_empty() { -0.15 } else { 0.05 };
                        caret = Some((start + wide + past, y));
                    }
                }
                Row::Choice(on) => {
                    // A chip, as a kind's is: outlined and quiet, in the accent's colours
                    // once chosen.
                    let font = Font::default();
                    let wide = self.measure.width(WITH_AGENT, font);
                    // On the field's edge, where there are pictures.
                    let start = inside.map_or(f32::from(x) - 0.2, |(left, _)| left);
                    let bounds = Bounds::cells(start, f32::from(y) + 0.12, wide + 1.4, 0.76);
                    if *on {
                        grid.panel(bounds, 34, Some(theme.tint), Some(theme.accent));
                    } else {
                        grid.panel(bounds, 34, None, Some(theme.popup_border));
                    }
                    let style = if *on { base } else { dim };
                    let at = start + 0.7;
                    grid.label_at(at, at.floor() as u16, y, WITH_AGENT, style, right, 100);
                }
                Row::Words(text) => {
                    let style = if matches!(side.phase, Phase::Failed(_) | Phase::Missing(_)) {
                        dim
                    } else {
                        base
                    };
                    grid.label(x, y, text, style, right);
                }
                Row::Doing(text) => {
                    grid.label(x, y, text, dim, right);
                }
            }
        }
        let lift = self.card_lift(last, 0.0);
        grid.lift_since(words_from, lift);
        if let Some((cx, cy)) = caret {
            grid.set_caret(cx, cy, 0.62);
            // The caret takes the colour of the cell it's in, which before a hint is the hint's:
            // it's the words' colour, as in every box (the ui-expert's review of q1-ask-field).
            if !self.measure.in_cells() {
                let cell = Rect::new(cx.max(0.0) as u16, cy, 1, 1);
                grid.set_style(cell, |style| Style {
                    fg: theme.text.fg,
                    ..style
                });
            }
            // Raised with the words it's among.
            if let Some(caret) = &mut grid.caret {
                caret.y -= cellix_render::units(lift);
            }
        }
        for (n, row) in card.buttons.iter().enumerate() {
            let y = card.button_row() + n as u16;
            let labels: Vec<&str> = row.iter().map(|(label, ..)| *label).collect();
            let placed: Vec<(String, u16)> = row
                .iter()
                .map(|(_, cells, x)| (cells.clone(), *x))
                .collect();
            let last_row = n + 1 == card.buttons.len();
            let filled = last_row && card.filled.is_some_and(|f| labels.last() == Some(&f));
            self.draw_card_buttons(grid, (y, 0.0), &placed, (&labels, filled));
        }
    }
}
