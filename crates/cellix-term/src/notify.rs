//! Desktop notifications through the terminal: a few bytes the terminal turns into its own
//! notification, so they reach the person over SSH and through tmux with nothing installed.
//! Terminals disagree on which sequence they read, so the one sent is chosen by name.

/// Which sequence a terminal reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Notice {
    /// `OSC 777 ; notify ; title ; body`, with a title: Ghostty, WezTerm, foot, rxvt, Konsole.
    Osc777,
    /// `OSC 9 ; text`, iTerm2's, which most others read too. No title of its own.
    Osc9,
    /// `OSC 99`, Kitty's own; it reads neither of the others.
    Osc99,
}

/// Terminals known to show `OSC 777`.
const WITH_777: [&str; 6] = ["ghostty", "wezterm", "foot", "rxvt", "konsole", "contour"];

impl Notice {
    /// Chooses from what the terminal is called: `TERM`, `TERM_PROGRAM`, `LC_TERMINAL`, and
    /// under tmux the name tmux heard from the terminal outside it, all in one string.
    pub fn for_terminal(names: &str) -> Self {
        let names = names.to_lowercase();
        if names.contains("kitty") {
            Self::Osc99
        } else if WITH_777.iter().any(|t| names.contains(t)) {
            Self::Osc777
        } else {
            Self::Osc9
        }
    }
}

/// What the terminal is called, from the environment through `var`, and from tmux when Cellix
/// runs inside it: there `TERM` names tmux, and over SSH it's often a stand-in.
pub fn terminal_names(var: impl Fn(&str) -> Option<String>) -> String {
    let mut names: Vec<String> = ["TERM", "TERM_PROGRAM", "LC_TERMINAL"]
        .iter()
        .filter_map(|name| var(name))
        .collect();
    if var("TMUX").is_some_and(|v| !v.is_empty()) {
        let asked = std::process::Command::new("tmux")
            .args(["display", "-p", "#{client_termname} #{client_termtype}"])
            .output();
        if let Ok(output) = asked
            && output.status.success()
        {
            names.push(String::from_utf8_lossy(&output.stdout).trim().to_string());
        }
    }
    names.join(" ")
}

/// Text as a terminal can carry it in a sequence: no control characters, which would end it
/// early or start another, and on one line.
fn clean(text: &str) -> String {
    let flat: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    flat.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The bytes that show `title` and `body` as a notification. Inside tmux they're wrapped to
/// go past it to the terminal, which tmux allows with `allow-passthrough`.
pub fn sequence(notice: Notice, title: &str, body: &str, tmux: bool) -> String {
    let (title, body) = (clean(title), clean(body));
    let inner = match notice {
        // The title ends at a semicolon, so one inside it would cut it short.
        Notice::Osc777 => format!("\x1b]777;notify;{};{body}\x1b\\", title.replace(';', ",")),
        Notice::Osc9 => format!("\x1b]9;{title}: {body}\x1b\\"),
        // The title, then the body as a second part of the same notification.
        Notice::Osc99 => {
            format!(
                "\x1b]99;i=cellix:d=0:p=title;{title}\x1b\\\x1b]99;i=cellix:d=1:p=body;{body}\x1b\\"
            )
        }
    };
    if tmux {
        format!("\x1bPtmux;{}\x1b\\", inner.replace('\x1b', "\x1b\x1b"))
    } else {
        inner
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_terminal_gets_the_sequence_it_reads() {
        assert_eq!(
            Notice::for_terminal("xterm-ghostty ghostty"),
            Notice::Osc777
        );
        assert_eq!(
            Notice::for_terminal("xterm-256color WezTerm"),
            Notice::Osc777
        );
        assert_eq!(Notice::for_terminal("xterm-kitty"), Notice::Osc99);
        assert_eq!(
            Notice::for_terminal("xterm-256color iTerm.app iTerm2"),
            Notice::Osc9
        );
        assert_eq!(Notice::for_terminal(""), Notice::Osc9);
        // Under tmux over SSH, the name tmux heard is what tells.
        assert_eq!(
            Notice::for_terminal("tmux-256color tmux xterm-ghostty"),
            Notice::Osc777
        );
    }

    #[test]
    fn a_notification_is_one_clean_sequence() {
        assert_eq!(
            sequence(Notice::Osc777, "Cellix · api", "Allow Bash?", false),
            "\x1b]777;notify;Cellix · api;Allow Bash?\x1b\\"
        );
        assert_eq!(
            sequence(Notice::Osc9, "Cellix · api", "Plan ready", false),
            "\x1b]9;Cellix · api: Plan ready\x1b\\"
        );
        // What an agent wrote can't end the sequence early, or split the title.
        assert_eq!(
            sequence(Notice::Osc777, "a;b", "run\x1b]0;x\x07 now\nplease", false),
            "\x1b]777;notify;a,b;run ]0;x now please\x1b\\"
        );
        let kitty = sequence(Notice::Osc99, "Cellix", "Done", false);
        assert!(kitty.contains("p=title;Cellix\x1b\\") && kitty.contains("p=body;Done\x1b\\"));
    }

    #[test]
    fn inside_tmux_it_goes_past_tmux() {
        assert_eq!(
            sequence(Notice::Osc9, "Cellix", "Done", true),
            "\x1bPtmux;\x1b\x1b]9;Cellix: Done\x1b\x1b\\\x1b\\"
        );
    }
}
