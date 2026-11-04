//! The comments on a document, beside it: each one a short conversation with whoever's
//! working on it, where the agent's answer lands under the question.

use cellix_render::{Bounds, Font, Grid, Icon, Measure, Rect, Style};
use cellix_review::Thread;
use cellix_review::threads::YOU;

use crate::theme::Theme;

const SIDE: u16 = 2;
const CARD_RADIUS: u8 = 33;
/// How far a card reaches above its first row and below its last. Text sits low in its row
/// and buttons fill theirs, so the two differ to look the same.
const CARD_AIR: (f32, f32) = (0.17, 0.45);
/// How far an agent's answer sits in from the card's text.
const INSET: f32 = 0.6;
const BUTTONS: [&str; 3] = ["Reply", "Ask agent", "Resolve"];
const CHOICES: [&str; 2] = ["Keep", "Dismiss"];
const REWRITE: [&str; 2] = ["Apply", "Not this"];
const CHECK: &str = "Check my review";
/// Columns the agent's icon takes on Check my review.
const CHECK_ICON: u16 = 2;
/// The kinds of comment a careful review covers, in the order the strip shows them.
pub const KINDS: [&str; 5] = [
    "Clarity",
    "Correctness",
    "Evidence",
    "Structure",
    "Reproducibility",
];
const BUTTON_PADDING: usize = 1;
/// Columns either side of a button's text, in pixels, about 24 px.
const BUTTON_ROOM: f32 = 1.4;
/// The size buttons set their text at, as `Grid::button` does.
const BUTTON_TEXT: u8 = 90;
const SMALL: u8 = 85;

/// What a click in the panel lands on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    Open(u32),
    Reply(u32),
    Ask(u32),
    Resolve(u32),
    /// Asks the agent to check the review and suggest more.
    Check,
    /// Takes an agent's suggestion as the reviewer's own, or drops it.
    Keep(u32),
    Dismiss(u32),
    /// Puts an agent's rewrite in the document as a suggestion, or turns it down.
    Apply(u32),
    Decline(u32),
}

/// An agent's verdict on a comment, from how its reply starts: `Agree (Evidence): …` or
/// `Disagree: …`. Gives what to show, the kind if it names one, and the rest of the reply.
/// The new words of an agent's rewrite, from its reply.
pub fn rewrite(text: &str) -> Option<&str> {
    text.strip_prefix("Rewrite: ")
}

pub fn verdict(text: &str) -> Option<(&'static str, &str, &str)> {
    let (shown, rest) = match text.strip_prefix("Agree") {
        Some(rest) => ("agrees", rest),
        None => ("pushes back", text.strip_prefix("Disagree")?),
    };
    let rest = rest.trim_start();
    let (kind, rest) = match rest.strip_prefix('(').and_then(|r| r.split_once(')')) {
        Some((kind, after)) => (kind.trim(), after),
        None => ("", rest),
    };
    let rest = rest.trim_start_matches(':').trim_start();
    Some((shown, kind, rest))
}

/// One row of the panel.
enum Line {
    Empty,
    /// The button that asks the agent to check the review, after how many comments are the
    /// reviewer's and how many the agent's.
    Check(usize, usize),
    /// How many comments of each kind there are, so what's not covered stands out. A narrow
    /// panel takes more than one row.
    Coverage(Vec<(&'static str, usize)>),
    /// At the top of a card: who wrote it, and its kind. True for an agent's suggestion.
    /// And, for a plan's comment on words its next version dropped, the version that did.
    Header(u32, String, bool, Option<usize>),
    /// Keep and Dismiss, under a suggestion.
    Choices(u32),
    /// Apply and Not this, under a rewrite still to be decided.
    Rewrite(u32),
    /// True for words a plan's next version dropped: struck, and on a quiet bar.
    Quote(u32, String, bool),
    Text(u32, String),
    /// Opens an answer: its verdict if it gives one, and who wrote it.
    Answer(u32, Option<String>, String),
    /// A row of an answer, set in from the comment it answers.
    Reply(u32, String),
    Status(String),
    Input(String),
    Hint,
    Buttons(u32),
    Gap,
}

/// A comment or reply being written, in place, so what it's about stays in view.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Draft {
    /// The thread it answers. None starts a new one.
    pub thread: Option<u32>,
    pub file: String,
    pub line: usize,
    pub quote: String,
    pub text: String,
    /// Goes straight to the agent when it's done.
    pub ask: bool,
    /// It's about code, so it also goes with the review.
    pub code: bool,
}

const NEW_DRAFT: u32 = 0;

/// Breaks text into rows no wider than `width`, at spaces.
pub fn wrap(text: &str, width: f32, measure: &dyn Measure) -> Vec<String> {
    let font = Font::default();
    let mut rows = Vec::new();
    for paragraph in text.lines() {
        let mut row = String::new();
        for word in paragraph.split(' ') {
            let longer = if row.is_empty() {
                word.to_string()
            } else {
                format!("{row} {word}")
            };
            if measure.width(&longer, font) > width && !row.is_empty() {
                rows.push(std::mem::take(&mut row));
                row = word.to_string();
            } else {
                row = longer;
            }
        }
        rows.push(row);
    }
    rows
}

/// What a comment is about, with a stand-in for a blank line. Math reads without its
/// dollar signs, as it does on the page.
fn quote_of(quote: &str, line: usize) -> String {
    if quote.is_empty() {
        format!("Line {line}")
    } else {
        quote.replace('$', "")
    }
}

/// The first letter raised, so an answer reads as a sentence once its verdict is taken off.
fn sentence(text: &str) -> String {
    let mut chars = text.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(chars).collect()
    })
}

fn small() -> Font {
    Font {
        size: SMALL,
        ..Font::default()
    }
}

/// A coverage pill's width, its text's and the room either side of it.
fn pill_width(text: &str, measure: &dyn Measure) -> f32 {
    measure.width(text, small()) + 0.9
}

const PILL_GAP: f32 = 0.35;

/// `a` over `b`, `amount` of the way.
fn mix(a: cellix_render::Rgb, b: cellix_render::Rgb, amount: f32) -> cellix_render::Rgb {
    let channel = |a: u8, b: u8| (a as f32 * amount + b as f32 * (1.0 - amount)).round() as u8;
    cellix_render::Rgb(channel(a.0, b.0), channel(a.1, b.1), channel(a.2, b.2))
}

fn draft_lines(draft: &Draft, width: f32, measure: &dyn Measure, out: &mut Vec<Line>) {
    let text = wrap(&format!("{} ", draft.text), width, measure);
    out.extend(text.into_iter().map(Line::Input));
    out.push(Line::Hint);
}

fn lines(
    threads: &[&Thread],
    draft: Option<&Draft>,
    agent: &str,
    width: f32,
    measure: &dyn Measure,
) -> Vec<Line> {
    let mut out = vec![Line::Gap];
    if !threads.is_empty() {
        let mine = threads.iter().filter(|t| t.suggested_by.is_empty()).count();
        out.push(Line::Check(mine, threads.len() - mine));
        let count = |kind: &str| threads.iter().filter(|t| t.kind == kind).count();
        // As many pills to a row as fit.
        let mut row = Vec::new();
        let mut used = 0.0;
        for kind in KINDS {
            let pill = pill_width(&format!("{kind} {}", count(kind)), measure) + PILL_GAP;
            if used + pill > width + 1.0 && !row.is_empty() {
                out.push(Line::Coverage(std::mem::take(&mut row)));
                used = 0.0;
            }
            row.push((kind, count(kind)));
            used += pill;
        }
        out.push(Line::Coverage(row));
        out.push(Line::Gap);
    }
    if let Some(new) = draft.filter(|d| d.thread.is_none()) {
        out.push(Line::Quote(
            NEW_DRAFT,
            quote_of(&new.quote, new.line),
            false,
        ));
        draft_lines(new, width, measure, &mut out);
        out.push(Line::Buttons(NEW_DRAFT));
        out.push(Line::Gap);
    } else if threads.is_empty() {
        out.push(Line::Empty);
    }
    for thread in threads {
        let id = thread.id;
        let suggested = !thread.suggested_by.is_empty();
        let kind = if thread.kind.is_empty() {
            String::new()
        } else {
            format!(" · {}", thread.kind)
        };
        let by = if suggested {
            format!("Suggested by {}", thread.suggested_by)
        } else {
            YOU.to_string()
        };
        // Words a plan's next version dropped: still said, and marked as no longer there.
        let removed = thread.earlier.map(|v| v + 1);
        out.push(Line::Header(id, format!("{by}{kind}"), suggested, removed));
        let quote = quote_of(&thread.quote, thread.line);
        out.push(Line::Quote(id, quote, removed.is_some()));
        let text = wrap(&thread.text, width, measure);
        out.extend(text.into_iter().map(|row| Line::Text(id, row)));
        for reply in &thread.replies {
            // A verdict reads as one: its word on a chip, who said it, then why.
            let (word, author, body) = match (verdict(&reply.text), rewrite(&reply.text)) {
                (Some((shown, _, rest)), _) => {
                    (Some(sentence(shown)), reply.author.clone(), sentence(rest))
                }
                (_, Some(words)) => (
                    None,
                    format!("{} suggests a rewrite", reply.author),
                    words.to_string(),
                ),
                _ => (None, reply.author.clone(), reply.text.clone()),
            };
            out.push(Line::Answer(id, word, author));
            let text = wrap(&body, width - INSET * 2.0, measure);
            out.extend(text.into_iter().map(|row| Line::Reply(id, row)));
        }
        // A suggestion is the agent's own, so there's nothing to wait for.
        let waiting = !suggested && thread.waiting_answer();
        if waiting && thread.told == Some(thread.replies.len()) && !agent.is_empty() {
            out.push(Line::Status(format!("Waiting for {agent}…")));
        } else if waiting && thread.asked {
            out.push(Line::Status(format!(
                "{agent} sees this with its next prompt"
            )));
        }
        if let Some(reply) = draft.filter(|d| d.thread == Some(id)) {
            draft_lines(reply, width, measure, &mut out);
        }
        // The last word a rewrite: it's for the reviewer to apply or not.
        if thread
            .replies
            .last()
            .is_some_and(|r| rewrite(&r.text).is_some())
        {
            out.push(Line::Rewrite(id));
        }
        out.push(if suggested {
            Line::Choices(id)
        } else {
            Line::Buttons(id)
        });
        out.push(Line::Gap);
    }
    out
}

/// Each button as (label, padded cells, start column).
fn buttons(area: Rect, measure: &dyn Measure) -> Vec<(&'static str, String, u16)> {
    row_of(area, &BUTTONS, measure)
}

/// Check my review, at the right of its row, as (padded cells, start column). In pixels it
/// has room for the agent's icon before its words.
fn check_button(area: Rect, measure: &dyn Measure) -> (String, u16) {
    let mut cells = button_cells(CHECK, measure);
    if !measure.in_cells() {
        cells.push_str(&" ".repeat(CHECK_ICON as usize));
    }
    let width = cells.chars().count() as u16;
    let x = area
        .right()
        .saturating_sub(SIDE + 1 + width)
        .max(area.x + SIDE + 1);
    (cells, x)
}

/// The cells a button takes: its letters and a space either side where every letter is a
/// cell, and in pixels its set width with the same room either side, whatever its letters.
fn button_cells(label: &str, measure: &dyn Measure) -> String {
    let padding = " ".repeat(BUTTON_PADDING);
    if measure.in_cells() {
        return format!("{padding}{label}{padding}");
    }
    let font = Font {
        size: BUTTON_TEXT,
        ..Font::default()
    };
    let width = (measure.width(label, font) + BUTTON_ROOM * 2.0).ceil() as usize;
    // Hidden under the paint, so only their count matters.
    " ".repeat(width.max(3))
}

fn row_of(
    area: Rect,
    labels: &[&'static str],
    measure: &dyn Measure,
) -> Vec<(&'static str, String, u16)> {
    let mut x = area.x + SIDE + 1;
    labels
        .iter()
        .map(|label| {
            let cells = button_cells(label, measure);
            let at = x;
            x += cells.chars().count() as u16 + 1;
            (*label, cells, at)
        })
        .collect()
}

pub fn hit(
    area: Rect,
    (threads, draft, agent): (&[&Thread], Option<&Draft>, &str),
    top: usize,
    (x, y): (u16, u16),
    measure: &dyn Measure,
) -> Option<Hit> {
    let width = area.w.saturating_sub(SIDE * 2 + 2) as f32;
    let all = lines(threads, draft, agent, width, measure);
    let row = top + y.checked_sub(area.y)? as usize;
    match all.get(row)? {
        Line::Buttons(id) => {
            let pressed = buttons(area, measure)
                .into_iter()
                .find(|(_, cells, at)| x >= *at && x < at + cells.chars().count() as u16);
            match pressed?.0 {
                "Reply" => Some(Hit::Reply(*id)),
                "Ask agent" => Some(Hit::Ask(*id)),
                _ => Some(Hit::Resolve(*id)),
            }
        }
        Line::Choices(id) => {
            let pressed = row_of(area, &CHOICES, measure)
                .into_iter()
                .find(|(_, cells, at)| x >= *at && x < at + cells.chars().count() as u16);
            match pressed?.0 {
                "Keep" => Some(Hit::Keep(*id)),
                _ => Some(Hit::Dismiss(*id)),
            }
        }
        Line::Rewrite(id) => {
            let pressed = row_of(area, &REWRITE, measure)
                .into_iter()
                .find(|(_, cells, at)| x >= *at && x < at + cells.chars().count() as u16);
            match pressed?.0 {
                "Apply" => Some(Hit::Apply(*id)),
                _ => Some(Hit::Decline(*id)),
            }
        }
        Line::Check(..) => {
            let (cells, at) = check_button(area, measure);
            (x >= at && x < at + cells.chars().count() as u16).then_some(Hit::Check)
        }
        Line::Quote(id, ..)
        | Line::Text(id, _)
        | Line::Answer(id, ..)
        | Line::Reply(id, _)
        | Line::Header(id, ..) => Some(Hit::Open(*id)),
        _ => None,
    }
}

pub fn draw(
    grid: &mut Grid,
    area: Rect,
    (threads, draft, agent): (&[&Thread], Option<&Draft>, &str),
    top: usize,
    hover: Option<(u16, u16)>,
    (measure, theme): (&dyn Measure, &Theme),
) {
    grid.fill(area, theme.sidebar);
    let width = area.w.saturating_sub(SIDE * 2 + 2) as f32;
    let all = lines(threads, draft, agent, width, measure);
    let (side, right) = (area.x + SIDE, area.right().saturating_sub(SIDE));
    let dim = Style {
        fg: theme.gutter.fg,
        ..theme.sidebar
    };
    let body = theme.sidebar;
    // Each card is drawn whole before what's on it, so text sits on top. A suggestion's is a
    // shade of the accent until it's kept, so it reads as offered rather than chosen.
    let suggested_card = mix(theme.accent, theme.text.bg, 0.06);
    let card_of = |id: u32| {
        if threads
            .iter()
            .any(|t| t.id == id && !t.suggested_by.is_empty())
        {
            suggested_card
        } else {
            theme.text.bg
        }
    };
    let row_y = |index: usize| area.y as f32 + index as f32 - top as f32;
    let shown = |y: f32, height: f32| y + height > area.y as f32 && y < area.bottom() as f32;
    let mut start = None;
    for (index, line) in all.iter().enumerate() {
        match line {
            Line::Header(..) => start = Some(index),
            Line::Quote(..) if start.is_none() => start = Some(index),
            Line::Buttons(_) | Line::Choices(_) => {
                if let Some(first) = start.take() {
                    let (y, height) = (row_y(first), (index - first + 1) as f32);
                    let bounds = Bounds::cells(
                        side as f32,
                        y - CARD_AIR.0,
                        (right - side) as f32,
                        height + CARD_AIR.0 + CARD_AIR.1,
                    );
                    let (fill, edge) = if matches!(line, Line::Choices(_)) {
                        (suggested_card, mix(theme.accent, theme.text.bg, 0.5))
                    } else {
                        (theme.text.bg, theme.border)
                    };
                    // Offered, not yet the reviewer's: a suggestion's edge is dashed until kept.
                    let offered = matches!(line, Line::Choices(_));
                    if shown(y, height) && offered {
                        grid.dashed_panel(bounds, CARD_RADIUS, Some(fill), edge);
                    } else if shown(y, height) {
                        grid.panel(bounds, CARD_RADIUS, Some(fill), Some(edge));
                    }
                }
            }
            _ => {}
        }
    }
    // An agent's answer is set in, on a shade of its card, so who said what is plain.
    let text_x = side + 1;
    let mut index = 0;
    while index < all.len() {
        if let Line::Answer(id, ..) = all[index] {
            let rows = 1 + all[index + 1..]
                .iter()
                .take_while(|l| matches!(l, Line::Reply(..)))
                .count();
            let (y, height) = (row_y(index), rows as f32);
            if shown(y, height) {
                let bounds = Bounds::cells(
                    text_x as f32,
                    y + 0.08,
                    right as f32 - 0.8 - text_x as f32,
                    height - 0.26,
                );
                let fill = theme.quiet_button(card_of(id));
                grid.panel(bounds, 22, Some(fill), None);
            }
            index += rows;
        } else {
            index += 1;
        }
    }
    let inner = Style {
        fg: theme.text.fg,
        bg: theme.text.bg,
        ..body
    };
    let inner_dim = Style {
        fg: theme.gutter.fg,
        ..inner
    };
    let reply_x = text_x as f32 + INSET;
    // Muted text loses contrast on a raised or tinted surface, so it's lifted there.
    let raised_muted = mix(theme.text.fg, theme.gutter.fg, 0.3);
    for (index, line) in all.iter().enumerate().skip(top) {
        let y = area.y + (index - top) as u16;
        if y >= area.bottom() {
            break;
        }
        match line {
            Line::Status(text) => {
                grid.label_at(text_x as f32, text_x, y, text, inner_dim, right, SMALL);
            }
            Line::Input(text) => {
                grid.label(text_x, y, text, inner, right);
                let last = matches!(all.get(index + 1), Some(Line::Hint));
                if last {
                    let font = Font::default();
                    let x = text_x as f32 + measure.width(text.trim_end(), font);
                    grid.set_caret(x, y, 0.62);
                }
            }
            Line::Hint => {
                let hint = "Enter sends. Shift+Enter starts a new line. Esc cancels.";
                grid.label_at(text_x as f32, text_x, y, hint, inner_dim, right, SMALL);
            }
            Line::Empty => {
                grid.label(side, y, "No comments yet.", dim, right);
                let hint = "Select text in a file, then press Alt+C.";
                grid.label_at(side as f32, side, y + 1, hint, dim, right, SMALL);
            }
            Line::Header(_, label, suggested, removed) => {
                let style = Style {
                    fg: if *suggested {
                        theme.accent
                    } else {
                        theme.gutter.fg
                    },
                    ..inner
                };
                let mut end = right;
                if let Some(version) = removed {
                    // A pill at the row's end, outlined in the warning colour like a kind
                    // with no comments: what it's about has left the plan.
                    let text = format!("Removed in version {version}");
                    let width = pill_width(&text, measure);
                    let x = (right as f32 - 1.0 - width).max(text_x as f32);
                    let bounds = Bounds::cells(x, y as f32 + 0.16, width, 0.68);
                    grid.panel(bounds, 34, None, Some(theme.attention));
                    let tag = Style {
                        fg: theme.attention,
                        ..inner
                    };
                    let at = x + 0.45;
                    grid.label_at(at, at.floor() as u16, y, &text, tag, right, SMALL);
                    end = x.floor() as u16;
                }
                grid.label_at(text_x as f32, text_x, y, label, style, end, SMALL);
            }
            Line::Quote(_, quote, struck) => {
                let bar = Bounds::cells(text_x as f32, y as f32 + 0.16, 0.22, 0.68);
                let x = text_x as f32 + 0.8;
                if *struck {
                    // Words that are gone: a quiet bar, dimmer words, and a line through them.
                    let quiet = mix(theme.gutter.fg, inner.bg, 0.55);
                    grid.panel(bar, 10, Some(quiet), None);
                    let style = Style { fg: quiet, ..inner };
                    grid.label_at(x, text_x + 1, y, quote, style, right, SMALL);
                    let width = measure.width(quote, small()).min(right as f32 - x);
                    grid.hairline(Bounds::cells(x, y as f32 + 0.56, width, 0.0), quiet);
                } else {
                    grid.panel(bar, 10, Some(theme.accent), None);
                    grid.label_at(x, text_x + 1, y, quote, inner_dim, right, SMALL);
                }
            }
            Line::Text(_, text) => {
                grid.label(text_x, y, text, inner, right);
            }
            Line::Answer(id, word, author) => {
                let block = theme.quiet_button(card_of(*id));
                let muted = Style {
                    fg: raised_muted,
                    bg: block,
                    ..inner
                };
                let mut x = reply_x;
                if let Some(word) = word {
                    // Agreeing is the good news; pushing back is what to look at.
                    let tone = if word == "Agrees" {
                        theme.git_added
                    } else {
                        theme.attention
                    };
                    let chip = measure.width(word, small()) + 0.8;
                    // Low in the row, so it keeps air under the block's top edge.
                    let bounds = Bounds::cells(x, y as f32 + 0.3, chip, 0.58);
                    grid.panel(bounds, 32, Some(mix(tone, block, 0.18)), None);
                    let style = Style {
                        fg: mix(tone, theme.text.fg, 0.6),
                        bold: true,
                        ..muted
                    };
                    grid.label_at(x + 0.4, x.floor() as u16, y, word, style, right, SMALL);
                    grid.drop_last(0.09);
                    x += chip + 0.4;
                }
                let at = (x.ceil() as u16).max(text_x + 1);
                grid.label_at(x, at, y, author, muted, right, SMALL);
                grid.drop_last(0.09);
            }
            Line::Reply(id, text) => {
                let style = Style {
                    bg: theme.quiet_button(card_of(*id)),
                    ..inner
                };
                grid.label_at(reply_x, text_x + 1, y, text, style, right, SMALL);
                // Lifted a little, so the block keeps air above the buttons and below its text.
                grid.drop_last(-0.12);
            }
            Line::Check(mine, suggested) => {
                let (cells, x) = check_button(area, measure);
                // Nothing is painted under the pane, so the button's corners need a ground.
                let ground = Bounds::cells(x as f32, y as f32, cells.chars().count() as f32, 1.0);
                grid.panel(ground, 0, Some(body.bg), None);
                let fill = theme.button;
                let style = Style {
                    bg: fill,
                    fg: theme.button_text,
                    ..body
                };
                if measure.in_cells() {
                    grid.button(x, y, &cells, CHECK, fill, style);
                } else {
                    // The pill, then the agent's icon and the words beside it, as one.
                    grid.button(x, y, &cells, "", fill, style);
                    let font = Font {
                        size: BUTTON_TEXT,
                        ..Font::default()
                    };
                    let words = measure.width(CHECK, font);
                    let whole = cells.chars().count() as f32;
                    let start = x as f32 + (whole - words - CHECK_ICON as f32) / 2.0;
                    grid.glyph(start - 0.2, y, Icon::Agents, theme.button_text, " ", style);
                    let at = start + CHECK_ICON as f32;
                    let text = Style {
                        painted: true,
                        ..style
                    };
                    grid.label_at(
                        at,
                        at.floor() as u16,
                        y,
                        CHECK,
                        text,
                        x + whole as u16,
                        BUTTON_TEXT,
                    );
                }
                let note = match suggested {
                    0 => format!("{mine} of yours"),
                    n => format!("{mine} of yours · {n} suggested"),
                };
                let end = x.saturating_sub(1);
                grid.label_at(text_x as f32, text_x, y, &note, dim, end, SMALL);
            }
            Line::Coverage(kinds) => {
                // A pill for each kind; one with no comments is only outlined, in the warning
                // colour, as the gap to look at.
                let mut x = text_x as f32;
                for (kind, count) in kinds {
                    let text = format!("{kind} {count}");
                    let width = pill_width(&text, measure);
                    let bounds = Bounds::cells(x, y as f32 + 0.16, width, 0.68);
                    let missing = *count == 0;
                    if missing {
                        grid.panel(bounds, 34, None, Some(theme.attention));
                    } else {
                        grid.panel(bounds, 34, Some(theme.quiet_button(body.bg)), None);
                    }
                    let style = Style {
                        fg: if missing {
                            theme.attention
                        } else {
                            theme.text.fg
                        },
                        ..body
                    };
                    let at = x + 0.45;
                    grid.label_at(at, at.floor() as u16, y, &text, style, right, SMALL);
                    x += width + PILL_GAP;
                }
            }
            Line::Rewrite(_) => {
                for (label, cells, x) in row_of(area, &REWRITE, measure) {
                    let main = label == "Apply";
                    let fill = if main {
                        theme.button
                    } else {
                        theme.quiet_button(theme.text.bg)
                    };
                    let style = Style {
                        bg: fill,
                        fg: if main { theme.button_text } else { inner.fg },
                        ..inner
                    };
                    grid.button(x, y, &cells, label, fill, style);
                }
            }
            Line::Choices(_) => {
                for (label, cells, x) in row_of(area, &CHOICES, measure) {
                    let width = cells.chars().count() as u16;
                    let on_it = hover.is_some_and(|h| h.1 == y && h.0 >= x && h.0 < x + width);
                    // Keep is the answer; Dismiss stays quiet until it's pointed at.
                    let keep = label == "Keep";
                    let fill = match (on_it, keep) {
                        (true, _) => theme.popup_hover,
                        (false, true) => theme.quiet_button(suggested_card),
                        (false, false) => suggested_card,
                    };
                    let style = Style {
                        bg: fill,
                        fg: if keep || on_it {
                            inner.fg
                        } else {
                            raised_muted
                        },
                        ..inner
                    };
                    grid.button(x, y, &cells, label, fill, style);
                }
            }
            Line::Buttons(NEW_DRAFT) => {}
            Line::Buttons(_) => {
                // Check my review is the panel's one filled button; these are all quiet.
                for (label, cells, x) in buttons(area, measure) {
                    let width = cells.chars().count() as u16;
                    let on_it = hover.is_some_and(|h| h.1 == y && h.0 >= x && h.0 < x + width);
                    let fill = if on_it {
                        theme.popup_hover
                    } else {
                        theme.quiet_button(theme.text.bg)
                    };
                    let style = Style {
                        bg: fill,
                        fg: inner.fg,
                        ..inner
                    };
                    grid.button(x, y, &cells, label, fill, style);
                }
            }
            Line::Gap => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cellix_render::CellMeasure;

    #[test]
    fn a_verdict_reads_its_word_and_kind() {
        assert_eq!(
            verdict("Agree (Evidence): yes"),
            Some(("agrees", "Evidence", "yes"))
        );
        assert_eq!(verdict("Disagree: no"), Some(("pushes back", "", "no")));
        assert_eq!(verdict("Thanks"), None);
    }

    #[test]
    fn text_wraps_at_spaces() {
        let rows = wrap("one two three four", 9.0, &CellMeasure);
        assert_eq!(rows, ["one two", "three", "four"]);
    }

    #[test]
    fn a_long_word_stays_whole() {
        assert_eq!(wrap("abcdefghij", 4.0, &CellMeasure), ["abcdefghij"]);
    }
}
