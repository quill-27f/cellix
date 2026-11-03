//! The comments on a document, beside it: each one a short conversation with whoever's
//! working on it, where the agent's answer lands under the question.

use cellix_render::{Bounds, Font, Grid, Measure, Rect, Style};
use cellix_review::Thread;

use crate::theme::Theme;

const SIDE: u16 = 2;
const CARD_RADIUS: u8 = 22;
const BUTTONS: [&str; 3] = ["Reply", "Ask agent", "Resolve"];
const CHOICES: [&str; 2] = ["Keep", "Dismiss"];
const CHECK: &str = "Check my review";
/// The kinds of comment a careful review covers, in the order the strip shows them.
pub const KINDS: [&str; 5] = [
    "Clarity",
    "Correctness",
    "Evidence",
    "Structure",
    "Reproducibility",
];
const BUTTON_PADDING: usize = 1;
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
}

/// An agent's verdict on a comment, from how its reply starts: `Agree (Evidence): …` or
/// `Disagree: …`. Gives what to show, the kind if it names one, and the rest of the reply.
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
    /// The button that asks the agent to check the review.
    Check(usize),
    /// How many comments of each kind there are, so what's not covered stands out.
    Coverage(Vec<(&'static str, usize)>),
    /// Above a suggestion: who suggested it, and its kind.
    Suggested(u32, String),
    /// Keep and Dismiss, under a suggestion.
    Choices(u32),
    Quote(u32, String),
    Text(u32, String),
    Author(u32, String),
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

/// What a comment is about, with a stand-in for a blank line.
fn quote_of(quote: &str, line: usize) -> String {
    if quote.is_empty() {
        format!("Line {line}")
    } else {
        quote.to_string()
    }
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
        out.push(Line::Check(mine));
        let count = |kind: &str| threads.iter().filter(|t| t.kind == kind).count();
        out.push(Line::Coverage(
            KINDS.iter().map(|k| (*k, count(k))).collect(),
        ));
        out.push(Line::Gap);
    }
    if let Some(new) = draft.filter(|d| d.thread.is_none()) {
        out.push(Line::Quote(NEW_DRAFT, quote_of(&new.quote, new.line)));
        draft_lines(new, width, measure, &mut out);
        out.push(Line::Buttons(NEW_DRAFT));
        out.push(Line::Gap);
    } else if threads.is_empty() {
        out.push(Line::Empty);
    }
    for thread in threads {
        let id = thread.id;
        let suggested = !thread.suggested_by.is_empty();
        if suggested {
            let kind = if thread.kind.is_empty() {
                String::new()
            } else {
                format!(" · {}", thread.kind)
            };
            out.push(Line::Suggested(
                id,
                format!("Suggested by {}{kind}", thread.suggested_by),
            ));
        }
        out.push(Line::Quote(id, quote_of(&thread.quote, thread.line)));
        let text = wrap(&thread.text, width, measure);
        out.extend(text.into_iter().map(|row| Line::Text(id, row)));
        for reply in &thread.replies {
            // A verdict reads as one: "Claude agrees", then why.
            let (author, body) = match verdict(&reply.text) {
                Some((shown, _, rest)) => (format!("{} {shown}", reply.author), rest),
                None => (reply.author.clone(), reply.text.as_str()),
            };
            out.push(Line::Author(id, author));
            let text = wrap(body, width, measure);
            out.extend(text.into_iter().map(|row| Line::Text(id, row)));
        }
        if thread.told == Some(thread.replies.len()) && !agent.is_empty() && thread.waiting_answer()
        {
            out.push(Line::Status(format!("Waiting for {agent}…")));
        } else if thread.asked && thread.waiting_answer() {
            out.push(Line::Status(format!(
                "{agent} sees this with its next prompt"
            )));
        }
        if let Some(reply) = draft.filter(|d| d.thread == Some(id)) {
            draft_lines(reply, width, measure, &mut out);
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
fn buttons(area: Rect) -> Vec<(&'static str, String, u16)> {
    row_of(area, &BUTTONS)
}

fn row_of(area: Rect, labels: &[&'static str]) -> Vec<(&'static str, String, u16)> {
    let mut x = area.x + SIDE + 1;
    labels
        .iter()
        .map(|label| {
            let padding = " ".repeat(BUTTON_PADDING);
            let cells = format!("{padding}{label}{padding}");
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
            let pressed = buttons(area)
                .into_iter()
                .find(|(_, cells, at)| x >= *at && x < at + cells.chars().count() as u16);
            match pressed?.0 {
                "Reply" => Some(Hit::Reply(*id)),
                "Ask agent" => Some(Hit::Ask(*id)),
                _ => Some(Hit::Resolve(*id)),
            }
        }
        Line::Choices(id) => {
            let pressed = row_of(area, &CHOICES)
                .into_iter()
                .find(|(_, cells, at)| x >= *at && x < at + cells.chars().count() as u16);
            match pressed?.0 {
                "Keep" => Some(Hit::Keep(*id)),
                _ => Some(Hit::Dismiss(*id)),
            }
        }
        Line::Check(_) => {
            let (_, cells, at) = row_of(area, &[CHECK]).pop()?;
            (x >= at && x < at + cells.chars().count() as u16).then_some(Hit::Check)
        }
        Line::Quote(id, _) | Line::Text(id, _) | Line::Author(id, _) | Line::Suggested(id, _) => {
            Some(Hit::Open(*id))
        }
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
    // Each card is drawn whole before what's on it, so text sits on top.
    let mut start = None;
    for (index, line) in all.iter().enumerate() {
        match line {
            Line::Suggested(..) => start = Some(index),
            Line::Quote(..) if start.is_none() => start = Some(index),
            Line::Buttons(_) | Line::Choices(_) => {
                if let Some(first) = start.take() {
                    let y = area.y as f32 + first as f32 - top as f32;
                    let height = (index - first + 1) as f32;
                    let bounds =
                        Bounds::cells(side as f32, y - 0.1, (right - side) as f32, height + 0.35);
                    // A suggestion's card has the accent's edge until it's kept.
                    let edge = if matches!(line, Line::Choices(_)) {
                        theme.accent
                    } else {
                        theme.border
                    };
                    if y + height > area.y as f32 && y < area.bottom() as f32 {
                        grid.panel(bounds, CARD_RADIUS, Some(theme.text.bg), Some(edge));
                    }
                }
            }
            _ => {}
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
    for (index, line) in all.iter().enumerate().skip(top) {
        let y = area.y + (index - top) as u16;
        if y >= area.bottom() {
            break;
        }
        let text_x = side + 1;
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
            Line::Quote(_, quote) => {
                let bar = Bounds::cells(text_x as f32, y as f32 + 0.16, 0.22, 0.68);
                grid.panel(bar, 10, Some(theme.accent), None);
                let x = text_x as f32 + 0.8;
                grid.label_at(x, text_x + 1, y, quote, inner_dim, right, SMALL);
            }
            Line::Text(_, text) => {
                grid.label(text_x, y, text, inner, right);
            }
            Line::Author(_, name) => {
                let strong = Style {
                    bold: true,
                    ..inner_dim
                };
                grid.label_at(text_x as f32, text_x, y, name, strong, right, SMALL);
            }
            Line::Check(mine) => {
                for (label, cells, x) in row_of(area, &[CHECK]) {
                    let fill = theme.button;
                    let style = Style {
                        bg: fill,
                        fg: theme.button_text,
                        ..body
                    };
                    grid.button(x, y, &cells, label, fill, style);
                    let note = format!("{mine} of yours");
                    let after = x + cells.chars().count() as u16 + 2;
                    grid.label_at(after as f32, after, y, &note, dim, right, SMALL);
                }
            }
            Line::Coverage(kinds) => {
                // Each kind and its count; a kind with none is in the warning colour.
                let mut x = side as f32 + 0.5;
                for (kind, count) in kinds {
                    let text = format!("{kind} {count}");
                    let style = Style {
                        fg: if *count == 0 {
                            theme.attention
                        } else {
                            theme.text.fg
                        },
                        ..body
                    };
                    let cell = x.floor() as u16;
                    grid.label_at(x, cell, y, &text, style, right, SMALL);
                    x += measure.width(
                        &text,
                        Font {
                            size: SMALL,
                            ..Font::default()
                        },
                    ) + 1.2;
                }
            }
            Line::Suggested(_, label) => {
                let accent = Style {
                    fg: theme.accent,
                    ..inner
                };
                grid.label_at(text_x as f32, text_x, y, label, accent, right, SMALL);
            }
            Line::Choices(_) => {
                for (label, cells, x) in row_of(area, &CHOICES) {
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
            Line::Buttons(NEW_DRAFT) => {}
            Line::Buttons(_) => {
                for (label, cells, x) in buttons(area) {
                    let width = cells.chars().count() as u16;
                    let on_it = hover.is_some_and(|h| h.1 == y && h.0 >= x && h.0 < x + width);
                    let main = label == "Ask agent";
                    let fill = match (main, on_it) {
                        (true, _) => theme.button,
                        (false, true) => theme.popup_hover,
                        (false, false) => theme.quiet_button(theme.text.bg),
                    };
                    let style = Style {
                        bg: fill,
                        fg: if main { theme.button_text } else { inner.fg },
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
