use std::io::{self, Write};
use std::process::Command;
use std::time::{Duration, Instant};

use crate::probe::{Env, Multiplexer};

const QUERY_ID: u32 = 31;
const REPLY_TIMEOUT: Duration = Duration::from_millis(250);
/// Terminals known to speak Kitty graphics, for when tmux hides the terminal from us.
const KITTY_TERMINALS: [&str; 3] = ["ghostty", "kitty", "wezterm"];

fn tmux_output(args: &[&str]) -> Option<String> {
    let output = Command::new("tmux").args(args).output().ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Sends the Kitty graphics query followed by a device attributes request, which every
/// terminal answers, so a terminal without graphics doesn't cost the full timeout.
/// Expects raw mode.
fn query_kitty(out: &mut impl Write) -> io::Result<bool> {
    write!(
        out,
        "\x1b_Gi={QUERY_ID},s=1,v=1,a=q,t=d,f=24;AAAA\x1b\\\x1b[c"
    )?;
    out.flush()?;
    let deadline = Instant::now() + REPLY_TIMEOUT;
    let mut reply = Vec::new();
    let mut buf = [0u8; 256];
    while Instant::now() < deadline {
        let left = deadline.saturating_duration_since(Instant::now());
        let mut pollfd = libc::pollfd {
            fd: libc::STDIN_FILENO,
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: pollfd is a valid, exclusively borrowed array of one element.
        let ready = unsafe { libc::poll(&mut pollfd, 1, left.as_millis() as libc::c_int) };
        if ready <= 0 {
            break;
        }
        // SAFETY: buf is valid for writes of buf.len() bytes.
        let n = unsafe { libc::read(libc::STDIN_FILENO, buf.as_mut_ptr().cast(), buf.len()) };
        if n <= 0 {
            break;
        }
        reply.extend_from_slice(&buf[..n as usize]);
        if reply_has(&reply, format!("_Gi={QUERY_ID};OK").as_bytes()) {
            return Ok(true);
        }
        if ends_device_attributes(&reply) {
            break;
        }
    }
    Ok(false)
}

fn reply_has(reply: &[u8], needle: &[u8]) -> bool {
    reply.windows(needle.len()).any(|w| w == needle)
}

/// A primary device attributes reply looks like `ESC [ ? ... c`.
fn ends_device_attributes(reply: &[u8]) -> bool {
    let Some(start) = reply.windows(3).rposition(|w| w == b"\x1b[?") else {
        return false;
    };
    reply[start..].contains(&b'c')
}

/// Works out what this terminal can do. Must run in raw mode, before input is read.
pub fn detect(var: impl Fn(&str) -> Option<String>) -> io::Result<Env> {
    let multiplexer = Multiplexer::detect(var);
    if multiplexer == Multiplexer::Tmux {
        // tmux swallows replies from the terminal outside it, so ask tmux instead.
        let client = tmux_output(&["display", "-p", "#{client_termname}"]).unwrap_or_default();
        let kitty_graphics = KITTY_TERMINALS.iter().any(|t| client.contains(t));
        let tmux_passthrough = tmux_output(&["show", "-Apv", "allow-passthrough"])
            .is_some_and(|v| v == "on" || v == "all");
        return Ok(Env {
            multiplexer,
            kitty_graphics,
            tmux_passthrough,
        });
    }
    Ok(Env {
        multiplexer,
        kitty_graphics: query_kitty(&mut io::stdout())?,
        tmux_passthrough: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_attributes_reply_is_recognised() {
        assert!(ends_device_attributes(b"\x1b[?62;22c"));
        assert!(ends_device_attributes(b"\x1b_Gi=31;EBADF\x1b\\\x1b[?62c"));
        assert!(!ends_device_attributes(b"\x1b[?62"));
        assert!(!ends_device_attributes(b"\x1b_Gi=31;OK\x1b\\"));
    }

    #[test]
    fn query_reply_is_recognised() {
        assert!(reply_has(b"\x1b_Gi=31;OK\x1b\\", b"_Gi=31;OK"));
        assert!(!reply_has(b"\x1b_Gi=31;EINVAL\x1b\\", b"_Gi=31;OK"));
    }
}
