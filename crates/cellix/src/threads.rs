//! The comments on a document, beside it: each one a short conversation with whoever's
//! working on it, where the agent's answer lands under the question.

use std::borrow::Cow;

use cellix_render::{Bounds, Font, Grid, Icon, Measure, Paint, Rect, Style};
use cellix_review::threads::{Reply, YOU};
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
    /// The order the comments are listed in, which a press changes.
    Sort,
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
    /// Between what's being written and the tools under it, in their box: the words stand
    /// a little down into it and the tools a little up, so neither is on the box's lines.
    Pad,
    /// Under what's being written: the emoji most reached for, the button for the rest, and
    /// the one that adds a picture.
    Tools,
    /// A row of a picture standing on a card: the comment it's on and which of its pictures,
    /// the word that names it, which of the picture's rows this is, and whether it's set in
    /// as an answer's words are.
    Picture {
        thread: u32,
        which: usize,
        name: String,
        row: usize,
        inset: bool,
    },
    /// The emoji whose names start as the one being typed after a colon does, each with its
    /// name, in place of the hint.
    Suggest(Vec<(&'static str, &'static str)>),
    Hint,
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

/// What the button that sends a comment being written says: what sending it does.
fn send_label(draft: &Draft) -> &'static str {
    match (draft.thread, draft.editing, draft.ask) {
        (Some(_), true, _) => "Save",
        (Some(_), false, _) => "Reply",
        (None, _, true) => "Ask agent",
        (None, _, false) => "Comment",
    }
}

const CANCEL: &str = "Cancel";

fn draft_lines(draft: &Draft, view: View, width: f32, measure: &dyn Measure, out: &mut Vec<Line>) {
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
    // The pictures it names so far, so what's being sent is in view.
    let named: Vec<String> = picture_spans(&draft.text)
        .into_iter()
        .map(|(start, end)| draft.text[start..end].to_string())
        .collect();
    picture_lines((DRAFT_PICTURE, false), &named, &mut 0, (view, width), out);
    out.push(Line::Pad);
    out.push(Line::Tools);
    let found = suggested(draft, view);
    out.push(if found.is_empty() {
        Line::Hint
    } else {
        Line::Suggest(found)
    });
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
fn reply_parts(reply: &Reply) -> (Option<String>, String, String) {
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
    (thread, inset): (u32, bool),
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
            inset,
        }));
        *which += 1;
    }
}

/// Where a picture stands on its rows: the column it starts in and how wide and tall it is.
fn picture_place(area: Rect, figure: &crate::rendered::Figure, inset: bool) -> (f32, (f32, f32)) {
    let width = f32::from(area.w.saturating_sub(SIDE * 2 + 2));
    let x = f32::from(area.x + SIDE + 1);
    if inset {
        (x + INSET, picture_size(figure, width - INSET * 2.0))
    } else {
        (x, picture_size(figure, width))
    }
}

/// The order the pane lists comments in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
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

/// How the pane shows what it lists: whether each file's stand under a heading, the comment
/// lit because its text was pressed in the document, and the order they're in.
#[derive(Clone, Copy, Debug)]
pub struct View<'a> {
    pub grouped: bool,
    pub lit: Option<u32>,
    pub sort: Sort,
    /// The emoji that stand under a comment being written. None where the screen can't
    /// draw them, and then names between colons stay as they're typed too.
    pub quick: Option<[&'static str; 8]>,
    /// The pictures comments name, where the screen shows pictures: each stands on its card
    /// under the words, in place of its name among them. Without them a name is a link.
    pub pictures: Option<&'a Pictures>,
}

impl Default for View<'_> {
    fn default() -> Self {
        Self {
            grouped: false,
            lit: None,
            sort: Sort::default(),
            quick: Some(crate::emoji::QUICK),
            pictures: None,
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
const TOOL_ICON: f32 = 0.6;
/// Columns between the emoji and the buttons after them.
const TOOL_GAP: u16 = 1;

/// Each tool under a comment being written and the column it starts in: as many of the
/// quick emoji as leave room for the buttons after them, where the screen draws emoji, and
/// the picture's button either way.
fn tools(area: Rect, view: View) -> Vec<(Tool, u16)> {
    let width = area.w.saturating_sub(SIDE * 2 + 2);
    let x = area.x + SIDE + 1;
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
    area: Rect,
    found: &[(&'static str, &'static str)],
    measure: &dyn Measure,
) -> Vec<(f32, f32)> {
    let right = f32::from(area.right().saturating_sub(SIDE + 1));
    let mut x = f32::from(area.x + SIDE + 1);
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

/// The order's control at the start of the summary row: where its words start, the cell
/// they start in, and how many columns it takes with its chevron.
fn sort_control(area: Rect, sort: Sort, measure: &dyn Measure) -> (f32, u16, u16) {
    let x = area.x + SIDE;
    let words = measure.width(sort.label(), small());
    (f32::from(x), x, (words + SORT_CHEVRON + 0.6).ceil() as u16)
}

/// Columns the chevron after the order's name takes, with its gap.
const SORT_CHEVRON: f32 = 1.4;

/// The row of the list a comment's card starts on, to scroll the pane to it.
pub fn top_of(area: Rect, shown: Shown, id: u32, measure: &dyn Measure) -> Option<usize> {
    let width = area.w.saturating_sub(SIDE * 2 + 2) as f32;
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

fn lines(
    (threads, draft, agent, resolved, view): Shown,
    width: f32,
    measure: &dyn Measure,
) -> Vec<Line> {
    let grouped = view.grouped;
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
        out.push(Line::Sort);
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
        // Its pictures are counted through its words and then its answers.
        let mut which = 0;
        match editing {
            Some(edit) => {
                draft_lines(edit, view, width, measure, &mut out);
                // Its own pictures are still counted, so its answers' keep their places.
                which += shown(&thread.text, view).1.len();
            }
            None => {
                let (words, pictures) = shown(&thread.text, view);
                let text = wrap(&words, width, measure);
                out.extend(text.into_iter().map(|row| Line::Text(id, row)));
                picture_lines((id, false), &pictures, &mut which, (view, width), &mut out);
            }
        }
        for reply in &thread.replies {
            // A verdict reads as one: its word on a chip, who said it, then why.
            let (word, author, body) = reply_parts(reply);
            out.push(Line::Answer(id, word, author));
            let room = width - INSET * 2.0;
            let (words, pictures) = shown(&body, view);
            let text = wrap(&words, room, measure);
            out.extend(text.into_iter().map(|row| Line::Reply(id, row)));
            picture_lines((id, true), &pictures, &mut which, (view, room), &mut out);
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
            draft_lines(reply, view, width, measure, &mut out);
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
        // While it's answered or changed its own buttons give way to the two that send
        // what's being written or drop it: one thing is being done to it at a time.
        let writing = draft.filter(|d| d.thread == Some(id));
        out.push(if let Some(written) = writing {
            Line::Send(id, send_label(written))
        } else if thread.resolved {
            Line::Reopen(id)
        } else if suggested {
            Line::Choices(id)
        } else {
            Line::Buttons(id)
        });
        // Edit and Delete go on a row of their own where the five don't fit on one.
        if writing.is_none() && !thread.resolved && !suggested && !all_fit(width, measure) {
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
        Line::Sort => {
            let (_, from, wide) = sort_control(area, shown.4.sort, measure);
            (x >= from && x < from + wide).then_some(Hit::Sort)
        }
        Line::Tools => tools(area, shown.4)
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
            inset,
            ..
        } => {
            if *thread == DRAFT_PICTURE {
                return None;
            }
            // The picture opens itself; the card beside it goes to the comment's place.
            let figure = shown.4.pictures.and_then(|pictures| pictures.get(name))?;
            let (left, (wide, _)) = picture_place(area, figure, *inset);
            let at = f32::from(x) + 0.5;
            if at >= left && at < left + wide {
                Some(Hit::Picture(*thread, *which))
            } else {
                Some(Hit::Open(*thread))
            }
        }
        Line::Suggest(found) => {
            let at = f32::from(x) + 0.5;
            suggest_spots(area, found, measure)
                .into_iter()
                .position(|(from, wide)| at >= from && at < from + wide)
                .map(Hit::Suggested)
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
        Line::Text(id, text) | Line::Reply(id, text) => {
            // A link takes the click; anywhere else on the row goes to the comment's place.
            let reply = matches!(all.get(row)?, Line::Reply(..));
            let text_x = f32::from(area.x + SIDE + 1);
            let (from, size) = if reply {
                (text_x + INSET, SMALL)
            } else {
                (text_x, 100)
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
                    Line::Text(of, text) | Line::Reply(of, text) if of == id => {
                        link_spans(text).len()
                    }
                    _ => 0,
                })
                .sum();
            Some(Hit::Link(*id, before + here))
        }
        Line::Quote(id, ..) | Line::Answer(id, ..) | Line::Pill(id, ..) | Line::Header(id, ..) => {
            Some(Hit::Open(*id))
        }
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
    let width = area.w.saturating_sub(SIDE * 2 + 2) as f32;
    let threads = shown.0;
    let view = shown.4;
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
            Line::Buttons(_)
            | Line::Edits(_)
            | Line::Choices(_)
            | Line::Reopen(_)
            | Line::Send(..) => {
                if let Some(first) = start.take() {
                    let (y, height) = (row_y(first), (index - first + 1) as f32);
                    // A column from the pane's edge, so its text stands two inside it.
                    let bounds = Bounds::cells(
                        side as f32 - CARD_REACH,
                        y - CARD_AIR.0,
                        (right - side) as f32 + CARD_REACH * 2.0,
                        height + CARD_AIR.0 + CARD_AIR.1,
                    );
                    let of = match line {
                        Line::Buttons(id) | Line::Edits(id) | Line::Choices(id) => Some(*id),
                        Line::Reopen(id) | Line::Send(id, _) => Some(*id),
                        _ => None,
                    };
                    let lit = view.lit.is_some() && of == view.lit;
                    let (fill, edge) = if lit {
                        // The one whose words were pressed in the document.
                        (mix(theme.accent, card, 0.1), theme.accent)
                    } else if matches!(line, Line::Choices(_)) {
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
    let text_x = side + 1;
    // What's being written stands in a box with the tools under it, as a field does: a
    // column inside the card, with the words a column inside it.
    let first_input = all.iter().position(|line| matches!(line, Line::Input(..)));
    if let Some(first) = first_input {
        let last = all[first..]
            .iter()
            .position(|line| matches!(line, Line::Tools))
            .map_or(first, |tools| first + tools);
        let (y, height) = (row_y(first), (last - first + 1) as f32);
        if shown(y, height) {
            let bounds = Bounds::cells(
                f32::from(text_x) - 1.0,
                y + FIELD_INSET,
                f32::from(right - text_x) + 1.0,
                height - FIELD_INSET * 2.0,
            );
            grid.panel(bounds, 16, Some(theme.input), Some(theme.popup_border));
        }
    }
    // An agent's answer is set in, on a shade of its card, so who said what is plain.
    let mut index = 0;
    while index < all.len() {
        if let Line::Answer(id, ..) = all[index] {
            let rows = 1 + all[index + 1..]
                .iter()
                .take_while(|l| matches!(l, Line::Reply(..) | Line::Picture { inset: true, .. }))
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
                | Line::Send(..)
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
                    // The caret goes down into the box with the words it's among.
                    if let Some(caret) = &mut grid.caret {
                        caret.y += cellix_render::units(FIELD_SHIFT);
                    }
                }
                grid.lift_since(from, -FIELD_SHIFT);
            }
            Line::Pad => {}
            Line::Tools => {
                for (tool, at) in tools(area, view) {
                    let on_it = hover.is_some_and(|h| h.1 == y && h.0 >= at && h.0 < at + TOOL);
                    // Each in the middle of its first two columns, with its chip round it.
                    let middle = f32::from(at) + 1.0;
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
                name, row, inset, ..
            } => {
                // Drawn whole from whichever of its rows is the first in view, and cut at
                // the pane's edges, which a card scrolled half out of it runs past.
                let figure = view.pictures.and_then(|pictures| pictures.get(name));
                if let Some(figure) = figure.filter(|_| *row == 0 || index == top) {
                    let (left, (wide, tall)) = picture_place(area, figure, *inset);
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
                let spots = suggest_spots(area, found, measure);
                // And Tab is named at the row's end, where there's room for it.
                let end = spots.last().map_or(0.0, |(x, wide)| x + wide);
                if end + SUGGEST_GAP + 3.0 <= f32::from(right) {
                    grid.label_right(right, y, "Tab", inner_dim, SMALL);
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
                        tools(area, view)
                            .into_iter()
                            .find(|(_, at)| h.0 >= *at && h.0 < at + TOOL)
                    })
                    .and_then(|(tool, _)| tool_hint(tool));
                let usual = "Enter sends. Shift+Enter starts a new line. Esc cancels.";
                let hint = pointed.unwrap_or(usual);
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
                let place = (f32::from(text_x), text_x, y);
                draw_linked(grid, place, text, (inner, 100, right), (measure, theme));
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
                let place = (reply_x, text_x + 1, y);
                draw_linked(grid, place, text, (style, SMALL, right), (measure, theme));
                // Lifted a little, so the block keeps air above the buttons and below its
                // text: the row and the links drawn over it together.
                grid.lift_since(from, 0.12);
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
            Line::Send(_, label) => {
                // What's being written is the one thing under way, so its button is filled.
                for (name, cells, x) in row_of(area, &[label, CANCEL], measure) {
                    let width = cells.chars().count() as u16;
                    let on_it = hover.is_some_and(|h| h.1 == y && h.0 >= x && h.0 < x + width);
                    let main = name != CANCEL;
                    let fill = match (main, on_it) {
                        (true, _) => theme.button,
                        (false, true) => theme.popup_hover,
                        (false, false) => theme.quiet_button(card),
                    };
                    let style = Style {
                        bg: fill,
                        fg: if main { theme.button_text } else { inner.fg },
                        ..inner
                    };
                    grid.button(x, y, &cells, name, fill, style);
                }
            }
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
            Line::Sort => {
                // Quiet words and a chevron, brighter under the pointer.
                let (at, cell, wide) = sort_control(area, view.sort, measure);
                let on_it = hover.is_some_and(|h| h.1 == y && h.0 >= cell && h.0 < cell + wide);
                let order = Style {
                    fg: if on_it { theme.text.fg } else { dim.fg },
                    ..dim
                };
                let name = view.sort.label();
                let end = grid.label_at(at, cell, y, name, order, right, SMALL);
                let after = at + measure.width(name, small()) + 0.4;
                let plain = Style {
                    bold: false,
                    ..order
                };
                let mark = if measure.in_cells() {
                    f32::from(end) + 1.0
                } else {
                    after
                };
                grid.glyph(mark, y, Icon::ChevronDown, order.fg, "▾", plain);
            }
            Line::Gap => {}
        }
        if actions {
            grid.lift_since(from, -ACTIONS_DROP);
        }
    }
    grid.keep_to_rows(begun, area.y, area.bottom());
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
        thread.replies.push(Reply {
            author: "Claude".to_string(),
            text: "Agree: https://b.example/y and plot.png".to_string(),
        });
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
