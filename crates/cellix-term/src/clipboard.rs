//! Paste events from the Kitty clipboard protocol (OSC 5522), which is how a terminal hands
//! over a picture: OSC 52 carries only text. With mode 5522 on, a paste doesn't arrive as
//! bracketed text but as a list of what the clipboard holds, with a one-time password. The
//! program asks for the kind it wants with that password, and the terminal sends it without
//! asking the person, since they pressed paste. Kitty since 0.44.1 and Ghostty since 1.4 do this.
//! Spec: <https://rockorager.dev/misc/bracketed-paste-mime/> and kitty's clipboard page.

use base64::Engine;

/// Turns paste events on and off.
pub const PASTE_EVENTS_ON: &str = "\x1b[?5522h";
pub const PASTE_EVENTS_OFF: &str = "\x1b[?5522l";
/// What a terminal is asked to learn whether it has paste events, with the questions at start.
pub const ASK_PASTE_EVENTS: &str = "\x1b[?5522$p";
/// The name a read with a paste's password gives, which the terminal checks the password by.
const PASTE_EVENT: &str = "Paste event";
/// Where an OSC 5522 message's body starts, after `ESC ]`.
pub const PREFIX: &str = "5522;";

/// Whether a reply to the questions at start says paste events are on offer: set or reset.
pub fn has_paste_events(reply: &[u8]) -> bool {
    let has = |s: &[u8]| reply.windows(s.len()).any(|w| w == s);
    has(b"?5522;1$y") || has(b"?5522;2$y")
}

/// What the terminal said, once a whole exchange is in.
#[derive(Debug, PartialEq, Eq)]
pub enum Pasted {
    /// The person pasted: what the clipboard holds, and the password to read it with.
    Offer {
        types: Vec<String>,
        password: Option<String>,
    },
    /// What was asked for.
    Data { mime: String, bytes: Vec<u8> },
    /// The terminal wouldn't, with its reason: EPERM, ENOSYS, EBUSY and so on.
    Refused(String),
}

/// Reads OSC 5522 messages, one body at a time, into what they add up to.
#[derive(Default)]
pub struct Reader {
    /// An exchange under way: its password, the types listed, and the data so far.
    open: Option<Exchange>,
}

#[derive(Default)]
struct Exchange {
    password: Option<String>,
    listed: Option<String>,
    mime: Option<String>,
    bytes: Vec<u8>,
}

impl Reader {
    /// Takes one message's body, the part after `ESC ] 5522 ;` and before its end.
    pub fn take(&mut self, body: &str) -> Option<Pasted> {
        let (meta, payload) = body.split_once(';').unwrap_or((body, ""));
        let keys: Vec<(&str, &str)> = meta
            .split(':')
            .filter_map(|pair| pair.split_once('='))
            .collect();
        let key = |name: &str| keys.iter().find(|(k, _)| *k == name).map(|(_, v)| *v);
        if key("type") != Some("read") {
            return None;
        }
        let password = key("pw").map(str::to_string);
        match key("status")? {
            "OK" => {
                self.open = Some(Exchange {
                    password,
                    ..Exchange::default()
                });
                None
            }
            "DATA" => {
                let exchange = self.open.get_or_insert_with(Exchange::default);
                exchange.password = exchange.password.take().or(password);
                let mime = key("mime").and_then(decode_text).unwrap_or_default();
                // Each chunk is padded on its own, so each is decoded on its own.
                let bytes = decode(payload).unwrap_or_default();
                if mime == "." {
                    let listed = exchange.listed.get_or_insert_with(String::new);
                    listed.push_str(&String::from_utf8_lossy(&bytes));
                } else if exchange.mime.as_deref().is_none_or(|m| m == mime) {
                    exchange.mime = Some(mime);
                    exchange.bytes.extend(bytes);
                }
                None
            }
            "DONE" => {
                let exchange = self.open.take()?;
                let password = exchange.password.or(password);
                if let Some(listed) = exchange.listed {
                    let types = listed.split_whitespace().map(str::to_string).collect();
                    return Some(Pasted::Offer { types, password });
                }
                Some(Pasted::Data {
                    mime: exchange.mime?,
                    bytes: exchange.bytes,
                })
            }
            error => {
                self.open = None;
                Some(Pasted::Refused(error.to_string()))
            }
        }
    }
}

/// What to read of what's offered: text if there's any, as an ordinary paste is, and
/// otherwise a picture, PNG first.
pub fn choose(types: &[String]) -> Option<&str> {
    let text = types
        .iter()
        .find(|t| *t == "text/plain" || t.starts_with("text/plain;"));
    let picture = [
        "image/png",
        "image/jpeg",
        "image/gif",
        "image/webp",
        "image/tiff",
    ]
    .iter()
    .find_map(|want| types.iter().find(|t| t == want));
    let any_picture = types.iter().find(|t| t.starts_with("image/"));
    text.or(picture).or(any_picture).map(String::as_str)
}

/// Asks for `mime`, with a paste's password so the terminal needn't ask the person.
pub fn read_request(mime: &str, password: Option<&str>) -> String {
    let engine = base64::engine::general_purpose::STANDARD;
    let proof = password.map_or_else(String::new, |pw| {
        format!(":pw={pw}:name={}", engine.encode(PASTE_EVENT))
    });
    format!("\x1b]5522;type=read{proof};{}\x1b\\", engine.encode(mime))
}

fn decode(text: &str) -> Option<Vec<u8>> {
    base64::engine::general_purpose::STANDARD.decode(text).ok()
}

fn decode_text(text: &str) -> Option<String> {
    String::from_utf8(decode(text)?).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b64(s: &[u8]) -> String {
        base64::engine::general_purpose::STANDARD.encode(s)
    }

    /// Feeds whole messages, as they'd arrive, and keeps what they add up to.
    fn read(reader: &mut Reader, bodies: &[String]) -> Vec<Pasted> {
        bodies.iter().filter_map(|b| reader.take(b)).collect()
    }

    #[test]
    fn a_paste_event_lists_what_the_clipboard_holds_with_its_password() {
        // As Ghostty 1.4 sends it: the password in every packet, the list ending in a newline.
        let pw = b64(b"one-time");
        let bodies = [
            format!("type=read:status=OK:pw={pw}"),
            format!(
                "type=read:status=DATA:mime=Lg==:pw={pw};{}",
                b64(b"text/plain image/png\n")
            ),
            format!("type=read:status=DONE:pw={pw}"),
        ];
        let got = read(&mut Reader::default(), &bodies);
        assert_eq!(
            got,
            vec![Pasted::Offer {
                types: vec!["text/plain".into(), "image/png".into()],
                password: Some(pw),
            }]
        );
    }

    #[test]
    fn data_comes_back_whole_from_its_chunks() {
        let picture: Vec<u8> = (0..9000u32).map(|i| (i % 251) as u8).collect();
        let mut bodies = vec!["type=read:status=OK".to_string()];
        for chunk in picture.chunks(4096) {
            bodies.push(format!(
                "type=read:status=DATA:mime={};{}",
                b64(b"image/png"),
                b64(chunk)
            ));
        }
        bodies.push("type=read:status=DONE".into());
        let got = read(&mut Reader::default(), &bodies);
        assert_eq!(
            got,
            vec![Pasted::Data {
                mime: "image/png".into(),
                bytes: picture
            }]
        );
    }

    #[test]
    fn a_refusal_says_why() {
        let got = read(
            &mut Reader::default(),
            &["type=read:status=EPERM".to_string()],
        );
        assert_eq!(got, vec![Pasted::Refused("EPERM".into())]);
        // What isn't a read, such as a write's answer, is none of this.
        assert_eq!(Reader::default().take("type=write:status=DONE"), None);
    }

    #[test]
    fn text_is_chosen_over_a_picture_and_png_over_other_pictures() {
        let types = |list: &[&str]| list.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            choose(&types(&["image/png", "text/plain"])),
            Some("text/plain")
        );
        assert_eq!(
            choose(&types(&["image/tiff", "image/png"])),
            Some("image/png")
        );
        // A Mac screenshot can be only TIFF.
        assert_eq!(choose(&types(&["image/tiff"])), Some("image/tiff"));
        assert_eq!(choose(&types(&["text/html"])), None);
    }

    #[test]
    fn a_read_carries_the_pastes_password_and_name() {
        assert_eq!(
            read_request("image/png", Some("cHc=")),
            "\x1b]5522;type=read:pw=cHc=:name=UGFzdGUgZXZlbnQ=;aW1hZ2UvcG5n\x1b\\"
        );
        assert!(has_paste_events(b"\x1b[?5522;2$y"));
        assert!(!has_paste_events(b"\x1b[?5522;0$y"));
    }
}
