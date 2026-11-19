//! The terminal's answers to Cellix's questions, told from keys whenever they come.
//!
//! The questions (`detect.rs`) are answered on the input the keys come on, and an answer that
//! comes after its wait has run out is read by the loop like anything else. crossterm makes keys
//! of what it doesn't know: the background's answer became Alt+], a key for each character of
//! `11;rgb:1010/1111/1414`, and Alt+\, and was typed into the open file. Device attributes, the
//! cell size and the modes' reports it swallows whole, but where a read ends just after an
//! answer's ESC it hands over an Esc and then the rest as keys. And where a wait ran out partway
//! through an answer, the rest of it comes with nothing to say what it is.
//!
//! So everything read goes through `Replies`, which knows the shape of every answer Cellix's
//! questions can bring and drops them wherever they arrive. What may be the start of one is held
//! until it's clear, and given back as it was when it isn't one. A wait that ran out says how far
//! it had read into an answer (`owed`), so the rest of it is known for what it is.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// How long after a question's wait ran out its answers may still come. In that time an Esc,
/// or an Alt+] or Alt+_ standing alone, is held a moment in case an answer's rest follows it.
pub const LATE: Duration = Duration::from_secs(10);

const ESC: u8 = 0x1b;
const BEL: u8 = 0x07;
/// The longest an OSC answer's body runs: `rgb:ffff/ffff/ffff` is 18 bytes.
const OSC_BODY: u16 = 64;
/// The longest a graphics answer's body runs, as an error that names a file.
const APC_BODY: u16 = 1024;
/// The most parameter bytes an answer's CSI has.
const CSI_LONG: u8 = 48;
/// How the answers past ESC begin: the background's, `ESC ] 11 ;`, and the graphics', `ESC _ G
/// i =`, which always carries the id it was asked with.
const OSC_START: &[u8] = b"11;";
const APC_START: &[u8] = b"Gi=";

/// How far into an answer the bytes so far are.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum State {
    #[default]
    Idle,
    /// An ESC, which starts every answer.
    Esc,
    /// `ESC ]` and so many bytes of `11;`.
    Osc(u8),
    /// `ESC _` and so many bytes of `Gi=`.
    Apc(u8),
    /// `ESC [` and so many parameter bytes of an answer: device attributes (`? 62 ; 22 c`), a
    /// mode's report (`? 1016 ; 2 $ y`), keyboard flags (`? 1 u`), or a size (`6 ; 37 ; 17 t`).
    Csi {
        private: bool,
        dollar: bool,
        len: u8,
    },
    /// The rest of a CSI whose start a wait read: up to its final byte, whatever it is.
    CsiTail(u8),
    /// An OSC's or APC's body, so long, up to `ESC \` or BEL, and whether an ESC just came.
    Body { osc: bool, len: u16, esc: bool },
}

/// What a byte comes to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Step {
    /// No answer under way: it's input.
    Pass,
    /// It may be an answer's start.
    Hold,
    /// It's an answer's, as is all that's held.
    Drop,
    /// What's held, and this, isn't an answer after all.
    Miss,
    /// The answer under way ended before this, which is taken again from the start.
    Over,
}

impl State {
    fn step(&mut self, byte: u8) -> Step {
        let (next, step) = match *self {
            State::Idle if byte == ESC => (State::Esc, Step::Hold),
            State::Idle => (State::Idle, Step::Pass),
            State::Esc => match byte {
                b']' => (State::Osc(0), Step::Hold),
                b'_' => (State::Apc(0), Step::Hold),
                b'[' => (
                    State::Csi {
                        private: false,
                        dollar: false,
                        len: 0,
                    },
                    Step::Hold,
                ),
                _ => (State::Idle, Step::Miss),
            },
            State::Osc(n) => started(true, n, byte),
            State::Apc(n) => started(false, n, byte),
            State::Csi {
                private,
                dollar,
                len,
            } => csi(private, dollar, len, byte),
            State::CsiTail(len) => match byte {
                0x20..=0x3f if len < CSI_LONG => (State::CsiTail(len + 1), Step::Drop),
                0x40..=0x7e => (State::Idle, Step::Drop),
                _ => (State::Idle, Step::Over),
            },
            State::Body { esc: true, .. } if byte == b'\\' => (State::Idle, Step::Drop),
            State::Body { esc: true, .. } => (State::Idle, Step::Over),
            State::Body { osc, len, .. } => match byte {
                ESC => (
                    State::Body {
                        osc,
                        len,
                        esc: true,
                    },
                    Step::Drop,
                ),
                BEL => (State::Idle, Step::Drop),
                0x20..=0x7e if len < if osc { OSC_BODY } else { APC_BODY } => (
                    State::Body {
                        osc,
                        len: len + 1,
                        esc: false,
                    },
                    Step::Drop,
                ),
                _ => (State::Idle, Step::Over),
            },
        };
        *self = next;
        step
    }

    /// A unit that stands for no bytes, such as a click or a focus change: never an answer's.
    fn interrupt(&mut self) -> Step {
        let step = match self {
            State::Idle => Step::Pass,
            State::Body { .. } | State::CsiTail(_) => Step::Over,
            _ => Step::Miss,
        };
        *self = State::Idle;
        step
    }
}

/// The next byte of an OSC's or APC's start: once it's all there, it's an answer's body.
fn started(osc: bool, n: u8, byte: u8) -> (State, Step) {
    let start = if osc { OSC_START } else { APC_START };
    let at = usize::from(n);
    if start.get(at) != Some(&byte) {
        (State::Idle, Step::Miss)
    } else if at + 1 == start.len() {
        (
            State::Body {
                osc,
                len: 0,
                esc: false,
            },
            Step::Drop,
        )
    } else if osc {
        (State::Osc(n + 1), Step::Hold)
    } else {
        (State::Apc(n + 1), Step::Hold)
    }
}

/// The next byte of a CSI that may be an answer, held until its final byte says it is one.
fn csi(private: bool, dollar: bool, len: u8, byte: u8) -> (State, Step) {
    let more = |private, dollar| {
        (
            State::Csi {
                private,
                dollar,
                len: len + 1,
            },
            Step::Hold,
        )
    };
    let done = (State::Idle, Step::Drop);
    match byte {
        _ if len >= CSI_LONG => (State::Idle, Step::Miss),
        b'?' if len == 0 => more(true, false),
        b'0'..=b'9' if !dollar => more(private, false),
        b';' if len > 0 && !dollar => more(private, false),
        b'$' if private && len > 1 && !dollar => more(true, true),
        b'y' if dollar => done,
        b'c' | b'u' if private && len > 1 && !dollar => done,
        b't' if !private && len > 0 && !dollar => done,
        _ => (State::Idle, Step::Miss),
    }
}

/// Drops the terminal's answers from what's read. A unit is what's read at once (a key event
/// for the loop, a byte in a test) with the bytes it stands for: Alt+] is `ESC ]`, Ctrl+G is
/// BEL. Units that stand for none, such as clicks, end any answer under way.
pub struct Replies<T> {
    state: State,
    /// Units held while they may start an answer, with their bytes.
    held: Vec<(T, Option<Vec<u8>>)>,
    /// The answer under way started in a wait, which read its start: nothing of it is held to
    /// give back if it turns out not to be one.
    seeded: bool,
    /// Until when answers may still come, after a wait that ran out.
    owed: Option<Instant>,
    /// The background's answer read so far, from its start.
    colour: Option<Vec<u8>>,
    /// The last background answer read whole, as `rgb:…`.
    background: Option<Vec<u8>>,
}

impl<T> Default for Replies<T> {
    fn default() -> Self {
        Self {
            state: State::Idle,
            held: Vec::new(),
            seeded: false,
            owed: None,
            colour: None,
            background: None,
        }
    }
}

impl<T> Replies<T> {
    /// Takes one unit, and gives back those that are input, in order: none while it may
    /// still be an answer's start, and what was held with it once it isn't.
    pub fn take(&mut self, unit: T, bytes: Option<&[u8]>) -> Vec<T> {
        let mut out = Vec::new();
        let mut queue = VecDeque::from([(unit, bytes.map(<[u8]>::to_vec))]);
        while let Some((unit, bytes)) = queue.pop_front() {
            let step = match &bytes {
                Some(bytes) => self.feed(bytes),
                None => {
                    self.colour = None;
                    self.state.interrupt()
                }
            };
            match step {
                Step::Pass => out.push(unit),
                Step::Hold => self.held.push((unit, bytes)),
                Step::Drop => {
                    self.held.clear();
                    if self.state == State::Idle {
                        self.seeded = false;
                    }
                }
                Step::Miss => {
                    let mut again: VecDeque<_> = std::mem::take(&mut self.held).into();
                    again.push_back((unit, bytes));
                    // The first can't start an answer; what came after it is looked at again.
                    // Where the start was read in a wait, none of what's held was the answer's.
                    if !std::mem::take(&mut self.seeded) {
                        out.extend(again.pop_front().map(|(unit, _)| unit));
                    }
                    while let Some(unit) = again.pop_back() {
                        queue.push_front(unit);
                    }
                }
                Step::Over => {
                    self.seeded = false;
                    queue.push_front((unit, bytes));
                }
            }
        }
        out
    }

    fn feed(&mut self, bytes: &[u8]) -> Step {
        let mut step = Step::Pass;
        for &byte in bytes {
            let before = self.state;
            step = self.state.step(byte);
            match (before, self.state, step) {
                (State::Osc(_), State::Body { .. }, _) => self.colour = Some(Vec::new()),
                (State::Body { osc: true, .. }, State::Body { esc: false, .. }, Step::Drop) => {
                    if let Some(colour) = &mut self.colour {
                        colour.push(byte);
                    }
                }
                (State::Body { osc: true, .. }, State::Idle, Step::Drop) => {
                    self.background = self.colour.take().or(self.background.take());
                }
                (_, _, Step::Miss | Step::Over) => self.colour = None,
                _ => {}
            }
            if matches!(step, Step::Miss | Step::Over) {
                break;
            }
        }
        step
    }

    /// No more input came: what's held wasn't an answer's start after all, and goes on as it
    /// was. Not where a wait read the start: the rest of that answer is still to come.
    pub fn release(&mut self) -> Vec<T> {
        if self.seeded || self.held.is_empty() {
            return Vec::new();
        }
        self.state = State::Idle;
        self.held.drain(..).map(|(unit, _)| unit).collect()
    }

    /// Whether something is held that, with answers owed, may be an answer's start, so more is
    /// worth waiting a moment for before what's held is given back.
    pub fn waiting(&self, now: Instant) -> bool {
        self.holding() && self.owed.is_some_and(|until| now < until)
    }

    /// Whether anything is held, owed or not.
    pub fn holding(&self) -> bool {
        !self.held.is_empty()
    }

    /// Takes what a wait left owing: answers that may still come, and how far into one it
    /// read, so the rest of that one is dropped too.
    pub fn owe(&mut self, owed: Owed) {
        self.owed = Some(self.owed.map_or(owed.until, |was| was.max(owed.until)));
        if let Some(seed) = owed.seed
            && self.state == State::Idle
            && self.held.is_empty()
        {
            self.state = seed;
            self.seeded = true;
        }
    }

    /// The background colour the last answer read whole said, as `rgb:…`, once.
    pub fn take_background(&mut self) -> Option<Vec<u8>> {
        self.background.take()
    }
}

/// What a wait left owing: its answers may still come, and it may have read partway into one.
#[derive(Clone, Copy, Debug)]
pub struct Owed {
    seed: Option<State>,
    until: Instant,
}

/// What the waits have left owing, until the loop takes it, and until when answers may come.
struct Owing {
    until: Option<Instant>,
    fresh: Option<Owed>,
}

static OWING: Mutex<Owing> = Mutex::new(Owing {
    until: None,
    fresh: None,
});

/// What the questions' waits have left owing since this was last asked, if anything.
pub fn owed() -> Option<Owed> {
    OWING.lock().ok()?.fresh.take()
}

/// Whether an earlier question's answers may still come, so a wait now may read them.
#[cfg(unix)]
pub(crate) fn owing(now: Instant) -> bool {
    OWING
        .lock()
        .is_ok_and(|owing| owing.until.is_some_and(|until| now < until))
}

/// A wait that ran out, or that may have read an earlier question's answers for its own:
/// answers may still come, and `read` says how far into one it got.
#[cfg(unix)]
pub(crate) fn owe(read: &Read) {
    let Ok(mut owing) = OWING.lock() else {
        return;
    };
    let until = Instant::now() + LATE;
    owing.until = Some(until);
    owing.fresh = Some(Owed {
        seed: read.seed(),
        until,
    });
}

/// What a wait read, followed byte by byte, to know whether it ended partway into an answer.
#[cfg(any(unix, test))]
#[derive(Default)]
pub(crate) struct Read(Replies<()>);

#[cfg(any(unix, test))]
impl Read {
    pub(crate) fn feed(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.0.take((), Some(&[byte]));
        }
    }

    /// Where the bytes read so far end: inside an answer, and how far. A CSI read in a wait is
    /// one, whatever it is, so its rest goes up to its final byte.
    fn seed(&self) -> Option<State> {
        match self.0.state {
            State::Idle => None,
            State::Csi { len, .. } => Some(State::CsiTail(len)),
            state => Some(state),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Filters a byte stream read in `reads`, each byte a unit, as the loop would: nothing
    /// given back between reads, which come before the loop's patience runs out.
    fn filtered(replies: &mut Replies<u8>, reads: &[&[u8]]) -> Vec<u8> {
        let mut out = Vec::new();
        for read in reads {
            for &byte in *read {
                out.extend(replies.take(byte, Some(&[byte])));
            }
        }
        out
    }

    /// The same once no more comes.
    fn whole(reads: &[&[u8]]) -> Vec<u8> {
        let mut replies = Replies::default();
        let mut out = filtered(&mut replies, reads);
        out.extend(replies.release());
        out
    }

    /// Every answer Cellix's questions can bring, ended each way it can end, as terminals,
    /// tmux and Zellij send them.
    const ANSWERS: [&[u8]; 12] = [
        b"\x1b]11;rgb:1010/1111/1414\x1b\\",
        b"\x1b]11;rgb:ffff/fefe/fdfd\x07",
        b"\x1b]11;rgba:0000/0000/0000/ffff\x1b\\",
        b"\x1b[?62;22c",
        b"\x1b[?1;2c",
        b"\x1b[6;37;17t",
        b"\x1b[4;888;1394t",
        b"\x1b[?1016;2$y",
        b"\x1b[?5522;0$y",
        b"\x1b[?1u",
        b"\x1b_Gi=31;OK\x1b\\",
        b"\x1b_Gi=32;EINVAL:refused, no file at /tmp/x\x1b\\",
    ];

    #[test]
    fn every_answer_is_dropped_and_the_keys_round_it_kept() {
        for answer in ANSWERS {
            let stream = [b"ab".as_slice(), answer, b"cd"].concat();
            assert_eq!(
                whole(&[&stream]),
                b"abcd",
                "{:?}",
                String::from_utf8_lossy(answer)
            );
        }
        // All of them at once, as a slow terminal sends the lot.
        let all = ANSWERS.concat();
        assert_eq!(whole(&[&all, b"x"]), b"x");
    }

    /// An answer can come in pieces, a read each, split anywhere: it's dropped all the same.
    #[test]
    fn an_answer_in_pieces_across_reads_is_dropped() {
        for answer in ANSWERS {
            let stream = [answer, b"x"].concat();
            for cut in 1..stream.len() {
                let (first, second) = stream.split_at(cut);
                assert_eq!(
                    whole(&[first, second]),
                    b"x",
                    "{:?} cut at {cut}",
                    String::from_utf8_lossy(answer)
                );
            }
            // A byte a read.
            let bytes: Vec<&[u8]> = stream.chunks(1).collect();
            assert_eq!(whole(&bytes), b"x", "{:?}", String::from_utf8_lossy(answer));
        }
    }

    /// A wait that ran out partway through an answer says how far it read, and the rest that
    /// comes later is dropped: from every point in every answer.
    #[test]
    fn the_rest_of_an_answer_a_wait_began_is_dropped() {
        for answer in ANSWERS {
            for cut in 1..answer.len() {
                let (started, rest) = answer.split_at(cut);
                let mut read = Read::default();
                read.feed(started);
                let mut replies = Replies::default();
                replies.owe(Owed {
                    seed: read.seed(),
                    until: Instant::now() + LATE,
                });
                let mut out = filtered(&mut replies, &[rest, b"x"]);
                out.extend(replies.release());
                assert_eq!(
                    out,
                    b"x",
                    "{:?} read up to {cut}",
                    String::from_utf8_lossy(answer)
                );
            }
        }
        // A wait that ended between answers leaves nothing to drop.
        let mut read = Read::default();
        read.feed(b"\x1b[?62c");
        assert_eq!(read.seed(), None);
    }

    /// What a person types is never an answer: Alt+] or Esc alone, text with `rgb:` in it,
    /// and keys that start as an answer would and then don't go on as one.
    #[test]
    fn typing_that_looks_like_an_answer_still_arrives() {
        for typed in [
            b"\x1b]".as_slice(),
            b"\x1b",
            b"\x1b_",
            b"11;rgb:1010/1111/1414",
            b"colour = rgb:ff/ff/ff;",
            b"\x1b]5522;type=read\x1b\\",
            b"\x1b]1x",
            b"\x1b_Gx",
            b"\x1b[?62;22x",
            b"\x1b[6;37;17A",
            b"\x1b[?1016$c",
            b"\x1bx\x1b\x1b]q",
            b"\x1b\\",
        ] {
            assert_eq!(
                whole(&[typed]),
                typed,
                "{:?}",
                String::from_utf8_lossy(typed)
            );
        }
        // Held while it may be an answer's start, and given back once no more comes.
        let mut replies = Replies::default();
        assert!(filtered(&mut replies, &[b"\x1b]1"]).is_empty());
        assert_eq!(replies.release(), b"\x1b]1");
        // An answer right after a key that looked like one's start: the key, then nothing.
        assert_eq!(whole(&[b"\x1b\x1b]11;rgb:1/1/1\x07"]), b"\x1b");
    }

    /// Held starts wait a moment for their rest only while answers are owed.
    #[test]
    fn a_held_start_waits_only_while_answers_are_owed() {
        let now = Instant::now();
        let mut replies = Replies::default();
        assert!(filtered(&mut replies, &[b"\x1b"]).is_empty());
        assert!(!replies.waiting(now), "nothing owed");
        replies.owe(Owed {
            seed: None,
            until: now + LATE,
        });
        assert!(replies.waiting(now));
        assert!(!replies.waiting(now + LATE));
        assert_eq!(replies.release(), b"\x1b");
        assert!(!replies.waiting(now), "nothing held");
    }

    /// The colour a late answer says is kept for the loop, the latest whole one.
    #[test]
    fn a_late_background_answer_says_its_colour() {
        let mut replies = Replies::default();
        let out = filtered(
            &mut replies,
            &[b"\x1b]11;rgb:1010/1111/1414\x1b\\a\x1b]11;rgb:eeee/eeee/eeee\x07"],
        );
        assert_eq!(out, b"a");
        assert_eq!(
            replies.take_background().as_deref(),
            Some(b"rgb:eeee/eeee/eeee".as_slice())
        );
        assert_eq!(replies.take_background(), None);
        // One cut short says nothing.
        filtered(&mut replies, &[b"\x1b]11;rgb:1010"]);
        replies.take(0, None);
        assert_eq!(replies.take_background(), None);
    }
}
