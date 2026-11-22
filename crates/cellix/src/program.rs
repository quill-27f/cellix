//! What a program in the side pane says of itself, through the Program Status Protocol herdr
//! made open: `ESC ] 7501 ; key=value:key=value… ST`, or ending in BEL. Any program can say
//! it's working, done or needs the person, with nothing installed, in one `printf`:
//!
//! ```sh
//! printf '\033]7501;state=blocked:app=deploy:msg=%s\033\\' "$(printf 'Deploy to prod?' | base64)"
//! ```
//!
//! The keys: `state` (idle, working, done, blocked, error, or clear to take it back), `app`
//! (who's saying it), `kind` (why it's blocked: permission, question or auth), `progress`
//! (0 to 100), `title` and `msg` (base64), and `id` for a part of the program, empty for the
//! program itself. Cellix keeps what it hears as it keeps what an agent's hooks say.

use base64::Engine;
use base64::alphabet;
use base64::engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig};
use cellix_agent::{AgentEvent, AgentState};

/// What names a program's news among the agents', after which comes its tab's key, and for a
/// part of it a `/` and the part's id. Hooks' news is named by the agent's own session id.
pub const SESSION: &str = "program:";
/// The longest name, title or message kept, in characters.
const LONGEST: usize = 200;

/// What a report says the program is doing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Word {
    Idle,
    Working,
    Done,
    Blocked,
    Error,
    /// Takes back what it said before.
    Clear,
}

/// One report, as the program sent it, with its words decoded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Report {
    pub word: Word,
    /// Why it's blocked, when it said: permission, question or auth.
    pub kind: Option<&'static str>,
    pub progress: Option<u8>,
    /// Which part of the program it's about: empty for the program itself.
    pub id: String,
    pub app: String,
    pub title: String,
    pub message: String,
}

/// Text as it may stand in a list: one line, no control characters, not too long.
fn plain(text: &str) -> String {
    let line: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    line.trim().chars().take(LONGEST).collect()
}

fn decoded(text: &str) -> String {
    let lenient = GeneralPurpose::new(
        &alphabet::STANDARD,
        GeneralPurposeConfig::new().with_decode_padding_mode(DecodePaddingMode::Indifferent),
    );
    let bytes = lenient.decode(text.trim()).unwrap_or_default();
    plain(&String::from_utf8_lossy(&bytes))
}

/// Reads a report from what came between `ESC ] 7501 ;` and its end. One without a state
/// Cellix knows is no report.
pub fn parse(payload: &[u8]) -> Option<Report> {
    let text = String::from_utf8_lossy(payload);
    let mut word = None;
    let mut report = Report {
        word: Word::Idle,
        kind: None,
        progress: None,
        id: String::new(),
        app: String::new(),
        title: String::new(),
        message: String::new(),
    };
    for field in text.split(':') {
        let Some((key, value)) = field.split_once('=') else {
            continue;
        };
        match key.trim() {
            "state" => {
                word = match value.trim() {
                    "idle" => Some(Word::Idle),
                    "working" => Some(Word::Working),
                    "done" => Some(Word::Done),
                    "blocked" => Some(Word::Blocked),
                    "error" => Some(Word::Error),
                    "clear" => Some(Word::Clear),
                    _ => None,
                };
            }
            "kind" => {
                report.kind = ["permission", "question", "auth"]
                    .into_iter()
                    .find(|k| *k == value.trim());
            }
            "progress" => {
                report.progress = value.trim().parse().ok().filter(|p| *p <= 100);
            }
            "id" => report.id = plain(value),
            "app" => report.app = plain(value),
            "title" => report.title = decoded(value),
            "msg" => report.message = decoded(value),
            _ => {}
        }
    }
    report.word = word?;
    Some(report)
}

/// Which of Cellix's agents a program's `app` names, as their tabs are named.
pub fn agent_of(app: &str) -> Option<&'static str> {
    let app = app.trim().to_lowercase().replace(['_', ' '], "-");
    Some(match app.as_str() {
        "claude" | "claude-code" | "claudecode" => "Claude Code",
        "codex" | "codex-cli" | "openai-codex" => "Codex",
        "gemini" | "gemini-cli" => "Gemini CLI",
        "opencode" => "OpenCode",
        "copilot" | "copilot-cli" | "github-copilot" => "Copilot CLI",
        "cursor" | "cursor-agent" | "cursor-cli" => "Cursor CLI",
        "kiro" | "kiro-cli" => "Kiro CLI",
        _ => return None,
    })
}

/// What to call the program saying it: one of Cellix's agents by its name, any other by the
/// name it gave, and one that gave none by its tab's.
pub fn name_of(app: &str, tab: &str) -> String {
    match agent_of(app) {
        Some(agent) => agent.to_string(),
        None if app.is_empty() => tab.to_string(),
        None => app.to_string(),
    }
}

/// A report as news of an agent, under `session`, from the program called `name`.
pub fn event(report: &Report, session: &str, name: &str, folder: &str) -> AgentEvent {
    let said = || {
        Some(&report.message)
            .filter(|m| !m.is_empty())
            .or(Some(&report.title).filter(|t| !t.is_empty()))
            .cloned()
    };
    let (state, detail) = match report.word {
        Word::Working => {
            let mut detail = said().unwrap_or_else(|| "Working".to_string());
            if let Some(progress) = report.progress {
                detail.push_str(&format!(" · {progress}%"));
            }
            (AgentState::Working, detail)
        }
        Word::Blocked => {
            let why = match report.kind {
                Some("permission") => format!("{name} asks for permission"),
                Some("question") => format!("{name} has a question"),
                Some("auth") => format!("{name} needs you to sign in"),
                _ => "Needs your attention".to_string(),
            };
            (AgentState::NeedsYou, said().unwrap_or(why))
        }
        // The word the hooks give a finished turn, which is what the inbox and tile go by.
        Word::Done => (AgentState::Idle, "Finished".to_string()),
        Word::Idle => (
            AgentState::Idle,
            said().unwrap_or_else(|| "Idle".to_string()),
        ),
        Word::Error => {
            let detail = match said() {
                Some(why) => format!("Stopped: {why}"),
                None => "Stopped on an error".to_string(),
            };
            (AgentState::Idle, detail)
        }
        Word::Clear => (AgentState::Ended, "Ended".to_string()),
    };
    AgentEvent {
        session: session.to_string(),
        agent: name.to_string(),
        state,
        detail,
        folder: folder.to_string(),
        pid: None,
        transcript: String::new(),
        name: String::new(),
        prompt: String::new(),
    }
}

/// Whether news came from a program's own report rather than an agent's hooks.
pub fn reported(event: &AgentEvent) -> bool {
    event.session.starts_with(SESSION)
}

/// The tab a program's news, or what waits from it under `key` (`ask:…`, `done:…`), is from.
pub fn tab_of(key: &str) -> Option<&str> {
    let session = key.split_once(':').map_or(key, |(_, rest)| rest);
    let session = session
        .strip_prefix(SESSION)
        .or_else(|| key.strip_prefix(SESSION))?;
    Some(session.split('/').next().unwrap_or(session))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b64(text: &str) -> String {
        base64::engine::general_purpose::STANDARD.encode(text)
    }

    #[test]
    fn a_report_reads_as_herdr_documents_it() {
        let payload = format!(
            "state=blocked:app=my-agent:kind=permission:progress=40:id=a/b:title={}:msg={}",
            b64("Run tests"),
            b64("Apply these changes?")
        );
        let report = parse(payload.as_bytes()).unwrap();
        assert_eq!(report.word, Word::Blocked);
        assert_eq!(report.kind, Some("permission"));
        assert_eq!(report.progress, Some(40));
        assert_eq!(report.id, "a/b");
        assert_eq!(report.app, "my-agent");
        assert_eq!(report.title, "Run tests");
        assert_eq!(report.message, "Apply these changes?");
        // Unpadded base64 reads too, and what isn't base64 is no message.
        let report = parse(b"state=done:msg=QXBwbHk:title=***").unwrap();
        assert_eq!(
            (report.message.as_str(), report.title.as_str()),
            ("Apply", "")
        );
    }

    #[test]
    fn what_isnt_a_report_is_left_alone() {
        assert_eq!(parse(b"app=x"), None, "no state");
        assert_eq!(parse(b"state=sleeping:app=x"), None, "a state nobody knows");
        let odd = parse(b"state=working:progress=140:kind=bored:color=red:junk").unwrap();
        assert_eq!((odd.progress, odd.kind), (None, None));
        // A message can't break the line it's shown on.
        let report = parse(format!("state=blocked:msg={}", b64("one\ntwo\x1b[31m")).as_bytes());
        assert_eq!(report.unwrap().message, "one two [31m");
    }

    #[test]
    fn each_word_becomes_the_state_cellix_keeps_for_agents() {
        let news = |payload: &str| {
            let report = parse(payload.as_bytes()).unwrap();
            let name = name_of(&report.app, "Terminal");
            let event = event(&report, "program:t", &name, "/work");
            (event.agent, event.state, event.detail)
        };
        let blocked = format!("state=blocked:app=deploy:msg={}", b64("Deploy to prod?"));
        assert_eq!(
            news(&blocked),
            (
                "deploy".into(),
                AgentState::NeedsYou,
                "Deploy to prod?".into()
            )
        );
        assert_eq!(
            news("state=blocked:app=deploy:kind=question").2,
            "deploy has a question"
        );
        assert_eq!(
            news("state=working:app=claude-code:progress=30"),
            (
                "Claude Code".into(),
                AgentState::Working,
                "Working · 30%".into()
            )
        );
        assert_eq!(news("state=done").0, "Terminal", "no app: its tab's name");
        assert_eq!(news("state=done").2, "Finished");
        assert_eq!(news("state=error").2, "Stopped on an error");
        assert_eq!(news("state=clear").1, AgentState::Ended);
        assert_eq!(agent_of("gemini"), Some("Gemini CLI"));
        assert_eq!(agent_of("Codex"), Some("Codex"));
        assert_eq!(agent_of("cursor-agent"), Some("Cursor CLI"));
        assert_eq!(agent_of("kiro-cli"), Some("Kiro CLI"));
        assert_eq!(agent_of("my-agent"), None);
    }

    #[test]
    fn whats_waiting_leads_back_to_its_tab() {
        assert_eq!(tab_of("ask:program:123-4"), Some("123-4"));
        assert_eq!(tab_of("done:program:123-4/tests"), Some("123-4"));
        assert_eq!(tab_of("program:tab-2"), Some("tab-2"));
        assert_eq!(tab_of("ask:5f0c-uuid"), None, "an agent's hooks' own");
    }
}
