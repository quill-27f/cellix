//! The comments on a document, beside it: each one a short conversation with whoever's
//! working on it, where the agent's answer lands under the question.

use cellix_render::{Bounds, Font, Grid, Measure, Rect, Style};
use cellix_review::Thread;

use crate::theme::Theme;

const SIDE: u16 = 2;
const CARD_RADIUS: u8 = 22;
const BUTTONS: [&str; 3] = ["Reply", "Ask agent", "Resolve"];
const BUTTON_PADDING: usize = 1;
const SMALL: u8 = 85;

/// What a click in the panel lands on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    Open(u32),
    Reply(u32),
    Ask(u32),
    Resolve(u32),
}

/// One row of the panel.
enum Line {
    Empty,
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
fn wrap(text: &str, width: f32, measure: &dyn Measure) -> Vec<String> {
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
        out.push(Line::Quote(id, quote_of(&thread.quote, thread.line)));
        let text = wrap(&thread.text, width, measure);
        out.extend(text.into_iter().map(|row| Line::Text(id, row)));
        for reply in &thread.replies {
            out.push(Line::Author(id, reply.author.clone()));
            let text = wrap(&reply.text, width, measure);
            out.extend(text.into_iter().map(|row| Line::Text(id, row)));
        }
        if thread.told == Some(thread.replies.len()) && !agent.is_empty() && thread.waiting_answer()
        {
            out.push(Line::Status(format!("Waiting for {agent}…")));
        }
        if let Some(reply) = draft.filter(|d| d.thread == Some(id)) {
            draft_lines(reply, width, measure, &mut out);
        }
        out.push(Line::Buttons(id));
        out.push(Line::Gap);
    }
    out
}

/// Each button as (label, padded cells, start column).
fn buttons(area: Rect) -> Vec<(&'static str, String, u16)> {
    let mut x = area.x + SIDE + 1;
    BUTTONS
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
        Line::Quote(id, _) | Line::Text(id, _) | Line::Author(id, _) => Some(Hit::Open(*id)),
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
            Line::Quote(..) => start = Some(index),
            Line::Buttons(_) => {
                if let Some(first) = start.take() {
                    let y = area.y as f32 + first as f32 - top as f32;
                    let height = (index - first + 1) as f32;
                    let bounds =
                        Bounds::cells(side as f32, y - 0.1, (right - side) as f32, height + 0.35);
                    if y + height > area.y as f32 && y < area.bottom() as f32 {
                        grid.panel(bounds, CARD_RADIUS, Some(theme.text.bg), Some(theme.border));
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
    fn text_wraps_at_spaces() {
        let rows = wrap("one two three four", 9.0, &CellMeasure);
        assert_eq!(rows, ["one two", "three", "four"]);
    }

    #[test]
    fn a_long_word_stays_whole() {
        assert_eq!(wrap("abcdefghij", 4.0, &CellMeasure), ["abcdefghij"]);
    }
}
