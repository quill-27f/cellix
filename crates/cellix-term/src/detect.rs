use std::io::{self, Write};
use std::process::Command;
#[cfg(unix)]
use std::time::{Duration, Instant};

use crate::probe::{Env, Multiplexer};

const QUERY_ID: u32 = 31;
#[cfg(unix)]
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
    background: Option<(u8, u8, u8)>,
}

/// Where the questions go.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Asking {
    /// The terminal itself.
    Terminal,
    /// tmux, which answers for the terminal from what it knows.
    Tmux,
    /// tmux, with passthrough, so the cell size can be asked of the terminal outside it.
    TmuxPassthrough,
}

/// What to ask a terminal. The graphics query goes only where it reaches the terminal
/// itself: tmux takes that kind of sequence as a window title and shows it in its status bar.
/// tmux makes up a cell size when its own terminal gives none (16 by 32, over a console), and
/// images sized for that are letterboxed, so with passthrough the cell size goes past it.
#[cfg(any(unix, test))]
fn questions(asking: Asking) -> String {
    let (cell, graphics) = match asking {
        Asking::Terminal => (
            "\x1b[16t".to_string(),
            format!("\x1b_Gi={QUERY_ID},s=1,v=1,a=q,t=d,f=24;AAAA\x1b\\"),
        ),
        Asking::Tmux => (String::new(), String::new()),
        Asking::TmuxPassthrough => ("\x1bPtmux;\x1b\x1b[16t\x1b\\".to_string(), String::new()),
    };
    format!(
        "{}{cell}\x1b[?1016$p{graphics}\x1b[c",
        background_question(asking)
    )
}

/// The question about the background. Inside tmux with passthrough it goes to the terminal
/// itself too: tmux keeps that answer rather than passing it on, but it's how tmux learns the
/// terminal's colour has changed since the client attached, so tmux's own answer, asked
/// afterwards, is current. It comes ahead of the cell size, which the terminal answers next and
/// tmux does pass on, so that answer arriving means tmux has the new colour.
#[cfg(any(unix, test))]
fn background_question(asking: Asking) -> &'static str {
    match asking {
        Asking::TmuxPassthrough => "\x1bPtmux;\x1b\x1b]11;?\x07\x1b\\",
        Asking::Terminal | Asking::Tmux => "\x1b]11;?\x1b\\",
    }
}

/// tmux's own answer about the background, asked once the terminal has told it.
#[cfg(any(unix, test))]
const ASK_TMUX_BACKGROUND: &str = "\x1b]11;?\x1b\\\x1b[c";

/// Asks the terminal for its background again, as when Cellix comes back into focus: it may
/// have turned from light to dark since Cellix started. Must run in raw mode, between reads
/// of input. Gives whether it's light, and its colour, when the terminal said.
pub fn background(multiplexer: Multiplexer, passthrough: bool) -> Option<(bool, (u8, u8, u8))> {
    let asking = match multiplexer {
        Multiplexer::Tmux if passthrough => Asking::TmuxPassthrough,
        Multiplexer::Tmux => Asking::Tmux,
        Multiplexer::None | Multiplexer::Zellij => Asking::Terminal,
    };
    let mut out = io::stdout();
    let reply = if asking == Asking::TmuxPassthrough {
        let refresh = format!(
            "{}\x1bPtmux;\x1b\x1b[16t\x1b\\\x1b[c",
            background_question(asking)
        );
        ask_for(&mut out, &refresh, asking, |reply| {
            parse_cell_size(reply).is_some()
        })
        .ok()?;
        ask_for(&mut out, ASK_TMUX_BACKGROUND, Asking::Tmux, |_| true).ok()?
    } else {
        let question = format!("{}\x1b[c", background_question(asking));
        ask_for(&mut out, &question, asking, |reply| {
            parse_background(reply).is_some()
        })
        .ok()?
    };
    let light = parse_background_is_light(&reply)?;
    let (r, g, b) = parse_background(&reply)?;
    let byte = |channel: f32| (channel * 255.0).round() as u8;
    Some((light, (byte(r), byte(g), byte(b))))
}

/// Asks for the cell size and for Kitty graphics, then device attributes, which every
/// terminal answers, so a terminal without graphics doesn't cost the full timeout.
/// Expects raw mode. Returns (kitty graphics, cell size).
fn query_terminal(out: &mut impl Write, asking: Asking) -> io::Result<Reply> {
    let mut reply = ask(out, asking)?;
    if asking == Asking::TmuxPassthrough {
        // The terminal told tmux its colour; now tmux's answer is current.
        reply.extend(ask_for(out, ASK_TMUX_BACKGROUND, Asking::Tmux, |_| true)?);
    }
    Ok(Reply {
        kitty: reply_has(&reply, format!("_Gi={QUERY_ID};OK").as_bytes()),
        cell: parse_cell_size(&reply),
        pixel_mouse: reply_has(&reply, b"?1016;1$y") || reply_has(&reply, b"?1016;2$y"),
        light: parse_background_is_light(&reply),
        background: parse_background(&reply).map(|(r, g, b)| {
            let byte = |channel: f32| (channel * 255.0).round() as u8;
            (byte(r), byte(g), byte(b))
        }),
    })
}

/// A Windows console hands its input over as key events, not bytes to poll, so nothing is
/// asked there: Cellix starts on the cell tier with its own colours, and nothing the terminal
/// would answer is left to arrive as keys.
#[cfg(not(unix))]
fn ask(_out: &mut impl Write, _asking: Asking) -> io::Result<Vec<u8>> {
    Ok(Vec::new())
}

#[cfg(not(unix))]
fn ask_for(
    _out: &mut impl Write,
    _question: &str,
    _asking: Asking,
    _done: impl Fn(&[u8]) -> bool,
) -> io::Result<Vec<u8>> {
    Ok(Vec::new())
}

/// Writes the questions and reads the replies until device attributes, or the timeout.
#[cfg(unix)]
fn ask(out: &mut impl Write, asking: Asking) -> io::Result<Vec<u8>> {
    let outside = asking == Asking::TmuxPassthrough;
    ask_for(out, &questions(asking), asking, |reply| {
        !outside || parse_cell_size(reply).is_some()
    })
}

/// Writes `question`, which ends by asking for device attributes, and reads the replies
/// until those come and `done` is satisfied, or the timeout.
#[cfg(unix)]
fn ask_for(
    out: &mut impl Write,
    question: &str,
    asking: Asking,
    done: impl Fn(&[u8]) -> bool,
) -> io::Result<Vec<u8>> {
    write!(out, "{question}")?;
    out.flush()?;
    // tmux answers device attributes at once, but the terminal outside answers through it a
    // round trip later, so that answer is waited for too, a little longer.
    let outside = asking == Asking::TmuxPassthrough;
    let timeout = if outside {
        REPLY_TIMEOUT * 3
    } else {
        REPLY_TIMEOUT
    };
    let deadline = Instant::now() + timeout;
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
        if ends_device_attributes(&reply) && done(&reply) {
            break;
        }
    }
    Ok(reply)
}

fn parse_background_is_light(reply: &[u8]) -> Option<bool> {
    let (r, g, b) = parse_background(reply)?;
    Some(0.2126 * r + 0.7152 * g + 0.0722 * b > 0.5)
}

/// Reads `ESC ] 11 ; rgb:RRRR/GGGG/BBBB`, as fractions of full brightness.
fn parse_background(reply: &[u8]) -> Option<(f32, f32, f32)> {
    let needle = b"]11;rgb:";
    let start = reply.windows(needle.len()).position(|w| w == needle)? + needle.len();
    let rest = std::str::from_utf8(&reply[start..]).ok()?;
    let mut channels = rest.split(['/', '\x1b', '\x07']).take(3).map(|part| {
        let digits = part.len().clamp(1, 4) as u32;
        u32::from_str_radix(part, 16)
            .ok()
            .map(|v| v as f32 / (16f32.powi(digits as i32) - 1.0))
    });
    Some((channels.next()??, channels.next()??, channels.next()??))
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
#[cfg(any(unix, test))]
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
        // The terminal's own name where tmux heard it, as well as its TERM: over SSH, TERM is
        // often a stand-in like xterm-256color when the server lacks the terminal's terminfo.
        let client = tmux_output(&["display", "-p", "#{client_termname} #{client_termtype}"])
            .unwrap_or_default()
            .to_lowercase();
        let kitty_graphics = KITTY_TERMINALS.iter().any(|t| client.contains(t));
        let tmux_passthrough = tmux_output(&["show", "-Apv", "allow-passthrough"])
            .is_some_and(|v| v == "on" || v == "all");
        // tmux answers the colour query itself, from what it knows of the terminal, which with
        // passthrough is brought up to date first.
        let asking = if tmux_passthrough {
            Asking::TmuxPassthrough
        } else {
            Asking::Tmux
        };
        let reply = query_terminal(&mut io::stdout(), asking)?;
        return Ok(Env {
            multiplexer,
            kitty_graphics,
            tmux_passthrough,
            cell: reply.cell,
            pixel_mouse: false,
            light: reply.light,
            background: reply.background,
        });
    }
    let reply = query_terminal(&mut io::stdout(), Asking::Terminal)?;
    Ok(Env {
        multiplexer,
        kitty_graphics: reply.kitty,
        tmux_passthrough: false,
        cell: reply.cell,
        pixel_mouse: reply.pixel_mouse,
        light: reply.light,
        background: reply.background,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inside_tmux_with_passthrough_the_terminal_itself_is_asked_its_colour() {
        // tmux's own answer is what it knew when the client attached, and goes stale.
        let asked = questions(Asking::TmuxPassthrough);
        assert!(asked.starts_with("\x1bPtmux;\x1b\x1b]11;?"), "{asked:?}");
        assert!(
            !asked.contains("\x1b]11;?\x1b\\\x1b["),
            "tmux isn't asked as well: {asked:?}"
        );
        assert!(questions(Asking::Terminal).starts_with("\x1b]11;?"));
        assert!(questions(Asking::Tmux).starts_with("\x1b]11;?"));
    }

    #[test]
    fn tmux_is_not_asked_about_graphics() {
        assert!(!questions(Asking::Tmux).contains("\x1b_G"));
        assert!(questions(Asking::Terminal).contains("\x1b_Gi=31"));
        assert!(questions(Asking::Tmux).ends_with("\x1b[c"));
    }

    #[test]
    fn the_cell_size_is_asked_past_tmux() {
        // tmux would answer a plain question with a size it made up.
        assert!(!questions(Asking::Tmux).contains("[16t"));
        let past = questions(Asking::TmuxPassthrough);
        assert!(past.contains("\x1bPtmux;\x1b\x1b[16t\x1b\\"), "{past:?}");
        assert!(!past.contains("\x1b_G"));
        // As tmux hands the outer terminal's answer back.
        assert_eq!(parse_cell_size(b"\x1b[?62c\x1b[6;36;17t"), Some((17, 36)));
    }

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
