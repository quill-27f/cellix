//! The comments on a document, beside it: each one a short conversation with whoever's
//! working on it, where the agent's answer lands under the question.

use cellix_render::{Bounds, Font, Grid, Icon, Measure, Rect, Style};
use cellix_review::threads::YOU;
use cellix_review::{About, Thread};

use crate::measures::BUTTON_GAP;
use crate::theme::Theme;

const SIDE: u16 = 2;
const CARD_RADIUS: u8 = 33;
/// How far a card reaches above its first row and below its last. Text sits low in its row
/// and buttons fill theirs, so the two differ to look the same.
const CARD_AIR: (f32, f32) = (0.17, 0.6);
/// How far a card reaches past the pane's text edge either side: it starts a column from
/// the pane's edge, and its own text two columns inside it.
const CARD_REACH: f32 = 1.0;
/// How far an agent's answer sits in from the card's text.
const INSET: f32 = 0.6;
const BUTTONS: [&str; 3] = ["Reply", "Ask agent", "Resolve"];
/// On the reviewer's own comment, after those, or on a row of their own when they don't fit.
const EDITS: [&str; 2] = ["Edit", "Delete"];
const CHOICES: [&str; 2] = ["Keep", "Dismiss"];
const REWRITE: [&str; 2] = ["Apply", "Not this"];
const REOPEN: [&str; 1] = ["Reopen"];
const HIDE_RESOLVED: &str = "Hide resolved";
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
/// How far the summary and Check my review are lifted in their row, so the pills under
/// them have half a row of clear air.
const CHECK_LIFT: f32 = 0.25;
/// How far a file's heading is lifted in its row, clear of the card under it.
const FILE_LIFT: f32 = 0.3;
const BUTTON_PADDING: usize = 1;
/// The size buttons set their text at, as `Grid::button` does.
pub(crate) const BUTTON_TEXT: u8 = crate::measures::BUTTON_TEXT;
pub(crate) const SMALL: u8 = 85;

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
    /// Opens a resolved comment again.
    Reopen(u32),
    /// Changes the reviewer's own comment, or removes it.
    Edit(u32),
    Delete(u32),
    /// Shows the resolved comments too, or hides them again.
    ToggleResolved,
    /// Opens the file a heading names, by a comment on it.
    File(u32),
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
    /// Nothing open; true when there are resolved comments to show.
    Empty(bool),
    /// The button that asks the agent to check the review, after how many comments are the
    /// reviewer's and how many the agent's, and how many files they're on when the list
    /// covers every file.
    Check(usize, usize, usize),
    /// The heading over a file's comments, when the list covers every file: a comment on it
    /// for a click to open it by, none for a terminal's output, then its name and how many.
    File(Option<u32>, String, usize),
    /// How many comments of each kind there are, so what's not covered stands out. A narrow
    /// panel takes more than one row.
    Coverage(Vec<(&'static str, usize)>),
    /// At the top of a card: who wrote it, and its kind. True for an agent's suggestion.
    /// And, for a plan's comment on words its next version changed or dropped, a pill saying
    /// so, when it fits beside them.
    Header(u32, String, bool, Option<(String, Words)>),
    /// That pill on a row of its own, under a header too wide to share with it.
    Pill(u32, String, Words),
    /// Keep and Dismiss, under a suggestion.
    Choices(u32),
    /// Edit and Delete, under the other buttons when they don't all fit on one row.
    Edits(u32),
    /// Apply and Not this, under a rewrite still to be decided.
    Rewrite(u32),
    /// What a comment is about, and what became of those words in a plan's next version.
    Quote(u32, String, Words),
    Text(u32, String),
    /// Opens an answer: its verdict if it gives one, and who wrote it.
    Answer(u32, Option<String>, String),
    /// A row of an answer, set in from the comment it answers.
    Reply(u32, String),
    Status(String),
    /// A row of what's being written, and where the caret is on it, in bytes, when it is.
    Input(String, Option<usize>),
    Hint,
    Buttons(u32),
    /// Reopen, under a resolved comment.
    Reopen(u32),
    /// The button that shows the resolved comments, how many there are, and whether they show.
    Resolved(usize, bool),
    Gap,
}

/// What became of a plan comment's words in a later version of the plan.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Words {
    Here,
    /// Reworded: on a quiet bar, as they were said about.
    Changed,
    /// Dropped: on a quiet bar and struck through.
    Removed,
}

/// The pill on a plan comment whose words a later version changed or dropped, and how.
fn words_of(thread: &Thread) -> (Words, Option<String>) {
    match thread.earlier.map(|v| v + 1) {
        Some(version) if thread.reworded => (
            Words::Changed,
            Some(format!("Changed in version {version} \u{203a}")),
        ),
        Some(version) => (
            Words::Removed,
            Some(format!("Removed in version {version} \u{203a}")),
        ),
        None => (Words::Here, None),
    }
}

/// A pill's colour: the warning colour for words that are gone, a quiet one for words
/// only reworded.
fn pill_tone(words: Words, theme: &Theme) -> cellix_render::Rgb {
    if words == Words::Changed {
        theme.gutter.fg
    } else {
        theme.attention
    }
}

/// Room between a header's words and a pill beside them, about 9 px.
const PILL_AIR: f32 = 1.0;

/// A comment or reply being written, in place, so what it's about stays in view.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Draft {
    /// The thread it answers. None starts a new one.
    pub thread: Option<u32>,
    pub file: String,
    pub line: usize,
    pub quote: String,
    /// All that was selected, when it ran past the quote's one line.
    pub selection: String,
    pub text: String,
    /// Where the caret is in `text`, in bytes.
    pub caret: usize,
    /// Goes straight to the agent when it's done.
    pub ask: bool,
    /// It's about code, so it also goes with the review.
    pub code: bool,
    /// Terminal output or a problem it's about, for the thread it becomes.
    pub about: Option<About>,
    /// It's the comment of `thread` being changed, in place of its text, not a reply to it.
    pub editing: bool,
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
/// What a card says it's about: a problem as the checker said it, else the words quoted.
fn shown_quote(quote: &str, line: usize, about: Option<&About>) -> String {
    match about {
        Some(About::Problem { message }) => message.clone(),
        _ => quote_of(quote, line),
    }
}

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
    measure.width(text, small()) + PILL_ROOM * 2.0
}

/// Columns either side of a pill's words, and between two pills.
const PILL_ROOM: f32 = 1.0;
const PILL_GAP: f32 = 1.0;
/// How far a card's buttons are dropped in their row, in rows, so they stand half a row
/// under the words above them: 13 px was the tightest words-to-button gap left.
const ACTIONS_DROP: f32 = 0.14;

/// A pill outlined in `tone`, its words in it, starting at `x`.
fn draw_pill(
    grid: &mut Grid,
    x: f32,
    y: u16,
    text: &str,
    (right, inner, tone): (u16, Style, cellix_render::Rgb),
    measure: &dyn Measure,
) {
    let width = pill_width(text, measure);
    let bounds = Bounds::cells(x, y as f32 + 0.16, width, 0.68);
    grid.panel(bounds, 34, None, Some(tone));
    let tag = Style { fg: tone, ..inner };
    let at = x + PILL_ROOM;
    grid.label_at(at, at.floor() as u16, y, text, tag, right, SMALL);
}

/// `a` over `b`, `amount` of the way.
fn mix(a: cellix_render::Rgb, b: cellix_render::Rgb, amount: f32) -> cellix_render::Rgb {
    let channel = |a: u8, b: u8| (a as f32 * amount + b as f32 * (1.0 - amount)).round() as u8;
    cellix_render::Rgb(channel(a.0, b.0), channel(a.1, b.1), channel(a.2, b.2))
}

/// Breaks what's being written into rows at spaces, as `wrap` does, as byte ranges of it.
/// Spaces stay where they were typed, at the end of a row, so a space moves the caret.
pub fn wrap_spans(text: &str, width: f32, measure: &dyn Measure) -> Vec<(usize, usize)> {
    let font = Font::default();
    let mut rows = Vec::new();
    let mut start = 0;
    for paragraph in text.split('\n') {
        let mut row = start;
        let mut word = start;
        for piece in paragraph.split(' ') {
            let end = word + piece.len();
            let before = &text[row..word];
            if !before.trim().is_empty() && measure.width(&text[row..end], font) > width {
                rows.push((row, word));
                row = word;
            }
            word = end + 1;
        }
        let end = start + paragraph.len();
        rows.push((row, end));
        start = end + 1;
    }
    rows
}

fn draft_lines(draft: &Draft, width: f32, measure: &dyn Measure, out: &mut Vec<Line>) {
    let spans = wrap_spans(&draft.text, width, measure);
    let caret = draft.caret.min(draft.text.len());
    // At a row's end and the next one's start, it's at the start of the next.
    let on = spans
        .iter()
        .rposition(|(start, end)| *start <= caret && caret <= *end);
    for (index, (start, end)) in spans.into_iter().enumerate() {
        let at = (on == Some(index)).then(|| caret - start);
        out.push(Line::Input(draft.text[start..end].to_string(), at));
    }
    out.push(Line::Hint);
}

/// What the panel shows: the comments, the one being written, the agent's name, how many
/// comments are resolved and whether they're among those shown, and whether each file's
/// stand under a heading, as they do when no one file is in view.
pub type Shown<'a> = (
    &'a [&'a Thread],
    Option<&'a Draft>,
    &'a str,
    (usize, bool),
    bool,
);

/// Where a comment is, as its heading says it: its file, or the terminal tab whose output
/// it's on.
pub fn place_of(thread: &Thread) -> String {
    match &thread.about {
        Some(About::Output { tab, .. }) => format!("{tab} output"),
        _ => thread.file.clone(),
    }
}

fn lines(
    (threads, draft, agent, resolved, grouped): Shown,
    width: f32,
    measure: &dyn Measure,
) -> Vec<Line> {
    let mut out = vec![Line::Gap];
    // The counts are of what's still open, whether or not the resolved ones show.
    let open: Vec<&Thread> = threads.iter().copied().filter(|t| !t.resolved).collect();
    if !open.is_empty() {
        let mine = open.iter().filter(|t| t.suggested_by.is_empty()).count();
        let mut places: Vec<String> = open.iter().map(|t| place_of(t)).collect();
        places.sort();
        places.dedup();
        let files = if grouped { places.len() } else { 0 };
        out.push(Line::Check(mine, open.len() - mine, files));
        let count = |kind: &str| open.iter().filter(|t| t.kind == kind).count();
        // As many pills to a row as fit.
        let mut row = Vec::new();
        let mut used = 0.0;
        for kind in KINDS {
            let pill = pill_width(&format!("{kind} {}", count(kind)), measure) + PILL_GAP;
            if used + pill > width + 2.0 + PILL_GAP && !row.is_empty() {
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
            shown_quote(&new.quote, new.line, new.about.as_ref()),
            Words::Here,
        ));
        draft_lines(new, width, measure, &mut out);
        out.push(Line::Buttons(NEW_DRAFT));
        out.push(Line::Gap);
    } else if open.is_empty() {
        out.push(Line::Empty(resolved.0 > 0));
    }
    let mut place = None;
    for thread in threads {
        let id = thread.id;
        // The list comes a file at a time, so a heading goes where the file changes.
        let here = place_of(thread);
        if grouped && place.as_ref() != Some(&here) {
            let count = threads.iter().filter(|t| place_of(t) == here).count();
            let on_output = matches!(thread.about, Some(About::Output { .. }));
            out.push(Line::File((!on_output).then_some(id), here.clone(), count));
            place = Some(here);
        }
        let suggested = !thread.suggested_by.is_empty();
        let kind = if thread.kind.is_empty() {
            String::new()
        } else {
            format!(" · {}", thread.kind)
        };
        // On a transcript, which step it's about, as the agent is told.
        let kind = if thread.step.is_empty() {
            kind
        } else {
            format!(" · {}{kind}", thread.step)
        };
        let by = if suggested {
            format!("Suggested by {}", thread.suggested_by)
        } else {
            YOU.to_string()
        };
        // Words a plan's next version changed or dropped: still said, and marked as such.
        // The pill sits at the header's end when both fit, and under it when they don't.
        let done = if thread.resolved { " · Resolved" } else { "" };
        let label = format!("{by}{kind}{done}");
        let (words, pill) = words_of(thread);
        let fits = |pill: &String| {
            measure.width(&label, small()) + PILL_AIR + pill_width(pill, measure) <= width
        };
        let (beside, under) = match pill {
            Some(pill) if fits(&pill) => (Some((pill, words)), None),
            under => (None, under),
        };
        out.push(Line::Header(id, label, suggested, beside));
        if let Some(pill) = under {
            out.push(Line::Pill(id, pill, words));
        }
        let quote = shown_quote(&thread.quote, thread.line, thread.about.as_ref());
        out.push(Line::Quote(id, quote, words));
        let editing = draft.filter(|d| d.thread == Some(id) && d.editing);
        match editing {
            Some(edit) => draft_lines(edit, width, measure, &mut out),
            None => {
                let text = wrap(&thread.text, width, measure);
                out.extend(text.into_iter().map(|row| Line::Text(id, row)));
            }
        }
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
        let waiting = !suggested && !thread.resolved && thread.waiting_answer();
        if waiting && thread.told == Some(thread.replies.len()) && !agent.is_empty() {
            out.push(Line::Status(format!("Waiting for {agent}…")));
        } else if waiting && thread.asked {
            out.push(Line::Status(format!(
                "{agent} sees this with its next prompt"
            )));
        }
        if let Some(reply) = draft.filter(|d| d.thread == Some(id) && !d.editing) {
            draft_lines(reply, width, measure, &mut out);
        }
        // The last word a rewrite: it's for the reviewer to apply or not.
        if !thread.resolved
            && thread
                .replies
                .last()
                .is_some_and(|r| rewrite(&r.text).is_some())
        {
            out.push(Line::Rewrite(id));
        }
        out.push(if thread.resolved {
            Line::Reopen(id)
        } else if suggested {
            Line::Choices(id)
        } else {
            Line::Buttons(id)
        });
        // Edit and Delete go on a row of their own where the five don't fit on one.
        if !thread.resolved && !suggested && !all_fit(width, measure) {
            out.push(Line::Edits(id));
        }
        out.push(Line::Gap);
    }
    if resolved.0 > 0 {
        out.push(Line::Resolved(resolved.0, resolved.1));
        out.push(Line::Gap);
    }
    out
}

/// What the button for resolved comments says.
fn resolved_label(count: usize, shown: bool) -> String {
    if shown {
        HIDE_RESOLVED.to_string()
    } else {
        format!("Show {count} resolved")
    }
}

/// That button's cells and start column.
fn resolved_button(area: Rect, count: usize, shown: bool, measure: &dyn Measure) -> (String, u16) {
    (
        button_cells(&resolved_label(count, shown), measure),
        area.x + SIDE + 1,
    )
}

/// Each button as (label, padded cells, start column): Edit and Delete with the others when
/// all five fit.
fn buttons(area: Rect, measure: &dyn Measure) -> Vec<(&'static str, String, u16)> {
    let width = area.w.saturating_sub(SIDE * 2 + 2) as f32;
    let mut placed = row_of(area, &BUTTONS, measure);
    if all_fit(width, measure) {
        placed.extend(edits(area, measure));
    }
    placed
}

/// Columns either side of a plain-word button's letters, for its wash under the pointer.
const PLAIN_ROOM: f32 = 0.5;
/// Columns between two of them.
const PLAIN_GAP: u16 = 1;

/// The cells a button of plain words takes: Edit and Delete, the least of a card's actions.
fn plain_cells(label: &str, measure: &dyn Measure) -> String {
    if measure.in_cells() {
        return format!(" {label} ");
    }
    let font = Font {
        size: BUTTON_TEXT,
        ..Font::default()
    };
    let width = (measure.width(label, font) + PLAIN_ROOM * 2.0).ceil() as usize;
    " ".repeat(width.max(2))
}

/// Edit and Delete as (label, cells, start column), ending at the card's far text edge, so
/// they stand apart from what answers the comment.
fn edits(area: Rect, measure: &dyn Measure) -> Vec<(&'static str, String, u16)> {
    let mut end = area.right().saturating_sub(SIDE + 1);
    let mut placed = Vec::new();
    for label in EDITS.iter().rev() {
        let cells = plain_cells(label, measure);
        let x = end.saturating_sub(cells.chars().count() as u16);
        placed.push((*label, cells, x));
        end = x.saturating_sub(PLAIN_GAP);
    }
    placed.reverse();
    placed
}

/// Whether the five fit on one row of a card `width` wide: the three chips, then Edit and
/// Delete with a button's gap before them.
fn all_fit(width: f32, measure: &dyn Measure) -> bool {
    let gap = BUTTON_GAP as usize;
    let chips: usize = BUTTONS
        .iter()
        .map(|label| button_cells(label, measure).chars().count() + gap)
        .sum();
    let words: usize = EDITS
        .iter()
        .map(|label| plain_cells(label, measure).chars().count() + PLAIN_GAP as usize)
        .sum();
    (chips + words) as f32 <= width + f32::from(PLAIN_GAP)
}

/// Check my review, at the right of its row, as (padded cells, start column). In pixels it
/// has room for the agent's icon before its words.
fn check_button(area: Rect, measure: &dyn Measure) -> (String, u16) {
    let mut cells = button_cells(CHECK, measure);
    if !measure.in_cells() {
        cells.push_str(&" ".repeat(CHECK_ICON as usize));
    }
    let width = cells.chars().count() as u16;
    let x = area.right().saturating_sub(SIDE + width).max(area.x + SIDE);
    (cells, x)
}

/// The cells a button takes: its letters and a space either side where every letter is a
/// cell, since a card's buttons are filled there, and in pixels the room every button has.
pub(crate) fn button_cells(label: &str, measure: &dyn Measure) -> String {
    let padding = " ".repeat(BUTTON_PADDING);
    if measure.in_cells() {
        return format!("{padding}{label}{padding}");
    }
    crate::measures::button_cells(label, measure)
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
            x += cells.chars().count() as u16 + BUTTON_GAP;
            (*label, cells, at)
        })
        .collect()
}

/// The furthest a panel `height` rows tall scrolls over `rows` of comments: the last card
/// whole, with the air under it.
fn last_of(rows: usize, height: u16) -> usize {
    (rows + 1).saturating_sub(height as usize)
}

/// The furthest the panel scrolls with what it shows.
pub fn last_top(area: Rect, shown: Shown, measure: &dyn Measure) -> usize {
    let width = area.w.saturating_sub(SIDE * 2 + 2) as f32;
    last_of(lines(shown, width, measure).len(), area.h)
}

pub fn hit(
    area: Rect,
    shown: Shown,
    top: usize,
    (x, y): (u16, u16),
    measure: &dyn Measure,
) -> Option<Hit> {
    let width = area.w.saturating_sub(SIDE * 2 + 2) as f32;
    let all = lines(shown, width, measure);
    let top = top.min(last_of(all.len(), area.h));
    let row = top + y.checked_sub(area.y)? as usize;
    match all.get(row)? {
        Line::Buttons(id) => {
            let pressed = buttons(area, measure)
                .into_iter()
                .find(|(_, cells, at)| x >= *at && x < at + cells.chars().count() as u16);
            match pressed?.0 {
                "Reply" => Some(Hit::Reply(*id)),
                "Ask agent" => Some(Hit::Ask(*id)),
                "Edit" => Some(Hit::Edit(*id)),
                "Delete" => Some(Hit::Delete(*id)),
                _ => Some(Hit::Resolve(*id)),
            }
        }
        Line::Edits(id) => {
            let pressed = edits(area, measure)
                .into_iter()
                .find(|(_, cells, at)| x >= *at && x < at + cells.chars().count() as u16);
            match pressed?.0 {
                "Edit" => Some(Hit::Edit(*id)),
                _ => Some(Hit::Delete(*id)),
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
        Line::File(opens, ..) => opens.map(Hit::File),
        Line::Reopen(id) => {
            let (_, cells, at) = row_of(area, &REOPEN, measure).pop()?;
            (x >= at && x < at + cells.chars().count() as u16).then_some(Hit::Reopen(*id))
        }
        Line::Resolved(count, shown) => {
            let (cells, at) = resolved_button(area, *count, *shown, measure);
            (x >= at && x < at + cells.chars().count() as u16).then_some(Hit::ToggleResolved)
        }
        Line::Quote(id, ..)
        | Line::Text(id, _)
        | Line::Answer(id, ..)
        | Line::Reply(id, _)
        | Line::Pill(id, ..)
        | Line::Header(id, ..) => Some(Hit::Open(*id)),
        _ => None,
    }
}

pub fn draw(
    grid: &mut Grid,
    area: Rect,
    shown: Shown,
    top: usize,
    hover: Option<(u16, u16)>,
    (measure, theme): (&dyn Measure, &Theme),
) {
    grid.fill(area, theme.sidebar);
    let width = area.w.saturating_sub(SIDE * 2 + 2) as f32;
    let threads = shown.0;
    let all = lines(shown, width, measure);
    let top = top.min(last_of(all.len(), area.h));
    let (side, right) = (area.x + SIDE, area.right().saturating_sub(SIDE));
    let dim = Style {
        fg: theme.gutter.fg,
        ..theme.sidebar
    };
    let body = theme.sidebar;
    // Each card is drawn whole before what's on it, so text sits on top. A suggestion's is a
    // shade of the accent until it's kept, so it reads as offered rather than chosen.
    // A card is raised over the pane, as a menu is over the page.
    let card = theme.popup;
    let suggested_card = mix(theme.accent, card, 0.06);
    let card_of = |id: u32| {
        if threads
            .iter()
            .any(|t| t.id == id && !t.suggested_by.is_empty())
        {
            suggested_card
        } else {
            card
        }
    };
    let row_y = |index: usize| area.y as f32 + index as f32 - top as f32;
    let shown = |y: f32, height: f32| y + height > area.y as f32 && y < area.bottom() as f32;
    let mut start = None;
    for (index, line) in all.iter().enumerate() {
        match line {
            Line::Header(..) => start = Some(index),
            Line::Quote(..) if start.is_none() => start = Some(index),
            Line::Buttons(_) if matches!(all.get(index + 1), Some(Line::Edits(_))) => {}
            Line::Buttons(_) | Line::Edits(_) | Line::Choices(_) | Line::Reopen(_) => {
                if let Some(first) = start.take() {
                    let (y, height) = (row_y(first), (index - first + 1) as f32);
                    // A column from the pane's edge, so its text stands two inside it.
                    let bounds = Bounds::cells(
                        side as f32 - CARD_REACH,
                        y - CARD_AIR.0,
                        (right - side) as f32 + CARD_REACH * 2.0,
                        height + CARD_AIR.0 + CARD_AIR.1,
                    );
                    let (fill, edge) = if matches!(line, Line::Choices(_)) {
                        (suggested_card, mix(theme.accent, card, 0.5))
                    } else {
                        (card, theme.border)
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
        bg: card,
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
        let from = grid.painted_so_far();
        let actions = matches!(
            line,
            Line::Buttons(_)
                | Line::Edits(_)
                | Line::Choices(_)
                | Line::Reopen(_)
                | Line::Rewrite(_)
        );
        match line {
            Line::Status(text) => {
                grid.label_at(text_x as f32, text_x, y, text, inner_dim, right, SMALL);
            }
            Line::Input(text, caret) => {
                grid.label(text_x, y, text, inner, right);
                if let Some(at) = caret {
                    let font = Font::default();
                    let x = text_x as f32 + measure.width(&text[..*at], font);
                    grid.set_caret(x, y, 0.62);
                }
            }
            Line::Hint => {
                let hint = "Enter sends. Shift+Enter starts a new line. Esc cancels.";
                grid.label_at(text_x as f32, text_x, y, hint, inner_dim, right, SMALL);
            }
            Line::Empty(resolved) => {
                let none = if *resolved {
                    "No open comments."
                } else {
                    "No comments yet."
                };
                grid.label(side, y, none, dim, right);
                let hint = "Select text in a file, then press Alt+C.";
                grid.label_at(side as f32, side, y + 1, hint, dim, right, SMALL);
            }
            Line::Header(_, label, suggested, pill) => {
                let style = Style {
                    fg: if *suggested {
                        theme.accent
                    } else {
                        theme.gutter.fg
                    },
                    ..inner
                };
                let mut end = right;
                if let Some((text, words)) = pill {
                    // At the row's end: what it's about has changed or left the plan.
                    let width = pill_width(text, measure);
                    let x = right as f32 - 1.0 - width;
                    let tone = pill_tone(*words, theme);
                    draw_pill(grid, x, y, text, (right, inner, tone), measure);
                    end = x.floor() as u16;
                }
                grid.label_at(text_x as f32, text_x, y, label, style, end, SMALL);
            }
            Line::Pill(_, text, words) => {
                let tone = pill_tone(*words, theme);
                draw_pill(grid, text_x as f32, y, text, (right, inner, tone), measure);
            }
            Line::Quote(_, quote, words) => {
                let bar = Bounds::cells(text_x as f32, y as f32 + 0.16, 0.22, 0.68);
                let x = text_x as f32 + 0.8;
                if *words == Words::Here {
                    grid.panel(bar, 10, Some(theme.accent), None);
                    grid.label_at(x, text_x + 1, y, quote, inner_dim, right, SMALL);
                } else {
                    // Words no longer as said: a quiet bar, the words still readable, and a
                    // line through them once they're gone. The pill says which.
                    let quiet = mix(theme.gutter.fg, inner.bg, 0.55);
                    grid.panel(bar, 10, Some(quiet), None);
                    grid.label_at(x, text_x + 1, y, quote, inner_dim, right, SMALL);
                    if *words == Words::Removed {
                        let width = measure.width(quote, small()).min(right as f32 - x);
                        let through = Bounds::cells(x, y as f32 + 0.5, width, 0.0);
                        grid.hairline(through, theme.gutter.fg);
                    }
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
            Line::File(opens, place, count) => {
                let from = grid.painted_so_far();
                let on_it = hover.is_some_and(|h| h.1 == y && h.0 >= side && h.0 < right);
                if opens.is_some() && on_it {
                    let wash = Bounds::cells(
                        side as f32 - 0.5,
                        y as f32 + 0.055,
                        (right - side) as f32 + 1.0,
                        0.89,
                    );
                    grid.panel(wash, 18, Some(theme.row_hover), None);
                }
                // How many, on a chip at the row's end.
                let number = count.to_string();
                let wide = measure.width(&number, small());
                let chip = (wide + 0.9).max(1.3);
                let chip_x = right as f32 - chip;
                let bounds = Bounds::cells(chip_x, y as f32 + 0.22, chip, 0.56);
                grid.panel(bounds, 40, Some(theme.tint), None);
                let at = chip_x + (chip - wide) / 2.0;
                let cell = right.saturating_sub(number.chars().count() as u16 + 1);
                grid.label_at(at, cell, y, &number, body, right, SMALL);
                let stop = (chip_x.floor() as u16).saturating_sub(1);
                // A file's own icon in its own colour, then its folder quietly and its name.
                let (folder, name) = match place.rsplit_once('/') {
                    Some((folder, name)) if opens.is_some() => (format!("{folder}/"), name),
                    _ => (String::new(), place.as_str()),
                };
                let (icon, colour, fallback) = if opens.is_some() {
                    (Icon::File, crate::explorer::file_color(name), "▪")
                } else {
                    (Icon::Terminal, theme.gutter.fg, ">")
                };
                grid.glyph(side as f32, y, icon, colour, fallback, body);
                let start = side + 2;
                let end = grid.label(start, y, &folder, dim, stop);
                let at = start as f32 + measure.width(&folder, Font::default());
                grid.label_at(at, end, y, name, body, stop, 100);
                // Its card starts right under its row, so it stands a little up in it, with
                // air from the card's edge and still more from the card before.
                grid.lift_since(from, FILE_LIFT);
            }
            Line::Check(mine, suggested, files) => {
                let from = grid.painted_so_far();
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
                    // The pill, then the agent's icon and the words beside it, as one. Its
                    // cells keep the pane's colour, since the pill is lifted off its row.
                    let style = Style {
                        bg: body.bg,
                        ..style
                    };
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
                let mut note = match suggested {
                    0 => format!("{mine} of yours"),
                    n => format!("{mine} of yours · {n} suggested"),
                };
                match files {
                    0 => {}
                    1 => note.push_str(" · 1 file"),
                    n => note.push_str(&format!(" · {n} files")),
                }
                let end = x.saturating_sub(1);
                grid.label_at(side as f32, side, y, &note, dim, end, SMALL);
                // Half a row clear over the pills under it.
                grid.lift_since(from, CHECK_LIFT);
            }
            Line::Coverage(kinds) => {
                // A pill for each kind: filled where there are comments of it, and only
                // outlined and quiet where there are none, so the filled ones read first.
                let mut x = side as f32;
                for (kind, count) in kinds {
                    let text = format!("{kind} {count}");
                    let width = pill_width(&text, measure);
                    let bounds = Bounds::cells(x, y as f32 + 0.16, width, 0.68);
                    let missing = *count == 0;
                    if missing {
                        grid.panel(bounds, 34, None, Some(theme.popup_border));
                    } else {
                        grid.panel(bounds, 34, Some(theme.quiet_button(body.bg)), None);
                    }
                    let style = Style {
                        fg: if missing {
                            theme.gutter.fg
                        } else {
                            theme.text.fg
                        },
                        ..body
                    };
                    let at = x + PILL_ROOM;
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
                        theme.quiet_button(card)
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
                    // Edit and Delete are the least of them: plain words until pointed at.
                    let plain = EDITS.contains(&label);
                    let fill = match (on_it, plain) {
                        (true, _) => theme.popup_hover,
                        (false, true) => card,
                        (false, false) => theme.quiet_button(card),
                    };
                    let style = Style {
                        bg: fill,
                        fg: if plain && !on_it {
                            theme.popup_dim
                        } else {
                            inner.fg
                        },
                        ..inner
                    };
                    grid.button(x, y, &cells, label, fill, style);
                }
            }
            Line::Edits(_) => {
                for (label, cells, x) in edits(area, measure) {
                    let width = cells.chars().count() as u16;
                    let on_it = hover.is_some_and(|h| h.1 == y && h.0 >= x && h.0 < x + width);
                    // Plain words until pointed at, as beside the other three.
                    let fill = if on_it { theme.popup_hover } else { card };
                    let style = Style {
                        bg: fill,
                        fg: if on_it { inner.fg } else { theme.popup_dim },
                        ..inner
                    };
                    grid.button(x, y, &cells, label, fill, style);
                }
            }
            Line::Reopen(_) => {
                for (label, cells, x) in row_of(area, &REOPEN, measure) {
                    let width = cells.chars().count() as u16;
                    let on_it = hover.is_some_and(|h| h.1 == y && h.0 >= x && h.0 < x + width);
                    let fill = if on_it {
                        theme.popup_hover
                    } else {
                        theme.quiet_button(card)
                    };
                    let style = Style {
                        bg: fill,
                        fg: inner.fg,
                        ..inner
                    };
                    grid.button(x, y, &cells, label, fill, style);
                }
            }
            Line::Resolved(count, shown) => {
                // Quiet, as Dismiss is: words only, filled while pointed at. Nothing is painted
                // under the pane, so the button's corners need a ground, as Check my review's do.
                let (cells, x) = resolved_button(area, *count, *shown, measure);
                let width = cells.chars().count() as u16;
                let on_it = hover.is_some_and(|h| h.1 == y && h.0 >= x && h.0 < x + width);
                let ground = Bounds::cells(x as f32, y as f32, width as f32, 1.0);
                grid.panel(ground, 0, Some(body.bg), None);
                let fill = if on_it { theme.popup_hover } else { body.bg };
                let style = Style {
                    bg: fill,
                    fg: if on_it {
                        theme.text.fg
                    } else {
                        theme.gutter.fg
                    },
                    ..body
                };
                let label = resolved_label(*count, *shown);
                grid.button(x, y, &cells, &label, fill, style);
            }
            Line::Gap => {}
        }
        if actions {
            grid.lift_since(from, -ACTIONS_DROP);
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
