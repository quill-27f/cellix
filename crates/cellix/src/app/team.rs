//! Agents starting, messaging and waiting for each other, as `cellix agent` asks: the app's
//! side of `cellix_agent::team`. A call is answered at once or kept until it can be: a start
//! waits for the person's yes and for the agent to be ready, a message for the person to stop
//! typing into that agent, and a wait for the agent to stop working. Each is looked at again
//! on every poll, and each has a deadline, so a caller always hears back.

use std::time::{Duration, Instant};

use cellix_agent::team::{self, Answer, Asked, Call, Caller, Children, code};
use cellix_agent::{AgentEvent, AgentState};

use super::*;
use crate::dock::Gone;
use crate::hold::Member;

/// How long after a person's last key in an agent a message to it waits.
const QUIET: Duration = Duration::from_secs(3);
/// How long a prompt the person has begun holds a message back, from their last key.
const DRAFT: Duration = Duration::from_secs(60);
/// Between a message's paste and its Enter, so the program sees them apart.
const ENTER_GAP: Duration = Duration::from_millis(200);
/// How long a program that says nothing of itself must have been quiet to be ready.
const SETTLED: Duration = Duration::from_millis(800);
/// How long a program has, once a message is entered, to say it took it before what it said
/// last counts again.
const TAKING: Duration = Duration::from_secs(5);
/// How long a start waits for the person to say yes.
const CONSENT_WAIT: Duration = Duration::from_secs(90);
/// The consent card's buttons, the last the one to press.
pub(super) const CONSENT_BUTTONS: [&str; 3] = ["Deny", "Always Allow Here", "Allow"];
/// The widest the consent card stands.
const CONSENT_WIDTH: u16 = 64;
/// Lines of a screen `read` gives when not told how many.
const SCREEN_LINES: usize = 60;
/// What a helper's row in the Agents list is named by before its hooks say its session.
pub(crate) const MEMBER_ROW: &str = "member:";

/// What an agent is doing, as its hooks last said.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Doing {
    /// Started, and hasn't said anything yet.
    Starting,
    Working,
    /// Asking the person something: leave to run a command, a question, a plan.
    Asking(String),
    /// Its turn is over, or it hasn't had one.
    Done(String),
    /// Its turn stopped on an error.
    Failed(String),
    Exited,
    /// A program that says nothing of itself, which is working until it ends.
    Running,
}

impl Doing {
    pub(crate) fn word(&self) -> &'static str {
        match self {
            Doing::Starting => "starting",
            Doing::Working => "working",
            Doing::Asking(_) => "asking",
            Doing::Done(_) => "done",
            Doing::Failed(_) => "error",
            Doing::Exited => "exited",
            Doing::Running => "running",
        }
    }

    pub(crate) fn working(&self) -> bool {
        matches!(self, Doing::Starting | Doing::Working | Doing::Running)
    }

    pub(crate) fn detail(&self) -> &str {
        match self {
            Doing::Asking(d) | Doing::Done(d) | Doing::Failed(d) => d,
            _ => "",
        }
    }
}

/// An agent to start, as a run asked.
#[derive(Clone, Debug)]
pub(super) struct Start {
    pub(super) name: String,
    kind: String,
    program: Vec<String>,
    folder: PathBuf,
    pub(super) task: Option<String>,
    parent: Option<String>,
    outlive: bool,
    /// Who asked, as the card names them.
    pub(super) by: String,
    /// Where it runs, as the card says it: None for this folder. Worked out as the start is
    /// asked for, so drawing the card asks nothing of the disk.
    place: Option<String>,
}

impl Start {
    /// What the card calls what it starts: the agent's name, or the command.
    pub(super) fn what(&self) -> String {
        match self.kind.as_str() {
            "command" => {
                let mut words = self.program.iter();
                let program = words.next().map(|p| {
                    let name = Path::new(p).file_name().and_then(|n| n.to_str());
                    name.unwrap_or(p).to_string()
                });
                program
                    .into_iter()
                    .chain(words.cloned())
                    .collect::<Vec<_>>()
                    .join(" ")
            }
            kind => crate::agents::display_name(kind).to_string(),
        }
    }

    fn agent(&self) -> bool {
        self.kind != "command"
    }
}

/// A call still under way.
pub(super) struct Pending {
    /// The caller's own session, by name, when it has one.
    caller: Option<String>,
    reply: Sender<String>,
    until: Instant,
    pub(super) step: Step,
    /// A note Cellix types into a parent of its own accord, which nobody waits on.
    note: bool,
}

impl Pending {
    /// The agent a message is on its way to.
    fn target(&self) -> Option<&str> {
        match &self.step {
            Step::Message { name, .. } => Some(name),
            _ => None,
        }
    }

    /// Whether this is the caller `parent` waiting on its child `child`, or on its children.
    fn waits_on(&self, parent: &str, child: &str) -> bool {
        self.caller.as_deref() == Some(parent)
            && match &self.step {
                Step::Wait {
                    on: On::One(name), ..
                } => name == child,
                Step::Wait {
                    on: On::Children(_),
                    ..
                } => true,
                _ => false,
            }
    }
}

pub(super) enum Step {
    /// Answered: nothing is left to do.
    Answered,
    /// Waiting for the person to say whether this may start.
    Consent(Start),
    /// Started, and waiting to be ready for its task.
    Ready {
        name: String,
        kind: String,
        task: Option<String>,
    },
    /// A message waiting for the person to stop typing, and then for its Enter.
    Message {
        name: String,
        text: String,
        force: bool,
        typed: Option<Instant>,
        then: Then,
    },
    /// Waiting for an agent, or the caller's children, to stop working.
    Wait {
        on: On,
        /// How many messages the caller had had when it began, as one more ends it.
        messages: u64,
        /// When the message it waits on went in, after `send --wait`.
        sent: Option<Instant>,
    },
}

/// What a message does once it's in.
pub(super) enum Then {
    Reply(Value),
    Wait,
}

pub(super) enum On {
    One(String),
    Children(Children),
}

/// The agents of this workspace as each other's, and the calls between them under way.
pub(crate) struct Team {
    pending: Vec<Pending>,
    /// Starts waiting for the person's yes; the first one's card shows.
    pub(super) asking: VecDeque<Pending>,
    /// What each named session last said through its hooks, and when it came.
    heard: HashMap<String, (AgentEvent, Instant)>,
    /// Messages Cellix has typed into each named session, which end a wait it's in.
    messages: HashMap<String, u64>,
    /// What closed agents last showed, by name, for a read after their tabs are gone.
    gone: HashMap<String, Gone>,
    /// Whether each member was working at the last look, and how many times it has stopped
    /// since it started, which is news for its parent.
    was_working: HashMap<String, bool>,
    stops: HashMap<String, u64>,
    /// How many of a child's stops its parent has heard of, by (parent, child).
    heard_by: HashMap<(String, String), u64>,
    /// What each member was doing at the last look, by its word, so a change is seen once.
    was: HashMap<String, &'static str>,
    /// What each member did that its parent should hear, counted, with the note that says so:
    /// it needs the person, it stopped on an error, or it ended.
    alerts: HashMap<String, (u64, String)>,
    /// How many of a child's alerts its parent has been told of, or saw, by (parent, child).
    told: HashMap<(String, String), u64>,
    /// Agents here may start others without asking, as the person said.
    pub(super) always: bool,
    /// How long a message waits after the person's last key, and for a prompt they've begun.
    pub(super) quiet: Duration,
    pub(super) draft: Duration,
    /// The name the next agent started is to have, if it's free: the reviewer's.
    pub(super) wanted: std::cell::RefCell<Option<String>>,
}

impl Default for Team {
    fn default() -> Self {
        Self {
            pending: Vec::new(),
            asking: VecDeque::new(),
            heard: HashMap::new(),
            messages: HashMap::new(),
            gone: HashMap::new(),
            was_working: HashMap::new(),
            stops: HashMap::new(),
            heard_by: HashMap::new(),
            was: HashMap::new(),
            alerts: HashMap::new(),
            told: HashMap::new(),
            always: false,
            quiet: QUIET,
            draft: DRAFT,
            wanted: std::cell::RefCell::new(None),
        }
    }
}

impl Team {
    /// Whether `parent` has news of `child` it hasn't heard: a stop since it last did.
    fn unheard(&self, parent: &str, child: &str) -> bool {
        let stops = self.stops.get(child).copied().unwrap_or(0);
        let key = (parent.to_string(), child.to_string());
        stops > self.heard_by.get(&key).copied().unwrap_or(0)
    }

    /// `parent` has heard everything so far of `child`.
    fn heard_of(&mut self, parent: &str, child: &str) {
        let key = (parent.to_string(), child.to_string());
        let stops = self.stops.get(child).copied().unwrap_or(0);
        self.heard_by.insert(key.clone(), stops);
        let alerts = self.alerts.get(child).map_or(0, |(count, _)| *count);
        self.told.insert(key, alerts);
    }

    /// Notes what `child` did that its parent should hear.
    fn alert(&mut self, child: &str, text: String) {
        let alert = self.alerts.entry(child.to_string()).or_default();
        *alert = (alert.0 + 1, text);
    }
}

/// What kind of thing an agent asking the person wants, in a word or two, from what its hook
/// said: leave to run something, a plan to look at, or a question.
fn asked_for(detail: &str) -> &'static str {
    let lower = detail.to_lowercase();
    if detail == cellix_agent::hook::PLAN_WAITING {
        "a plan to review"
    } else if lower.contains("permission")
        || lower.starts_with("allow")
        || lower.contains("asks to run")
    {
        "permission"
    } else {
        "a question"
    }
}

/// What a parent is told of a child that can't tell it itself: it needs the person, it
/// stopped on an error, or it ended.
fn alert_of(name: &str, doing: &Doing) -> Option<String> {
    let read = format!("cellix agent read {name}");
    match doing {
        Doing::Asking(what) => Some(format!(
            "{name} needs you ({}). See: {read}",
            asked_for(what)
        )),
        Doing::Failed(why) => {
            let why = why.trim_start_matches("Stopped: ");
            Some(format!("{name} stopped on an error ({why}). See: {read}"))
        }
        Doing::Exited => Some(format!("{name} exited. Read: {read}")),
        _ => None,
    }
}

fn answer(reply: &Sender<String>, answer: Answer) {
    let _ = reply.send(answer.line());
}

fn not_found(name: &str) -> Answer {
    Answer::fail(
        code::NOT_FOUND,
        format!("no agent here is called {name}; cellix agent ls lists them"),
    )
}

/// A program's name as it can stand in an agent's: lowercase, with what isn't a letter or
/// digit made a dash.
fn name_part(program: &str) -> String {
    let base = Path::new(program)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("command");
    let mut part: String = base
        .chars()
        .map(|c| {
            let c = c.to_ascii_lowercase();
            if c.is_ascii_lowercase() || c.is_ascii_digit() {
                c
            } else {
                '-'
            }
        })
        .take(24)
        .collect();
    if !part.starts_with(|c: char| c.is_ascii_lowercase()) {
        part.insert(0, 'p');
    }
    part
}

impl App {
    /// The session other agents call `name`.
    pub(crate) fn member_index(&self, name: &str) -> Option<usize> {
        self.dock
            .sessions
            .iter()
            .position(|s| s.called() == Some(name))
    }

    /// The session a call came from, by where it's held or else by its name.
    fn caller_index(&self, from: &Caller) -> Option<usize> {
        let held = from.held.as_deref().and_then(|held| {
            let mut sessions = self.dock.sessions.iter();
            sessions.position(|s| s.socket().is_some_and(|socket| socket == Path::new(held)))
        });
        held.or_else(|| from.name.as_deref().and_then(|n| self.member_index(n)))
    }

    /// A name for a new member of `base`'s kind no session here has, nor any that closed.
    fn free_name(&self, base: &str) -> String {
        let taken =
            |name: &str| self.member_index(name).is_some() || self.team.gone.contains_key(name);
        (1..)
            .map(|n| format!("{base}-{n}"))
            .find(|name| !taken(name))
            .expect("a free number")
    }

    /// What a session about to start is among the agents here: a name for others to reach it
    /// by, and its kind. Shells and the browser are nobody's to message.
    pub(super) fn member_for(&self, display: &str, argv: &[String]) -> Option<Member> {
        if display == crate::agents::SHELL.0 || display == BROWSER {
            return None;
        }
        let kind = match crate::agents::program_of(display) {
            Some(program) => program.to_string(),
            None => team::kind_of(argv.first().map_or("", String::as_str)).to_string(),
        };
        let base = match kind.as_str() {
            "command" => name_part(argv.first().map_or("command", String::as_str)),
            agent => agent.to_string(),
        };
        // A name asked for, as the reviewer's, when it's free; numbered after it when not.
        let free =
            |name: &str| self.member_index(name).is_none() && !self.team.gone.contains_key(name);
        let name = match self.team.wanted.take() {
            Some(wanted) if free(&wanted) => wanted,
            Some(wanted) => self.free_name(&wanted),
            None => self.free_name(&base),
        };
        Some(Member {
            name,
            kind,
            parent: None,
            outlive: false,
        })
    }

    /// Takes in what an agent's hooks said, for the session it runs in: by the name Cellix
    /// gave that session, and only from the session's own program, not another agent it ran.
    pub(super) fn heard_from(&mut self, event: &AgentEvent) {
        if event.name.is_empty() {
            return;
        }
        let Some(index) = self.member_index(&event.name) else {
            return;
        };
        let own = self.dock.sessions[index].program_pid();
        if let (Some(said), Some(own)) = (event.pid, own)
            && said != own
        {
            return;
        }
        self.team
            .heard
            .insert(event.name.clone(), (event.clone(), Instant::now()));
        self.look_at(index);
    }

    /// What names a member's row in the Agents list: its agent's own session, once its hooks
    /// have said, or else its name.
    fn row_key(session: &Session) -> Option<String> {
        let called = session.called()?;
        Some(
            session
                .agent_session
                .clone()
                .unwrap_or_else(|| format!("{MEMBER_ROW}{called}")),
        )
    }

    /// The row a helper stands under in the Agents list: the agent that started it.
    pub(super) fn parent_row(&self, session: &Session) -> Option<String> {
        let parent = session.member.as_ref()?.parent.as_deref()?;
        let index = self.member_index(parent)?;
        Self::row_key(&self.dock.sessions[index])
    }

    /// Rows for helpers that haven't said anything through hooks yet, or never will, as a
    /// command doesn't, and for agents with helpers that haven't: they stand where they
    /// belong all the same, doing what Cellix knows.
    pub(super) fn helper_rows(&self) -> Vec<crate::agents::Row> {
        use crate::agents::{Row, Urgency};
        let has_helpers = |s: &Session| {
            let called = s.called();
            let mut members = self.dock.sessions.iter().filter_map(|c| c.member.as_ref());
            called.is_some_and(|name| members.any(|m| m.parent.as_deref() == Some(name)))
        };
        let helpers = self.dock.sessions.iter().enumerate().filter(|(_, s)| {
            let placed = self.parent_row(s).is_some() || has_helpers(s);
            !s.exited && s.agent_session.is_none() && placed
        });
        helpers
            .filter_map(|(index, session)| {
                let doing = self.doing(index);
                // As an agent's hooks say it: a turn it finished is done, a dot at its row's
                // end, as one with a tab has it until that's in front. One ready for a turn
                // it hasn't had is done too, as peek's header says: its row stood with no
                // mark under a heading that counted it (172).
                let urgency = match &doing {
                    Doing::Asking(_) => Urgency::NeedsYou,
                    Doing::Done(_) => Urgency::Done,
                    doing if doing.working() => Urgency::Working,
                    _ => Urgency::Idle,
                };
                Some(Row {
                    icon: session.icon(),
                    // A helper by the name it was given; its agent by its tab's.
                    name: match self.parent_row(session) {
                        Some(_) => session.called()?.to_string(),
                        None => session.label(),
                    },
                    urgency,
                    since: None,
                    detail: doing.detail().to_string(),
                    session: Self::row_key(session)?,
                    parent: self.parent_row(session),
                })
            })
            .collect()
    }

    /// Takes in what the program in tab `index` said of itself (OSC 7501), as its hooks' word
    /// would be: `clear` takes back what it said.
    pub(super) fn heard_report(&mut self, index: usize, event: &AgentEvent, clear: bool) {
        let Some(name) = self.dock.sessions[index].called().map(str::to_string) else {
            return;
        };
        if clear {
            self.team.heard.remove(&name);
            return;
        }
        let said = AgentEvent {
            name: name.clone(),
            ..event.clone()
        };
        self.team.heard.insert(name, (said, Instant::now()));
        self.look_at(index);
    }

    /// Notes what has changed in what the member in tab `index` is doing since the last look:
    /// a stop from working, and asking, failing or ending, each news for its parent once each
    /// time it happens. Looked at as each report comes, so two in a row are both seen.
    fn look_at(&mut self, index: usize) {
        let Some(name) = self.dock.sessions[index].called().map(str::to_string) else {
            return;
        };
        let doing = self.doing(index);
        let working = doing.working();
        if self.team.was_working.insert(name.clone(), working) == Some(true) && !working {
            *self.team.stops.entry(name.clone()).or_default() += 1;
        }
        let word = doing.word();
        if self.team.was.insert(name.clone(), word) != Some(word)
            && let Some(text) = alert_of(&name, &doing)
        {
            self.team.alert(&name, text);
        }
    }

    /// What the agent in tab `index` is doing.
    pub(crate) fn doing(&self, index: usize) -> Doing {
        let Some(session) = self.dock.sessions.get(index) else {
            return Doing::Exited;
        };
        if session.exited {
            return Doing::Exited;
        }
        let Some(member) = &session.member else {
            return Doing::Running;
        };
        let heard = self.team.heard.get(&member.name);
        // A message just went in: it's taking it until it says otherwise.
        if let Some(at) = session.prompted
            && at.elapsed() < TAKING
            && heard.is_none_or(|(_, when)| *when < at)
        {
            return Doing::Working;
        }
        match heard {
            Some((event, _)) => match event.state {
                AgentState::Working => Doing::Working,
                AgentState::NeedsYou => Doing::Asking(event.detail.clone()),
                AgentState::Idle if event.detail.starts_with("Stopped") => {
                    Doing::Failed(event.detail.clone())
                }
                AgentState::Idle => Doing::Done(event.detail.clone()),
                AgentState::Ended => Doing::Exited,
            },
            // An agent that hasn't said it's up after the longest a start waits for it has no
            // hooks to say anything with.
            None if member.kind == "command" => Doing::Running,
            None if session.born.elapsed() > Duration::from_secs(team::READY_MOST) => {
                Doing::Running
            }
            None => Doing::Starting,
        }
    }

    /// Answers a call from `cellix agent`, at once or once it can.
    pub(crate) fn team_call(&mut self, asked: Asked, reply: Sender<String>) {
        let Asked { from, call } = asked;
        let caller = self
            .caller_index(&from)
            .and_then(|i| self.dock.sessions[i].called().map(str::to_string));
        let pending = |step, until| Pending {
            caller: caller.clone(),
            reply: reply.clone(),
            until,
            step,
            note: false,
        };
        match call {
            Call::Ls => answer(&reply, Answer::ok(self.team_list())),
            Call::Read { name, lines } => {
                let read = self.team_read(&name, lines, &caller);
                answer(&reply, read);
            }
            Call::Stop { name } => match self.member_index(&name) {
                Some(index) => {
                    self.dock.sessions[index].stop();
                    let by = caller.as_deref().unwrap_or("an agent");
                    self.say(format!("{name} was stopped by {by}"));
                    answer(&reply, Answer::ok(json!({"stopped": name})));
                }
                None => answer(&reply, not_found(&name)),
            },
            Call::Send {
                name,
                text,
                wait,
                force,
                timeout,
            } => {
                let Some(index) = self.member_index(&name) else {
                    return answer(&reply, not_found(&name));
                };
                if self.dock.sessions[index].exited {
                    let error = format!(
                        "{name} has ended; read what it said with cellix agent read {name}"
                    );
                    return answer(&reply, Answer::fail(code::FAILED, error));
                }
                if caller.as_deref() == Some(name.as_str()) {
                    let error = "an agent can't send to itself";
                    return answer(&reply, Answer::fail(code::USAGE, error));
                }
                if let Some(refused) = self.refuse_asking(index, &name, force) {
                    return answer(&reply, refused);
                }
                let text = self.as_sent(&caller, index, &text);
                let then = if wait {
                    Then::Wait
                } else {
                    Then::Reply(json!({"sent": name}))
                };
                let step = Step::Message {
                    name,
                    text,
                    force,
                    typed: None,
                    then,
                };
                let until = Instant::now() + Duration::from_secs(timeout);
                let mut sending = pending(step, until);
                if !self.progress(&mut sending) {
                    self.team.pending.push(sending);
                }
            }
            Call::Wait {
                name,
                children,
                timeout,
            } => {
                let on = match (name, children) {
                    (Some(name), _) => {
                        let known = self.member_index(&name).is_some()
                            || self.team.gone.contains_key(&name);
                        if !known {
                            return answer(&reply, not_found(&name));
                        }
                        On::One(name)
                    }
                    (None, Some(children)) => {
                        if caller.is_none() {
                            let error = "only an agent Cellix started has children to wait for";
                            return answer(&reply, Answer::fail(code::USAGE, error));
                        }
                        On::Children(children)
                    }
                    (None, None) => {
                        return answer(&reply, Answer::fail(code::USAGE, team::usage()));
                    }
                };
                let step = Step::Wait {
                    on,
                    messages: self.messages_of(&caller),
                    sent: None,
                };
                let until = Instant::now() + Duration::from_secs(timeout);
                let mut waiting = pending(step, until);
                // Already not working: that's the answer.
                if !self.progress(&mut waiting) {
                    self.team.pending.push(waiting);
                }
            }
            Call::Run {
                name,
                cwd,
                program,
                task,
                outlive_parent,
            } => {
                let planned = self.plan_start(name, &cwd, program, task, outlive_parent, &from);
                let start = match planned {
                    Ok(start) => start,
                    Err(refused) => return answer(&reply, refused),
                };
                // The person is asked the first time an agent starts another here.
                let agent_asks = match self.caller_index(&from) {
                    Some(index) => self.dock.sessions[index].member.is_some(),
                    None => from.agent,
                };
                if agent_asks && !self.team.always {
                    // It waits with what else waits for the person, so it reaches them as a
                    // notification when Cellix isn't in front, or this workspace isn't.
                    let parent = self
                        .caller_index(&from)
                        .map(|i| self.dock.sessions[i].name.clone());
                    self.inbox.note(Waiting {
                        kind: Waits::Permission,
                        key: format!("start:{}", start.name),
                        title: format!("{} wants to start {}", start.by, start.what()),
                        detail: "Allow or deny it in the card".to_string(),
                        since: Instant::now(),
                        target: WaitTarget::Session(parent.unwrap_or_default()),
                    });
                    let until = Instant::now() + CONSENT_WAIT;
                    self.team
                        .asking
                        .push_back(pending(Step::Consent(start), until));
                    return;
                }
                let mut started = pending(Step::Consent(start), Instant::now());
                self.begin(&mut started);
                if !self.progress(&mut started) {
                    self.team.pending.push(started);
                }
            }
        }
    }

    /// The text of a message as it arrives: from an agent to an agent, marked with who sent it.
    fn as_sent(&self, caller: &Option<String>, to: usize, text: &str) -> String {
        let to_agent = self.dock.sessions[to]
            .member
            .as_ref()
            .is_some_and(|m| m.kind != "command");
        match caller {
            Some(sender) if to_agent => team::tagged(sender, text),
            _ => text.to_string(),
        }
    }

    /// Refuses to type into an agent that's asking the person something, since what's typed
    /// would answer it, unless forced.
    fn refuse_asking(&self, index: usize, name: &str, force: bool) -> Option<Answer> {
        let Doing::Asking(what) = self.doing(index) else {
            return None;
        };
        if force {
            return None;
        }
        let what = if what.is_empty() {
            "something".to_string()
        } else {
            what
        };
        let error = format!(
            "{name} is asking the person: {what}. Nothing was typed, since it would answer \
             that. Leave the answer to them; read it with cellix agent read {name}"
        );
        let out = json!({"name": name, "state": "asking", "detail": what});
        Some(Answer::fail(code::ASKING, error).with(out))
    }

    /// What the card calls the folder a start runs in: None for this one.
    fn place_of(&self, folder: &Path) -> Option<String> {
        let root = self.workspace.root();
        let here = dunce::canonicalize(root).is_ok_and(|root| root == folder);
        (!here && folder != root).then(|| self.relative(folder))
    }

    /// Checks a run and works out what it starts, without starting anything.
    fn plan_start(
        &self,
        name: Option<String>,
        cwd: &str,
        program: Vec<String>,
        task: Option<String>,
        outlive: bool,
        from: &Caller,
    ) -> Result<Start, Answer> {
        let Ok(folder) = dunce::canonicalize(cwd)
            .map_err(|_| ())
            .and_then(|f| if f.is_dir() { Ok(f) } else { Err(()) })
        else {
            return Err(Answer::fail(code::USAGE, format!("{cwd} isn't a folder")));
        };
        let Some(first) = program.first() else {
            return Err(Answer::fail(code::USAGE, "run needs a program"));
        };
        let kind = team::kind_of(first).to_string();
        let name = match name {
            Some(name) if self.member_index(&name).is_some() => {
                let error = format!("an agent here is already called {name}: give another --name");
                return Err(Answer::fail(code::USAGE, error));
            }
            Some(name) => name,
            None if kind == "command" => self.free_name(&name_part(first)),
            None => self.free_name(&kind),
        };
        let caller = self.caller_index(from);
        let parent = caller.and_then(|i| self.dock.sessions[i].called().map(str::to_string));
        let someone = if from.agent { "An agent" } else { "You" };
        // The person knows an agent by its tab, as it's called there.
        let tab = caller.filter(|_| parent.is_some());
        let by = tab.map_or_else(|| someone.to_string(), |i| self.dock.sessions[i].label());
        let place = self.place_of(&folder);
        Ok(Start {
            name,
            kind,
            program,
            folder,
            task,
            by,
            parent,
            outlive,
            place,
        })
    }

    /// Starts what `pending` holds, now the person has said yes or needn't, and turns it into
    /// a wait for it to be ready.
    fn begin(&mut self, pending: &mut Pending) {
        let Step::Consent(start) = &pending.step else {
            return;
        };
        let start = start.clone();
        match self.start_member(&start) {
            Ok(()) => {
                pending.until = Instant::now() + Duration::from_secs(team::READY_MOST);
                pending.step = Step::Ready {
                    name: start.name.clone(),
                    kind: start.kind.clone(),
                    task: start.task.clone(),
                };
                let what = start.what();
                self.say(format!("{} started {} ({what})", start.by, start.name));
            }
            Err(e) => {
                pending.step = Step::Answered;
                let error = format!("could not start {}: {e}", start.what());
                answer(&pending.reply, Answer::fail(code::FAILED, error));
            }
        }
    }

    /// Starts an agent or command in a session of its own, without taking the keyboard or the
    /// pane from what the person is doing.
    fn start_member(&mut self, start: &Start) -> io::Result<()> {
        let (display, mut env, argv) = if start.agent() {
            let (env, launch) = self.agent_launch(&start.kind);
            let display = crate::agents::display_name(&start.kind).to_string();
            (display, env, launch)
        } else {
            let mut env = Vec::new();
            if let Some(path) = cellix_on_path() {
                env.push(format!("PATH={path}"));
            }
            (start.name.clone(), env, start.program.clone())
        };
        env.push(format!("{}={}", team::NAME_VAR, start.name));
        let member = Member {
            name: start.name.clone(),
            kind: start.kind.clone(),
            parent: start.parent.clone(),
            outlive: start.outlive,
        };
        self.team.gone.remove(&start.name);
        let index = self.spawn_session(&display, &argv, &env, &start.folder, Some(member))?;
        // A helper has no tab of its own: it stands under its agent in the Agents list, and
        // is looked at there, over the pane.
        self.dock.sessions[index].tabbed = start.parent.is_none();
        Ok(())
    }

    /// Moves `pending` on as far as it can go now. Returns whether it's answered.
    fn progress(&mut self, pending: &mut Pending) -> bool {
        let step = std::mem::replace(&mut pending.step, Step::Answered);
        match self.advance(pending, step) {
            Some(next) => {
                pending.step = next;
                false
            }
            None => true,
        }
    }

    /// One step of a call: what it waits on next, or None once it's answered.
    fn advance(&mut self, pending: &mut Pending, step: Step) -> Option<Step> {
        match step {
            Step::Answered => None,
            Step::Consent(start) => Some(Step::Consent(start)),
            Step::Ready { name, kind, task } => self.ready(pending, name, kind, task),
            Step::Message {
                name,
                text,
                force,
                typed,
                then,
            } => self.type_message(pending, (name, text, force, typed), then),
            Step::Wait { on, messages, sent } => {
                if self.messages_of(&pending.caller) > messages {
                    let note = "a message came for you: it's your next input";
                    let out = json!({"reason": "message", "note": note});
                    answer(&pending.reply, Answer::ok(out));
                    return None;
                }
                let result = match &on {
                    On::One(name) => self.stopped_one(name, sent),
                    On::Children(children) => {
                        let caller = pending.caller.clone();
                        self.stopped_children(caller.as_deref(), *children)
                    }
                };
                if let Some(result) = result {
                    answer(&pending.reply, result);
                    return None;
                }
                if Instant::now() >= pending.until {
                    let what = match &on {
                        On::One(name) => format!("cellix agent wait {name}"),
                        On::Children(Children::All) => "cellix agent wait --children all".into(),
                        On::Children(Children::Any) => "cellix agent wait --children any".into(),
                    };
                    let error = format!("still working; wait again: {what}");
                    let out = json!({"reason": "timeout", "state": "working"});
                    answer(
                        &pending.reply,
                        Answer::fail(code::STILL_WORKING, error).with(out),
                    );
                    return None;
                }
                Some(Step::Wait { on, messages, sent })
            }
        }
    }

    /// A message on its way in: held while the person types there, refused while the agent
    /// asks them something, pasted, and entered a moment later.
    fn type_message(
        &mut self,
        pending: &mut Pending,
        (name, text, force, typed): (String, String, bool, Option<Instant>),
        then: Then,
    ) -> Option<Step> {
        let Some(index) = self.member_index(&name) else {
            let error = format!("{name} closed before the message went in");
            answer(&pending.reply, Answer::fail(code::FAILED, error));
            return None;
        };
        let (quiet, draft) = (self.team.quiet, self.team.draft);
        let Some(at) = typed else {
            if let Some(refused) = self.refuse_asking(index, &name, force) {
                answer(&pending.reply, refused);
                return None;
            }
            let session = &mut self.dock.sessions[index];
            let typed = if session.typed.hold(quiet, draft).is_zero() {
                session.deliver(&text);
                Some(Instant::now())
            } else if Instant::now() >= pending.until {
                let error = format!(
                    "the person kept typing in {name}, so nothing was typed; try again later"
                );
                answer(&pending.reply, Answer::fail(code::FAILED, error));
                return None;
            } else {
                None
            };
            let step = Step::Message {
                name,
                text,
                force,
                typed,
                then,
            };
            return Some(step);
        };
        if at.elapsed() < ENTER_GAP {
            let step = Step::Message {
                name,
                text,
                force,
                typed,
                then,
            };
            return Some(step);
        }
        self.dock.sessions[index].submit();
        *self.team.messages.entry(name.clone()).or_default() += 1;
        match then {
            Then::Reply(out) => {
                answer(&pending.reply, Answer::ok(out));
                None
            }
            Then::Wait => {
                let wait = Step::Wait {
                    on: On::One(name),
                    messages: self.messages_of(&pending.caller),
                    sent: Some(Instant::now()),
                };
                self.advance(pending, wait)
            }
        }
    }

    /// How many messages Cellix has typed into the caller's own session.
    fn messages_of(&self, caller: &Option<String>) -> u64 {
        let count = caller.as_ref().and_then(|c| self.team.messages.get(c));
        count.copied().unwrap_or(0)
    }

    /// A started agent waiting to be ready for its task.
    fn ready(
        &mut self,
        pending: &mut Pending,
        name: String,
        kind: String,
        task: Option<String>,
    ) -> Option<Step> {
        let Some(index) = self.member_index(&name) else {
            let error = format!("{name} closed as it started");
            answer(&pending.reply, Answer::fail(code::FAILED, error));
            return None;
        };
        let started = json!({"name": name, "kind": kind, "started": true});
        let not_sent = || {
            let mut out = started.clone();
            out["task_sent"] = json!(false);
            out
        };
        let session = &self.dock.sessions[index];
        if session.exited {
            let said = session
                .lines()
                .into_iter()
                .rev()
                .find(|l| !l.trim().is_empty());
            let said = said.map(|l| format!(": {}", l.trim())).unwrap_or_default();
            let error = format!("{name} ended as it started{said}");
            answer(
                &pending.reply,
                Answer::fail(code::FAILED, error).with(started),
            );
            return None;
        }
        let ready = match self.doing(index) {
            Doing::Asking(what) => {
                let error = format!(
                    "{name} started and is asking the person something first ({what}). Its task \
                     wasn't typed in; once they've answered, cellix agent send {name} <task>"
                );
                let out = if task.is_some() { not_sent() } else { started };
                answer(&pending.reply, Answer::fail(code::ASKING, error).with(out));
                return None;
            }
            // An agent says when it's ready; anything else is, once its output has stopped a
            // moment.
            Doing::Starting => false,
            Doing::Running => kind == "command" && session.settled(SETTLED),
            _ => true,
        };
        if !ready {
            if Instant::now() < pending.until {
                return Some(Step::Ready { name, kind, task });
            }
            let display = crate::agents::display_name(&kind);
            let note = format!(
                "{name} hasn't said it's ready after {} seconds, so its task wasn't typed in. It \
                 may be asking the person something, such as whether to trust this folder, or \
                 {display}'s Cellix hooks aren't on. Read it with cellix agent read {name} \
                 before sending the task",
                team::READY_MOST
            );
            let out = if task.is_some() { not_sent() } else { started };
            answer(&pending.reply, Answer::ok(out).noting(note));
            return None;
        }
        let Some(task) = task else {
            answer(&pending.reply, Answer::ok(started));
            return None;
        };
        let mut out = started;
        out["task_sent"] = json!(true);
        let text = match &pending.caller {
            Some(sender) if kind != "command" => team::tagged(sender, &task),
            _ => task,
        };
        pending.until = Instant::now() + self.team.draft + TAKING;
        let message = (name, text, false, None);
        self.type_message(pending, message, Then::Reply(out))
    }

    /// The answer to a wait on one agent, once it has stopped working.
    fn stopped_one(&self, name: &str, sent: Option<Instant>) -> Option<Answer> {
        let Some(index) = self.member_index(name) else {
            // Its tab closed: it ended, or the person closed it.
            let closed = self.team.gone.get(name).is_some_and(|g| !g.exited);
            let state = if closed { "closed" } else { "exited" };
            let out = json!({"name": name, "state": state, "reason": "exit"});
            return Some(Answer::ok(out));
        };
        let doing = self.doing(index);
        if let Some(sent) = sent {
            // After a send, its last word is from before it. One that says nothing of
            // itself is working until it ends.
            let heard = self.team.heard.get(name).map(|(_, when)| *when);
            let since = heard.is_some_and(|when| when > sent);
            if !since && doing != Doing::Exited {
                if doing == Doing::Running || sent.elapsed() < TAKING {
                    return None;
                }
                return Some(Answer::ok(json!({
                    "name": name,
                    "state": doing.word(),
                    "reason": "not_started",
                    "note": "it didn't start working on the message; read it before sending again",
                })));
            }
        }
        (!doing.working()).then(|| {
            Answer::ok(json!({
                "name": name,
                "state": doing.word(),
                "detail": doing.detail(),
                "reason": if doing == Doing::Exited { "exit" } else { "status" },
            }))
        })
    }

    /// The caller's children: each one's name and what it's doing.
    pub(crate) fn children_of(&self, parent: &str) -> Vec<(String, Doing)> {
        let mine = self.dock.sessions.iter().enumerate().filter(|(_, s)| {
            s.member
                .as_ref()
                .is_some_and(|m| m.parent.as_deref() == Some(parent))
        });
        mine.filter_map(|(i, s)| Some((s.called()?.to_string(), self.doing(i))))
            .collect()
    }

    /// The answer to a wait on the caller's children, once it's over: with `all`, when none
    /// is working; with `any`, when one has stopped since the caller last heard of it.
    fn stopped_children(&mut self, caller: Option<&str>, children: Children) -> Option<Answer> {
        let caller = caller?;
        let mine = self.children_of(caller);
        let news = mine
            .iter()
            .any(|(name, doing)| !doing.working() && self.team.unheard(caller, name));
        let none_working = mine.iter().all(|(_, d)| !d.working());
        let over = match children {
            Children::All => none_working,
            Children::Any => news || none_working,
        };
        if !over {
            return None;
        }
        for (name, _) in &mine {
            self.team.heard_of(caller, name);
        }
        let each: Value = mine
            .iter()
            .map(|(name, doing)| {
                json!({"name": name, "state": doing.word(), "detail": doing.detail()})
            })
            .collect();
        Some(Answer::ok(json!({"reason": "children", "children": each})))
    }

    /// `cellix agent ls`: every agent here, with what it's doing.
    fn team_list(&self) -> Value {
        let list = self
            .dock
            .sessions
            .iter()
            .enumerate()
            .filter_map(|(index, session)| {
                let member = session.member.as_ref()?;
                let doing = self.doing(index);
                Some(json!({
                    "name": member.name,
                    "kind": member.kind,
                    "state": doing.word(),
                    "detail": doing.detail(),
                    "parent": member.parent,
                }))
            });
        list.collect()
    }

    /// `cellix agent read`: what it last said, from its transcript, or else its screen.
    fn team_read(&mut self, name: &str, lines: Option<usize>, caller: &Option<String>) -> Answer {
        // What it says now is all its parent needs to hear of it.
        if let Some(caller) = caller {
            self.team.heard_of(caller, name);
        }
        let last = |text: String| -> Value {
            let all: Vec<&str> = text.lines().collect();
            let keep = lines.unwrap_or(all.len());
            Value::String(all[all.len().saturating_sub(keep)..].join("\n"))
        };
        let keep = lines.unwrap_or(SCREEN_LINES);
        let Some(index) = self.member_index(name) else {
            let Some(gone) = self.team.gone.get(name) else {
                return not_found(name);
            };
            let reply = gone
                .transcript
                .as_deref()
                .and_then(|path| crate::transcript::read(path).ok())
                .and_then(|t| t.last_reply());
            return Answer::ok(match reply {
                Some(reply) => last(reply),
                None => {
                    let shown = &gone.lines[gone.lines.len().saturating_sub(keep)..];
                    Value::String(shown.join("\n"))
                }
            });
        };
        let said = self
            .team
            .heard
            .get(name)
            .map(|(e, _)| PathBuf::from(&e.transcript))
            .filter(|p| p.is_file())
            .or_else(|| self.resolve_transcript(index));
        let reply = said
            .and_then(|path| crate::transcript::read(&path).ok())
            .and_then(|t| t.last_reply());
        if let Some(reply) = reply {
            return Answer::ok(last(reply));
        }
        let mut screen = self.dock.sessions[index].lines();
        while screen.last().is_some_and(|l| l.trim().is_empty()) {
            screen.pop();
        }
        let shown = &screen[screen.len().saturating_sub(keep)..];
        Answer::ok(Value::String(shown.join("\n")))
    }

    /// Whether a call, a card or a note is under way, or an agent was just given a message:
    /// what moves those on is timed, so the loop keeps looking as often as it did.
    pub(super) fn team_soon(&self) -> bool {
        let prompted =
            |s: &crate::dock::Session| s.prompted.is_some_and(|at| at.elapsed() < TAKING);
        let members = self.dock.sessions.iter().filter_map(|s| s.member.as_ref());
        let gone = self.team.gone.values().map(|g| &g.member);
        // News for a parent that's still here to be told it.
        let untold = members.chain(gone).any(|member| {
            let Some(parent) = &member.parent else {
                return false;
            };
            let here = self
                .member_index(parent)
                .is_some_and(|i| !self.dock.sessions[i].exited);
            let told = self.team.told.get(&(parent.clone(), member.name.clone()));
            let told = told.copied().unwrap_or(0);
            here && self
                .team
                .alerts
                .get(&member.name)
                .is_some_and(|(count, _)| *count > told)
        });
        !self.team.pending.is_empty()
            || !self.team.asking.is_empty()
            || self.dock.sessions.iter().any(prompted)
            || untold
    }

    /// Moves every call under way on, and answers those that are done. Returns whether the
    /// screen may have changed.
    pub(super) fn poll_team(&mut self) -> bool {
        for gone in std::mem::take(&mut self.dock.gone) {
            let name = gone.member.name.clone();
            self.team.heard.remove(&name);
            self.team.was_working.remove(&name);
            *self.team.stops.entry(name.clone()).or_default() += 1;
            // One that had already ended was said to have when it did.
            if self.team.was.remove(&name) != Some(Doing::Exited.word()) {
                let read = format!("cellix agent read {name}");
                let text = if gone.exited {
                    format!("{name} exited. Read: {read}")
                } else {
                    format!("{name} was closed. Read: {read}")
                };
                self.team.alert(&name, text);
            }
            self.team.gone.insert(name, gone);
        }
        // Each member that stopped working since the last look is news for its parent.
        for index in 0..self.dock.sessions.len() {
            self.look_at(index);
        }
        self.end_orphans();
        self.tell_parents();
        // A card that went unanswered says so, and the next one shows.
        let mut changed = false;
        while self
            .team
            .asking
            .front()
            .is_some_and(|p| Instant::now() >= p.until)
        {
            let late = self.team.asking.pop_front().expect("one at the front");
            if let Step::Consent(start) = &late.step {
                self.inbox.drop_key(&format!("start:{}", start.name));
            }
            let error = format!(
                "the person didn't answer whether it may start in {} seconds, so nothing was \
                 started; ask them",
                CONSENT_WAIT.as_secs()
            );
            answer(&late.reply, Answer::fail(code::DENIED, error));
            changed = true;
        }
        if self.team.pending.is_empty() {
            return changed;
        }
        let mut pending = std::mem::take(&mut self.team.pending);
        pending.retain_mut(|call| !self.progress(call));
        // Calls made meanwhile are kept with them.
        pending.append(&mut self.team.pending);
        self.team.pending = pending;
        changed
    }

    /// Types what each parent hasn't heard of its children into it, as one note: when it
    /// isn't waiting on them already (the wait tells it), isn't asking the person something
    /// itself, and the person isn't typing in it. A command takes no notes.
    fn tell_parents(&mut self) {
        let mut news: std::collections::BTreeMap<String, Vec<(String, u64, String)>> =
            std::collections::BTreeMap::new();
        let members = self.dock.sessions.iter().filter_map(|s| s.member.as_ref());
        let gone = self.team.gone.values().map(|g| &g.member);
        for member in members.chain(gone) {
            let Some(parent) = &member.parent else {
                continue;
            };
            let Some((count, text)) = self.team.alerts.get(&member.name) else {
                continue;
            };
            let key = (parent.clone(), member.name.clone());
            if *count > self.team.told.get(&key).copied().unwrap_or(0) {
                let child = (member.name.clone(), *count, text.clone());
                news.entry(parent.clone()).or_default().push(child);
            }
        }
        for (parent, children) in news {
            let waited = |child: &str| self.team.pending.iter().any(|p| p.waits_on(&parent, child));
            let (waited, untold): (Vec<_>, Vec<_>) =
                children.into_iter().partition(|(child, ..)| waited(child));
            for (child, count, _) in waited {
                self.team.told.insert((parent.clone(), child), count);
            }
            let Some(index) = self.member_index(&parent) else {
                continue;
            };
            let session = &self.dock.sessions[index];
            let takes_notes = session.member.as_ref().is_some_and(|m| m.kind != "command");
            if untold.is_empty() || !takes_notes || session.exited {
                for (child, count, _) in untold {
                    self.team.told.insert((parent.clone(), child), count);
                }
                continue;
            }
            let typing = !session
                .typed
                .hold(self.team.quiet, self.team.draft)
                .is_zero();
            let noting = self
                .team
                .pending
                .iter()
                .any(|p| p.note && p.target() == Some(parent.as_str()));
            if typing || noting || matches!(self.doing(index), Doing::Asking(_)) {
                continue;
            }
            let notes: Vec<String> = untold.iter().map(|(_, _, t)| team::note(t)).collect();
            for (child, count, _) in untold {
                self.team.told.insert((parent.clone(), child), count);
            }
            let (reply, _) = std::sync::mpsc::channel();
            self.team.pending.push(Pending {
                caller: None,
                reply,
                until: Instant::now() + TAKING,
                note: true,
                step: Step::Message {
                    name: parent,
                    text: notes.join("\n"),
                    force: false,
                    typed: None,
                    then: Then::Reply(Value::Null),
                },
            });
        }
    }

    /// Ends the children of a parent that has ended or closed, but those started to outlive
    /// it, which go on as agents of their own.
    fn end_orphans(&mut self) {
        for index in 0..self.dock.sessions.len() {
            let session = &self.dock.sessions[index];
            let Some(parent) = session.member.as_ref().and_then(|m| m.parent.clone()) else {
                continue;
            };
            let alive = self
                .member_index(&parent)
                .is_some_and(|i| !self.dock.sessions[i].exited);
            if alive {
                continue;
            }
            let session = &mut self.dock.sessions[index];
            match &mut session.member {
                // On its own now, it has a tab of its own.
                Some(member) if member.outlive => {
                    member.parent = None;
                    session.tabbed = true;
                }
                _ if !session.exited => {
                    session.stop();
                    let name = session.called().unwrap_or_default().to_string();
                    self.say(format!("{name} ended with {parent}, which started it"));
                }
                _ => {}
            }
        }
    }

    /// Whether the consent card shows.
    pub(super) fn asking_consent(&self) -> bool {
        !self.team.asking.is_empty() && matches!(self.overlay, Overlay::None)
    }

    /// The consent card's place: in the side pane a row under its tabs, a column in from
    /// each side, or in the window's corner with no pane.
    pub(super) fn consent_area(&self) -> Option<Rect> {
        let Step::Consent(start) = &self.team.asking.front()?.step else {
            return None;
        };
        let (x, width, top) = match self.dock_area().filter(|a| a.w >= 30) {
            // A column in from each side, however wide the pane is: capped, it stood at the
            // pane's left with its right side open.
            Some(pane) => (pane.x + 1, pane.w.saturating_sub(2), Some(pane.y + 2)),
            None => {
                let width = CONSENT_WIDTH.min(self.size.0.saturating_sub(4));
                (self.size.0.saturating_sub(width + 2), width, None)
            }
        };
        let height = self.consent_lines(start, width).len() as u16 + 4 + self.card_gap();
        let y = top.unwrap_or_else(|| self.size.1.saturating_sub(height + 2));
        Some(Rect::new(x, y, width, height))
    }

    /// The card's words under its title: the task quoted, then what starting it means.
    fn consent_lines(&self, start: &Start, width: u16) -> Vec<(String, bool)> {
        // From after the bar to two columns short of the card's edge.
        let room = f32::from(width.saturating_sub(6));
        let mut lines = Vec::new();
        let quoted = match &start.task {
            Some(task) => {
                let task = task.split_whitespace().collect::<Vec<_>>().join(" ");
                cut_to(
                    &format!("as {}: \"{task}\"", start.name),
                    room,
                    &*self.measure,
                )
            }
            None => format!("as {}", start.name),
        };
        lines.push((quoted, true));
        let cost = if start.agent() {
            "It runs as you do, in this folder, and costs its own tokens."
        } else {
            "It runs as you do, in this folder."
        };
        let note = match &start.place {
            None => cost.to_string(),
            Some(place) => cost.replace("in this folder", &format!("in {place}")),
        };
        for line in self.wrap(&note, f32::from(width.saturating_sub(4))) {
            lines.push((line, false));
        }
        lines
    }

    /// The buttons along the card's foot, at its right end, as (cells, x).
    fn consent_buttons(&self, area: Rect) -> Vec<(String, u16)> {
        let placed = self.card_buttons(0, &CONSENT_BUTTONS);
        let end = placed
            .last()
            .map_or(0, |(cells, x)| x + cells.chars().count() as u16);
        let start = area.right().saturating_sub(2 + end).max(area.x + 2);
        placed
            .into_iter()
            .map(|(cells, x)| (cells, x + start))
            .collect()
    }

    pub(super) fn draw_consent(&self, grid: &mut Grid) {
        if !self.asking_consent() {
            return;
        }
        let (Some(area), Some(Step::Consent(start))) = (
            self.consent_area(),
            self.team.asking.front().map(|p| &p.step),
        ) else {
            return;
        };
        let theme = &self.theme;
        // Over the side pane, a column from the sheet's edge on both sides.
        let in_pane = self
            .dock_area()
            .is_some_and(|pane| pane.contains(area.x, area.y));
        let lead = if in_pane { self.pane_card_lead() } else { 0.0 };
        let last = measures::DialogLast::Words;
        self.draw_card_led(grid, area, (lead, last));
        // Its words raised over its buttons as a dialog's are (`card_lift`).
        let words = grid.painted_so_far();
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
        let right = area.right().saturating_sub(2);
        let kind = start
            .parent
            .as_deref()
            .and_then(|p| self.member_index(p))
            .and_then(|i| self.dock.sessions[i].member.as_ref())
            .map_or("command", |m| m.kind.as_str());
        let (icon, fallback) = crate::agents::mark_of(kind);
        let colour = crate::dock::icon_colour(icon, theme);
        grid.glyph(
            f32::from(area.x + 2),
            area.y + 1,
            icon,
            colour,
            fallback,
            base,
        );
        let title = format!("{} wants to start {}", start.by, start.what());
        grid.label(area.x + 4, area.y + 1, &title, strong, right);
        for (i, (line, quoted)) in self.consent_lines(start, area.w).iter().enumerate() {
            let y = area.y + 2 + i as u16;
            if *quoted {
                // A quote, with the accent's bar before it.
                let bar = Style {
                    fg: theme.accent,
                    painted: true,
                    ..base
                };
                grid.put_str(area.x + 2, y, "▎", bar, right);
                let bounds =
                    Bounds::cells(f32::from(area.x) + 2.3, f32::from(y) + 0.12, 0.14, 0.76);
                grid.panel(bounds, 0, Some(theme.accent), None);
                grid.label(area.x + 4, y, line, base, right);
            } else {
                grid.label(area.x + 2, y, line, dim, right);
            }
        }
        grid.lift_since(words, self.card_lift(last, lead));
        let y = area.bottom().saturating_sub(2);
        // As far in from the card's right edge as its words are from its left: the panel
        // starts `lead` into its first column, so the buttons are painted that much right of
        // their cells. They ended 33 px from the edge where the words start 24 in.
        let buttons = grid.painted_so_far();
        let (px, py) = self.pointer;
        for (i, (cells, x)) in self.consent_buttons(area).iter().enumerate() {
            let label = CONSENT_BUTTONS[i];
            let width = cells.chars().count() as u16;
            let hovered = self.hover.is_some() && on_large_button((*x, y), width, (px - lead, py));
            // Allow is the answer; Always Allow Here and Deny are the one quiet kind every
            // other action is. Deny was only its words, a third kind beside them.
            let fill = if label == "Allow" {
                theme.button
            } else {
                theme.secondary(theme.popup, hovered)
            };
            let fg = if label == "Allow" {
                theme.button_text
            } else {
                theme.text.fg
            };
            let style = Style {
                fg,
                bg: fill,
                ..base
            };
            grid.large_button(*x, y, cells, label, fill, style);
        }
        grid.nudge_since(buttons, lead);
    }

    /// A press on the consent card. Returns whether the card took it.
    pub(super) fn click_consent(&mut self, mouse: MouseEvent) -> bool {
        if !self.asking_consent() {
            return false;
        }
        let Some(area) = self.consent_area() else {
            return false;
        };
        if !area.contains(mouse.x, mouse.y) {
            return false;
        }
        let y = area.bottom().saturating_sub(2);
        // Where they're painted, `lead` right of their cells.
        let in_pane = self
            .dock_area()
            .is_some_and(|pane| pane.contains(area.x, area.y));
        let lead = if in_pane { self.pane_card_lead() } else { 0.0 };
        let (fine_x, fine_y) = mouse.at();
        let hit = self
            .consent_buttons(area)
            .into_iter()
            .position(|(cells, x)| {
                let width = cells.chars().count() as u16;
                on_large_button((x, y), width, (fine_x - lead, fine_y))
            });
        if let Some(index) = hit {
            self.answer_consent(CONSENT_BUTTONS[index]);
        }
        true
    }

    /// The person's answer to the card in front.
    pub(super) fn answer_consent(&mut self, choice: &str) {
        let Some(mut asked) = self.team.asking.pop_front() else {
            return;
        };
        let Step::Consent(start) = &asked.step else {
            return;
        };
        self.inbox.drop_key(&format!("start:{}", start.name));
        if choice == "Deny" {
            let error = format!(
                "the person said no to starting {}; nothing was started",
                start.what()
            );
            answer(&asked.reply, Answer::fail(code::DENIED, error));
            self.say(format!("{} wasn't started", start.name));
            return;
        }
        if choice == "Always Allow Here" {
            self.team.always = true;
            self.keep_state();
        }
        self.begin(&mut asked);
        if !self.progress(&mut asked) {
            self.team.pending.push(asked);
        }
    }
}

/// `text` cut with an ellipsis to fit `room` cells, its closing quote kept.
fn cut_to(text: &str, room: f32, measure: &dyn Measure) -> String {
    let font = Font::default();
    if measure.width(text, font) <= room {
        return text.to_string();
    }
    let mut cut: String = text.to_string();
    while !cut.is_empty() && measure.width(&format!("{cut}…\""), font) > room {
        cut.pop();
    }
    let cut = cut.trim_end().trim_end_matches(['.', ',', ';', ':']);
    format!("{cut}…\"")
}

/// PATH with the folder Cellix runs from in front, so what it starts finds `cellix`.
fn cellix_on_path() -> Option<String> {
    let folder = std::env::current_exe().ok()?.parent()?.to_path_buf();
    path_with(&folder, &std::env::var_os("PATH")?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cellix_input::Mods;
    use std::sync::mpsc::{Receiver, channel};

    /// How long a test waits on a program in a terminal, on a machine that may be busy.
    const BACKGROUND: Duration = Duration::from_secs(30);

    /// A stand-in for an agent: says it's up, then answers each line it's given, as an agent
    /// takes a prompt. It never reports through hooks itself; the tests say what its hooks
    /// would, and a real run of them is proved with the real binary in `tests/pty.rs`.
    const FAKE: &str = "#!/bin/sh\nprintf 'fake agent up\\n'\n\
        while IFS= read -r line; do printf 'got: %s\\n' \"$line\"; done\n";

    fn project() -> (tempfile::TempDir, App, String) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("alpha.txt"), "first line\n").unwrap();
        let fake = dir.path().join("fake-agent");
        std::fs::write(&fake, FAKE).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::Permissions::from_mode(0o755);
            std::fs::set_permissions(&fake, mode).unwrap();
        }
        let mut app = App::new(dir.path().to_path_buf(), (240, 40));
        let fake = fake.display().to_string();
        app.agent_program = Some(fake.clone());
        app.team.quiet = Duration::from_millis(100);
        (dir, app, fake)
    }

    fn screen(app: &App) -> Grid {
        let mut grid = Grid::new(app.size().0, app.size().1, app.background());
        app.draw(&mut grid);
        grid
    }

    fn find(grid: &Grid, text: &str) -> (u16, u16) {
        for y in 0..grid.height() {
            if let Some(i) = grid.row_text(y).find(text) {
                return (grid.row_text(y)[..i].chars().count() as u16, y);
            }
        }
        panic!("{text:?} not on screen:\n{}", grid.text());
    }

    fn click(app: &mut App, (x, y): (u16, u16)) {
        for kind in [MouseKind::Left, MouseKind::Up] {
            app.handle(Event::Mouse(MouseEvent {
                kind,
                x: x + 1,
                y,
                fine_x: f32::from(x + 1),
                fine_y: f32::from(y) + 0.5,
                mods: Mods::default(),
            }));
        }
    }

    fn call(app: &mut App, from: &Caller, call: Call) -> Receiver<String> {
        let (reply, answered) = channel();
        let asked = Asked {
            from: from.clone(),
            call,
        };
        app.serve(Request::Team { asked, reply });
        answered
    }

    fn answered(app: &mut App, said: &Receiver<String>) -> Answer {
        let deadline = Instant::now() + BACKGROUND;
        loop {
            if let Ok(line) = said.try_recv() {
                return serde_json::from_str(&line).unwrap();
            }
            assert!(Instant::now() < deadline, "no answer came");
            app.poll();
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// Polls for `time`, and says the call is still unanswered.
    fn unanswered(app: &mut App, said: &Receiver<String>, time: Duration) {
        let until = Instant::now() + time;
        while Instant::now() < until {
            app.poll();
            if let Ok(line) = said.try_recv() {
                panic!("answered too soon: {line}");
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// What the agent called `name` shows, its rows run together, as a long line wraps.
    fn lines_of(app: &App, name: &str) -> String {
        let index = app.member_index(name).expect("that agent");
        app.dock.sessions[index].lines().concat()
    }

    /// Waits for `want` on the screen of the agent called `name`.
    fn shows_in(app: &mut App, name: &str, want: &str) {
        let deadline = Instant::now() + BACKGROUND;
        loop {
            app.poll();
            let text = lines_of(app, name);
            if text.contains(want) {
                return;
            }
            assert!(Instant::now() < deadline, "no {want:?} in {name}:\n{text}");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// What `name`'s hooks would say.
    fn says(app: &mut App, name: &str, state: AgentState, detail: &str) {
        app.serve(Request::Agent(AgentEvent {
            session: format!("s-{name}"),
            agent: "Codex".into(),
            state,
            detail: detail.into(),
            folder: String::new(),
            pid: None,
            transcript: String::new(),
            name: name.into(),
        }));
    }

    fn run(root: &Path, name: &str, program: &str, task: Option<&str>) -> Call {
        Call::Run {
            name: Some(name.into()),
            cwd: root.display().to_string(),
            program: vec![program.into()],
            task: task.map(str::to_string),
            outlive_parent: false,
        }
    }

    fn send(name: &str, text: &str) -> Call {
        Call::Send {
            name: name.into(),
            text: text.into(),
            wait: false,
            force: false,
            timeout: 30,
        }
    }

    /// An agent the person started, which then starts another.
    fn lead(app: &mut App, fake: &str) -> Caller {
        let root = app.workspace.root().to_path_buf();
        app.start_session("Claude Code", &[fake.to_string()], &[], &root);
        shows_in(app, "claude-1", "fake agent up");
        Caller {
            name: Some("claude-1".into()),
            held: None,
            agent: true,
        }
    }

    #[test]
    #[cfg(unix)]
    fn agents_start_message_and_wait_for_each_other() {
        let (dir, mut app, fake) = project();
        // The stand-in runs, before anything relies on it.
        let lead = lead(&mut app, &fake);

        // An agent starting another asks the person first, in a card that says who, what and
        // what it costs. Nothing starts before they answer.
        let task = "Review alpha.txt";
        let reviewer = run(dir.path(), "reviewer", "codex", Some(task));
        let started = call(&mut app, &lead, reviewer);
        let text = screen(&app).text();
        assert!(text.contains("Claude Code wants to start Codex"), "{text}");
        assert!(text.contains("as reviewer: \"Review alpha.txt\""), "{text}");
        assert!(text.contains("and costs its own"), "{text}");
        for button in ["[Deny]", "[Always Allow Here]", "[Allow]"] {
            assert!(text.contains(button), "no {button}: {text}");
        }
        assert!(app.member_index("reviewer").is_none());
        // It waits with the rest of what waits for the person, to reach them from anywhere.
        assert!(app.inbox.has("start:reviewer"));
        let at = find(&screen(&app), "[Allow]");
        click(&mut app, at);
        assert!(!screen(&app).text().contains("wants to start"));
        assert!(!app.inbox.has("start:reviewer"));

        // It starts in a tab of its own without taking the keyboard, and its task waits for
        // it to say it's ready.
        assert_eq!(app.dock.active, Some(0), "the lead stays in front");
        shows_in(&mut app, "reviewer", "fake agent up");
        unanswered(&mut app, &started, Duration::from_millis(300));
        assert!(!lines_of(&app, "reviewer").contains("got:"));
        says(&mut app, "reviewer", AgentState::Idle, "Ready");
        let answer = answered(&mut app, &started);
        assert_eq!(answer.code, code::OK, "{answer:?}");
        let out = json!({"name": "reviewer", "kind": "codex", "started": true, "task_sent": true});
        assert_eq!(answer.out, out);
        // Between agents, a message says who sent it.
        let tagged = "got: <cellix from=\"claude-1\">Review alpha.txt</cellix>";
        shows_in(&mut app, "reviewer", tagged);
        let listed = call(&mut app, &lead, Call::Ls);
        let list = answered(&mut app, &listed).out;
        let entries = list.as_array().unwrap();
        let reviewer = entries.iter().find(|a| a["name"] == "reviewer");
        let reviewer = reviewer.expect("listed");
        assert_eq!(reviewer["kind"], "codex");
        assert_eq!(reviewer["parent"], "claude-1");

        // A wait lasts until it stops working, and is over at once when it isn't.
        says(&mut app, "reviewer", AgentState::Working, "Working");
        let wait = Call::Wait {
            name: Some("reviewer".into()),
            children: None,
            timeout: 30,
        };
        let waiting = call(&mut app, &lead, wait.clone());
        unanswered(&mut app, &waiting, Duration::from_millis(300));
        says(&mut app, "reviewer", AgentState::Idle, "Finished");
        let done = answered(&mut app, &waiting);
        assert_eq!(done.out["state"], "done", "{done:?}");
        let again = call(&mut app, &lead, wait.clone());
        assert!(again.try_recv().is_ok(), "already done: no wait");

        // While it asks the person something, a message would answer it, so it's refused and
        // nothing is typed.
        says(&mut app, "reviewer", AgentState::NeedsYou, "Allow Bash?");
        let refusing = call(&mut app, &lead, send("reviewer", "carry on"));
        let refused = answered(&mut app, &refusing);
        assert_eq!(refused.code, code::ASKING, "{refused:?}");
        assert!(refused.error.contains("Allow Bash?"), "{refused:?}");
        let waited = call(&mut app, &lead, wait);
        let waited = answered(&mut app, &waited);
        assert_eq!(waited.out["state"], "asking", "a wait ends on a question");
        std::thread::sleep(Duration::from_millis(300));
        app.poll();
        assert!(!lines_of(&app, "reviewer").contains("carry on"));

        // The person has begun a prompt in it: the next message waits until they clear it.
        says(&mut app, "reviewer", AgentState::Working, "Working");
        app.dock.active = app.member_index("reviewer");
        app.dock.focused = true;
        app.dock.front = Front::Session;
        for c in "half".chars() {
            app.handle(Event::Key(KeyEvent::plain(Key::Char(c))));
        }
        let sending = call(&mut app, &lead, send("reviewer", "next please"));
        unanswered(&mut app, &sending, Duration::from_millis(500));
        assert!(!lines_of(&app, "reviewer").contains("next please"));
        let clear = KeyEvent {
            key: Key::Char('u'),
            mods: Mods {
                ctrl: true,
                ..Mods::default()
            },
        };
        app.handle(Event::Key(clear));
        let sent = answered(&mut app, &sending);
        assert_eq!(sent.out, json!({"sent": "reviewer"}));
        let next = "got: <cellix from=\"claude-1\">next please</cellix>";
        shows_in(&mut app, "reviewer", next);

        // With no transcript, its screen is what it said.
        let read = Call::Read {
            name: "reviewer".into(),
            lines: Some(3),
        };
        let reading = call(&mut app, &lead, read);
        let said = answered(&mut app, &reading).out;
        assert!(said.as_str().unwrap().contains("got:"), "{said}");

        // And it can be stopped.
        let stop = Call::Stop {
            name: "reviewer".into(),
        };
        let stopping = call(&mut app, &lead, stop);
        assert_eq!(answered(&mut app, &stopping).code, code::OK);
        let deadline = Instant::now() + BACKGROUND;
        let going = |app: &App| {
            let index = app.member_index("reviewer");
            index.is_some_and(|i| app.doing(i) != Doing::Exited)
        };
        while going(&app) {
            assert!(Instant::now() < deadline, "it went on");
            app.poll();
            std::thread::sleep(Duration::from_millis(10));
        }
        let missing = call(&mut app, &lead, send("nobody", "hi"));
        assert_eq!(answered(&mut app, &missing).code, code::NOT_FOUND);
    }

    #[test]
    #[cfg(unix)]
    fn a_person_starts_and_messages_without_being_asked_or_tagged() {
        let (dir, mut app, fake) = project();
        let person = Caller::default();
        // A command, which says nothing of itself, is ready once its output stops.
        let helper = run(dir.path(), "helper", &fake, Some("hello"));
        let started = call(&mut app, &person, helper);
        assert!(!app.asking_consent(), "the person needn't ask themselves");
        let answer = answered(&mut app, &started);
        assert_eq!(answer.out["task_sent"], true, "{answer:?}");
        shows_in(&mut app, "helper", "fake agent up");
        shows_in(&mut app, "helper", "got: hello");
        let sending = call(&mut app, &person, send("helper", "more"));
        assert_eq!(answered(&mut app, &sending).code, code::OK);
        shows_in(&mut app, "helper", "got: more");
        assert!(!lines_of(&app, "helper").contains("<cellix"));
        // A name is a name of one.
        let twice = call(&mut app, &person, run(dir.path(), "helper", &fake, None));
        let twice = answered(&mut app, &twice);
        assert_eq!(twice.code, code::USAGE, "{twice:?}");
    }

    #[test]
    #[cfg(unix)]
    fn the_person_can_deny_an_agent_starting_another_or_always_allow_it_here() {
        let (dir, mut app, fake) = project();
        let lead = lead(&mut app, &fake);
        let denied = call(&mut app, &lead, run(dir.path(), "one", "codex", None));
        let at = find(&screen(&app), "[Deny]");
        click(&mut app, at);
        let answer = answered(&mut app, &denied);
        assert_eq!(answer.code, code::DENIED, "{answer:?}");
        assert!(app.member_index("one").is_none(), "nothing started");

        let allowed = call(&mut app, &lead, run(dir.path(), "two", "codex", None));
        let at = find(&screen(&app), "[Always Allow Here]");
        click(&mut app, at);
        assert!(app.member_index("two").is_some());
        says(&mut app, "two", AgentState::Idle, "Ready");
        assert_eq!(answered(&mut app, &allowed).code, code::OK);

        // Remembered with the folder, so the next Cellix here doesn't ask either.
        let mut next = App::new(dir.path().to_path_buf(), (240, 40));
        next.agent_program = Some(fake.clone());
        next.restore_state();
        let lead = super::tests::lead(&mut next, &fake);
        let third = call(&mut next, &lead, run(dir.path(), "three", "codex", None));
        assert!(!next.asking_consent(), "asked again");
        assert!(next.member_index("three").is_some());
        says(&mut next, "three", AgentState::Idle, "Ready");
        assert_eq!(answered(&mut next, &third).code, code::OK);
        let _ = std::fs::remove_dir_all(crate::hold::folder_of(dir.path()));
    }

    /// The row of the screen that says `words` after the row that says `after`.
    fn row_after(grid: &Grid, after: &str, words: &str) -> u16 {
        let from = (0..grid.height()).find(|y| grid.row_text(*y).contains(after));
        let from = from.unwrap_or_else(|| panic!("no {after:?}: {}", grid.text()));
        (from..grid.height())
            .find(|y| grid.row_text(*y).contains(words))
            .unwrap_or_else(|| panic!("no {words:?} after {after:?}: {}", grid.text()))
    }

    #[test]
    #[cfg(unix)]
    fn a_helper_stands_under_the_agent_that_started_it_in_the_agents_list() {
        let (dir, mut app, fake) = project();
        app.team.always = true;
        let lead = lead(&mut app, &fake);
        says(&mut app, "claude-1", AgentState::Working, "Working");
        let started = call(&mut app, &lead, run(dir.path(), "reviewer", "codex", None));
        shows_in(&mut app, "reviewer", "fake agent up");
        // Before its hooks say anything it stands under its agent, by the name it was given,
        // a step in, with a line bent round from its agent's mark and the word for what it is.
        app.run(Command::ListAgents);
        let grid = screen(&app);
        let agent = row_after(&grid, "WORKING", "Claude Code");
        let helper = row_after(&grid, "WORKING", "reviewer");
        assert_eq!(helper, agent + 1, "{}", grid.text());
        let row = grid.row_text(helper);
        assert!(row.contains('└') && row.contains("helper"), "{row}");
        // Once it reports, it's the same row. Asking the person, it goes to the top, in the
        // section its own mark says, and says who started it: under "WORKING" it showed the
        // chip that asks, and a finished one stood there with a green dot (0.11.0's final
        // review, 172). The heading counts only the rows in it.
        says(&mut app, "reviewer", AgentState::Idle, "Ready");
        assert_eq!(answered(&mut app, &started).code, code::OK);
        says(&mut app, "reviewer", AgentState::NeedsYou, "Allow Bash?");
        app.poll();
        let grid = screen(&app);
        let helper = row_after(&grid, "NEEDS YOU 1", "reviewer");
        let agent = row_after(&grid, "WORKING 1", "Claude Code");
        assert!(helper < agent, "{}", grid.text());
        let row = grid.row_text(helper);
        assert!(
            row.contains("from Claude Code") && !row.contains('└'),
            "{row}"
        );
        // Working again, it's back under its agent.
        says(&mut app, "reviewer", AgentState::Working, "Working");
        let grid = screen(&app);
        let agent = row_after(&grid, "WORKING 2", "Claude Code");
        let helper = row_after(&grid, "WORKING 2", "reviewer");
        assert_eq!(helper, agent + 1, "{}", grid.text());
        assert!(!grid.text().contains("NEEDS YOU"), "{}", grid.text());
    }

    /// A helper's row, its mark, its section and peek's header say one state: finished, its
    /// row has the done dot peek shows, its name bright and bold, under "DONE" with who
    /// started it, where it stood under its working agent in "WORKING 2" with a green dot
    /// (0.11.0's final review, 172). The line bent round to one under its agent is one that
    /// can be seen, and starts clear of the mark over it.
    #[test]
    #[cfg(unix)]
    fn a_helpers_row_mark_section_and_peek_say_one_state() {
        let (dir, mut app, fake) = project();
        app.team.always = true;
        let lead = lead(&mut app, &fake);
        says(&mut app, "claude-1", AgentState::Working, "Working");
        let started = call(&mut app, &lead, run(dir.path(), "reviewer", "codex", None));
        shows_in(&mut app, "reviewer", "fake agent up");
        says(&mut app, "reviewer", AgentState::Idle, "Ready");
        assert_eq!(answered(&mut app, &started).code, code::OK);
        app.run(Command::ListAgents);
        let grid = screen(&app);
        let helper = row_after(&grid, "DONE 1", "reviewer");
        assert!(
            grid.row_text(helper).contains("from Claude Code"),
            "{}",
            grid.text()
        );
        assert!(grid.text().contains("WORKING 1"), "{}", grid.text());
        let x = grid.row_text(helper).find("reviewer").unwrap();
        let x = grid.row_text(helper)[..x].chars().count() as u16;
        let name = grid.cell(x, helper).style;
        assert!(name.bold, "bold, as its section's names are");
        assert_eq!(name.fg, app.theme.sidebar.fg, "in the words' own colour");
        // Ready, it has the done dot peek's header shows for it: with nothing at its end it
        // stood under a heading that counted it as working.
        let dot = grid.row_text(helper).chars().position(|c| c == '●');
        let dot = dot.expect("its own mark") as u16;
        assert_eq!(grid.cell(dot, helper).style.fg, app.theme.git_added);
        // And peek's header says "● done" in that colour, where it said "done" in grey.
        let x = grid.row_text(helper).find("reviewer").unwrap() as u16;
        click(&mut app, (x, helper));
        let grid = screen(&app);
        let (_, head) = find(&grid, "done");
        let header = grid.row_text(head);
        let word = header[..header.find("done").unwrap()].chars().count() as u16;
        let dot = (0..word).rev().find(|x| grid.cell(*x, head).symbol == "●");
        let dot = dot.unwrap_or_else(|| panic!("a dot before done: {header}"));
        assert_eq!(grid.cell(dot, head).style.fg, app.theme.git_added);
        assert_eq!(grid.cell(word, head).style.fg, app.theme.git_added);
        // Closed, and the list in front again: peek opened over its agent's tab.
        click(&mut app, (2, 20));
        app.run(Command::ListAgents);
        // Its turn over, it has its own done dot, in the section that says so.
        says(&mut app, "reviewer", AgentState::Working, "Working");
        says(&mut app, "reviewer", AgentState::Idle, "Finished");
        let grid = screen(&app);
        let helper = row_after(&grid, "DONE", "reviewer");
        assert!(grid.row_text(helper).contains('●'), "{}", grid.text());
        // Working, under its agent: with pictures, the line to it is stronger than a hairline
        // and starts under the agent's mark, not on it.
        says(&mut app, "reviewer", AgentState::Working, "Working");
        app.set_measure(std::rc::Rc::new(cellix_render::FontMeasure::new(17, 37)));
        let grid = screen(&app);
        let helper = row_after(&grid, "WORKING 2", "reviewer");
        let name = grid.row_text(helper).find("reviewer").unwrap();
        let name = grid.row_text(helper)[..name].chars().count() as u16;
        assert!(
            grid.cell(name, helper).style.bold,
            "bold, as its section's are"
        );
        let lines: Vec<(Bounds, Rgb)> = grid
            .paint
            .iter()
            .filter_map(|p| match p {
                cellix_render::Paint::Hairline { bounds, color } => Some((*bounds, *color)),
                _ => None,
            })
            .filter(|(b, _)| b.w == 0 && b.y < i32::from(helper) * 64 + 32)
            .filter(|(b, _)| b.y + b.h > i32::from(helper) * 64)
            .collect();
        let (down, color) = lines.first().copied().expect("the line down to it");
        assert_ne!(
            color, app.theme.border,
            "stronger than the sheet's hairline"
        );
        assert!(
            down.y >= i32::from(helper) * 64 - 4,
            "under the mark: {down:?}"
        );
    }

    /// The panels painted on the grid, as (bounds, fill, border).
    fn panels(grid: &Grid) -> Vec<(Bounds, Option<Rgb>, Option<Rgb>)> {
        grid.paint
            .iter()
            .filter_map(|p| match p {
                Paint::Panel {
                    bounds,
                    fill,
                    border,
                    ..
                } => Some((*bounds, *fill, *border)),
                _ => None,
            })
            .collect()
    }

    /// The asking card and peek stand over the side pane a column from its sheet's edge on
    /// both sides, where they stood 6 px from it on the left and 15 on the right. On the
    /// asking card only Allow is filled, Deny and Always Allow Here quiet buttons alike.
    /// Peek's header has its parts 8 px apart and nothing cut while there's room, the state
    /// a mark and a word in its colour, and Keep as Tab a quiet button.
    #[test]
    #[cfg(unix)]
    fn cards_over_the_side_pane_stand_alike_on_both_sides() {
        let (dir, mut app, fake) = project();
        let lead = lead(&mut app, &fake);
        says(&mut app, "claude-1", AgentState::Working, "Working");
        app.set_measure(std::rc::Rc::new(cellix_render::FontMeasure::new(17, 37)));
        let started = call(&mut app, &lead, run(dir.path(), "reviewer", "codex", None));
        let pane = app.dock_area().unwrap();
        let unit = |cells: f32| (cells * 64.0).round() as i32;
        // The sheet starts half way into the pane's first column, and ends with its last.
        let (sheet, end) = (unit(f32::from(pane.x) + 0.5), unit(f32::from(pane.right())));
        let grid = screen(&app);
        let theme = &app.theme;
        let card = panels(&grid)
            .into_iter()
            .find(|(_, fill, border)| *fill == Some(theme.popup) && border.is_some())
            .map(|(bounds, ..)| bounds)
            .expect("the asking card");
        assert_eq!(
            card.x - sheet,
            end - (card.x + card.w),
            "{card:?} in {pane:?}"
        );
        assert_eq!(card.x - sheet, 64, "a column from the sheet's edge");
        let quiet = theme.quiet_button(theme.popup);
        let fills: Vec<Rgb> = panels(&grid)
            .into_iter()
            .filter(|(b, _, _)| b.y > card.y && b.y < card.y + card.h && b.w < card.w / 2)
            // Not the quote's bar, which is the accent too.
            .filter(|(b, _, _)| b.w >= 128)
            .filter_map(|(_, fill, _)| fill)
            .filter(|fill| *fill == quiet || *fill == theme.button)
            .collect();
        assert_eq!(
            fills,
            [quiet, quiet, theme.button],
            "Deny, Always Allow Here, Allow"
        );
        // Its buttons end as far in from its right edge as its words start from its left:
        // they ended 33 px in where the words start 24 in.
        let allow = panels(&grid)
            .into_iter()
            .filter(|(b, fill, _)| *fill == Some(theme.button) && b.y > card.y && b.w >= 128)
            .map(|(bounds, ..)| bounds)
            .next()
            .expect("Allow");
        let words = grid
            .paint
            .iter()
            .find_map(|p| match p {
                cellix_render::Paint::Text { x, text, .. } if text.contains("wants to start") => {
                    Some(*x)
                }
                _ => None,
            })
            .expect("its title");
        // The title follows the mark, a column and a half after the text's edge.
        let text_edge = words - unit(2.0);
        assert!(
            ((card.x + card.w) - (allow.x + allow.w) - (text_edge - card.x)).abs() <= 2,
            "{} in at the right, {} at the left",
            (card.x + card.w) - (allow.x + allow.w),
            text_edge - card.x
        );

        app.answer_consent("Allow");
        shows_in(&mut app, "reviewer", "fake agent up");
        says(&mut app, "reviewer", AgentState::Working, "Working");
        assert_eq!(answered(&mut app, &started).code, code::OK);
        app.open_peek("reviewer");
        let grid = screen(&app);
        let theme = &app.theme;
        let edge = panels(&grid)
            .into_iter()
            .find(|(_, fill, border)| fill.is_none() && *border == Some(theme.popup_border))
            .map(|(bounds, ..)| bounds)
            .expect("peek's card");
        assert_eq!(
            edge.x - sheet,
            end - (edge.x + edge.w),
            "{edge:?} in {pane:?}"
        );
        // Its header: who started it whole, and what it's doing in the accent.
        let texts: Vec<(i32, String, Rgb)> = grid
            .paint
            .iter()
            .filter_map(|p| match p {
                Paint::Text {
                    x, y, text, color, ..
                } if *y == edge.y => Some((*x, text.clone(), *color)),
                _ => None,
            })
            .collect();
        let said = |words: &str| {
            texts
                .iter()
                .find(|(_, text, _)| text == words)
                .cloned()
                .unwrap_or_else(|| panic!("no {words:?} in {texts:?}"))
        };
        let (_, _, colour) = said("working");
        assert_eq!(colour, theme.accent);
        let (from, ..) = said("· from Claude Code");
        let (name, ..) = said("reviewer");
        assert!(name < from);
        let card_row = |b: &Bounds| b.y >= edge.y && b.y < edge.y + 64;
        let keep = panels(&grid)
            .into_iter()
            .find(|(b, fill, _)| card_row(b) && *fill == Some(quiet))
            .map(|(bounds, ..)| bounds)
            .expect("Keep as Tab, a quiet button");
        assert!(from < keep.x, "who started it stops short of Keep as Tab");
    }

    #[test]
    #[cfg(unix)]
    fn a_helper_has_no_tab_and_peek_shows_it_live_over_the_pane() {
        let (dir, mut app, fake) = project();
        app.team.always = true;
        let lead = lead(&mut app, &fake);
        says(&mut app, "claude-1", AgentState::Working, "Working");
        let started = call(&mut app, &lead, run(dir.path(), "reviewer", "codex", None));
        shows_in(&mut app, "reviewer", "fake agent up");
        says(&mut app, "reviewer", AgentState::Working, "Working");
        assert_eq!(answered(&mut app, &started).code, code::OK);

        // It has no tab of its own: the pane's row has the lead's and the list's.
        app.run(Command::ListAgents);
        let grid = screen(&app);
        let pane = app.dock_area().unwrap();
        let tabs = &grid.row_text(pane.y);
        assert!(
            tabs.contains("Claude Code") && !tabs.contains("Codex"),
            "{tabs}"
        );

        // A press on its row opens it over the pane: who it is, what it's doing, who started
        // it, Keep as Tab and ×, and its terminal, live.
        let row = row_after(&grid, "WORKING", "reviewer");
        let x = grid.row_text(row).find("reviewer").unwrap() as u16;
        click(&mut app, (x, row));
        let text = screen(&app).text();
        assert!(text.contains("· from Claude Code"), "{text}");
        assert!(text.contains("[Keep as Tab]"), "{text}");
        assert!(text.contains("working"), "{text}");
        assert!(text.contains("fake agent up"), "{text}");
        // The keyboard is its while it's open.
        for c in "hello".chars() {
            app.handle(Event::Key(KeyEvent::plain(Key::Char(c))));
        }
        app.handle(Event::Key(KeyEvent::plain(Key::Enter)));
        shows_in(&mut app, "reviewer", "got: hello");
        let deadline = Instant::now() + BACKGROUND;
        while !screen(&app).text().contains("got: hello") {
            assert!(Instant::now() < deadline, "{}", screen(&app).text());
            app.poll();
            std::thread::sleep(Duration::from_millis(10));
        }

        // A press outside it closes it, and nothing else happens.
        click(&mut app, (2, 20));
        assert!(app.peek.is_none());
        assert!(!screen(&app).text().contains("Keep as Tab"));

        // It opened over its agent's tab, which stays in front. Kept as a tab from there, it
        // has one, in front.
        assert_eq!(app.dock.active, Some(0));
        app.run(Command::ListAgents);
        let grid = screen(&app);
        let row = row_after(&grid, "WORKING", "reviewer");
        click(&mut app, (x, row));
        assert!(app.peek.is_some());
        let at = find(&screen(&app), "[Keep as Tab]");
        click(&mut app, at);
        assert!(app.peek.is_none());
        let index = app.member_index("reviewer").unwrap();
        assert!(app.dock.sessions[index].tabbed);
        assert_eq!(app.dock.active, Some(index));
        let grid = screen(&app);
        assert!(grid.row_text(pane.y).contains("Codex"), "{}", grid.text());
    }

    #[test]
    #[cfg(unix)]
    fn a_ctrl_click_on_an_agents_name_in_a_terminal_looks_at_it() {
        let (dir, mut app, fake) = project();
        app.team.always = true;
        let lead = lead(&mut app, &fake);
        let started = call(&mut app, &lead, run(dir.path(), "reviewer", "codex", None));
        shows_in(&mut app, "reviewer", "fake agent up");
        says(&mut app, "reviewer", AgentState::Idle, "Ready");
        assert_eq!(answered(&mut app, &started).code, code::OK);

        // The helper tells the lead something, which comes with its name.
        let reviewer = Caller {
            name: Some("reviewer".into()),
            held: None,
            agent: true,
        };
        let told = call(&mut app, &reviewer, send("claude-1", "see alpha.txt"));
        assert_eq!(answered(&mut app, &told).code, code::OK);
        let tag = "from=\"reviewer\">";
        shows_in(&mut app, "claude-1", tag);
        let deadline = Instant::now() + BACKGROUND;
        while !screen(&app).text().contains(tag) {
            assert!(Instant::now() < deadline, "{}", screen(&app).text());
            app.poll();
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(app.dock.active, Some(0));
        let ctrl_click = |app: &mut App, (x, y): (u16, u16)| {
            let mods = Mods {
                ctrl: true,
                ..Mods::default()
            };
            for kind in [MouseKind::Left, MouseKind::Up] {
                app.handle(Event::Mouse(MouseEvent {
                    kind,
                    x,
                    y,
                    fine_x: f32::from(x) + 0.5,
                    fine_y: f32::from(y) + 0.5,
                    mods,
                }));
            }
        };

        // A plain press on its name does what a press in a terminal does, and no more.
        let (x, y) = find(&screen(&app), tag);
        let name = (x + 7, y);
        click(&mut app, name);
        assert!(app.peek.is_none());

        // Ctrl+click on its name opens it in peek, over the lead's tab.
        ctrl_click(&mut app, name);
        assert_eq!(app.peek.as_deref(), Some("reviewer"));
        let text = screen(&app).text();
        assert!(text.contains("· from Claude Code"), "{text}");
        assert_eq!(app.dock.active, Some(0));
        click(&mut app, (2, 20));
        assert!(app.peek.is_none());

        // Other words are as they were: a file's name still opens the file.
        let (x, y) = find(&screen(&app), "alpha.txt<");
        ctrl_click(&mut app, (x + 1, y));
        assert!(app.peek.is_none());
        let opened = app
            .editor_mut()
            .and_then(|e| e.buffer.path().map(Path::to_path_buf));
        assert!(opened.is_some_and(|p| p.ends_with("alpha.txt")));
    }

    /// Polls for `time`, and says `name` never showed `unwanted`.
    fn never_shows(app: &mut App, name: &str, unwanted: &str, time: Duration) {
        let until = Instant::now() + time;
        while Instant::now() < until {
            app.poll();
            let text = lines_of(app, name);
            assert!(!text.contains(unwanted), "{unwanted:?} in {name}:\n{text}");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    #[cfg(unix)]
    fn a_parent_hears_of_its_children_waits_for_them_and_takes_them_with_it() {
        let (dir, mut app, fake) = project();
        app.team.always = true;
        let lead = lead(&mut app, &fake);
        let start = |app: &mut App, name: &str, outlive_parent: bool| {
            let child = Call::Run {
                name: Some(name.into()),
                cwd: dir.path().display().to_string(),
                program: vec!["codex".into()],
                task: None,
                outlive_parent,
            };
            let started = call(app, &lead, child);
            shows_in(app, name, "fake agent up");
            says(app, name, AgentState::Idle, "Ready");
            assert_eq!(answered(app, &started).code, code::OK);
        };
        start(&mut app, "one", false);
        start(&mut app, "two", false);
        start(&mut app, "kept", true);

        // A wait on its children: the next to stop, then all of them.
        for name in ["one", "two", "kept"] {
            says(&mut app, name, AgentState::Working, "Working");
        }
        let children = |which| Call::Wait {
            name: None,
            children: Some(which),
            timeout: 30,
        };
        let any = call(&mut app, &lead, children(Children::Any));
        unanswered(&mut app, &any, Duration::from_millis(300));
        says(&mut app, "one", AgentState::Idle, "Finished");
        let news = answered(&mut app, &any);
        let list = news.out["children"].as_array().unwrap().clone();
        let one = list.iter().find(|c| c["name"] == "one").expect("one");
        assert_eq!(one["state"], "done", "{news:?}");
        let all = call(&mut app, &lead, children(Children::All));
        unanswered(&mut app, &all, Duration::from_millis(300));
        // Nothing is typed into a parent about a child it's waiting on: the wait tells it.
        says(&mut app, "two", AgentState::NeedsYou, "Allow Bash?");
        says(&mut app, "kept", AgentState::Idle, "Finished");
        let over = answered(&mut app, &all);
        assert_eq!(over.out["reason"], "children", "{over:?}");
        never_shows(
            &mut app,
            "claude-1",
            "needs you",
            Duration::from_millis(500),
        );

        // Not waiting, it's told as a note when a child needs the person.
        says(&mut app, "two", AgentState::Working, "Working");
        says(&mut app, "two", AgentState::NeedsYou, "Allow Bash?");
        let note = "got: <cellix>two needs you (permission). See: cellix agent read two</cellix>";
        shows_in(&mut app, "claude-1", note);

        // While the parent asks the person something itself, no note goes in, and once it's
        // back at work the note it missed does.
        says(&mut app, "claude-1", AgentState::NeedsYou, "Allow Edit?");
        says(&mut app, "one", AgentState::Idle, "Stopped: rate limit");
        never_shows(
            &mut app,
            "claude-1",
            "one stopped",
            Duration::from_millis(500),
        );
        says(&mut app, "claude-1", AgentState::Working, "Working");
        let failed = "got: <cellix>one stopped on an error (rate limit). See: cellix agent read one</cellix>";
        shows_in(&mut app, "claude-1", failed);

        // Children end with their parent, but one started to outlive it, which is its own now.
        let stop = Call::Stop {
            name: "claude-1".into(),
        };
        let stopping = call(&mut app, &Caller::default(), stop);
        assert_eq!(answered(&mut app, &stopping).code, code::OK);
        let running = |app: &App, name: &str| {
            let index = app.member_index(name);
            index.is_some_and(|i| app.doing(i) != Doing::Exited)
        };
        let deadline = Instant::now() + BACKGROUND;
        while running(&app, "one") || running(&app, "two") {
            assert!(Instant::now() < deadline, "the children went on");
            app.poll();
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(running(&app, "kept"));
        let index = app.member_index("kept").unwrap();
        let member = app.dock.sessions[index].member.as_ref().unwrap();
        assert_eq!(member.parent, None, "on its own now");
    }
}
