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

struct Reply {
    kitty: bool,
    cell: Option<(usize, usize)>,
    pixel_mouse: bool,
    light: Option<bool>,
}

/// Asks for the cell size and for Kitty graphics, then device attributes, which every
/// terminal answers, so a terminal without graphics doesn't cost the full timeout.
/// Expects raw mode. Returns (kitty graphics, cell size).
fn query_terminal(out: &mut impl Write) -> io::Result<Reply> {
    write!(
        out,
        "\x1b[16t\x1b]11;?\x1b\\\x1b[?1016$p\x1b_Gi={QUERY_ID},s=1,v=1,a=q,t=d,f=24;AAAA\x1b\\\x1b[c"
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
        if ends_device_attributes(&reply) {
            break;
        }
    }
    Ok(Reply {
        kitty: reply_has(&reply, format!("_Gi={QUERY_ID};OK").as_bytes()),
        cell: parse_cell_size(&reply),
        pixel_mouse: reply_has(&reply, b"?1016;1$y") || reply_has(&reply, b"?1016;2$y"),
        light: parse_background_is_light(&reply),
    })
}

/// Reads `ESC ] 11 ; rgb:RRRR/GGGG/BBBB`, and says whether that colour is light.
fn parse_background_is_light(reply: &[u8]) -> Option<bool> {
    let needle = b"]11;rgb:";
    let start = reply.windows(needle.len()).position(|w| w == needle)? + needle.len();
    let rest = std::str::from_utf8(&reply[start..]).ok()?;
    let mut channels = rest.split(['/', '\x1b', '\x07']).take(3).map(|part| {
        let digits = part.len().clamp(1, 4) as u32;
        u32::from_str_radix(part, 16)
            .ok()
            .map(|v| v as f32 / (16f32.powi(digits as i32) - 1.0))
    });
    let (r, g, b) = (channels.next()??, channels.next()??, channels.next()??);
    Some(0.2126 * r + 0.7152 * g + 0.0722 * b > 0.5)
}

/// Finds a `ESC [ 6 ; height ; width t` reply.
fn parse_cell_size(reply: &[u8]) -> Option<(usize, usize)> {
    let start = reply.windows(4).position(|w| w == b"\x1b[6;")? + 4;
    let rest = std::str::from_utf8(&reply[start..]).ok()?;
    let (body, _) = rest.split_once('t')?;
    let (height, width) = body.split_once(';')?;
    Some((width.parse().ok()?, height.parse().ok()?))
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
        // tmux answers the colour query itself, from what it knows of the terminal.
        let light = query_terminal(&mut io::stdout())?.light;
        return Ok(Env {
            multiplexer,
            kitty_graphics,
            tmux_passthrough,
            cell: None,
            pixel_mouse: false,
            light,
        });
    }
    let reply = query_terminal(&mut io::stdout())?;
    Ok(Env {
        multiplexer,
        kitty_graphics: reply.kitty,
        tmux_passthrough: false,
        cell: reply.cell,
        pixel_mouse: reply.pixel_mouse,
        light: reply.light,
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
    fn cell_size_reply_is_parsed() {
        let zellij = b"\x1b[6;37;17t\x1b_Gi=31;OK\x1b\\\x1b[4;888;1394t\x1b[?62;52c";
        assert_eq!(parse_cell_size(zellij), Some((17, 37)));
        assert_eq!(parse_cell_size(b"\x1b[?62c"), None);
    }

    #[test]
    fn the_background_decides_light_or_dark() {
        let dark = b"\x1b]11;rgb:1e1e/1e1e/1e1e\x1b\\\x1b[?62c";
        assert_eq!(parse_background_is_light(dark), Some(false));
        assert_eq!(
            parse_background_is_light(b"\x1b]11;rgb:ffff/fefe/fdfd\x07"),
            Some(true)
        );
        assert_eq!(
            parse_background_is_light(b"\x1b]11;rgb:ee/ee/ee\x07"),
            Some(true)
        );
        assert_eq!(parse_background_is_light(b"\x1b[?62c"), None);
    }

    #[test]
    fn query_reply_is_recognised() {
        assert!(reply_has(b"\x1b_Gi=31;OK\x1b\\", b"_Gi=31;OK"));
        assert!(!reply_has(b"\x1b_Gi=31;EINVAL\x1b\\", b"_Gi=31;OK"));
    }
}
