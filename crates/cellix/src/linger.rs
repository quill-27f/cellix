//! Whether this machine stops a person's programs when they log out. Some Linux servers do
//! (systemd's `KillUserProcesses`), which would take every agent and shell with the SSH
//! connection, as it would in tmux. Lingering keeps them, and the person can turn it on.

use std::process::Command;

/// Whether logging out here ends what Cellix leaves running: logind kills a user's
/// processes at logout, and lingering is off for them. Anything that can't be asked, as on a
/// machine without systemd, counts as no.
pub fn logout_stops_programs(user: &str) -> bool {
    let linger = Command::new("loginctl")
        .args(["show-user", user, "--property=Linger", "--value"])
        .output();
    let Ok(linger) = linger else {
        return false;
    };
    if !linger.status.success() || String::from_utf8_lossy(&linger.stdout).trim() == "yes" {
        return false;
    }
    kills_at_logout()
}

/// logind's own answer, or failing that, what its configuration files say.
fn kills_at_logout() -> bool {
    let asked = Command::new("busctl")
        .args([
            "get-property",
            "org.freedesktop.login1",
            "/org/freedesktop/login1",
            "org.freedesktop.login1.Manager",
            "KillUserProcesses",
        ])
        .output();
    if let Ok(answer) = asked
        && answer.status.success()
    {
        return String::from_utf8_lossy(&answer.stdout).trim() == "b true";
    }
    let mut files = vec![std::path::PathBuf::from("/etc/systemd/logind.conf")];
    let mut extra: Vec<_> = std::fs::read_dir("/etc/systemd/logind.conf.d")
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|e| e == "conf"))
        .collect();
    extra.sort();
    files.extend(extra);
    let texts: Vec<String> = files
        .iter()
        .filter_map(|file| std::fs::read_to_string(file).ok())
        .collect();
    kills_in(&texts)
}

/// What a run of logind configuration files, in order, says of `KillUserProcesses`: the last
/// setting wins, and it's off unless set.
fn kills_in(texts: &[String]) -> bool {
    let mut kills = false;
    for text in texts {
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            if key.trim() == "KillUserProcesses" {
                kills = matches!(value.trim(), "yes" | "true" | "1" | "on");
            }
        }
    }
    kills
}

/// The command that turns lingering on for the person running Cellix.
pub fn command(user: &str) -> String {
    format!("loginctl enable-linger {user}")
}

/// Turns lingering on for the person running Cellix, which logind allows for oneself on most
/// systems. Gives what went wrong otherwise.
pub fn turn_on(program: &[String]) -> Result<(), String> {
    let (first, rest) = program.split_first().ok_or("no command")?;
    let output = Command::new(first)
        .args(rest)
        .output()
        .map_err(|e| e.to_string())?;
    if output.status.success() {
        return Ok(());
    }
    let said = String::from_utf8_lossy(&output.stderr).trim().to_string();
    Err(if said.is_empty() {
        format!("it exited with {}", output.status)
    } else {
        said
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_last_setting_of_kill_user_processes_wins() {
        let text = |s: &str| s.to_string();
        assert!(!kills_in(&[]), "off unless set");
        assert!(!kills_in(&[text("[Login]\n#KillUserProcesses=yes\n")]));
        assert!(kills_in(&[text("[Login]\nKillUserProcesses=yes\n")]));
        assert!(!kills_in(&[
            text("[Login]\nKillUserProcesses=yes\n"),
            text("[Login]\nKillUserProcesses=no\n"),
        ]));
        assert!(kills_in(&[
            text("KillUserProcesses = no"),
            text(" KillUserProcesses=true ")
        ]));
    }
}
