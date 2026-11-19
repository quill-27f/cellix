//! The comments on a document, beside it: each one a short conversation with whoever's
//! working on it, where the agent's answer lands under the question.

use std::borrow::Cow;
use std::cell::{Cell, RefCell};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::rc::Rc;

use cellix_render::{Bounds, Font, Grid, Icon, Measure, Paint, Rect, Style};
use cellix_review::threads::{ADDRESSED, Reply, YOU};
use cellix_review::{About, Thread};

use crate::measures::BUTTON_GAP;
use crate::theme::Theme;

/// Columns from the pane's edge to its text edge, where every view in the pane starts its
/// words, and where the list's head and a card's edge stand.
const SIDE: u16 = 2;
const CARD_RADIUS: u8 = 33;
/// A card's buttons say what the palette calls the same thing, in its Title Case: "Ask agent"
/// stood on a card beside "Ask Agent" in its own hint, Problems and the selection's buttons.
const ASK: &str = "Ask Agent";
const BUTTONS: [&str; 3] = ["Reply", ASK, "Resolve"];
/// On the reviewer's own comment, after those, or behind a "⋯" at the row's end when they
/// don't fit: on a row of their own, a narrow card had three rows of buttons.
const EDITS: [&str; 2] = ["Edit", "Delete"];
/// The button that opens a menu of Edit and Delete, where they don't fit beside the others.
const MORE: &str = "\u{22ef}";
/// On a comment an agent says it addressed: settle it, or say it isn't done.
const ADDRESSED_BUTTONS: [&str; 2] = ["Resolve", NOT_DONE];
const NOT_DONE: &str = "Not Done";
/// Under a suggestion: take it, ask about it without taking it, or drop it.
const CHOICES: [&str; 3] = ["Keep", "Reply", "Dismiss"];
/// Under an agent's rewrite: take it in place of the words, put it in as a suggestion to
/// settle on the page, or turn it down.
const ACCEPT_REWRITE: &str = "Accept Rewrite";
const APPLY_REWRITE: &str = "Apply as Suggestion";
const REWRITE: [&str; 3] = [ACCEPT_REWRITE, APPLY_REWRITE, "Not This"];
const REOPEN: [&str; 1] = ["Reopen"];
/// The palette's "Comments: Check My Review".
const CHECK: &str = "Check My Review";
/// What the row over the comments offers for the document's suggestions.
const SETTLE: [&str; 3] = ["Accept All", "Reject All", "Next"];
/// Under the words that say no comment is open: how one is made.
/// It named only Alt+C, not the button a selection shows, which names the keys as they're set.
const EMPTY_HINT: &str = "Select text, then press Comment.";
/// Columns the agent's icon takes on Check My Review.
const CHECK_ICON: u16 = 2;
/// The kinds of comment a careful review covers, in the order the strip shows them.
pub const KINDS: [&str; 5] = [
    "Clarity",
    "Correctness",
    "Evidence",
    "Structure",
    "Reproducibility",
];
/// How far the summary and Check My Review are lifted in their row, so the pills under
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
    /// A link in a comment's words or a reply: the comment, and which of its links, counting
    /// through its words and then its replies as `links` does.
    Link(u32, usize),
    /// Asks which order to list the comments in.
    Sort,
    /// One of the emoji that stand under a comment being written, by its place among them.
    Quick(usize),
    /// The button after them, which lists every emoji.
    Emoji,
    /// One of the emoji offered for the name being typed, by its place among them.
    Suggested(usize),
    /// The button that adds a picture to the comment being written.
    AddPicture,
    /// Sends the comment being written, as Enter does, or drops it, as Esc does.
    Send,
    Cancel,
    /// A picture on a comment's card: the comment, and which of its pictures, counting
    /// through its words and then its replies as `pictures_of` does.
    Picture(u32, usize),
    Open(u32),
    Reply(u32),
    Ask(u32),
    Resolve(u32),
    /// Asks the agent to check the review and suggest more.
    Check,
    /// Sends every comment no agent has had to the agent, in one message.
    SendAll,
    /// Takes an agent's suggestion as the reviewer's own, or drops it.
    Keep(u32),
    Dismiss(u32),
    /// Puts an agent's rewrite in place of the words it's about, or in the document as a
    /// suggestion, or turns it down.
    AcceptRewrite(u32),
    Apply(u32),
    Decline(u32),
    /// Opens a resolved comment again.
    Reopen(u32),
    /// Changes the reviewer's own comment, or removes it.
    Edit(u32),
    Delete(u32),
    /// Opens a menu of those two, from the "⋯" that stands for them on a narrow card.
    More(u32),
    /// Shows the resolved comments too, or hides them again.
    ToggleResolved,
    /// Opens the file a heading names, by a comment on it.
    File(u32),
    /// Lists only the comments of a kind, from its pill, or all of them again.
    Kind(&'static str),
    /// Picks the kind of the comment being written, from its chip, or none again.
    DraftKind(&'static str),
    /// Shows on a comment's card how its words changed since it was made, or hides it.
    Changes(u32),
    /// Answers an agent that says it addressed a comment, that it isn't done.
    NotDone(u32),
    /// Resolves every comment an agent says it addressed.
    ResolveAddressed,
    /// Settles every suggestion in the document, or goes to the next one.
    AcceptAll,
    RejectAll,
    NextSuggestion,
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
    /// The order the comments are listed in, which a press changes, and how many comments
    /// agents say they addressed, which a button at its end resolves.
    Sort(usize),
    /// Nothing open; true when there are resolved comments to show.
    Empty(bool),
    /// Under it, how a comment is made, a row of it. It has rows of its own, as many as a
    /// narrow pane breaks it into: the button that shows the resolved comments comes next,
    /// or their cards.
    EmptyHint(String),
    /// How many suggestions the document holds, which of them the caret is in, and whether
    /// Accept All, Reject All and Next fit beside the words; when they don't they come next.
    Suggestions(usize, Option<usize>, bool),
    /// Those three on a row of their own.
    Settle,
    /// The button that asks the agent to check the review, after how many comments are the
    /// reviewer's and how many the agent's, and how many files they're on when the list
    /// covers every file. With a kind's pill pressed, that kind and how many of it are open,
    /// which the head says in place of the others: it said "3 of yours · 1 suggested" over the
    /// one card listed.
    Check(usize, usize, usize, Option<(&'static str, usize)>),
    /// Under Check My Review: the button that sends the comments no agent has had, how many.
    Deliver(usize),
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
    /// Resolve and Not Done, on a comment an agent says it addressed.
    Addressed(u32),
    /// Accept Rewrite, Apply as Suggestion and Not This, under a rewrite still to be decided:
    /// which row of them, as many as a narrow card breaks them into.
    Rewrite(u32, usize),
    /// What a comment is about, and what became of those words in a plan's next version.
    Quote(u32, String, Words),
    /// A row of how those words changed since: each word or space, with whether it went (-1),
    /// stayed (0) or came (1).
    Diff(u32, Vec<(i8, String)>),
    Text(u32, String),
    /// Opens an answer, under a line across the card: its verdict if it gives one, and who
    /// wrote it.
    Answer(u32, Option<String>, String),
    /// A row of an answer, on the card's column. It stood in a box whose words were 10 px from
    /// its edge where the card's are 24 from its own.
    Reply(u32, String),
    /// A row of the words an agent's rewrite puts in place of the comment's, in a box of their
    /// own, `BOX_PADDING` round them.
    Proposed(u32, String),
    Status(String),
    /// A row of what's being written, and where the caret is on it, in bytes, when it is.
    Input(String, Option<usize>),
    /// Between what's being written and the tools under it, in their box: the words stand
    /// a little down into it and the tools a little up, so neither is on the box's lines.
    Pad,
    /// Under what's being written: the emoji most reached for, the button for the rest, and
    /// the one that adds a picture.
    Tools,
    /// A row of a picture standing on a card: the comment it's on and which of its pictures,
    /// the word that names it, and which of the picture's rows this is.
    Picture {
        thread: u32,
        which: usize,
        name: String,
        row: usize,
    },
    /// The emoji whose names start as the one being typed after a colon does, each with its
    /// name, in place of the hint.
    Suggest(Vec<(&'static str, &'static str)>),
    Hint,
    /// Under the hint, while a comment of the reviewer's own is made or changed: the kinds, a
    /// chip each, to pick its kind by. As many to a row as fit.
    Kinds(Vec<&'static str>),
    /// Under what's being written: the button that sends it, named for what it does, and
    /// Cancel. The comment it's on, so its card ends here while it's answered or changed.
    Send(u32, &'static str),
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
    /// Only for a pill's colour: an agent says it did what the comment asks.
    Addressed,
}

/// The pills at a card's top: that an agent says it addressed it, and what became of its
/// words. The first stands beside who wrote it where it fits, the rest under it.
fn pills_of(thread: &Thread) -> Vec<(String, Words)> {
    let mut pills = Vec::new();
    if let Some(by) = thread.addressed_by() {
        pills.push((format!("Addressed by {by}"), Words::Addressed));
    }
    let (words, pill) = words_of(thread);
    pills.extend(pill.map(|pill| (pill, words)));
    pills
}

/// The pill on a comment whose words changed or went: in a plan's later version, or in the
/// file since it was made. The card still quotes the words it was made on.
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
        None if thread.gone => (Words::Removed, Some(WORDS_GONE.to_string())),
        None if !thread.now.is_empty() => {
            (Words::Changed, Some(format!("{CHANGED_SINCE} \u{203a}")))
        }
        None => (Words::Here, None),
    }
}

/// What a short card's row of buttons says while the pointer isn't on it: how many answers
/// it has and what it waits on. None with nothing to say, and then its buttons stand.
fn short_note(line: &Line, threads: &[&Thread], agent: &str, view: View) -> Option<String> {
    let (Line::Buttons(id) | Line::Choices(id) | Line::Addressed(id)) = line else {
        return None;
    };
    let thread = threads.iter().find(|t| t.id == *id)?;
    let mut said = Vec::new();
    if !thread.replies.is_empty() {
        said.push(replies_note(thread.replies.len()));
    }
    let asked = thread.suggested_by.is_empty() || thread.replies.iter().any(|r| r.author == YOU);
    let waiting = asked && !thread.resolved && thread.waiting_answer();
    let unsent = view.unsent.is_some_and(|unsent| unsent.has(*id));
    if waiting && (thread.waiting() || unsent) {
        said.push("Not sent yet".to_string());
    } else if waiting && thread.told == Some(thread.replies.len()) {
        said.push(format!("Waiting for {agent}\u{2026}"));
    }
    (!said.is_empty()).then(|| said.join(" \u{b7} "))
}

/// How many rows of its words a short card shows.
const SHORT_ROWS: usize = 2;

/// Whether a comment's card is short: in a narrow pane, while it isn't lit and nothing's
/// being written on it.
fn is_short(id: u32, draft: Option<&Draft>, view: View) -> bool {
    view.compact && view.lit != Some(id) && draft.is_none_or(|d| d.thread != Some(id))
}

/// How a short card says how many answers it has.
fn replies_note(count: usize) -> String {
    match count {
        1 => "1 reply".to_string(),
        n => format!("{n} replies"),
    }
}

/// How far a quote's words stand in from a card's text, past its bar.
const QUOTE_IN: f32 = 0.8;

/// Breaks a word diff into rows no wider than `width`, at its spaces, measured as the
/// quote's small words are.
fn diff_rows(
    changes: Vec<(i8, String)>,
    width: f32,
    measure: &dyn Measure,
) -> Vec<Vec<(i8, String)>> {
    let mut rows = vec![Vec::new()];
    let mut wide = 0.0;
    for (kind, piece) in changes {
        let blank = piece.trim().is_empty();
        let piece_wide = measure.width(&piece, small());
        let row = rows.last_mut().expect("a row");
        if blank && row.is_empty() {
            continue;
        }
        if !blank && !row.is_empty() && wide + piece_wide > width {
            rows.push(Vec::new());
            wide = 0.0;
        }
        rows.last_mut().expect("a row").push((kind, piece));
        wide += piece_wide;
    }
    rows.retain(|row| !row.is_empty());
    rows
}

/// What a card's pill says when the words it was made on read differently now, or went.
pub const CHANGED_SINCE: &str = "Changed since your comment";
pub const WORDS_GONE: &str = "Words gone";

/// A pill's colour: the removed colour for words that are gone, as a diff's removed lines
/// are, a quiet one for words only reworded. Gone words were in the colour of what needs you,
/// which said something else.
fn pill_tone(words: Words, theme: &Theme) -> cellix_render::Rgb {
    match words {
        Words::Changed => theme.gutter.fg,
        Words::Addressed => theme.git_added,
        Words::Removed | Words::Here => theme.error,
    }
}

/// Room between a header's words and a pill beside them, about 9 px.
const PILL_AIR: f32 = 1.0;

/// A comment or reply being written, in place, so what it's about stays in view.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
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
    /// The kind picked for it from the chips under it, empty for none: a comment of the
    /// reviewer's own being made or changed has them, a reply doesn't.
    pub kind: String,
}

const NEW_DRAFT: u32 = 0;

/// How wide a stretch of a paragraph's letters is, from the advances the whole of it was
/// measured to: `from` and `count` are in letters.
fn stretch(advances: &[f32], from: usize, count: usize) -> f32 {
    let end = (from + count).min(advances.len());
    advances[from.min(end)..end].iter().sum()
}

/// Breaks text into rows no wider than `width`, at spaces.
///
/// A paragraph is measured once, whole, and its rows are sums of that. Measuring each row
/// again as it grew by a word asked for a different string per word, and a long list of
/// comments then held more strings than the measure remembers, so every frame shaped all
/// of them afresh: two thirds of a second for three hundred comments.
pub fn wrap(text: &str, width: f32, measure: &dyn Measure) -> Vec<String> {
    let font = Font::default();
    let mut rows = Vec::new();
    for paragraph in text.lines() {
        let advances = measure.advances(paragraph, font);
        let mut row = String::new();
        // How wide the row is so far, and how many of the paragraph's letters are behind.
        let mut wide = 0.0;
        let mut letters = 0;
        for word in paragraph.split(' ') {
            let count = word.chars().count();
            let word_wide = stretch(&advances, letters, count);
            // The space that parts it from the word before.
            let space = stretch(
                &advances,
                letters.saturating_sub(1),
                usize::from(letters > 0),
            );
            letters += count + 1;
            if row.is_empty() {
                row.push_str(word);
                wide = word_wide;
            } else if wide + space + word_wide > width {
                rows.push(std::mem::take(&mut row));
                row.push_str(word);
                wide = word_wide;
            } else {
                row.push(' ');
                row.push_str(word);
                wide += space + word_wide;
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
        // Words over several lines read as one run, as the page shows them.
        let flat: Vec<&str> = quote.split_whitespace().collect();
        flat.join(" ").replace('$', "")
    }
}

/// The first letter raised, so an answer reads as a sentence once its verdict is taken off.
fn sentence(text: &str) -> String {
    // A link or a file's name is spelled as it is.
    if link_spans(text)
        .first()
        .is_some_and(|(start, _)| *start == 0)
    {
        return text.to_string();
    }
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

/// A kind's pill in the strip over the list: its kind and count, and when it's the kind listed
/// alone, the × that says a press lists them all again.
fn coverage_width(kind: &str, count: usize, picked: bool, measure: &dyn Measure) -> f32 {
    let words = pill_width(&format!("{kind} {count}"), measure);
    match (picked, measure.in_cells()) {
        (false, _) => words,
        // " ×" after its count.
        (true, true) => words + 2.0,
        (true, false) => words + CLEAR_ROOM,
    }
}

/// Columns a picked kind's pill keeps after its count for its ×: half a column of air, then
/// the ×'s strokes, the pill's own room after them.
const CLEAR_ROOM: f32 = 1.0;
/// How tall the square the × is drawn in is, in rows: its strokes take 11 of its 24.
const CLEAR_ICON: f32 = 0.5;

/// Columns either side of a pill's words, and between two pills.
const PILL_ROOM: f32 = 1.0;
const PILL_GAP: f32 = 1.0;
/// Where a pill starts in its row and how tall it is, in rows: three quarters of a row,
/// with a tail 2 px inside its foot and a capital 3 px under its top. At two thirds, words
/// of 11 pt stood on its outline.
const PILL: (f32, f32) = (0.12, 0.76);
/// How far a verdict's word and the name after it are dropped in their row, with its chip.
const VERDICT_DROP: f32 = 0.05;
/// How far a card's buttons are dropped in their row, in rows, so they stand half a row
/// under the words above them: 13 px was the tightest words-to-button gap left.
const ACTIONS_DROP: f32 = 0.14;
/// `BOX_PADDING` in columns: how far a rewrite's words stand in from its box's edge. On cells,
/// where there's no box, nothing.
fn box_padding(measure: &dyn Measure) -> f32 {
    crate::measures::columns(crate::measures::BOX_PADDING, measure).unwrap_or(0.0)
}

/// How far a rewrite's box, its words and the answer over them are lifted in their rows, in
/// rows, so the box's foot is `BOX_PADDING` over the buttons under it, which stand as every
/// card's do: its words' tails `BOX_PADDING` over its foot ran it to 4 px into the buttons'
/// row. Its buttons were lifted instead, 10 px under the box and 23 over the comment's own.
fn rewrite_lift(measure: &dyn Measure) -> f32 {
    let (Some(tails), Some((_, row))) = (small_tails(measure), measure.cell_pixels()) else {
        return 0.0;
    };
    let row = row as f32;
    let line = row / measure.row_air().max(1.0);
    let (top, _) = cellix_render::button_in_row(row, line);
    let padding = crate::measures::BOX_PADDING;
    let lift = padding * 2.0 + tails - row - top - ACTIONS_DROP * row;
    (lift / row).max(0.0)
}

/// How far a short card's note is dropped in the row its buttons stand in while it's
/// pointed at, in rows: its words' tails to the buttons' foot, so the card's foot is as far
/// under either.
fn note_drop(measure: &dyn Measure) -> f32 {
    let (Some(tails), Some((_, row))) = (small_tails(measure), measure.cell_pixels()) else {
        return 0.0;
    };
    let row = row as f32;
    let line = row / measure.row_air().max(1.0);
    let (_, foot) = cellix_render::button_in_row(row, line);
    ((foot + ACTIONS_DROP * row - tails) / row).max(0.0)
}

/// Where the tails of words set small, as a note's and an answer's are, end in their row, in
/// pixels: nearer the row's middle than the interface's words' by as much as they're smaller.
/// Taken as the others', a rewrite's box had 19 px under its words where it had 16 over them.
fn small_tails(measure: &dyn Measure) -> Option<f32> {
    let (_, tails) = crate::measures::words_in_row(measure)?;
    let (_, row) = measure.cell_pixels()?;
    let middle = row as f32 / 2.0;
    Some(middle + (tails - middle) * f32::from(SMALL) / 100.0)
}

/// The "⋯" that opens a menu of Edit and Delete: a plain mark, its cells washed while pointed
/// at as a plain-word button's are, and on cells the letter.
fn draw_more(
    grid: &mut Grid,
    (x, y): (u16, u16),
    cells: &str,
    (fill, style): (cellix_render::Rgb, Style),
    measure: &dyn Measure,
) {
    if measure.in_cells() {
        grid.button(x, y, cells, MORE, fill, style);
        return;
    }
    grid.button(x, y, cells, "", fill, style);
    let wide = cells.chars().count() as f32;
    let side = icon_columns(MORE_MARK, measure);
    let left = f32::from(x) + (wide - side) / 2.0;
    let top = f32::from(y) + (1.0 - MORE_MARK) / 2.0;
    grid.paint.push(Paint::Icon {
        bounds: Bounds::cells(left, top, side, MORE_MARK),
        icon: Icon::More,
        color: style.fg,
    });
}

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
    let bounds = Bounds::cells(x, y as f32 + PILL.0, width, PILL.1);
    grid.panel(bounds, 34, None, Some(tone));
    let tag = Style { fg: tone, ..inner };
    let at = x + PILL_ROOM;
    grid.label_at(at, at.floor() as u16, y, text, tag, right, SMALL);
}

/// The × in a picked kind's pill, its strokes starting half a column after the count that
/// ends at `ends`, in the middle of its row; on cells, a "×" a cell after the count.
fn draw_clear(
    grid: &mut Grid,
    (ends, cell, y): (f32, u16, u16),
    (icon, color): (Icon, cellix_render::Rgb),
    style: Style,
    measure: &dyn Measure,
) {
    let painted = Style {
        painted: true,
        fg: color,
        ..style
    };
    grid.put_str(cell, y, "\u{d7}", painted, cell + 1);
    if measure.in_cells() {
        return;
    }
    // The square's side in columns, as the painter sizes an icon by the words' line; its
    // strokes start 6.5 of its 24 in.
    let line = measure.cell_pixels().map_or(2.0, |(wide, tall)| {
        tall as f32 / measure.row_air().max(1.0) / wide.max(1) as f32
    });
    let side = CLEAR_ICON * line;
    let left = ends + CLEAR_ROOM / 2.0 - side * 6.5 / 24.0;
    let top = f32::from(y) + (1.0 - CLEAR_ICON) / 2.0;
    grid.paint.push(Paint::Icon {
        bounds: Bounds::cells(left, top, side, CLEAR_ICON),
        icon,
        color,
    });
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
        // Measured once, as `wrap` does, so the two break a comment at the same words.
        let advances = measure.advances(paragraph, font);
        let mut row = start;
        let mut word = start;
        // How wide the row is up to this word, and how many letters are behind.
        let mut wide = 0.0;
        let mut letters = 0;
        for piece in paragraph.split(' ') {
            let count = piece.chars().count();
            let piece_wide = stretch(&advances, letters, count);
            let end = word + piece.len();
            let before = &text[row..word];
            if !before.trim().is_empty() && wide + piece_wide > width {
                rows.push((row, word));
                row = word;
                wide = 0.0;
            }
            // With the space after it, which stays at the row's end.
            wide += piece_wide + stretch(&advances, letters + count, 1);
            letters += count + 1;
            word = end + 1;
        }
        let end = start + paragraph.len();
        rows.push((row, end));
        start = end + 1;
    }
    rows
}

/// What the button that sends a comment being written says: what sending it does.
fn send_label(draft: &Draft) -> &'static str {
    match (draft.thread, draft.editing, draft.ask) {
        (Some(_), true, _) => "Save",
        (Some(_), false, _) => "Reply",
        (None, _, true) => ASK,
        (None, _, false) => "Comment",
    }
}

const CANCEL: &str = "Cancel";

fn draft_lines(draft: &Draft, view: View, width: f32, measure: &dyn Measure, out: &mut Vec<Line>) {
    // Inside its box, which stands on the card's columns with the box's padding inside it.
    let inside = width - field_pad(measure) * 2.0;
    let spans = wrap_spans(&draft.text, inside, measure);
    let caret = draft.caret.min(draft.text.len());
    // At a row's end and the next one's start, it's at the start of the next.
    let on = spans
        .iter()
        .rposition(|(start, end)| *start <= caret && caret <= *end);
    for (index, (start, end)) in spans.into_iter().enumerate() {
        let at = (on == Some(index)).then(|| caret - start);
        out.push(Line::Input(draft.text[start..end].to_string(), at));
    }
    // The pictures it names so far, so what's being sent is in view.
    let named: Vec<String> = picture_spans(&draft.text)
        .into_iter()
        .map(|(start, end)| draft.text[start..end].to_string())
        .collect();
    picture_lines(DRAFT_PICTURE, &named, &mut 0, (view, inside), out);
    out.push(Line::Pad);
    out.push(Line::Tools);
    let found = suggested(draft, view);
    out.push(if found.is_empty() {
        Line::Hint
    } else {
        Line::Suggest(found)
    });
    // Its kind, picked rather than typed, for a comment and not for a reply.
    if draft.thread.is_none() || draft.editing {
        let mut row = Vec::new();
        let mut used = 0.0;
        for kind in KINDS {
            let chip = pill_width(kind, measure) + PILL_GAP;
            if used + chip > width + PILL_GAP && !row.is_empty() {
                out.push(Line::Kinds(std::mem::take(&mut row)));
                used = 0.0;
            }
            row.push(kind);
            used += chip;
        }
        out.push(Line::Kinds(row));
    }
}

/// Columns between the composer box's edge and the words in it, where it's painted: a column,
/// as a field's are. On cells there's no box, and its words are on the card's column.
fn field_pad(measure: &dyn Measure) -> f32 {
    if measure.in_cells() { 0.0 } else { 1.0 }
}

/// Where the cards stand across the pane, and where what they hold starts and ends, in
/// columns with their fractions.
#[derive(Clone, Copy, Debug)]
struct Across {
    /// A card's edges: at the pane's text edge, where every view in the pane starts its words,
    /// and as far from the sheet's edge on its right. Cards stood a column from the pane's
    /// edge, so the docked Comments started at x27 where the Agents list started at x45.
    left: f32,
    right: f32,
    /// Where a card's words start, and where what stands at its right end ends: `CARD_INSET`
    /// inside its edges.
    text: f32,
    end: f32,
}

fn across(area: Rect, view: View, measure: &dyn Measure) -> Across {
    let left = f32::from(area.x + SIDE);
    let air = left - (f32::from(area.x) + view.edge);
    let right = f32::from(area.right()) - air;
    let inset = crate::measures::card_inset(measure);
    Across {
        left,
        right,
        text: left + inset,
        end: right - inset,
    }
}

/// How wide a card's words run.
fn text_width(area: Rect, view: View, measure: &dyn Measure) -> f32 {
    let a = across(area, view, measure);
    (a.end - a.text).max(1.0)
}

/// A place in columns as the cell it starts in and how far into that cell it is: what's laid
/// out in cells, a row of buttons say, is painted that much further right.
fn cell_of(at: f32) -> (u16, f32) {
    let cell = at.max(0.0).floor();
    (cell as u16, at - cell)
}

/// Where each kind's chip stands under a comment being written: its kind, and the columns
/// it starts and ends, from `from`, the card's text column.
fn chips(
    from: f32,
    kinds: &[&'static str],
    measure: &dyn Measure,
) -> Vec<(&'static str, f32, f32)> {
    let mut x = from;
    kinds
        .iter()
        .map(|kind| {
            let width = pill_width(kind, measure);
            let at = x;
            x += width + PILL_GAP;
            (*kind, at, at + width)
        })
        .collect()
}

/// What the panel shows: the comments, the one being written, the agent's name, how many
/// comments are resolved and whether they're among those shown, and whether each file's
/// stand under a heading, as they do when no one file is in view.
pub type Shown<'a> = (
    &'a [&'a Thread],
    Option<&'a Draft>,
    &'a str,
    (usize, bool),
    View<'a>,
);

/// The pictures comments name, by the word that names each, read small to stand on a card.
pub type Pictures = std::collections::HashMap<String, crate::rendered::Figure>;

/// The endings a picture's file has.
const PICTURE_ENDINGS: [&str; 6] = ["png", "jpg", "jpeg", "gif", "webp", "svg"];
/// The most rows a picture takes on a card, and the air it keeps under it.
const PICTURE_ROWS: f32 = 8.0;
const PICTURE_AIR: f32 = 0.5;
/// How far under its row's top a picture starts: as clear of the words over it as of what
/// comes after it.
const PICTURE_DROP: f32 = 0.36;
/// The comment a picture's rows say they're on while it's still being written.
const DRAFT_PICTURE: u32 = u32::MAX;

/// How wide and tall a picture stands on a card with `width` columns for it: its own size
/// where that fits, and smaller in its own shape where it doesn't.
fn picture_size(figure: &crate::rendered::Figure, width: f32) -> (f32, f32) {
    let across = width / figure.width.max(0.01);
    let down = PICTURE_ROWS / figure.height.max(0.01);
    let scale = across.min(down).min(1.0);
    (figure.width * scale, figure.height * scale)
}

/// Each word of a comment that names a picture, as the bytes it takes: a path whose file
/// ends as a picture's does, without the bracket or stop a sentence puts around it. A web
/// address isn't one, wherever it leads.
pub fn picture_spans(text: &str) -> Vec<(usize, usize)> {
    let mut spans = Vec::new();
    let mut rest = 0;
    while let Some(found) = text[rest..].find(|c: char| !c.is_whitespace()) {
        let start = rest + found;
        let end = text[start..]
            .find(char::is_whitespace)
            .map_or(text.len(), |i| start + i);
        rest = end;
        let word = &text[start..end];
        let lead = word.len() - word.trim_start_matches(['(', '[', '"', '\'', '<']).len();
        let name =
            word[lead..].trim_end_matches(['.', ',', ';', ':', '!', '?', ')', ']', '"', '\'', '>']);
        let named = name.rsplit_once('.').is_some_and(|(stem, ending)| {
            !stem.is_empty()
                && !stem.ends_with('/')
                && PICTURE_ENDINGS
                    .iter()
                    .any(|known| ending.eq_ignore_ascii_case(known))
        });
        if named && !name.contains("://") {
            spans.push((start + lead, start + lead + name.len()));
        }
    }
    spans
}

/// A comment's words as its card shows them, and the pictures that stand under them: a name
/// between colons as its emoji, where the screen draws emoji, since an agent's answer may
/// spell one out; and a picture's name taken out, where the picture itself is shown.
fn shown<'t>(text: &'t str, view: View) -> (Cow<'t, str>, Vec<String>) {
    let words = if view.quick.is_some() {
        crate::emoji::expand(text)
    } else {
        Cow::Borrowed(text)
    };
    let Some(pictures) = view.pictures else {
        return (words, Vec::new());
    };
    let spans: Vec<(usize, usize)> = picture_spans(&words)
        .into_iter()
        .filter(|(start, end)| pictures.contains_key(&words[*start..*end]))
        .collect();
    if spans.is_empty() {
        return (words, Vec::new());
    }
    let names = spans
        .iter()
        .map(|(start, end)| words[*start..*end].to_string())
        .collect();
    let mut left = String::new();
    let mut from = 0;
    for (start, end) in spans {
        left.push_str(&words[from..start]);
        // The space after it goes with it, or the one before where it ends a line.
        from = end;
        if words[end..].starts_with(' ') {
            from += 1;
        } else if left.ends_with(' ') {
            left.pop();
        }
    }
    left.push_str(&words[from..]);
    (Cow::Owned(left.trim().to_string()), names)
}

/// An answer as its card shows it: its verdict's word if it gives one, who wrote it, and
/// its words.
pub(crate) fn reply_parts(reply: &Reply) -> (Option<String>, String, String) {
    // Addressed, on its chip as a verdict is, and what the agent says it did.
    if let Some(rest) = reply.text.strip_prefix(ADDRESSED) {
        let rest = rest.trim_start();
        return (
            Some(ADDRESSED_WORD.to_string()),
            reply.author.clone(),
            sentence(rest),
        );
    }
    match (verdict(&reply.text), rewrite(&reply.text)) {
        (Some((shown, _, rest)), _) => {
            (Some(sentence(shown)), reply.author.clone(), sentence(rest))
        }
        (_, Some(words)) => (
            None,
            format!("{} suggests a rewrite", reply.author),
            words.to_string(),
        ),
        _ => (None, reply.author.clone(), reply.text.clone()),
    }
}

/// A comment's words and then each of its answers', as its card shows them, each with the
/// pictures under it.
fn shown_parts(thread: &Thread, view: View) -> Vec<(String, Vec<String>)> {
    let own = |(words, pictures): (Cow<'_, str>, Vec<String>)| (words.into_owned(), pictures);
    let mut parts = vec![own(shown(&thread.text, view))];
    for reply in &thread.replies {
        let (_, _, body) = reply_parts(reply);
        parts.push(own(shown(&body, view)));
    }
    parts
}

/// The pictures on a comment's card, by the words that name them: those under its own
/// words, then those under each answer.
pub fn pictures_of(thread: &Thread, view: View) -> Vec<String> {
    shown_parts(thread, view)
        .into_iter()
        .flat_map(|(_, pictures)| pictures)
        .collect()
}

/// A picture's rows on a card, after the words it was named in.
fn picture_lines(
    thread: u32,
    names: &[String],
    which: &mut usize,
    (view, width): (View, f32),
    out: &mut Vec<Line>,
) {
    for name in names {
        let Some(figure) = view.pictures.and_then(|pictures| pictures.get(name)) else {
            continue;
        };
        let rows = (picture_size(figure, width).1 + PICTURE_AIR).ceil() as usize;
        out.extend((0..rows).map(|row| Line::Picture {
            thread,
            which: *which,
            name: name.clone(),
            row,
        }));
        *which += 1;
    }
}

/// Where a picture stands on its rows: the column it starts in and how wide and tall it is.
/// On a card it stands on the card's column; one being written, in the box with the words.
fn picture_place(
    area: Rect,
    (thread, view): (u32, View),
    figure: &crate::rendered::Figure,
    measure: &dyn Measure,
) -> (f32, (f32, f32)) {
    let a = across(area, view, measure);
    let pad = if thread == DRAFT_PICTURE {
        field_pad(measure)
    } else {
        0.0
    };
    let width = (a.end - a.text - pad * 2.0).max(1.0);
    (a.text + pad, picture_size(figure, width))
}

/// The word on an answer's chip that says it addressed the comment.
const ADDRESSED_WORD: &str = "Addressed";

/// The colour of a verdict's word, as an answer's chip says it here and on the card the
/// pointer brings up over the page: agreeing, or having done it, is the good news; pushing
/// back is what to look at.
pub(crate) fn verdict_tone(word: &str, theme: &Theme) -> cellix_render::Rgb {
    if word == "Agrees" || word == ADDRESSED_WORD {
        theme.git_added
    } else {
        theme.attention
    }
}

/// What the button that resolves every addressed comment says.
fn resolve_all_label(count: usize) -> String {
    format!("Resolve {count} addressed")
}

/// That button's cells and start column, at the end of the order's row. None with nothing
/// addressed.
fn resolve_all(
    area: Rect,
    (count, view): (usize, View),
    measure: &dyn Measure,
) -> Option<(String, u16)> {
    if count == 0 {
        return None;
    }
    let cells = button_cells(&resolve_all_label(count), measure);
    let width = cells.chars().count() as u16;
    let (check, x) = check_button(area, view, measure);
    let end = x + check.chars().count() as u16;
    Some((cells, end.saturating_sub(width)))
}

/// The order the pane lists comments in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Sort {
    /// As they stand in the document, top to bottom, a file at a time.
    #[default]
    Document,
    /// The last one made first.
    Newest,
    /// The first one made first.
    Oldest,
}

impl Sort {
    pub const ALL: [Sort; 3] = [Sort::Document, Sort::Newest, Sort::Oldest];

    pub fn label(self) -> &'static str {
        match self {
            Sort::Document => "Document order",
            Sort::Newest => "Newest first",
            Sort::Oldest => "Oldest first",
        }
    }
}

/// How a comment no agent has yet reaches one, which each waiting card says.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Goes {
    /// No agent is here to hear it: it waits for one's next prompt.
    #[default]
    Nobody,
    /// The agent's hooks hand it over with its next prompt, and Ask Agent sends it now.
    Prompt,
    /// An agent outside the pane takes it with its next prompt, and nothing here can send it
    /// sooner.
    Later,
    /// The agent here has no hook that hands it comments, so only Ask Agent sends it.
    Ask,
}

/// What a waiting card says of how it reaches `agent`.
fn goes_note(goes: Goes, agent: &str) -> String {
    match goes {
        Goes::Prompt => format!("Goes to {agent} with your next prompt, or now with Ask Agent."),
        Goes::Later => format!("{agent} sees this with its next prompt."),
        Goes::Ask => format!("{agent} isn't handed comments with its prompts: Ask Agent sends it."),
        Goes::Nobody => "Goes to an agent with its next prompt. Ask Agent starts one.".to_string(),
    }
}

/// Comments on code no agent has had yet: each is also one of the review's comments, which
/// go together, so it's waiting though its thread says it was told. The app says which,
/// each time the pane is shown.
#[derive(Debug, Default)]
pub struct Unsent(RefCell<Vec<u32>>);

impl Unsent {
    pub fn set(&self, ids: Vec<u32>) {
        *self.0.borrow_mut() = ids;
    }

    fn has(&self, id: u32) -> bool {
        self.0.borrow().contains(&id)
    }
}

/// How the pane shows what it lists: whether each file's stand under a heading, the comment
/// lit because its text was pressed in the document, and the order they're in.
#[derive(Clone, Copy, Debug)]
pub struct View<'a> {
    pub grouped: bool,
    pub lit: Option<u32>,
    pub sort: Sort,
    /// How what waits reaches the agent, and the comments on code that wait with the review.
    pub goes: Goes,
    /// The only kind listed, from a press on its pill.
    pub kind: Option<&'static str>,
    /// How many comments no agent has had, where there's an agent to send them to now.
    pub send: Option<usize>,
    /// The comment whose card shows how its words changed since it was made.
    pub changes: Option<u32>,
    /// The pane is narrow, so a card is short until it's lit: who and when, what it's about,
    /// two rows of its words and how many answers, with its buttons while it's pointed at.
    pub compact: bool,
    /// Now, in seconds since 1970, for how long ago each comment and reply was written.
    pub now: u64,
    pub unsent: Option<&'a Unsent>,
    /// The emoji that stand under a comment being written. None where the screen can't
    /// draw them, and then names between colons stay as they're typed too.
    pub quick: Option<[&'static str; 8]>,
    /// The pictures comments name, where the screen shows pictures: each stands on its card
    /// under the words, in place of its name among them. Without them a name is a link.
    pub pictures: Option<&'a Pictures>,
    /// Where the list's rows are kept from one look at them to the next. Without it
    /// they're laid out at each.
    pub laid: Option<&'a Laid>,
    /// How far into the area's first column its sheet starts: half the frame's column for
    /// the pane beside the file. A card stands as far from the sheet's edge on its right.
    pub edge: f32,
    /// The suggestions in the document in view: how many, and which the caret is in. None
    /// when the list covers every file, or the file in view isn't a document.
    pub suggestions: Option<(usize, Option<usize>)>,
}

/// A list's rows as they were last laid out, with a mark of all they were laid out from.
/// Each notch of the wheel, each press and each frame asks for the rows, and in a long list
/// laying them out is nearly all any of those costs: three hundred comments took 4 ms a
/// time, so a trackpad's hundred notches held the screen still for a third of a second.
/// With the same mark the rows are the ones already made.
#[derive(Default)]
pub struct Laid {
    rows: RefCell<Option<(u64, Rc<Vec<Line>>)>>,
    /// How many times a list has been laid out here, which a test holds it to.
    times: Cell<usize>,
}

impl Laid {
    #[cfg(test)]
    pub fn times(&self) -> usize {
        self.times.get()
    }
}

impl std::fmt::Debug for Laid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Laid({})", self.times.get())
    }
}

/// A number that's the same while everything the rows are laid out from is: the comments
/// whole, the one being written with its caret, and how and how wide they're shown. It
/// reads every word of them, and still costs a tenth of what laying them out does.
fn mark((threads, draft, agent, resolved, view): Shown, width: f32, measure: &dyn Measure) -> u64 {
    let mut hasher = DefaultHasher::new();
    threads.hash(&mut hasher);
    draft.hash(&mut hasher);
    agent.hash(&mut hasher);
    resolved.hash(&mut hasher);
    (
        view.grouped,
        view.lit,
        view.sort,
        view.quick,
        view.goes,
        view.kind,
    )
        .hash(&mut hasher);
    (view.changes, view.compact).hash(&mut hasher);
    view.send.hash(&mut hasher);
    // How long ago says something new each minute, where any comment says when it was made.
    let dated = threads
        .iter()
        .any(|t| t.at > 0 || t.replies.iter().any(|r| r.at > 0));
    dated.then_some(view.now / 60).hash(&mut hasher);
    view.unsent
        .map(|unsent| unsent.0.borrow().clone())
        .hash(&mut hasher);
    view.suggestions.hash(&mut hasher);
    // A picture takes rows by its size. They're summed, since a map has no order.
    let pictures = view.pictures.map(|pictures| {
        let sum = pictures.iter().fold(0u64, |sum, (name, figure)| {
            let mut one = DefaultHasher::new();
            (name, figure.width.to_bits(), figure.height.to_bits()).hash(&mut one);
            sum.wrapping_add(one.finish())
        });
        (pictures.len(), sum)
    });
    pictures.hash(&mut hasher);
    (width.to_bits(), measure.in_cells(), measure.cell_pixels()).hash(&mut hasher);
    hasher.finish()
}

/// The list's rows: those kept, when nothing they were laid out from has changed.
fn lines(shown: Shown, width: f32, measure: &dyn Measure) -> Rc<Vec<Line>> {
    let Some(laid) = shown.4.laid else {
        return Rc::new(lay_out(shown, width, measure));
    };
    let mark = mark(shown, width, measure);
    if let Some((of, rows)) = &*laid.rows.borrow()
        && *of == mark
    {
        return rows.clone();
    }
    let rows = Rc::new(lay_out(shown, width, measure));
    laid.times.set(laid.times.get() + 1);
    *laid.rows.borrow_mut() = Some((mark, rows.clone()));
    rows
}

impl Default for View<'_> {
    fn default() -> Self {
        Self {
            grouped: false,
            lit: None,
            sort: Sort::default(),
            goes: Goes::default(),
            unsent: None,
            kind: None,
            changes: None,
            compact: false,
            send: None,
            now: 0,
            quick: Some(crate::emoji::QUICK),
            pictures: None,
            laid: None,
            edge: 0.0,
            suggestions: None,
        }
    }
}

/// How many emoji a name being typed is offered.
pub const SUGGESTED: usize = 5;

/// The emoji offered for the name being typed at a draft's caret, each with its name.
pub fn suggested(draft: &Draft, view: View) -> Vec<(&'static str, &'static str)> {
    let caret = draft.caret.min(draft.text.len());
    let Some(quick) = view.quick else {
        return Vec::new();
    };
    crate::emoji::open_code(&draft.text, caret)
        .map(|(_, typed)| crate::emoji::suggest(typed, &quick, SUGGESTED))
        .unwrap_or_default()
}

/// What stands under a comment being written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tool {
    Quick(usize),
    Emoji,
    Picture,
}

/// How far the box round a comment being written stands inside its first and last rows, in
/// rows: clear of the quote over it and the hint under it.
pub(crate) const FIELD_INSET: f32 = 0.22;
/// How far the words are dropped and the tools lifted into the row kept between them, so
/// each is a quarter of a row and more inside the box's line.
const FIELD_SHIFT: f32 = 0.3;
/// Columns a tool takes: an emoji's two and one before the next.
const TOOL: u16 = 3;
/// The size the quick emoji are drawn at, as a percentage of the words'.
const QUICK_SIZE: u8 = 115;
/// How tall the icons after them are, in rows: the emoji's size.
const TOOL_ICON: f32 = 0.7;
/// Columns between the emoji and the buttons after them.
const TOOL_GAP: u16 = 1;

/// Each tool under a comment being written and the column it starts in: as many of the
/// quick emoji as leave room for the buttons after them, where the screen draws emoji, and
/// the picture's button either way.
fn tools(area: Rect, view: View, measure: &dyn Measure) -> Vec<(Tool, u16)> {
    let (x, width) = tools_from(area, view, measure);
    if view.quick.is_none() {
        return vec![(Tool::Picture, x)];
    }
    let count = (width.saturating_sub(TOOL_GAP + TOOL * 2) / TOOL).min(8);
    let mut out: Vec<(Tool, u16)> = (0..count)
        .map(|i| (Tool::Quick(usize::from(i)), x + TOOL * i))
        .collect();
    let after = x + TOOL * count + TOOL_GAP.min(count);
    out.push((Tool::Emoji, after));
    out.push((Tool::Picture, after + TOOL));
    out
}

/// The cell the tools under a comment being written start in, inside its box with the words,
/// and how many columns they have.
fn tools_from(area: Rect, view: View, measure: &dyn Measure) -> (u16, u16) {
    let a = across(area, view, measure);
    let pad = field_pad(measure);
    let (x, _) = cell_of(a.text + pad);
    let width = (a.end - pad).floor() as u16;
    (x, width.saturating_sub(x))
}

/// What the hint under a comment being written says of the tool the pointer is on.
fn tool_hint(tool: Tool) -> Option<&'static str> {
    match tool {
        Tool::Quick(_) => None,
        Tool::Emoji => Some("Every emoji. Or type a name between colons, like :tada:"),
        Tool::Picture => Some("Add a picture. Or paste one, or drop its file here."),
    }
}

/// Where each emoji offered for a name starts, and how many columns it and its name take.
fn suggest_spots(
    (from, end): (f32, f32),
    found: &[(&'static str, &'static str)],
    measure: &dyn Measure,
) -> Vec<(f32, f32)> {
    // Short of the row's end by the room "Tab" takes there: the key that takes the first of
    // them stays in view, and it's the last of the names that gives way.
    let right = end - SUGGEST_KEY;
    let mut x = from;
    let mut out = Vec::new();
    for (emoji, code) in found {
        let wide = suggest_emoji(emoji, measure) + measure.width(&format!(":{code}:"), small());
        if x + wide > right && !out.is_empty() {
            break;
        }
        out.push((x, wide));
        x += wide + SUGGEST_GAP;
    }
    out
}

/// Columns an offered emoji takes before its name: itself and a gap.
fn suggest_emoji(emoji: &str, measure: &dyn Measure) -> f32 {
    let wide = measure.width(emoji, Font::default()) + 0.5;
    // In cells its name starts on a cell; painted, right where that comes to.
    if measure.in_cells() {
        wide.ceil()
    } else {
        wide
    }
}

/// Columns between one offered emoji's name and the next emoji.
const SUGGEST_GAP: f32 = 2.0;
/// Columns kept at the end of that row for the key's name, and the gap before it.
const SUGGEST_KEY: f32 = SUGGEST_GAP + 3.0;

/// Each web address in a run of words, as the bytes it takes: `http://` or `https://` to the
/// next space, without the stop or bracket a sentence puts after it.
pub fn web_spans(text: &str) -> Vec<(usize, usize)> {
    let mut spans = Vec::new();
    let mut from = 0;
    while let Some(found) = text[from..].find("http") {
        let start = from + found;
        let rest = &text[start..];
        let starts_word = text[..start]
            .chars()
            .next_back()
            .is_none_or(|before| !before.is_alphanumeric());
        if !starts_word || !(rest.starts_with("http://") || rest.starts_with("https://")) {
            from = start + 4;
            continue;
        }
        let run = rest.find(char::is_whitespace).unwrap_or(rest.len());
        let link =
            rest[..run].trim_end_matches(['.', ',', ';', ':', '!', '?', ')', ']', '"', '\'']);
        // The scheme alone links to nothing.
        if link.len() > "https://".len() {
            spans.push((start, start + link.len()));
        }
        from = start + run.max(4);
    }
    spans
}

/// Each thing in a run of words that a click opens, as the bytes it takes: a web address,
/// or the name of a picture's file. Where a picture stands on its card its name isn't among
/// the words, so the names left are of pictures that can't be shown.
pub fn link_spans(text: &str) -> Vec<(usize, usize)> {
    let mut spans = web_spans(text);
    spans.extend(picture_spans(text));
    spans.sort_unstable();
    spans
}

/// Every link on a comment's card, in its words and then its answers in order.
pub fn links(thread: &Thread, view: View) -> Vec<String> {
    shown_parts(thread, view)
        .into_iter()
        .flat_map(|(words, _)| {
            link_spans(&words)
                .into_iter()
                .map(|(start, end)| words[start..end].to_string())
                .collect::<Vec<_>>()
        })
        .collect()
}

/// Draws a row of a comment's words with its links in the accent, underlined: the row, then
/// each link over its own letters, which a measured start puts exactly on them.
fn draw_linked(
    grid: &mut Grid,
    (x, cell, y): (f32, u16, u16),
    text: &str,
    (style, size, right): (Style, u8, u16),
    (measure, theme): (&dyn Measure, &Theme),
) {
    grid.label_at(x, cell, y, text, style, right, size);
    let font = Font {
        size,
        ..Font::default()
    };
    for (start, end) in link_spans(text) {
        let before = &text[..start];
        let at = x + measure.width(before, font);
        let from = cell + before.chars().count() as u16;
        if from >= right {
            continue;
        }
        let link = Style {
            fg: theme.accent,
            underline: true,
            ..style
        };
        grid.label_at(at, from, y, &text[start..end], link, right, size);
        // Painted text takes its line from the paint, not from the cells' style.
        if let Some(Paint::Text { underline, .. }) = grid.paint.last_mut() {
            *underline = true;
        }
    }
}

/// Which of a row's links column `x` is on, counting from the row's first, where the row
/// starts at `from` and is set at `size`.
fn link_at(text: &str, (from, x): (f32, u16), size: u8, measure: &dyn Measure) -> Option<usize> {
    let font = Font {
        size,
        ..Font::default()
    };
    let x = f32::from(x) + 0.5;
    link_spans(text).into_iter().position(|(start, end)| {
        let left = from + measure.width(&text[..start], font);
        let right = from + measure.width(&text[..end], font);
        x >= left && x < right
    })
}

/// Which side of a control's words its chevron stands, and which way it points.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Chevron {
    /// After the words, pointing down: the control opens a list.
    Opens,
    /// Before them: the control unfolds what's under it, and points down while that shows.
    Unfolds(bool),
}

/// Columns of air inside a control's chip, either side of what it holds: a column, as the
/// kinds' pills over it keep, so its words start where theirs do and where a card's do.
const CONTROL_ROOM: f32 = 1.0;
/// How tall the square a chevron is drawn in is, in rows: one that unfolds, as a folder's
/// in the explorer, and the smaller one of a menu, as the branch's and the heading's are.
const FOLD_CHEVRON: f32 = cellix_render::GLYPH;
const MENU_CHEVRON: f32 = 0.6;
/// Columns between a chevron and the words: after one that unfolds, and before a menu's.
const FOLD_GAP: f32 = 0.5;
const MENU_GAP: f32 = 0.65;
/// How far down its row a control is painted where it follows the kinds or a card, in
/// rows: into the empty row under it, so it stands a little nearer what it's over than
/// what's over it, and the two rows it takes aren't three.
const CONTROL_DROP: f32 = 0.4;

/// Where a control's parts stand, in columns from its chip's left edge.
struct Parts {
    /// How wide the chip is: what it holds and the room either side, not whole cells.
    wide: f32,
    /// Where the square its chevron is drawn in starts, and how tall that is, in rows.
    square: (f32, f32),
    /// Where its words start.
    words: f32,
}

/// A control's parts in pixels. `labels` are all it ever says, the one it says now first:
/// it's as wide as the widest, so it's the same chip folded and unfolded.
fn control_parts(labels: &[&str], chevron: Chevron, measure: &dyn Measure) -> Parts {
    let font = Font {
        size: BUTTON_TEXT,
        ..Font::default()
    };
    let width = |label: &&str| measure.width(label, font);
    let words = labels.iter().map(width).fold(0.0, f32::max);
    let across = crate::measures::rows_to_columns(measure);
    // A chevron's strokes take the middle 11 of the 24 its square is drawn to.
    let ink = |tall: f32| (tall * across * 6.5 / 24.0, tall * across * 11.0 / 24.0);
    match chevron {
        Chevron::Opens => {
            let (inset, mark) = ink(MENU_CHEVRON);
            let at = CONTROL_ROOM + words + MENU_GAP;
            Parts {
                wide: at + mark + CONTROL_ROOM,
                square: (at - inset, MENU_CHEVRON),
                words: CONTROL_ROOM,
            }
        }
        Chevron::Unfolds(_) => {
            let (inset, mark) = ink(FOLD_CHEVRON);
            let at = CONTROL_ROOM + mark + FOLD_GAP;
            Parts {
                wide: at + words + CONTROL_ROOM,
                square: (CONTROL_ROOM - inset, FOLD_CHEVRON),
                words: at,
            }
        }
    }
}

/// A control of the list, the order or the resolved comments: a quiet chip with a
/// chevron, starting at the pane's text edge, where the summary and the kinds start. As
/// its cells and the column it starts in. Where every letter is a cell the chevron is one
/// of them; in pixels the cells are what the chip covers, holding what they can of its
/// words so the screen's text says what it is.
fn control(area: Rect, labels: &[&str], chevron: Chevron, measure: &dyn Measure) -> (String, u16) {
    let x = head(area);
    let label = labels.first().copied().unwrap_or_default();
    if measure.in_cells() {
        let cells = match chevron {
            Chevron::Opens => format!(" {label} \u{25be} "),
            Chevron::Unfolds(true) => format!(" \u{25be} {label} "),
            Chevron::Unfolds(false) => format!(" \u{25b8} {label} "),
        };
        return (cells, x);
    }
    let width = control_parts(labels, chevron, measure).wide.ceil() as usize;
    let held: String = label.chars().take(width).collect();
    (format!("{held:<width$}"), x)
}

/// The column the list's head starts in, its summary, its kinds and its controls: the pane's
/// text edge, where the cards' edges are too, and where every view in the pane starts. The
/// right-hand buttons on its rows end where the cards do (`head_end`).
fn head(area: Rect) -> u16 {
    area.x + SIDE
}

/// The cell the head's right-hand buttons end at, and how much further right they're painted:
/// on the cards' right edge.
fn head_end(area: Rect, view: View) -> (u16, f32) {
    let right = f32::from(area.right()) - (f32::from(SIDE) - view.edge);
    cell_of(right)
}

/// How far down its row the control on row `index` is painted: part of a row where it
/// follows the kinds or a card, and not at all under the words that say no comment is
/// open, which it's already apart from, or where every letter is a cell.
fn control_drop(all: &[Line], index: usize, measure: &dyn Measure) -> f32 {
    let under_hint = index
        .checked_sub(2)
        .and_then(|over| all.get(over))
        .is_some_and(|line| matches!(line, Line::EmptyHint(_)));
    match all.get(index) {
        Some(Line::Sort(_) | Line::Resolved(..)) if !measure.in_cells() && !under_hint => {
            CONTROL_DROP
        }
        _ => 0.0,
    }
}

/// Whether the control on row `index` of the list, on the screen's row `y`, would stand past
/// the pane's foot: one painted part of a row down, on the pane's last row. It isn't painted
/// there, nor pressed, till a scroll brings it up: "Show 1 resolved" stood cut in half by the
/// docked pane's foot.
fn cut_at_foot(all: &[Line], (index, y): (usize, u16), area: Rect, measure: &dyn Measure) -> bool {
    y + 1 >= area.bottom() && control_drop(all, index, measure) > 0.0
}

/// A thumb at the list's right, in the air between the cards and the sheet's edge, where the
/// list runs past the pane: as much of it as is in view, and as far down. The docked list ran
/// on past its foot with nothing to say there was more. On cells, half blocks in the pane's
/// last column.
fn draw_thumb(
    grid: &mut Grid,
    area: Rect,
    (rows, top): (usize, usize),
    x: f32,
    (measure, theme): (&dyn Measure, &Theme),
) {
    let last = last_of(rows, area.h);
    if last == 0 {
        return;
    }
    let shown = f32::from(area.h);
    // Every row the list scrolls through, with the air kept under its last card.
    let all = (rows + 1) as f32;
    let tall = (shown * shown / all).max(THUMB_ROWS);
    let y = f32::from(area.y) + (shown - tall) * top.min(last) as f32 / last as f32;
    if measure.in_cells() {
        let style = Style {
            fg: theme.scrollbar,
            ..theme.sidebar
        };
        let column = area.right().saturating_sub(1);
        let from = y.round() as u16;
        let to = (y + tall).round().max(f32::from(from) + 1.0) as u16;
        for row in from..to.min(area.bottom()) {
            grid.put_str(column, row, "\u{2590}", style, column + 1);
        }
        return;
    }
    let thumb = Bounds::cells(x - THUMB_WIDE / 2.0, y + 0.1, THUMB_WIDE, tall - 0.2);
    grid.panel(thumb, 12, Some(theme.scrollbar), None);
}

/// The fewest rows the list's thumb is, and how wide it is in columns: the editor's, 8 px.
const THUMB_ROWS: f32 = 1.0;
const THUMB_WIDE: f32 = 0.5;

/// Draws such a control: its chip, lighter under the pointer, with its chevron and its
/// words from its left edge, `drop` of a row down its row.
fn draw_control(
    grid: &mut Grid,
    (x, y): (u16, u16),
    (cells, labels, chevron): (&str, &[&str], Chevron),
    (on_it, drop): (bool, f32),
    (measure, theme): (&dyn Measure, &Theme),
) {
    let ground = theme.sidebar.bg;
    let quiet = theme.quiet_button(ground);
    let fill = if on_it {
        theme.quiet_button(quiet)
    } else {
        quiet
    };
    let style = Style {
        bg: fill,
        fg: theme.text.fg,
        ..theme.sidebar
    };
    if measure.in_cells() {
        grid.button(x, y, cells, "", fill, style);
        return;
    }
    let from = grid.painted_so_far();
    // Its cells keep the pane's colour: the chip isn't as wide as they are, and may stand
    // part of a row under them.
    let under = Style {
        bg: ground,
        painted: true,
        ..style
    };
    let end = x + cells.chars().count() as u16;
    grid.put_str(x, y, cells, under, end);
    let parts = control_parts(labels, chevron, measure);
    let left = f32::from(x);
    grid.chip(left, y, parts.wide, fill);
    let (icon, tone) = match chevron {
        Chevron::Opens => (Icon::ChevronDown, 0.78),
        Chevron::Unfolds(true) => (Icon::ChevronDown, 0.5),
        Chevron::Unfolds(false) => (Icon::ChevronRight, 0.5),
    };
    let (square, tall) = parts.square;
    let across = tall * crate::measures::rows_to_columns(measure);
    grid.paint.push(Paint::Icon {
        bounds: Bounds::cells(
            left + square,
            f32::from(y) + (1.0 - tall) / 2.0,
            across,
            tall,
        ),
        icon,
        color: mix(theme.text.fg, theme.gutter.fg, tone),
    });
    let words = left + parts.words;
    let label = labels.first().copied().unwrap_or_default();
    grid.label_at(
        words,
        words.floor() as u16,
        y,
        label,
        under,
        end,
        BUTTON_TEXT,
    );
    grid.lift_since(from, -drop);
}

/// Where the order's button is, for the menu it drops: the column its chip starts at, the
/// list head's, its row, and how far down that row it's painted (`control_drop`). None when
/// the button isn't in the list.
pub fn order_button(
    area: Rect,
    shown: Shown,
    top: usize,
    measure: &dyn Measure,
) -> Option<(u16, u16, f32)> {
    let width = text_width(area, shown.4, measure);
    let all = lines(shown, width, measure);
    let top = top.min(last_of(all.len(), area.h));
    let index = all.iter().position(|line| matches!(line, Line::Sort(_)))?;
    let row = area.y + index.checked_sub(top)? as u16;
    Some((head(area), row, control_drop(&all, index, measure)))
}

/// The row of the buttons that send or drop what's being written, the first at or under
/// `from`: the emoji picker drops from under them, so it covers neither the hint over them
/// nor the buttons.
pub fn send_row(
    area: Rect,
    shown: Shown,
    top: usize,
    from: u16,
    measure: &dyn Measure,
) -> Option<u16> {
    let width = text_width(area, shown.4, measure);
    let all = lines(shown, width, measure);
    let top = top.min(last_of(all.len(), area.h));
    let first = top + usize::from(from.checked_sub(area.y)?);
    let index = (first..all.len()).find(|i| matches!(all[*i], Line::Send(..)))?;
    Some(area.y + index.checked_sub(top)? as u16)
}

/// The row of the list a comment's card starts on, to scroll the pane to it.
pub fn top_of(area: Rect, shown: Shown, id: u32, measure: &dyn Measure) -> Option<usize> {
    let width = text_width(area, shown.4, measure);
    lines(shown, width, measure)
        .iter()
        .position(|line| matches!(line, Line::Header(of, ..) | Line::Quote(of, ..) if *of == id))
}

/// Where a comment is, as its heading says it: its file, or the terminal tab whose output
/// it's on.
pub fn place_of(thread: &Thread) -> String {
    match &thread.about {
        Some(About::Output { tab, .. }) => format!("{tab} output"),
        _ => thread.file.clone(),
    }
}

fn lay_out(
    (threads, draft, agent, resolved, view): Shown,
    width: f32,
    measure: &dyn Measure,
) -> Vec<Line> {
    let grouped = view.grouped;
    let mut out = vec![Line::Gap];
    // How wide the list's head is: from a card's left edge to its right.
    let head_width = width + 2.0 * crate::measures::card_inset(measure);
    // A hint under the head is set small, and wraps where small words reach its end: measured
    // as the interface's words, "Comment." stood alone on a row with a third of the row left.
    let hint_width = if measure.in_cells() {
        head_width
    } else {
        head_width * 100.0 / f32::from(SMALL)
    };
    // The document's suggestions come before its comments, as Google Docs and Overleaf keep
    // the two together, with what settles them all.
    if let Some((count, at)) = view.suggestions.filter(|(count, _)| *count > 0) {
        let words = suggestions_said(count, at);
        let buttons: f32 = settle_buttons(Rect::new(0, 0, 200, 1), view, measure)
            .iter()
            .map(|(_, cells, _)| cells.chars().count() as f32 + f32::from(BUTTON_GAP))
            .sum();
        let fits = measure.width(&words, small()) + 1.0 + buttons <= head_width;
        out.push(Line::Suggestions(count, at, fits));
        if !fits {
            out.push(Line::Settle);
        }
        out.push(Line::Gap);
    }
    // The counts are of what's still open, whether or not the resolved ones show.
    let open: Vec<&Thread> = threads.iter().copied().filter(|t| !t.resolved).collect();
    if !open.is_empty() {
        let mine = open.iter().filter(|t| t.suggested_by.is_empty()).count();
        let mut places: Vec<String> = open.iter().map(|t| place_of(t)).collect();
        places.sort();
        places.dedup();
        let files = if grouped { places.len() } else { 0 };
        let filter = view
            .kind
            .map(|kind| (kind, open.iter().filter(|t| t.kind == kind).count()));
        out.push(Line::Check(mine, open.len() - mine, files, filter));
        if let Some(waiting) = view.send.filter(|n| *n > 0) {
            out.push(Line::Deliver(waiting));
        }
        // The reviewer's own: a suggestion not yet kept covers nothing, and was counted.
        let count = |kind: &str| {
            let mine = |t: &&&Thread| t.kind == kind && t.suggested_by.is_empty();
            open.iter().filter(mine).count()
        };
        // As many pills to a row as fit.
        let mut row = Vec::new();
        let mut used = 0.0;
        for kind in KINDS {
            let picked = view.kind == Some(kind);
            let pill = coverage_width(kind, count(kind), picked, measure) + PILL_GAP;
            if used + pill > head_width + PILL_GAP && !row.is_empty() {
                out.push(Line::Coverage(std::mem::take(&mut row)));
                used = 0.0;
            }
            row.push((kind, count(kind)));
            used += pill;
        }
        out.push(Line::Coverage(row));
        let addressed = open.iter().filter(|t| t.addressed_by().is_some()).count();
        out.push(Line::Sort(addressed));
        out.push(Line::Gap);
    }
    if let Some(new) = draft.filter(|d| d.thread.is_none()) {
        out.push(Line::Quote(
            NEW_DRAFT,
            shown_quote(&new.quote, new.line, new.about.as_ref()),
            Words::Here,
        ));
        draft_lines(new, view, width, measure, &mut out);
        out.push(Line::Send(NEW_DRAFT, send_label(new)));
        out.push(Line::Gap);
    } else if open.is_empty() {
        out.push(Line::Empty(resolved.0 > 0));
        // With suggestions in the text, it names them and how to go to each: "3 suggestions"
        // over "No comments yet." read as the pane saying two things.
        let said = match view.suggestions.map_or(0, |(count, _)| count) {
            0 => EMPTY_HINT.to_string(),
            1 => format!("1 suggestion in the text: Next goes to it. {EMPTY_HINT}"),
            n => format!("{n} suggestions in the text: Next goes to each. {EMPTY_HINT}"),
        };
        // It starts at the pane's text edge, a column outside a card's words either side.
        let hint = wrap(&said, hint_width, measure);
        out.extend(hint.into_iter().map(Line::EmptyHint));
        out.push(Line::Gap);
    }
    let mut place = None;
    // Those still open, then the resolved ones under the button that shows them: it stays
    // where it is when it's pressed, and what it shows unfolds under it.
    let still_open = threads.iter().filter(|t| !t.resolved);
    let mut unfolded = false;
    let mut listed = 0;
    for thread in still_open.chain(threads.iter().filter(|t| t.resolved)) {
        let id = thread.id;
        // A press on a kind's pill lists only that kind.
        if view.kind.is_some_and(|kind| thread.kind != kind) {
            continue;
        }
        listed += 1;
        if thread.resolved && !unfolded {
            unfolded = true;
            out.push(Line::Resolved(resolved.0, true));
            out.push(Line::Gap);
            place = None;
        }
        // The list comes a file at a time, so a heading goes where the file changes.
        let here = place_of(thread);
        if grouped && place.as_ref() != Some(&here) {
            let same = |t: &&&Thread| t.resolved == thread.resolved && place_of(t) == here;
            let count = threads.iter().filter(same).count();
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
        // And how long ago it was made, where that's known.
        let ago = age(thread.at, view.now).map_or_else(String::new, |ago| format!(" · {ago}"));
        let label = format!("{by}{kind}{done}{ago}");
        let (words, _) = words_of(thread);
        let fits = |pill: &String| {
            measure.width(&label, small()) + PILL_AIR + pill_width(pill, measure) <= width
        };
        let open = view.changes == Some(id);
        let mut pills =
            pills_of(thread)
                .into_iter()
                .map(|(pill, tone)| match pill.strip_suffix('\u{203a}') {
                    Some(words) if open && pill.starts_with(CHANGED_SINCE) => {
                        (format!("{words}\u{25be}"), tone)
                    }
                    _ => (pill, tone),
                });
        let first = pills.next();
        let (beside, under) = match first {
            Some((pill, tone)) if fits(&pill) => (Some((pill, tone)), None),
            under => (None, under),
        };
        out.push(Line::Header(id, label, suggested, beside));
        for (pill, tone) in under.into_iter().chain(pills) {
            out.push(Line::Pill(id, pill, tone));
        }
        let quote = shown_quote(&thread.quote, thread.line, thread.about.as_ref());
        // Opened from its pill, the quote's place shows the words as they were, struck where
        // they went, with what came in their place: one run, on the quote's bar, where a row
        // under the quote said six of its seven words again.
        if view.changes == Some(id) && !thread.now.is_empty() && thread.earlier.is_none() {
            let changes = cellix_review::threads::word_changes(&thread.quote, &thread.now);
            let rows = diff_rows(changes, width - QUOTE_IN, measure);
            out.extend(rows.into_iter().map(|row| Line::Diff(id, row)));
        } else {
            out.push(Line::Quote(id, quote, words));
        }
        let editing = draft.filter(|d| d.thread == Some(id) && d.editing);
        // In a narrow pane a card is short until it's lit or being written on.
        let short = is_short(id, draft, view);
        // Its pictures are counted through its words and then its answers.
        let mut which = 0;
        match editing {
            Some(edit) => {
                draft_lines(edit, view, width, measure, &mut out);
                // Its own pictures are still counted, so its answers' keep their places.
                which += shown(&thread.text, view).1.len();
            }
            None if short => {
                // Two rows of its words, the second ending "…" where there's more.
                let (words, _) = shown(&thread.text, view);
                let mut text = wrap(&words, width, measure);
                if text.len() > SHORT_ROWS {
                    text.truncate(SHORT_ROWS);
                    if let Some(last) = text.last_mut() {
                        last.push('\u{2026}');
                    }
                }
                out.extend(text.into_iter().map(|row| Line::Text(id, row)));
            }
            None => {
                let (words, pictures) = shown(&thread.text, view);
                let text = wrap(&words, width, measure);
                out.extend(text.into_iter().map(|row| Line::Text(id, row)));
                picture_lines(id, &pictures, &mut which, (view, width), &mut out);
            }
        }
        let answers = thread.replies.len();
        // The last word a rewrite: it's for the reviewer to apply or not, with its own buttons.
        let deciding = !short
            && !thread.resolved
            && thread
                .replies
                .last()
                .is_some_and(|r| rewrite(&r.text).is_some());
        for (n, reply) in thread.replies.iter().enumerate().filter(|_| !short) {
            // A verdict reads as one: its word on a chip, who said it, then why.
            let (word, author, body) = reply_parts(reply);
            // The pill at the card's top says "Addressed by" while that's the last word, so
            // the answer's chip doesn't say it again: one card said "Addressed" twice.
            let pill_says = n + 1 == answers && thread.addressed_by().is_some();
            let word = word.filter(|word| !(pill_says && word == ADDRESSED_WORD));
            let author = match age(reply.at, view.now) {
                Some(ago) => format!("{author} · {ago}"),
                None => author,
            };
            out.push(Line::Answer(id, word, author));
            let (words, pictures) = shown(&body, view);
            // A rewrite still to be decided stands in a box of its own over its buttons.
            if deciding && n + 1 == answers {
                let room = width - box_padding(measure) * 2.0;
                let text = wrap(&words, room, measure);
                out.extend(text.into_iter().map(|row| Line::Proposed(id, row)));
                continue;
            }
            let text = wrap(&words, width, measure);
            out.extend(text.into_iter().map(|row| Line::Reply(id, row)));
            picture_lines(id, &pictures, &mut which, (view, width), &mut out);
        }
        // A suggestion is the agent's own, so there's nothing to wait for. What no agent has
        // yet says how it gets to one, so nobody wonders whether it went.
        // A suggestion asked about is waited on like the reviewer's own comment.
        let asked_about = !suggested || thread.replies.iter().any(|r| r.author == YOU);
        // A short card says what it waits on once it's lit.
        let waiting = !short && asked_about && !thread.resolved && thread.waiting_answer();
        let unsent = view.unsent.is_some_and(|unsent| unsent.has(id));
        if waiting && (thread.waiting() || unsent) {
            // Asked of an agent outside the pane, it goes with that one's next prompt.
            let goes = if thread.asked { Goes::Later } else { view.goes };
            let note = wrap(&goes_note(goes, agent), width, measure);
            out.extend(note.into_iter().map(Line::Status));
        } else if waiting && thread.told == Some(thread.replies.len()) && !agent.is_empty() {
            out.push(Line::Status(format!("Waiting for {agent}…")));
        }
        if let Some(reply) = draft.filter(|d| d.thread == Some(id) && !d.editing) {
            draft_lines(reply, view, width, measure, &mut out);
        }
        if deciding {
            let rows = rewrite_rows(width, measure).len();
            out.extend((0..rows).map(|row| Line::Rewrite(id, row)));
        }
        // While it's answered or changed its own buttons give way to the two that send
        // what's being written or drop it: one thing is being done to it at a time.
        let writing = draft.filter(|d| d.thread == Some(id));
        out.push(if let Some(written) = writing {
            Line::Send(id, send_label(written))
        } else if thread.resolved {
            Line::Reopen(id)
        } else if suggested {
            Line::Choices(id)
        } else if thread.addressed_by().is_some() {
            Line::Addressed(id)
        } else {
            Line::Buttons(id)
        });
        out.push(Line::Gap);
    }
    // A kind none of them is: say so, and how to list them all again.
    if let Some(kind) = view.kind.filter(|_| listed == 0 && !threads.is_empty()) {
        let none = format!("No {kind} comments. Press {kind} again to list them all.");
        out.extend(
            wrap(&none, hint_width, measure)
                .into_iter()
                .map(Line::EmptyHint),
        );
        out.push(Line::Gap);
    }
    if resolved.0 > 0 && !unfolded {
        out.push(Line::Resolved(resolved.0, false));
        out.push(Line::Gap);
    }
    out
}

/// How long ago something written at `at` was, as a card says it: "now", "5 min", "2 h",
/// "3 d". None where when it was written isn't known.
pub fn age(at: u64, now: u64) -> Option<String> {
    if at == 0 {
        return None;
    }
    let ago = now.saturating_sub(at);
    Some(match ago {
        0..60 => "now".to_string(),
        60..3600 => format!("{} min", ago / 60),
        3600..86400 => format!("{} h", ago / 3600),
        _ => format!("{} d", ago / 86400),
    })
}

/// Where each kind's pill stands on its row, as its kind and the columns it starts and ends.
/// `picked` is the kind listed alone, whose pill has room for its ×.
fn pills(
    area: Rect,
    kinds: &[(&'static str, usize)],
    picked: Option<&str>,
    measure: &dyn Measure,
) -> Vec<(&'static str, f32, f32)> {
    let mut x = f32::from(head(area));
    kinds
        .iter()
        .map(|(kind, count)| {
            let width = coverage_width(kind, *count, picked == Some(*kind), measure);
            let at = x;
            x += width + PILL_GAP;
            (*kind, at, at + width)
        })
        .collect()
}

/// What the button for resolved comments says, and then what it says the other way: the
/// count stays, so it's as wide folded as unfolded.
fn resolved_labels(count: usize, shown: bool) -> [String; 2] {
    let (show, hide) = (
        format!("Show {count} resolved"),
        format!("Hide {count} resolved"),
    );
    if shown { [hide, show] } else { [show, hide] }
}

/// That button's cells and start column.
fn resolved_button(area: Rect, count: usize, shown: bool, measure: &dyn Measure) -> (String, u16) {
    let labels = resolved_labels(count, shown);
    let labels = [labels[0].as_str(), labels[1].as_str()];
    control(area, &labels, Chevron::Unfolds(shown), measure)
}

/// The buttons under a rewrite, a row at a time: all three on one where they fit a card
/// `width` wide, else Accept Rewrite on its own and the two quiet ones together under it, so
/// what takes the rewrite stands apart from what doesn't; each alone where even two don't fit.
fn rewrite_rows(width: f32, measure: &dyn Measure) -> Vec<Vec<&'static str>> {
    let fits = |labels: &[&str]| {
        let cells: usize = labels
            .iter()
            .map(|label| button_cells(label, measure).chars().count())
            .sum();
        let gaps = labels.len().saturating_sub(1) * usize::from(BUTTON_GAP);
        (cells + gaps) as f32 <= width + 1.0
    };
    let [take, apply, decline] = REWRITE;
    if fits(&REWRITE) {
        vec![REWRITE.to_vec()]
    } else if fits(&[apply, decline]) {
        vec![vec![take], vec![apply, decline]]
    } else {
        REWRITE.iter().map(|label| vec![*label]).collect()
    }
}

/// Each button as (label, padded cells, start column): Edit and Delete with the others when
/// all five fit, and the "⋯" that opens a menu of them when they don't. With how far right of
/// their cells those at the row's end are painted, so their words end on the card's right
/// column.
fn buttons(
    area: Rect,
    view: View,
    measure: &dyn Measure,
) -> (Vec<(&'static str, String, u16)>, f32) {
    let width = text_width(area, view, measure);
    let mut placed = row_of(area, &BUTTONS, measure);
    let ends: &[&'static str] = if all_fit(width, measure) {
        &EDITS
    } else {
        &[MORE]
    };
    let (edits, nudge) = edits(area, ends, view, measure);
    placed.extend(edits);
    (placed, nudge)
}

/// Columns either side of a plain-word button's letters, for its wash under the pointer.
const PLAIN_ROOM: f32 = 0.5;
/// Columns between two of them.
const PLAIN_GAP: u16 = 1;

/// The cells a button of plain words takes: Edit and Delete, the least of a card's actions,
/// and the "⋯" in their place.
fn plain_cells(label: &str, measure: &dyn Measure) -> String {
    if measure.in_cells() {
        return format!(" {label} ");
    }
    " ".repeat(plain_width(label, measure).ceil().max(2.0) as usize)
}

/// How wide what a plain-word button shows is, in columns: its words, or the "⋯"'s mark.
fn plain_shown(label: &str, measure: &dyn Measure) -> f32 {
    if label == MORE && !measure.in_cells() {
        // Its dots take the middle 16 of the 24 its square is drawn to.
        return icon_columns(MORE_MARK, measure) * 16.0 / 24.0;
    }
    let font = Font {
        size: BUTTON_TEXT,
        ..Font::default()
    };
    measure.width(label, font)
}

/// That and the room either side of it.
fn plain_width(label: &str, measure: &dyn Measure) -> f32 {
    plain_shown(label, measure) + PLAIN_ROOM * 2.0
}

/// How tall the square the "⋯" is drawn in is, in rows.
const MORE_MARK: f32 = 0.6;

/// How wide an icon `tall` rows tall is on screen, in columns: the painter sizes an icon by
/// the line words are set for, not the row.
fn icon_columns(tall: f32, measure: &dyn Measure) -> f32 {
    measure.cell_pixels().map_or(tall * 2.0, |(wide, high)| {
        tall * high as f32 / measure.row_air().max(1.0) / wide.max(1) as f32
    })
}

/// Edit and Delete, or the "⋯", as (label, cells, start column), ending at the card's right,
/// so they stand apart from what answers the comment; and how far right of their cells
/// they're painted, so what the last shows ends on the card's right column, `CARD_INSET` in.
fn edits(
    area: Rect,
    labels: &[&'static str],
    view: View,
    measure: &dyn Measure,
) -> (Vec<(&'static str, String, u16)>, f32) {
    let last = labels.last().copied().unwrap_or_default();
    let slack =
        (plain_cells(last, measure).chars().count() as f32 - plain_shown(last, measure)) / 2.0;
    let (mut end, nudge) = cell_of(across(area, view, measure).end + slack);
    let mut placed = Vec::new();
    for label in labels.iter().rev() {
        let cells = plain_cells(label, measure);
        let x = end.saturating_sub(cells.chars().count() as u16);
        placed.push((*label, cells, x));
        end = x.saturating_sub(PLAIN_GAP);
    }
    placed.reverse();
    (placed, nudge)
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

/// What the row over the comments says of the document's suggestions: how many, or which
/// of them the caret is in.
fn suggestions_said(count: usize, at: Option<usize>) -> String {
    match (count, at) {
        (_, Some(at)) => format!("Suggestion {} of {count}", at + 1),
        (1, None) => "1 suggestion".to_string(),
        (_, None) => format!("{count} suggestions"),
    }
}

/// Accept All, Reject All and Next as (label, cells, start column): at the right of the
/// row, ending where Check My Review under them does.
fn settle_buttons(
    area: Rect,
    view: View,
    measure: &dyn Measure,
) -> Vec<(&'static str, String, u16)> {
    let (cells, x) = check_button(area, view, measure);
    let mut end = x + cells.chars().count() as u16;
    let mut placed: Vec<_> = SETTLE
        .iter()
        .rev()
        .map(|label| {
            let cells = button_cells(label, measure);
            let x = end.saturating_sub(cells.chars().count() as u16);
            end = x.saturating_sub(BUTTON_GAP);
            (*label, cells, x)
        })
        .collect();
    placed.reverse();
    placed
}

/// The same three on a row of their own, from where the list's head starts.
fn settle_row(area: Rect, measure: &dyn Measure) -> Vec<(&'static str, String, u16)> {
    let mut x = head(area);
    SETTLE
        .iter()
        .map(|label| {
            let cells = button_cells(label, measure);
            let at = x;
            x += cells.chars().count() as u16 + BUTTON_GAP;
            (*label, cells, at)
        })
        .collect()
}

/// What the button that sends what waits says: how many, and to whom.
fn deliver_label(count: usize, agent: &str) -> String {
    format!("Send {count} to {agent}")
}

/// That button, under Check My Review and ending where it does, as (padded cells, start
/// column): the two the pane's actions, one over the other.
fn deliver_button(
    area: Rect,
    (count, agent): (usize, &str),
    view: View,
    measure: &dyn Measure,
) -> (String, u16) {
    let cells = button_cells(&deliver_label(count, agent), measure);
    let width = cells.chars().count() as u16;
    let (check, x) = check_button(area, view, measure);
    let end = x + check.chars().count() as u16;
    (cells, end.saturating_sub(width).max(head(area)))
}

/// Check My Review, at the right of its row, as (padded cells, start column). In pixels it
/// has room for the agent's icon before its words. It ends where the cards end, and is
/// painted as far past its cells as they are (`head_end`).
fn check_button(area: Rect, view: View, measure: &dyn Measure) -> (String, u16) {
    let mut cells = button_cells(CHECK, measure);
    if !measure.in_cells() {
        cells.push_str(&" ".repeat(CHECK_ICON as usize));
    }
    let width = cells.chars().count() as u16;
    let (end, _) = head_end(area, view);
    let x = end.saturating_sub(width).max(area.x + SIDE);
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
    let width = text_width(area, shown.4, measure);
    last_of(lines(shown, width, measure).len(), area.h)
}

/// What a press at column `x`, row `y` lands on. `fine_y` is the row with its fraction,
/// where the terminal says, and the row's middle where it doesn't.
pub fn hit(
    area: Rect,
    shown: Shown,
    top: usize,
    (x, y, fine_y): (u16, u16, f32),
    measure: &dyn Measure,
) -> Option<Hit> {
    let width = text_width(area, shown.4, measure);
    let all = lines(shown, width, measure);
    let top = top.min(last_of(all.len(), area.h));
    let mut row = top + y.checked_sub(area.y)? as usize;
    // A control on the pane's last row isn't painted, as it would stand past its foot.
    if cut_at_foot(&all, (row, y), area, measure) {
        return None;
    }
    // A control painted part of a row down is pressed where it's painted: not on the top
    // of its own row, and on the top of the row under it.
    let within = (fine_y - f32::from(y)).clamp(0.0, 1.0);
    if within < control_drop(&all, row, measure) {
        return None;
    }
    if let Some(over) = row.checked_sub(1)
        && within < control_drop(&all, over, measure)
    {
        row = over;
    }
    let view = shown.4;
    let a = across(area, view, measure);
    match all.get(row)? {
        Line::Buttons(id) => {
            let pressed = buttons(area, view, measure)
                .0
                .into_iter()
                .find(|(_, cells, at)| x >= *at && x < at + cells.chars().count() as u16);
            match pressed?.0 {
                "Reply" => Some(Hit::Reply(*id)),
                ASK => Some(Hit::Ask(*id)),
                "Edit" => Some(Hit::Edit(*id)),
                "Delete" => Some(Hit::Delete(*id)),
                MORE => Some(Hit::More(*id)),
                _ => Some(Hit::Resolve(*id)),
            }
        }
        Line::Send(_, label) => {
            let pressed = row_of(area, &[label, CANCEL], measure)
                .into_iter()
                .find(|(_, cells, at)| x >= *at && x < at + cells.chars().count() as u16);
            match pressed?.0 {
                CANCEL => Some(Hit::Cancel),
                _ => Some(Hit::Send),
            }
        }
        Line::Choices(id) => {
            let pressed = row_of(area, &CHOICES, measure)
                .into_iter()
                .find(|(_, cells, at)| x >= *at && x < at + cells.chars().count() as u16);
            match pressed?.0 {
                "Keep" => Some(Hit::Keep(*id)),
                "Reply" => Some(Hit::Reply(*id)),
                _ => Some(Hit::Dismiss(*id)),
            }
        }
        Line::Rewrite(id, row) => {
            let labels = rewrite_rows(width, measure).into_iter().nth(*row)?;
            let pressed = row_of(area, &labels, measure)
                .into_iter()
                .find(|(_, cells, at)| x >= *at && x < at + cells.chars().count() as u16);
            match pressed?.0 {
                ACCEPT_REWRITE => Some(Hit::AcceptRewrite(*id)),
                APPLY_REWRITE => Some(Hit::Apply(*id)),
                _ => Some(Hit::Decline(*id)),
            }
        }
        Line::Check(..) => {
            let (cells, at) = check_button(area, view, measure);
            (x >= at && x < at + cells.chars().count() as u16).then_some(Hit::Check)
        }
        line @ (Line::Suggestions(_, _, true) | Line::Settle) => {
            let placed = if matches!(line, Line::Settle) {
                settle_row(area, measure)
            } else {
                settle_buttons(area, view, measure)
            };
            let pressed = placed
                .into_iter()
                .find(|(_, cells, at)| x >= *at && x < at + cells.chars().count() as u16);
            match pressed?.0 {
                "Accept All" => Some(Hit::AcceptAll),
                "Reject All" => Some(Hit::RejectAll),
                _ => Some(Hit::NextSuggestion),
            }
        }
        Line::Deliver(count) => {
            let (cells, at) = deliver_button(area, (*count, shown.2), view, measure);
            (x >= at && x < at + cells.chars().count() as u16).then_some(Hit::SendAll)
        }
        Line::Sort(addressed) => {
            if let Some((cells, at)) = resolve_all(area, (*addressed, view), measure)
                && x >= at
                && x < at + cells.chars().count() as u16
            {
                return Some(Hit::ResolveAddressed);
            }
            let (cells, at) = control(area, &[shown.4.sort.label()], Chevron::Opens, measure);
            (x >= at && x < at + cells.chars().count() as u16).then_some(Hit::Sort)
        }
        Line::Addressed(id) => {
            let pressed = row_of(area, &ADDRESSED_BUTTONS, measure)
                .into_iter()
                .find(|(_, cells, at)| x >= *at && x < at + cells.chars().count() as u16);
            match pressed?.0 {
                "Resolve" => Some(Hit::Resolve(*id)),
                _ => Some(Hit::NotDone(*id)),
            }
        }
        Line::Tools => tools(area, view, measure)
            .into_iter()
            .find(|(_, at)| x >= *at && x < at + TOOL)
            .map(|(tool, _)| match tool {
                Tool::Quick(which) => Hit::Quick(which),
                Tool::Emoji => Hit::Emoji,
                Tool::Picture => Hit::AddPicture,
            }),
        Line::Picture {
            thread,
            which,
            name,
            ..
        } => {
            if *thread == DRAFT_PICTURE {
                return None;
            }
            // The picture opens itself; the card beside it goes to the comment's place.
            let figure = view.pictures.and_then(|pictures| pictures.get(name))?;
            let (left, (wide, _)) = picture_place(area, (*thread, view), figure, measure);
            let at = f32::from(x) + 0.5;
            if at >= left && at < left + wide {
                Some(Hit::Picture(*thread, *which))
            } else {
                Some(Hit::Open(*thread))
            }
        }
        Line::Suggest(found) => {
            let at = f32::from(x) + 0.5;
            suggest_spots((a.text, a.end), found, measure)
                .into_iter()
                .position(|(from, wide)| at >= from && at < from + wide)
                .map(Hit::Suggested)
        }
        Line::File(opens, ..) => opens.map(Hit::File),
        Line::Kinds(kinds) => {
            let at = f32::from(x) + 0.5;
            chips(a.text, kinds, measure)
                .into_iter()
                .find(|(_, from, to)| at >= *from && at < *to)
                .map(|(kind, ..)| Hit::DraftKind(kind))
        }
        Line::Coverage(kinds) => {
            let at = f32::from(x) + 0.5;
            pills(area, kinds, shown.4.kind, measure)
                .into_iter()
                .find(|(_, from, to)| at >= *from && at < *to)
                .map(|(kind, ..)| Hit::Kind(kind))
        }
        Line::Reopen(id) => {
            let (_, cells, at) = row_of(area, &REOPEN, measure).pop()?;
            (x >= at && x < at + cells.chars().count() as u16).then_some(Hit::Reopen(*id))
        }
        Line::Resolved(count, shown) => {
            let (cells, at) = resolved_button(area, *count, *shown, measure);
            (x >= at && x < at + cells.chars().count() as u16).then_some(Hit::ToggleResolved)
        }
        Line::Text(id, text) | Line::Reply(id, text) | Line::Proposed(id, text) => {
            // A link takes the click; anywhere else on the row goes to the comment's place.
            let (from, size) = match all.get(row)? {
                Line::Reply(..) => (a.text, SMALL),
                Line::Proposed(..) => (a.text + box_padding(measure), SMALL),
                _ => (a.text, 100),
            };
            let Some(here) = link_at(text, (from, x), size, measure) else {
                return Some(Hit::Open(*id));
            };
            // Its place among all the comment's links: those on its rows before this one.
            let first = all
                .iter()
                .position(
                    |line| matches!(line, Line::Header(of, ..) | Line::Quote(of, ..) if of == id),
                )
                .unwrap_or(0);
            let before: usize = all[first..row]
                .iter()
                .map(|line| match line {
                    Line::Text(of, text) | Line::Reply(of, text) | Line::Proposed(of, text)
                        if of == id =>
                    {
                        link_spans(text).len()
                    }
                    _ => 0,
                })
                .sum();
            Some(Hit::Link(*id, before + here))
        }
        Line::Pill(id, text, _) if text.starts_with(CHANGED_SINCE) => {
            let from = a.text;
            let at = f32::from(x) + 0.5;
            let on = at >= from && at < from + pill_width(text, measure);
            Some(if on {
                Hit::Changes(*id)
            } else {
                Hit::Open(*id)
            })
        }
        Line::Header(id, _, _, Some((text, _))) if text.starts_with(CHANGED_SINCE) => {
            let from = a.end - pill_width(text, measure);
            let at = f32::from(x) + 0.5;
            Some(if at >= from {
                Hit::Changes(*id)
            } else {
                Hit::Open(*id)
            })
        }
        Line::Quote(id, ..)
        | Line::Diff(id, _)
        | Line::Answer(id, ..)
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
    // A card is painted whole, and one half out of the pane would land on the bar under it.
    let begun = grid.painted_so_far();
    let width = text_width(area, shown.4, measure);
    let threads = shown.0;
    let shown_draft = shown.1;
    let shown_agent = shown.2;
    let view = shown.4;
    let all = lines(shown, width, measure);
    let top = top.min(last_of(all.len(), area.h));
    let (side, right) = (area.x + SIDE, area.right().saturating_sub(SIDE));
    let dim = Style {
        fg: theme.gutter.fg,
        ..theme.sidebar
    };
    let body = theme.sidebar;
    // Each card is drawn whole before what's on it, so text sits on top. A card is raised
    // over the pane, as a menu is over the page. A suggestion's is the same ground, with a
    // dashed edge until it's kept, so it reads as offered rather than chosen: the accent's
    // tint and edge are the lit card's alone. Tinted too, a suggestion under a lit card was
    // a shade from it (#2a2f38 beside #2d3440), and told apart by its dashes only.
    let card = theme.popup;
    let offered_edge = mix(theme.gutter.fg, card, 0.5);
    let row_y = |index: usize| area.y as f32 + index as f32 - top as f32;
    let shown = |y: f32, height: f32| y + height > area.y as f32 && y < area.bottom() as f32;
    // Where the cards stand across the pane and their words in them, `CARD_INSET` in from
    // every edge: their tops over their first words' capitals and their feet under their
    // buttons too.
    let a = across(area, view, measure);
    let (over, under) = crate::measures::card_air(ACTIONS_DROP, measure);
    // Each card's rows, so a short card shows its buttons while the pointer is on it.
    let mut cards: Vec<(usize, usize, u32)> = Vec::new();
    let mut start = None;
    for (index, line) in all.iter().enumerate() {
        match line {
            Line::Header(..) => start = Some(index),
            Line::Quote(..) if start.is_none() => start = Some(index),
            Line::Buttons(_)
            | Line::Choices(_)
            | Line::Addressed(_)
            | Line::Reopen(_)
            | Line::Send(..) => {
                if let Some(first) = start.take() {
                    let (y, height) = (row_y(first), (index - first + 1) as f32);
                    let bounds =
                        Bounds::cells(a.left, y - over, a.right - a.left, height + over + under);
                    let of = match line {
                        Line::Buttons(id) | Line::Choices(id) | Line::Addressed(id) => Some(*id),
                        Line::Reopen(id) | Line::Send(id, _) => Some(*id),
                        _ => None,
                    };
                    if let Some(of) = of {
                        cards.push((first, index, of));
                    }
                    let lit = view.lit.is_some() && of == view.lit;
                    let (fill, edge) = if lit {
                        // The one whose words were pressed in the document.
                        (mix(theme.accent, card, 0.1), theme.accent)
                    } else if matches!(line, Line::Choices(_)) {
                        (card, offered_edge)
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
    // A card's words are laid out from this cell, and painted from `a.text`; its buttons are
    // laid out in whole cells from it, and painted as far right of them.
    let text_x = side + 1;
    let nudge = a.text - f32::from(text_x);
    // What's being written stands in a box with the tools under it, as a field does: on the
    // card's columns, with its words a field's padding inside it.
    let pad = field_pad(measure);
    let first_input = all.iter().position(|line| matches!(line, Line::Input(..)));
    if let Some(first) = first_input {
        let last = all[first..]
            .iter()
            .position(|line| matches!(line, Line::Tools))
            .map_or(first, |tools| first + tools);
        let (y, height) = (row_y(first), (last - first + 1) as f32);
        if shown(y, height) {
            let bounds = Bounds::cells(
                a.text,
                y + FIELD_INSET,
                a.end - a.text,
                height - FIELD_INSET * 2.0,
            );
            grid.panel(bounds, 16, Some(theme.input), Some(theme.popup_border));
        }
    }
    // An agent's rewrite still to be decided stands in a box, `BOX_PADDING` round its words
    // and over its buttons, on the card's columns; the answer over it is lifted with it.
    let boxed = box_padding(measure);
    let lift = rewrite_lift(measure);
    let mut index = 0;
    while index < all.len() {
        if let Line::Proposed(..) = all[index] {
            let rows = all[index..]
                .iter()
                .take_while(|l| matches!(l, Line::Proposed(..)))
                .count();
            let (y, height) = (row_y(index), rows as f32);
            if shown(y, height)
                && let (Some((caps, _)), Some(tails)) =
                    (crate::measures::words_in_row(measure), small_tails(measure))
                && let Some(padding) = crate::measures::rows(crate::measures::BOX_PADDING, measure)
                && let Some((_, row)) = measure.cell_pixels()
            {
                let row = row as f32;
                let top = y + caps / row - padding - lift;
                let foot = y + height - 1.0 + tails / row + padding - lift;
                let bounds = Bounds::cells(a.text, top, a.end - a.text, foot - top);
                grid.panel(bounds, 22, Some(theme.quiet_button(card)), None);
            }
            index += rows.max(1);
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
    // The colour a card is painted in: the lit one's tint, else its own.
    let card_fill = |id: u32| {
        if view.lit == Some(id) {
            mix(theme.accent, card, 0.1)
        } else {
            card
        }
    };
    let pointed = hover
        .filter(|(x, y)| area.contains(*x, *y))
        .map(|(_, y)| top + usize::from(y - area.y));
    let draft = shown_draft;
    // A short card's buttons stand only while the pointer is on the card.
    let hidden = |index: usize| {
        cards.iter().any(|(first, last, id)| {
            (*first..=*last).contains(&index)
                && is_short(*id, draft, view)
                && !pointed.is_some_and(|row| (*first..=*last).contains(&row))
        })
    };
    // Muted text loses contrast on a raised or tinted surface, so it's lifted there.
    let raised_muted = mix(theme.text.fg, theme.gutter.fg, 0.3);
    for (index, line) in all.iter().enumerate().skip(top) {
        let y = area.y + (index - top) as u16;
        if y >= area.bottom() {
            break;
        }
        if cut_at_foot(&all, (index, y), area, measure) {
            continue;
        }
        let from = grid.painted_so_far();
        // A short card's buttons stand while the pointer is on it. Till then their row says
        // how many answers it has and what it waits on, where there's that to say; where
        // there isn't, the buttons stand, so the row is never empty.
        if hidden(index)
            && let Some(note) = short_note(line, threads, shown_agent, view)
        {
            // Down to where the buttons' foot is, so the card's foot is as far under these
            // words as under its buttons: 44 px under them where it had 19 over its first.
            grid.label_at(a.text, text_x, y, &note, inner_dim, right, SMALL);
            grid.lift_since(from, -note_drop(measure));
            continue;
        }
        let actions = matches!(
            line,
            Line::Buttons(_)
                | Line::Choices(_)
                | Line::Addressed(_)
                | Line::Reopen(_)
                | Line::Send(..)
                | Line::Rewrite(..)
        );
        match line {
            Line::Kinds(kinds) => {
                // As the coverage strip's pills are: outlined and quiet, the one picked in the
                // accent's colours, as a kind listed alone is.
                let picked = shown_draft.map(|draft| draft.kind.as_str());
                for (kind, x, end) in chips(a.text, kinds, measure) {
                    let bounds = Bounds::cells(x, y as f32 + PILL.0, end - x, PILL.1);
                    let on_it = hover.is_some_and(|h| {
                        let at = f32::from(h.0) + 0.5;
                        h.1 == y && at >= x && at < end
                    });
                    let chosen = picked == Some(kind);
                    if chosen {
                        let fill = mix(theme.accent, card, 0.28);
                        grid.panel(bounds, 34, Some(fill), Some(theme.accent));
                    } else {
                        let fill = on_it.then(|| theme.quiet_button(card));
                        grid.panel(bounds, 34, fill, Some(theme.popup_border));
                    }
                    let style = Style {
                        fg: if chosen || on_it {
                            theme.text.fg
                        } else {
                            raised_muted
                        },
                        ..inner
                    };
                    let at = x + PILL_ROOM;
                    grid.label_at(at, at.floor() as u16, y, kind, style, right, SMALL);
                }
            }
            Line::Status(text) => {
                grid.label_at(a.text, text_x, y, text, inner_dim, right, SMALL);
            }
            Line::Input(text, caret) => {
                // A field's padding inside its box, which stands on the card's columns.
                let start = a.text + pad;
                let (cell, _) = cell_of(start);
                grid.label_at(start, cell, y, text, inner, right, 100);
                if let Some(at) = caret {
                    let font = Font::default();
                    let x = start + measure.width(&text[..*at], font);
                    grid.set_caret(x, y, 0.62);
                    // The caret goes down into the box with the words it's among.
                    if let Some(caret) = &mut grid.caret {
                        caret.y += cellix_render::units(FIELD_SHIFT);
                    }
                }
                grid.lift_since(from, -FIELD_SHIFT);
            }
            Line::Pad => {}
            Line::Tools => {
                // In the box with the words, from where they start.
                let (_, past) = cell_of(a.text + pad);
                for (tool, at) in tools(area, view, measure) {
                    let on_it = hover.is_some_and(|h| h.1 == y && h.0 >= at && h.0 < at + TOOL);
                    // Each in the middle of its first two columns, with its chip round it.
                    let middle = f32::from(at) + 1.0 + past;
                    if on_it {
                        let chip = crate::measures::glyph_chip(middle, y, measure);
                        grid.panel(chip, 18, Some(theme.popup_hover), None);
                    }
                    match tool {
                        Tool::Quick(which) => {
                            // A size up from the words, where the small ones blur.
                            let emoji = view.quick.map_or("", |quick| quick[which]);
                            let x =
                                crate::measures::emoji_start(emoji, middle, QUICK_SIZE, measure);
                            let x = x.max(f32::from(at));
                            grid.label_at(x, at, y, emoji, inner, at + TOOL, QUICK_SIZE);
                        }
                        Tool::Emoji | Tool::Picture => {
                            let colour = if on_it { theme.text.fg } else { raised_muted };
                            let style = Style {
                                fg: colour,
                                ..inner
                            };
                            let (icon, cells) = if tool == Tool::Emoji {
                                (Icon::Smile, "☺")
                            } else {
                                (Icon::Picture, "▣")
                            };
                            // The size the emoji beside it are, and in its chip's middle.
                            let painted = Style {
                                painted: true,
                                ..style
                            };
                            grid.put_str(at, y, cells, painted, at + TOOL);
                            let wide = TOOL_ICON * crate::measures::rows_to_columns(measure);
                            let top = f32::from(y) + (1.0 - TOOL_ICON) / 2.0;
                            grid.paint.push(Paint::Icon {
                                bounds: Bounds::cells(middle - wide / 2.0, top, wide, TOOL_ICON),
                                icon,
                                color: colour,
                            });
                        }
                    }
                }
                // Up into the row kept over them, clear of the box's bottom line.
                grid.lift_since(from, FIELD_SHIFT);
            }
            Line::Picture {
                thread, name, row, ..
            } => {
                // Drawn whole from whichever of its rows is the first in view, and cut at
                // the pane's edges, which a card scrolled half out of it runs past.
                let figure = view.pictures.and_then(|pictures| pictures.get(name));
                if let Some(figure) = figure.filter(|_| *row == 0 || index == top) {
                    let (left, (wide, tall)) =
                        picture_place(area, (*thread, view), figure, measure);
                    let above = f32::from(y) - *row as f32;
                    let bounds = Bounds::cells(left, above + PICTURE_DROP, wide, tall);
                    let clip = Bounds::cells(
                        f32::from(area.x),
                        f32::from(area.y),
                        f32::from(area.w),
                        f32::from(area.h),
                    );
                    grid.paint.push(Paint::Image {
                        bounds,
                        clip,
                        bitmap: figure.bitmap.clone(),
                        color: theme.text.fg,
                    });
                    // A line round it, so a dark picture has an edge on a dark card. Only
                    // where it's whole: a line can't be cut at the pane's edge as it is.
                    if *row == 0 && f32::from(y) + tall < f32::from(area.bottom()) {
                        grid.panel(bounds, 0, None, Some(theme.popup_border));
                    }
                }
            }
            Line::Suggest(found) => {
                // The first is the one Tab takes, so its name is the plain one.
                let spots = suggest_spots((a.text, a.end), found, measure);
                // And Tab is named at the row's end, where there's room for it.
                let end = spots.last().map_or(0.0, |(x, wide)| x + wide);
                if end + SUGGEST_KEY <= a.end {
                    let (cell, past) = cell_of(a.end);
                    let named = grid.painted_so_far();
                    grid.label_right(cell, y, "Tab", inner_dim, SMALL);
                    grid.nudge_since(named, past);
                }
                for (index, ((emoji, code), (x, wide))) in found.iter().zip(spots).enumerate() {
                    let pointed = hover.is_some_and(|h| {
                        let at = f32::from(h.0) + 0.5;
                        h.1 == y && at >= x && at < x + wide
                    });
                    if index == 0 || pointed {
                        let fill = if index == 0 {
                            theme.pick
                        } else {
                            theme.popup_hover
                        };
                        let chip = Bounds::cells(x - 0.5, f32::from(y) + 0.08, wide + 1.0, 0.84);
                        grid.panel(chip, 18, Some(fill), None);
                    }
                    let cell = x.floor() as u16;
                    grid.label_at(x, cell, y, emoji, inner, right, 100);
                    let name = Style {
                        fg: if index == 0 || pointed {
                            theme.text.fg
                        } else {
                            raised_muted
                        },
                        ..inner
                    };
                    let after = suggest_emoji(emoji, measure);
                    let at = x + after;
                    let code = format!(":{code}:");
                    grid.label_at(at, cell + after as u16, y, &code, name, right, SMALL);
                }
            }
            Line::Hint => {
                // What the tool under the pointer does, when it's on one of the row above.
                let pointed = hover
                    .filter(|h| h.1 + 1 == y && matches!(all.get(index - 1), Some(Line::Tools)))
                    .and_then(|h| {
                        tools(area, view, measure)
                            .into_iter()
                            .find(|(_, at)| h.0 >= *at && h.0 < at + TOOL)
                    })
                    .and_then(|(tool, _)| tool_hint(tool));
                let usual = "Enter sends. Shift+Enter starts a new line. Esc cancels.";
                let hint = pointed.unwrap_or(usual);
                grid.label_at(a.text, text_x, y, hint, inner_dim, right, SMALL);
            }
            Line::Empty(resolved) => {
                let none = if *resolved {
                    "No open comments."
                } else {
                    "No comments yet."
                };
                grid.label(side, y, none, dim, right);
            }
            Line::EmptyHint(hint) => {
                grid.label_at(side as f32, side, y, hint, dim, right, SMALL);
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
                    // At the row's end, on the card's right column: what it's about has
                    // changed or left the plan.
                    let width = pill_width(text, measure);
                    let x = a.end - width;
                    let tone = pill_tone(*words, theme);
                    draw_pill(grid, x, y, text, (right, inner, tone), measure);
                    end = x.floor() as u16;
                }
                grid.label_at(a.text, text_x, y, label, style, end, SMALL);
            }
            Line::Pill(_, text, words) => {
                let tone = pill_tone(*words, theme);
                draw_pill(grid, a.text, y, text, (right, inner, tone), measure);
            }
            Line::Quote(_, quote, words) => {
                let bar = Bounds::cells(a.text, y as f32 + 0.16, 0.22, 0.68);
                let x = a.text + QUOTE_IN;
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
            Line::Diff(_, row) => {
                // On the quote's quiet bar, as words no longer as said are. What went in the
                // removed colour, struck through; what came in the added colour; what stayed
                // as the quote's words are.
                let bar = Bounds::cells(a.text, y as f32 + 0.16, 0.22, 0.68);
                grid.panel(bar, 10, Some(mix(theme.gutter.fg, inner.bg, 0.55)), None);
                let mut x = a.text + QUOTE_IN;
                for (kind, piece) in row {
                    let fg = match kind {
                        -1 => theme.error,
                        1 => theme.git_added,
                        _ => theme.gutter.fg,
                    };
                    let style = Style { fg, ..inner };
                    grid.label_at(x, x.floor() as u16, y, piece, style, right, SMALL);
                    let wide = measure.width(piece, small());
                    if *kind == -1 && !piece.trim().is_empty() {
                        let through = Bounds::cells(x, y as f32 + 0.5, wide, 0.0);
                        grid.hairline(through, theme.error);
                    }
                    x += wide;
                }
            }
            Line::Text(_, text) => {
                let place = (a.text, text_x, y);
                draw_linked(grid, place, text, (inner, 100, right), (measure, theme));
            }
            Line::Answer(_, word, author) => {
                // A line across the card over it, between what was said and the answer, its
                // words on the card's column: it stood in a box whose words were 10 px from its
                // edge where the card's are 24 from its own.
                let rule = Bounds::cells(a.text, f32::from(y), a.end - a.text, 0.0);
                grid.hairline(rule, theme.popup_border);
                let lines_from = grid.painted_so_far();
                let muted = Style {
                    fg: raised_muted,
                    ..inner
                };
                let mut x = a.text;
                if let Some(word) = word {
                    let tone = verdict_tone(word, theme);
                    let chip = measure.width(word, small()) + 0.8;
                    // Low in the row, so it keeps air under the line over it, and tall enough
                    // for the tail of "Agrees", which stood under it.
                    let bounds = Bounds::cells(x, y as f32 + 0.22, chip, 0.74);
                    grid.panel(bounds, 32, Some(mix(tone, card, 0.18)), None);
                    let style = Style {
                        fg: mix(tone, theme.text.fg, 0.6),
                        bold: true,
                        ..muted
                    };
                    grid.label_at(x + 0.4, x.floor() as u16, y, word, style, right, SMALL);
                    grid.drop_last(VERDICT_DROP);
                    x += chip + 0.4;
                }
                let at = (x.ceil() as u16).max(text_x);
                grid.label_at(x, at, y, author, muted, right, SMALL);
                grid.drop_last(VERDICT_DROP);
                // Over a rewrite's box, up with it.
                if matches!(all.get(index + 1), Some(Line::Proposed(..))) {
                    grid.lift_since(lines_from, lift);
                }
            }
            Line::Reply(_, text) => {
                let place = (a.text, text_x, y);
                draw_linked(grid, place, text, (inner, SMALL, right), (measure, theme));
            }
            Line::Proposed(_, text) => {
                // In its box, its padding in from the card's column, and up with it.
                let start = a.text + boxed;
                let style = Style {
                    bg: theme.quiet_button(card),
                    ..inner
                };
                let place = (start, cell_of(start).0, y);
                draw_linked(grid, place, text, (style, SMALL, right), (measure, theme));
                grid.lift_since(from, lift);
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
                // Ending where the cards under it end.
                let chip_x = a.right - chip;
                let bounds = Bounds::cells(chip_x, y as f32 + 0.19, chip, 0.62);
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
                    let icon = crate::explorer::file_icon(name);
                    (icon, crate::explorer::file_color(name), "▪")
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
            Line::Check(mine, suggested, files, filter) => {
                let from = grid.painted_so_far();
                let (cells, x) = check_button(area, view, measure);
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
                // Ending on the cards' right edge.
                grid.nudge_since(from, head_end(area, view).1);
                // With a kind's pill pressed, what's listed: that kind, and how many of all.
                let mut note = match (filter, suggested) {
                    (Some((kind, listed)), _) => {
                        format!("{kind} · {listed} of {}", mine + suggested)
                    }
                    (None, 0) => format!("{mine} of yours"),
                    (None, n) => format!("{mine} of yours · {n} suggested"),
                };
                match files {
                    0 => {}
                    1 => note.push_str(" · 1 file"),
                    n => note.push_str(&format!(" · {n} files")),
                }
                let end = x.saturating_sub(1);
                let start = head(area);
                grid.label_at(f32::from(start), start, y, &note, dim, end, SMALL);
                // Half a row clear over the pills under it.
                grid.lift_since(from, CHECK_LIFT);
            }
            line @ (Line::Suggestions(..) | Line::Settle) => {
                let placed = match line {
                    Line::Suggestions(count, at, beside) => {
                        let words = suggestions_said(*count, *at);
                        let placed = if *beside {
                            settle_buttons(area, view, measure)
                        } else {
                            Vec::new()
                        };
                        let end = placed
                            .first()
                            .map_or(right, |(_, _, x)| x.saturating_sub(1));
                        // Where the summary under it starts, as the head of the list does.
                        let start = head(area);
                        grid.label_at(f32::from(start), start, y, &words, dim, end, SMALL);
                        placed
                    }
                    _ => settle_row(area, measure),
                };
                let from = grid.painted_so_far();
                let alone = matches!(line, Line::Settle);
                // Quiet, as every button on the pane is but Check My Review, and a control's
                // height in its row, as Check My Review is.
                for (label, cells, x) in placed {
                    let width = cells.chars().count() as u16;
                    let on_it = hover.is_some_and(|h| h.1 == y && h.0 >= x && h.0 < x + width);
                    let fill = theme.secondary(body.bg, on_it);
                    // Nothing is painted under the pane, so the chip's corners need a ground,
                    // and its cells keep the pane's colour round it.
                    let ground = Bounds::cells(f32::from(x), f32::from(y), f32::from(width), 1.0);
                    grid.panel(ground, 0, Some(body.bg), None);
                    let style = Style {
                        bg: if measure.in_cells() { fill } else { body.bg },
                        fg: theme.text.fg,
                        ..body
                    };
                    grid.button(x, y, &cells, label, fill, style);
                }
                // On a row of their own they start where the head does, as its controls do;
                // beside the words they end where Check My Review does.
                if !alone {
                    grid.nudge_since(from, head_end(area, view).1);
                }
            }
            Line::Deliver(count) => {
                // Quiet: Check My Review is the pane's filled button. As high in its row as
                // that one is in its own, so the pills under it keep the same air.
                let from = grid.painted_so_far();
                let (cells, x) = deliver_button(area, (*count, shown_agent), view, measure);
                let width = cells.chars().count() as u16;
                let on_it = hover.is_some_and(|h| h.1 == y && h.0 >= x && h.0 < x + width);
                let fill = theme.secondary(body.bg, on_it);
                let ground = Bounds::cells(f32::from(x), f32::from(y), f32::from(width), 1.0);
                grid.panel(ground, 0, Some(body.bg), None);
                let style = Style {
                    bg: if measure.in_cells() { fill } else { body.bg },
                    fg: theme.text.fg,
                    ..body
                };
                let label = deliver_label(*count, shown_agent);
                grid.button(x, y, &cells, &label, fill, style);
                grid.nudge_since(from, head_end(area, view).1);
                grid.lift_since(from, CHECK_LIFT);
            }
            Line::Coverage(kinds) => {
                // A pill for each kind, outlined: filled grey, those with comments read as
                // switched on beside the empty ones. Its kind is in the words' colour where
                // there are comments of it and quiet where there are none, so what isn't
                // covered stands out, and its count is quiet in every one. The one whose
                // comments alone are listed is the accent's, as a pick is, with a × that says
                // a press lists them all again.
                for (kind, x, end) in pills(area, kinds, view.kind, measure) {
                    let count = kinds
                        .iter()
                        .find(|(k, _)| *k == kind)
                        .map_or(0, |(_, n)| *n);
                    let picked = view.kind == Some(kind);
                    let bounds = Bounds::cells(x, y as f32 + PILL.0, end - x, PILL.1);
                    let on_it = hover.is_some_and(|h| {
                        let at = f32::from(h.0) + 0.5;
                        h.1 == y && at >= x && at < end
                    });
                    if picked {
                        let fill = mix(theme.accent, body.bg, 0.28);
                        grid.panel(bounds, 34, Some(fill), Some(theme.accent));
                    } else {
                        let fill = on_it.then(|| theme.quiet_button(body.bg));
                        grid.panel(bounds, 34, fill, Some(theme.popup_border));
                    }
                    let quiet = Style {
                        fg: theme.gutter.fg,
                        ..body
                    };
                    let named = if count > 0 || picked || on_it {
                        Style {
                            fg: theme.text.fg,
                            ..body
                        }
                    } else {
                        quiet
                    };
                    let at = x + PILL_ROOM;
                    grid.label_at(at, at.floor() as u16, y, kind, named, right, SMALL);
                    let number = count.to_string();
                    let after = at + measure.width(&format!("{kind} "), small());
                    let counted = if picked { named } else { quiet };
                    let cell = at.floor() as u16 + kind.chars().count() as u16 + 1;
                    grid.label_at(after, cell, y, &number, counted, right, SMALL);
                    if picked {
                        let cell = cell + number.chars().count() as u16 + 1;
                        let ends = after + measure.width(&number, small());
                        let clear = (Icon::Close, theme.text.fg);
                        draw_clear(grid, (ends, cell, y), clear, named, measure);
                    }
                }
            }
            Line::Rewrite(_, row) => {
                let labels = rewrite_rows(width, measure).into_iter().nth(*row);
                for (label, cells, x) in row_of(area, &labels.unwrap_or_default(), measure) {
                    let width = cells.chars().count() as u16;
                    let on_it = hover.is_some_and(|h| h.1 == y && h.0 >= x && h.0 < x + width);
                    // Three choices, none of them the pane's main action, which is Check My
                    // Review: Accept Rewrite was filled too, two filled buttons at once, as Keep
                    // had been beside Dismiss.
                    let fill = theme.secondary(card, on_it);
                    let style = Style {
                        bg: fill,
                        fg: inner.fg,
                        ..inner
                    };
                    grid.button(x, y, &cells, label, fill, style);
                }
            }
            Line::Choices(_) => {
                // Keep, Reply and Dismiss: the first two answer it, the last drops it.
                for (label, cells, x) in row_of(area, &CHOICES, measure) {
                    let width = cells.chars().count() as u16;
                    let on_it = hover.is_some_and(|h| h.1 == y && h.0 >= x && h.0 < x + width);
                    // Neither is the panel's main action, so both are the one quiet kind, with
                    // its hover: Dismiss was plain words beside a filled Keep.
                    let fill = theme.secondary(card, on_it);
                    let style = Style {
                        bg: fill,
                        fg: inner.fg,
                        ..inner
                    };
                    grid.button(x, y, &cells, label, fill, style);
                }
            }
            Line::Send(_, label) => {
                // What's being written is the one thing under way, so its button is filled.
                for (name, cells, x) in row_of(area, &[label, CANCEL], measure) {
                    let width = cells.chars().count() as u16;
                    let on_it = hover.is_some_and(|h| h.1 == y && h.0 >= x && h.0 < x + width);
                    let main = name != CANCEL;
                    let fill = if main {
                        theme.button
                    } else {
                        theme.secondary(card, on_it)
                    };
                    let style = Style {
                        bg: fill,
                        fg: if main { theme.button_text } else { inner.fg },
                        ..inner
                    };
                    grid.button(x, y, &cells, name, fill, style);
                }
            }
            Line::Buttons(id) => {
                // Check My Review is the panel's one filled button; these are all quiet, on
                // the card's own colour, which a lit card's tint is: Edit and Delete stood on
                // the unlit card's there, two dark slabs.
                let card = card_fill(*id);
                let (placed, at_end) = buttons(area, view, measure);
                for (label, cells, x) in placed {
                    let width = cells.chars().count() as u16;
                    let on_it = hover.is_some_and(|h| h.1 == y && h.0 >= x && h.0 < x + width);
                    // Edit and Delete are the least of them: plain words until pointed at, and
                    // the "⋯" in their place a plain mark.
                    let plain = EDITS.contains(&label) || label == MORE;
                    let fill = match (on_it, plain) {
                        // Pointed at, a plain one takes the quiet fill the others rest in.
                        (true, true) => theme.secondary(card, false),
                        (_, false) => theme.secondary(card, on_it),
                        (false, true) => card,
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
                    let drawn = grid.painted_so_far();
                    if label == MORE {
                        draw_more(grid, (x, y), &cells, (fill, style), measure);
                    } else {
                        grid.button(x, y, &cells, label, fill, style);
                    }
                    // The plain ones end on the card's right column, the others start on its
                    // left one.
                    grid.nudge_since(drawn, if plain { at_end } else { nudge });
                }
            }
            Line::Reopen(_) => {
                for (label, cells, x) in row_of(area, &REOPEN, measure) {
                    let width = cells.chars().count() as u16;
                    let on_it = hover.is_some_and(|h| h.1 == y && h.0 >= x && h.0 < x + width);
                    let fill = theme.secondary(card, on_it);
                    let style = Style {
                        bg: fill,
                        fg: inner.fg,
                        ..inner
                    };
                    grid.button(x, y, &cells, label, fill, style);
                }
            }
            Line::Resolved(count, shown) => {
                // A button that unfolds the resolved comments under it, and folds them away.
                let (cells, x) = resolved_button(area, *count, *shown, measure);
                let width = cells.chars().count() as u16;
                let on_it = hover.is_some_and(|h| h.1 == y && h.0 >= x && h.0 < x + width);
                let labels = resolved_labels(*count, *shown);
                let labels = [labels[0].as_str(), labels[1].as_str()];
                let what = (cells.as_str(), &labels[..], Chevron::Unfolds(*shown));
                let how = (on_it, control_drop(&all, index, measure));
                draw_control(grid, (x, y), what, how, (measure, theme));
            }
            Line::Sort(addressed) => {
                // A button that opens the list of orders, saying the one in use.
                let names = [view.sort.label()];
                let (cells, x) = control(area, &names, Chevron::Opens, measure);
                let width = cells.chars().count() as u16;
                let on_it = hover.is_some_and(|h| h.1 == y && h.0 >= x && h.0 < x + width);
                let what = (cells.as_str(), &names[..], Chevron::Opens);
                let drop = control_drop(&all, index, measure);
                draw_control(grid, (x, y), what, (on_it, drop), (measure, theme));
                // At the row's end, the button that resolves what agents say they addressed.
                if let Some((cells, x)) = resolve_all(area, (*addressed, view), measure) {
                    let from = grid.painted_so_far();
                    let width = cells.chars().count() as u16;
                    let on_it = hover.is_some_and(|h| h.1 == y && h.0 >= x && h.0 < x + width);
                    let fill = theme.secondary(body.bg, on_it);
                    let ground = Bounds::cells(f32::from(x), f32::from(y), f32::from(width), 1.0);
                    grid.panel(ground, 0, Some(body.bg), None);
                    let style = Style {
                        bg: if measure.in_cells() { fill } else { body.bg },
                        fg: theme.text.fg,
                        ..body
                    };
                    let label = resolve_all_label(*addressed);
                    grid.button(x, y, &cells, &label, fill, style);
                    grid.nudge_since(from, head_end(area, view).1);
                    grid.lift_since(from, -drop);
                }
            }
            Line::Addressed(_) => {
                for (label, cells, x) in row_of(area, &ADDRESSED_BUTTONS, measure) {
                    let width = cells.chars().count() as u16;
                    let on_it = hover.is_some_and(|h| h.1 == y && h.0 >= x && h.0 < x + width);
                    let fill = theme.secondary(card, on_it);
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
        if actions {
            // A row of buttons stands on the card's left column, but for those at its end,
            // which `Buttons` puts in their place itself.
            if !matches!(line, Line::Buttons(_)) {
                grid.nudge_since(from, nudge);
            }
            grid.lift_since(from, -ACTIONS_DROP);
        }
    }
    // The thumb in the middle of the air right of the cards.
    let thumb_x = (a.right + f32::from(area.right())) / 2.0;
    draw_thumb(grid, area, (all.len(), top), thumb_x, (measure, theme));
    grid.keep_to_rows(begun, area.y, area.bottom());
}

#[cfg(test)]
mod tests {
    use super::*;
    use cellix_render::CellMeasure;

    /// A rewrite's three buttons share a row where they fit; on a narrower card Accept Rewrite
    /// stands alone and the two that don't take it stay together under it, where Not This was
    /// left on a row of its own.
    #[test]
    fn a_rewrites_buttons_wrap_with_the_quiet_ones_together() {
        let pixels = cellix_render::FontMeasure::new(17, 37);
        let measures: [&dyn Measure; 2] = [&pixels, &CellMeasure];
        for measure in measures {
            assert_eq!(rewrite_rows(200.0, measure), vec![REWRITE.to_vec()]);
            let narrow = (10..200)
                .map(|width| rewrite_rows(width as f32, measure))
                .find(|rows| rows.len() == 2)
                .expect("a width that breaks them");
            assert_eq!(
                narrow,
                vec![vec![ACCEPT_REWRITE], vec![APPLY_REWRITE, "Not This"]]
            );
        }
    }

    /// The row of emoji offered for a name ends with "Tab", the key that takes the first of
    /// them, and keeps room for it: in a pane too narrow for every name and the key, the
    /// last names give way. The key gave way before, which words a sixth larger showed in a
    /// window of 150 columns, where it had been in view.
    #[test]
    fn the_emoji_offered_for_a_name_leave_room_for_the_key_that_takes_one() {
        let found = [
            ("🚀", "rocket"),
            ("🪨", "rock"),
            ("🍀", "shamrock"),
            ("🥦", "broccoli"),
            ("🎸", "guitar"),
        ];
        let pixels = cellix_render::FontMeasure::new(17, 37);
        let measures: [&dyn Measure; 2] = [&pixels, &CellMeasure];
        for measure in measures {
            let mut fewer = false;
            for width in 24..=110 {
                let area = Rect::new(0, 0, width, 10);
                let a = across(area, View::default(), measure);
                let right = a.end;
                let spots = suggest_spots((a.text, a.end), &found, measure);
                assert!(!spots.is_empty(), "the first is always offered");
                let (x, wide) = spots[spots.len() - 1];
                if spots.len() > 1 {
                    assert!(
                        x + wide + SUGGEST_KEY <= right,
                        "{width} columns: {} names end at {} of {right}",
                        spots.len(),
                        x + wide
                    );
                }
                fewer |= spots.len() > 1 && spots.len() < found.len();
            }
            assert!(fewer, "some width holds a few of them and the key");
            let wide = across(Rect::new(0, 0, 110, 10), View::default(), measure);
            let all = suggest_spots((wide.text, wide.end), &found, measure);
            assert_eq!(all.len(), found.len(), "a wide pane holds them all");
        }
    }

    #[test]
    fn a_verdict_reads_its_word_and_kind() {
        assert_eq!(
            verdict("Agree (Evidence): yes"),
            Some(("agrees", "Evidence", "yes"))
        );
        assert_eq!(verdict("Disagree: no"), Some(("pushes back", "", "no")));
        assert_eq!(verdict("Thanks"), None);
    }

    /// Counts what it's asked to measure, and measures as the cell tier does.
    struct Counting(Cell<usize>);

    impl Measure for Counting {
        fn advances(&self, text: &str, font: Font) -> Rc<Vec<f32>> {
            self.0.set(self.0.get() + 1);
            CellMeasure.advances(text, font)
        }

        fn in_cells(&self) -> bool {
            true
        }
    }

    /// A paragraph is measured once, whole, however many rows it breaks into: measured a
    /// word at a time, a long list of comments asked for more strings than the measure
    /// keeps, and shaped all of them again at every frame.
    #[test]
    fn a_paragraph_is_measured_once_however_many_rows_it_breaks_into() {
        let text =
            "one two three four five six seven eight\nand a second one, far too long for a row";
        let measure = Counting(Cell::new(0));
        let rows = wrap(text, 12.0, &measure);
        assert_eq!(
            rows,
            [
                "one two",
                "three four",
                "five six",
                "seven eight",
                "and a second",
                "one, far too",
                "long for a",
                "row"
            ]
        );
        assert_eq!(measure.0.get(), 2, "once a paragraph");
        // And they're the rows that measuring each row by itself gives.
        let by_row = |row: &String| CellMeasure.width(row, Font::default()) <= 12.0;
        assert!(rows.iter().all(by_row));

        // What's being written breaks at the same words, measured as few times.
        let measure = Counting(Cell::new(0));
        let spans = wrap_spans(text, 12.0, &measure);
        let typed: Vec<&str> = spans
            .iter()
            .map(|(start, end)| text[*start..*end].trim_end())
            .collect();
        assert_eq!(typed, rows);
        assert_eq!(measure.0.get(), 2);
    }

    /// In pixels the order's button is painted part of a row down, into the empty row
    /// under it, and is pressed where it's painted: not on the bare top of its own row, and
    /// on the part of it that's in the row below. On cells it's its own row, whole.
    #[test]
    fn a_control_painted_down_its_row_is_pressed_where_it_is_painted() {
        let thread: Thread =
            serde_json::from_str(r#"{"id":1,"file":"a.md","line":1,"quote":"","text":"open"}"#)
                .unwrap();
        let threads = [&thread];
        let shown: Shown = (&threads, None, "", (0, false), View::default());
        let area = Rect::new(0, 0, 60, 30);
        let pixels = cellix_render::FontMeasure::new(17, 37);
        let pressed = |measure: &dyn Measure, fine: f32| {
            hit(
                area,
                shown,
                0,
                (area.x + SIDE + 2, fine as u16, fine),
                measure,
            )
        };
        let middle = |measure: &dyn Measure| {
            (0..30u8)
                .map(f32::from)
                .find(|row| pressed(measure, row + 0.5) == Some(Hit::Sort))
                .expect("the order's button")
        };
        let row = middle(&pixels);
        assert_eq!(pressed(&pixels, row + 0.2), None, "over the chip");
        assert_eq!(pressed(&pixels, row + 0.6), Some(Hit::Sort));
        assert_eq!(
            pressed(&pixels, row + 1.2),
            Some(Hit::Sort),
            "its foot is in the row under"
        );
        assert_eq!(pressed(&pixels, row + 1.6), None, "under the chip");
        // Its list drops from under what's painted of it, on its row and with its drop, from
        // the pane's text edge where the list's head stands.
        let button = order_button(area, shown, 0, &pixels);
        assert_eq!(button, Some((area.x + SIDE, row as u16, CONTROL_DROP)));

        let row = middle(&CellMeasure);
        assert_eq!(pressed(&CellMeasure, row + 0.2), Some(Hit::Sort));
        assert_eq!(pressed(&CellMeasure, row + 1.2), None);
        assert_eq!(
            order_button(area, shown, 0, &CellMeasure),
            Some((area.x + SIDE, row as u16, 0.0))
        );
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

    fn words_at<'t>(text: &'t str, spans: &[(usize, usize)]) -> Vec<&'t str> {
        spans
            .iter()
            .map(|(start, end)| &text[*start..*end])
            .collect()
    }

    #[test]
    fn a_word_that_ends_as_a_pictures_file_does_names_a_picture() {
        let text = "See (shots/a.PNG), then \"b c.jpg\" and .cellix/pasted/p-1.webp. Not a.txt";
        let named = words_at(text, &picture_spans(text));
        assert_eq!(named, ["shots/a.PNG", "c.jpg", ".cellix/pasted/p-1.webp"]);
        // A web address is a link wherever it leads, and an ending alone names nothing.
        let text = "https://example.com/a.png .png and folder/.gif";
        assert!(picture_spans(text).is_empty());
        assert_eq!(
            words_at(text, &link_spans(text)),
            ["https://example.com/a.png"]
        );
        // Both are links to a click, in the order they're written.
        let text = "a.png https://x.org/y b.svg";
        let links = words_at(text, &link_spans(text));
        assert_eq!(links, ["a.png", "https://x.org/y", "b.svg"]);
    }

    #[test]
    fn a_picture_that_is_shown_leaves_the_words_and_one_that_is_not_stays() {
        let figure = crate::rendered::Figure {
            bitmap: cellix_render::Bitmap {
                id: 1,
                width: 40,
                height: 20,
                kind: cellix_render::Pixels::Rgba,
                pixels: std::sync::Arc::new(vec![0; 40 * 20 * 4]),
            },
            width: 4.0,
            height: 1.0,
            baseline: 0.0,
        };
        let mut pictures = Pictures::new();
        pictures.insert("a.png".to_string(), figure.clone());
        let view = View {
            pictures: Some(&pictures),
            ..View::default()
        };
        let (words, held) = shown("Look a.png here, and b.png :+1:", view);
        assert_eq!(words, "Look here, and b.png 👍");
        assert_eq!(held, ["a.png"]);
        let (words, held) = shown("Look\na.png", view);
        assert_eq!((words.as_ref(), held.len()), ("Look", 1));
        // With no pictures to show, every name stays where it was written.
        let (words, held) = shown("Look a.png", View::default());
        assert_eq!((words.as_ref(), held.len()), ("Look a.png", 0));
        // A picture keeps its shape, at its own size where that fits and smaller where not.
        assert_eq!(picture_size(&figure, 40.0), (4.0, 1.0));
        assert_eq!(picture_size(&figure, 2.0), (2.0, 0.5));
        let tall = crate::rendered::Figure {
            width: 10.0,
            height: 32.0,
            ..figure
        };
        assert_eq!(picture_size(&tall, 40.0), (2.5, 8.0));
    }

    /// An answer that begins with a link or a file's name keeps its spelling, and its links
    /// are counted as its card shows them, so a click opens the one under it.
    #[test]
    fn an_answers_links_are_counted_as_its_card_shows_them() {
        let fields =
            r#""id":3,"file":"a.txt","line":1,"quote":"q","text":"See https://a.example/x""#;
        let mut thread: Thread = serde_json::from_str(&format!("{{{fields}}}")).unwrap();
        thread.replies.push(Reply::new(
            "Claude".to_string(),
            "Agree: https://b.example/y and plot.png".to_string(),
        ));
        let found = links(&thread, View::default());
        assert_eq!(
            found,
            ["https://a.example/x", "https://b.example/y", "plot.png"]
        );
        let (_, _, body) = reply_parts(&thread.replies[0]);
        assert_eq!(body, "https://b.example/y and plot.png");
        assert_eq!(sentence("yes, it does"), "Yes, it does");
    }
}
