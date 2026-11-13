//! Emoji for a comment: the whole set by the names GitHub gives them, typed as `:name:`, the
//! most used a click away, and a picker that lists every one, searched by name.

use std::borrow::Cow;

use cellix_input::Key;
use cellix_render::{Bounds, Font, Grid, Icon, Measure, Rect, Style};
use emojis::{Emoji, EmojiVersion, Group};

use crate::measures::{emoji_start, glyph_chip};
use crate::theme::Theme;

/// The ones a click away beside a comment being written, until the person's own use says
/// which they reach for.
pub const QUICK: [&str; 8] = ["👍", "💯", "❤️", "🎉", "😄", "👀", "🚀", "✅"];

/// How many letters of a name bring its suggestions up: one would offer something at every
/// `:p` and `:D`.
const SUGGEST_FROM: usize = 2;

fn code_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '+' | '-')
}

/// Whether a `:name:` may start after `before`: at the start of the words, or after anything
/// but a letter, a digit or another colon, so `a::b::c` and 10:30:00 stay as they're typed.
fn may_start(before: Option<char>) -> bool {
    before.is_none_or(|c| !c.is_alphanumeric() && c != ':')
}

/// Whether an emoji is one to offer: those from the last year or two are missing from the
/// fonts and terminals of many machines, where they'd show as a box. One typed by its name
/// is still taken.
fn offered(emoji: &Emoji) -> bool {
    emoji.emoji_version() <= EmojiVersion::new(15, 0)
}

fn named(name: &str) -> Option<&'static Emoji> {
    emojis::get_by_shortcode(&name.to_ascii_lowercase())
}

/// The name being typed at the caret, as `:roc`: where its colon is, and its letters so far.
pub fn open_code(text: &str, caret: usize) -> Option<(usize, &str)> {
    let before = text.get(..caret)?;
    let colon = before.rfind(':')?;
    let name = &before[colon + 1..];
    let whole = !name.is_empty() && name.chars().all(code_char);
    (whole && may_start(before[..colon].chars().next_back())).then_some((colon, name))
}

/// Turns the `:name:` that ends at the caret into its emoji, as its second colon is typed.
/// Returns the emoji when it did.
pub fn close_code(text: &mut String, caret: &mut usize) -> Option<&'static str> {
    let inner = text.get(..*caret)?.strip_suffix(':')?;
    let (colon, name) = open_code(inner, inner.len())?;
    let emoji = named(name)?.as_str();
    text.replace_range(colon..*caret, emoji);
    *caret = colon + emoji.len();
    Some(emoji)
}

/// Words with each `:name:` in them shown as its emoji: an agent's answer may spell them out.
pub fn expand(text: &str) -> Cow<'_, str> {
    let mut out = String::new();
    let (mut copied, mut from) = (0, 0);
    while let Some(found) = text[from..].find(':') {
        let colon = from + found;
        let after = &text[colon + 1..];
        let length = after.find(|c: char| !code_char(c)).unwrap_or(after.len());
        let end = colon + 1 + length + 1;
        let closes = length > 0 && after[length..].starts_with(':');
        // Nothing may run on from it either: `:x::y` is a path, not a cross.
        let alone = may_start(text[..colon].chars().next_back())
            && may_start(text.get(end..).and_then(|rest| rest.chars().next()));
        match named(&after[..length]).filter(|_| closes && alone) {
            Some(emoji) => {
                out.push_str(&text[copied..colon]);
                out.push_str(emoji.as_str());
                (copied, from) = (end, end);
            }
            None => from = colon + 1,
        }
    }
    if copied == 0 {
        return Cow::Borrowed(text);
    }
    out.push_str(&text[copied..]);
    Cow::Owned(out)
}

/// How well a name answers what's typed: the name itself, one that starts with it, a word of
/// it that does, or one that only holds it.
fn rank(typed: &str, name: &str) -> Option<u8> {
    if name == typed {
        Some(0)
    } else if name.starts_with(typed) {
        Some(1)
    } else if name
        .split([' ', '_', '-'])
        .any(|word| word.starts_with(typed))
    {
        Some(2)
    } else {
        name.contains(typed).then_some(3)
    }
}

/// The emoji whose names answer what's typed after a colon, each with the name that did:
/// the name itself first, then those in `first`, then short names before long.
pub fn suggest(
    typed: &str,
    first: &[&'static str],
    limit: usize,
) -> Vec<(&'static str, &'static str)> {
    let typed = typed.to_ascii_lowercase();
    if typed.len() < SUGGEST_FROM {
        return Vec::new();
    }
    let mut found = Vec::new();
    for (order, emoji) in emojis::iter().filter(|e| offered(e)).enumerate() {
        let best = emoji
            .shortcodes()
            .filter_map(|code| Some((rank(&typed, code)?, code.len(), code)))
            .min();
        if let Some((rank, length, code)) = best {
            let reached_for = !first.contains(&emoji.as_str());
            let exact = rank != 0;
            found.push((
                (exact, rank > 1, reached_for, rank, length, order),
                emoji.as_str(),
                code,
            ));
        }
    }
    found.sort();
    found.truncate(limit);
    found
        .into_iter()
        .map(|(_, emoji, code)| (emoji, code))
        .collect()
}

/// An emoji's own string in the set, for one written with or without its variation mark.
pub fn known(emoji: &str) -> Option<&'static str> {
    emojis::get(emoji).map(Emoji::as_str)
}

/// The eight that stand beside a comment being written: the ones last used, then the usual.
pub fn quick(used: &[&'static str]) -> [&'static str; 8] {
    let mut out = QUICK;
    let all = used
        .iter()
        .copied()
        .chain(QUICK.iter().filter_map(|usual| known(usual)))
        .fold(Vec::new(), |mut all: Vec<&'static str>, emoji| {
            if !all.contains(&emoji) {
                all.push(emoji);
            }
            all
        });
    for (place, emoji) in out.iter_mut().zip(all) {
        *place = emoji;
    }
    out
}

fn group_name(group: Group) -> &'static str {
    match group {
        Group::SmileysAndEmotion => "SMILEYS",
        Group::PeopleAndBody => "PEOPLE",
        Group::AnimalsAndNature => "NATURE",
        Group::FoodAndDrink => "FOOD",
        Group::TravelAndPlaces => "PLACES",
        Group::Activities => "ACTIVITIES",
        Group::Objects => "OBJECTS",
        Group::Symbols => "SYMBOLS",
        Group::Flags => "FLAGS",
    }
}

/// Emoji to a row.
const COLUMNS: usize = 10;
/// Columns an emoji's square takes: its own two and one before the next.
const SQUARE: u16 = 3;
/// Columns between the panel's edge and what's in it.
const INSET: u16 = 2;
/// The field and the line under it.
const HEADER_ROWS: u16 = 2;
const LIST_ROWS: u16 = 9;
/// A line, and the name of the one pointed at.
const FOOT_ROWS: u16 = 2;
/// Where the line under the field is, in rows from the panel's top: the middle of the gap.
const HEADER_LINE: f32 = 1.5;
/// How far the panel reaches past its first and last rows, in rows, so the field and the
/// name at its foot keep clear of its edges. Dropped from the row under a comment's tools,
/// that puts its top edge on the bottom line of the box they stand in.
const PADDING: f32 = crate::threads::FIELD_INSET;
/// The least the thumb at the list's edge is, in rows: less is a speck.
const THUMB: f32 = 1.4;
/// How far a heading after the first is dropped in its row, toward the emoji it names.
const HEADING_DROP: f32 = 0.2;
/// An emoji in the picker is drawn a little larger than text, as a percentage.
const SIZE: u8 = 115;
const MOST_USED: &str = "MOST USED";

/// A row of the picker's list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Row {
    Heading(&'static str),
    /// Emoji of `shown`, from this one, this many.
    Emoji(usize, usize),
}

/// Every emoji in a grid, under a field that searches them by name.
pub struct Picker {
    origin: (u16, u16),
    pub query: String,
    /// The ones listed first while nothing is typed.
    first: Vec<&'static str>,
    shown: Vec<&'static Emoji>,
    rows: Vec<Row>,
    pub selected: usize,
    top: usize,
}

impl Picker {
    /// A picker dropping from `at`, or standing over it where the screen ends first.
    pub fn new(at: (u16, u16), screen: Rect, first: &[&'static str]) -> Self {
        let (width, height) = Self::size();
        // A column clear of the screen's edge, so its own edge and shadow show.
        let x =
            at.0.min(screen.right().saturating_sub(width + 1))
                .max(screen.x);
        let y = if at.1 + height > screen.bottom() {
            at.1.saturating_sub(height + 1).max(screen.y)
        } else {
            at.1
        };
        let mut picker = Self {
            origin: (x, y),
            query: String::new(),
            first: first.to_vec(),
            shown: Vec::new(),
            rows: Vec::new(),
            selected: 0,
            top: 0,
        };
        picker.list();
        picker
    }

    fn size() -> (u16, u16) {
        // The last emoji has no gap to keep after it, so each side has the same air.
        (
            INSET * 2 + SQUARE * COLUMNS as u16 - 1,
            HEADER_ROWS + LIST_ROWS + FOOT_ROWS,
        )
    }

    pub fn area(&self) -> Rect {
        let (width, height) = Self::size();
        Rect::new(self.origin.0, self.origin.1, width, height)
    }

    /// Lists what the field asks for: everything by kind under the most used, or what's
    /// named like what's typed, the nearest first.
    fn list(&mut self) {
        self.shown.clear();
        self.rows.clear();
        (self.selected, self.top) = (0, 0);
        let typed = self.query.trim().to_lowercase();
        if typed.is_empty() {
            let first: Vec<&'static Emoji> =
                self.first.iter().filter_map(|e| emojis::get(e)).collect();
            if !first.is_empty() {
                self.rows.push(Row::Heading(MOST_USED));
                self.add(first);
            }
            for group in Group::iter() {
                self.rows.push(Row::Heading(group_name(group)));
                self.add(group.emojis().filter(|e| offered(e)).collect());
            }
            return;
        }
        let mut found: Vec<((u8, usize), &'static Emoji)> = emojis::iter()
            .filter(|e| offered(e))
            .enumerate()
            .filter_map(|(order, emoji)| {
                let names = emoji.shortcodes().chain(std::iter::once(emoji.name()));
                let best = names.filter_map(|name| rank(&typed, name)).min()?;
                Some(((best, order), emoji))
            })
            .collect();
        found.sort_by_key(|(key, _)| *key);
        self.add(found.into_iter().map(|(_, emoji)| emoji).collect());
    }

    /// Puts these after what's listed, ten to a row.
    fn add(&mut self, emoji: Vec<&'static Emoji>) {
        let mut from = self.shown.len();
        self.shown.extend(emoji);
        while from < self.shown.len() {
            let count = (self.shown.len() - from).min(COLUMNS);
            self.rows.push(Row::Emoji(from, count));
            from += count;
        }
    }

    /// The emoji that Enter or a click takes.
    pub fn current(&self) -> Option<&'static str> {
        self.shown.get(self.selected).map(|emoji| emoji.as_str())
    }

    /// Which row of the list an emoji is on, and where on it.
    fn place_of(&self, index: usize) -> Option<(usize, usize)> {
        self.rows
            .iter()
            .enumerate()
            .find_map(|(row, kind)| match kind {
                Row::Emoji(from, count) if (*from..from + count).contains(&index) => {
                    Some((row, index - from))
                }
                _ => None,
            })
    }

    /// The emoji on the nearest row of them above or below, in the same column or its last.
    fn beside(&self, down: bool) -> usize {
        let Some((row, column)) = self.place_of(self.selected) else {
            return self.selected;
        };
        let rows: Box<dyn Iterator<Item = &Row>> = if down {
            Box::new(self.rows[row + 1..].iter())
        } else {
            Box::new(self.rows[..row].iter().rev())
        };
        rows.filter_map(|kind| match kind {
            Row::Emoji(from, count) => Some(from + column.min(count - 1)),
            Row::Heading(_) => None,
        })
        .next()
        .unwrap_or(self.selected)
    }

    /// A key in the picker: the arrows move, and anything typed is looked for.
    pub fn key(&mut self, key: Key) {
        let last = self.shown.len().saturating_sub(1);
        match key {
            Key::Left => self.selected = self.selected.saturating_sub(1),
            Key::Right => self.selected = (self.selected + 1).min(last),
            Key::Up => self.selected = self.beside(false),
            Key::Down => self.selected = self.beside(true),
            Key::Backspace => {
                self.query.pop();
                self.list();
            }
            Key::Char(c) => {
                self.query.push(c);
                self.list();
            }
            _ => {}
        }
        self.keep_in_view();
    }

    /// Takes pasted words into the field.
    pub fn paste(&mut self, text: &str) {
        self.query.push_str(text.trim());
        self.list();
    }

    fn keep_in_view(&mut self) {
        let Some((row, _)) = self.place_of(self.selected) else {
            return;
        };
        // Its heading comes with the first row under it.
        let above = match row.checked_sub(1).map(|r| self.rows[r]) {
            Some(Row::Heading(_)) => row - 1,
            _ => row,
        };
        if above < self.top {
            self.top = above;
        }
        if row >= self.top + LIST_ROWS as usize {
            self.top = row + 1 - LIST_ROWS as usize;
        }
    }

    /// Moves the list by rows, as the wheel does.
    pub fn scroll(&mut self, by: isize) {
        let last = self.rows.len().saturating_sub(LIST_ROWS as usize) as isize;
        self.top = (self.top as isize + by).clamp(0, last.max(0)) as usize;
    }

    /// The emoji at a place on screen.
    pub fn at(&self, x: u16, y: u16) -> Option<usize> {
        let area = self.area();
        let row = y.checked_sub(area.y + HEADER_ROWS)?;
        if row >= LIST_ROWS {
            return None;
        }
        let column = (x.checked_sub(area.x + INSET)? / SQUARE) as usize;
        match self.rows.get(self.top + row as usize)? {
            Row::Emoji(from, count) if column < *count => Some(from + column),
            _ => None,
        }
    }

    pub fn draw(
        &self,
        grid: &mut Grid,
        hover: Option<(u16, u16)>,
        measure: &dyn Measure,
        theme: &Theme,
    ) {
        let area = self.area();
        let base = Style {
            fg: theme.text.fg,
            bg: theme.popup,
            ..theme.bar
        };
        grid.fill(area, base);
        let panel = Bounds::inset(area, 0.0, -PADDING);
        grid.popup(panel, 28, theme.popup, theme.popup_border);
        let dim = Style {
            fg: theme.popup_dim,
            ..base
        };
        let left = area.x + INSET;
        let right = area.right().saturating_sub(INSET);
        grid.glyph(
            f32::from(left),
            area.y,
            Icon::Search,
            theme.popup_dim,
            "⌕",
            base,
        );
        let x = f32::from(left) + 2.3;
        let cell = left + 2;
        if self.query.is_empty() {
            grid.label_at(x + 0.45, cell, area.y, "Search emoji", dim, right, 100);
        } else {
            grid.label_at(x, cell, area.y, &self.query, base, right, 100);
        }
        let caret = x - 0.1 + measure.width(&self.query, Font::default());
        grid.set_caret(caret, area.y, 0.56);
        let across = f32::from(area.w);
        let line = Bounds::cells(
            f32::from(area.x),
            f32::from(area.y) + HEADER_LINE,
            across,
            0.0,
        );
        grid.hairline(line, theme.popup_border);
        let pointed = hover.and_then(|h| self.at(h.0, h.1));
        for (line, row) in self
            .rows
            .iter()
            .skip(self.top)
            .take(LIST_ROWS as usize)
            .enumerate()
        {
            let y = area.y + HEADER_ROWS + line as u16;
            match row {
                Row::Heading(name) => {
                    let small = Style { bold: true, ..dim };
                    let from = grid.painted_so_far();
                    grid.label_at(f32::from(left), left, y, name, small, right, 80);
                    // Nearer the emoji it names than the ones above it.
                    if line > 0 {
                        grid.lift_since(from, -HEADING_DROP);
                    }
                }
                Row::Emoji(from, count) => {
                    for (column, index) in (*from..from + count).enumerate() {
                        let x = left + SQUARE * column as u16;
                        let emoji = self.shown[index].as_str();
                        if index == self.selected || pointed == Some(index) {
                            let fill = if index == self.selected {
                                theme.pick
                            } else {
                                theme.popup_hover
                            };
                            let chip = glyph_chip(f32::from(x) + 1.0, y, measure);
                            grid.panel(chip, 18, Some(fill), None);
                        }
                        // In the middle of its two columns, however wide the font draws it.
                        let at = emoji_start(emoji, f32::from(x) + 1.0, SIZE, measure);
                        grid.label_at(at.max(f32::from(x)), x, y, emoji, base, x + SQUARE, SIZE);
                    }
                }
            }
        }
        // A thumb at the list's edge, where there's more than is in view.
        if self.rows.len() > LIST_ROWS as usize {
            let all = self.rows.len() as f32;
            let tall = (f32::from(LIST_ROWS) * f32::from(LIST_ROWS) / all).max(THUMB);
            let room = f32::from(LIST_ROWS) - tall;
            let last = (all - f32::from(LIST_ROWS)).max(1.0);
            let top = f32::from(area.y + HEADER_ROWS) + room * self.top as f32 / last;
            let thumb = Bounds::cells(f32::from(area.right()) - 0.95, top + 0.1, 0.4, tall - 0.2);
            grid.panel(thumb, 12, Some(theme.scrollbar), None);
        }
        if self.shown.is_empty() {
            let y = area.y + HEADER_ROWS;
            let none = "No emoji by that name";
            grid.label_at(f32::from(left), left, y, none, dim, right, 100);
        }
        // What's pointed at, or picked: its name, and how it's typed.
        let foot = area.bottom().saturating_sub(1);
        let rule = Bounds::cells(f32::from(area.x), f32::from(foot) - 0.5, across, 0.0);
        grid.hairline(rule, theme.popup_border);
        let Some(emoji) = pointed
            .or(Some(self.selected))
            .and_then(|i| self.shown.get(i))
        else {
            return;
        };
        let code = emoji.shortcode().map(|code| format!(":{code}:"));
        let code_width = code.as_deref().map_or(0.0, |code| {
            measure.width(
                code,
                Font {
                    size: 85,
                    ..Font::default()
                },
            )
        });
        let end = (f32::from(right) - code_width - 1.0).floor() as u16;
        let name = emoji.name();
        grid.label_at(f32::from(left), left, foot, name, base, end.max(left), 90);
        if let Some(code) = &code {
            grid.label_right(right, foot, code, dim, 85);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_between_colons_becomes_its_emoji_as_its_second_colon_is_typed() {
        let mut text = "good :+1:".to_string();
        let mut caret = text.len();
        assert_eq!(close_code(&mut text, &mut caret), Some("👍"));
        assert_eq!((text.as_str(), caret), ("good 👍", "good 👍".len()));
        // In the middle of the words, with what follows left alone.
        let mut text = "a :100: b".to_string();
        let mut caret = "a :100:".len();
        assert_eq!(close_code(&mut text, &mut caret), Some("💯"));
        assert_eq!(text, "a 💯 b");
        assert_eq!(caret, "a 💯".len());
    }

    #[test]
    fn a_path_a_time_and_an_unknown_name_stay_as_typed() {
        for words in ["std::x:", "at 10:30:", "say :nothing_by_this_name:", "::"] {
            let mut text = words.to_string();
            let mut caret = text.len();
            assert_eq!(close_code(&mut text, &mut caret), None, "{words}");
            assert_eq!(text, words);
        }
        assert_eq!(expand("a::x::b and 10:30:00"), "a::x::b and 10:30:00");
        assert_eq!(expand("no colons here"), "no colons here");
    }

    #[test]
    fn spelled_out_names_show_as_their_emoji() {
        assert_eq!(expand("Done :tada: (:rocket:)"), "Done 🎉 (🚀)");
        assert_eq!(expand(":+1:"), "👍");
        assert_eq!(expand(":Rocket: up"), "🚀 up");
    }

    #[test]
    fn what_is_typed_after_a_colon_is_looked_for_among_the_names() {
        assert_eq!(open_code("go :roc", 7), Some((3, "roc")));
        assert_eq!(open_code("go :roc ket", 11), None);
        assert_eq!(open_code("a:roc", 5), None, "inside a word");
        let found = suggest("roc", &QUICK, 4);
        assert_eq!(found.first(), Some(&("🚀", "rocket")));
        // The name itself comes before a longer one that starts with it, and one reached
        // for often before others that start the same.
        assert_eq!(suggest("heart", &[], 3).first(), Some(&("❤️", "heart")));
        let first = suggest("th", &["👍"], 3);
        assert_eq!(first.first(), Some(&("👍", "thumbsup")));
        assert!(
            suggest("r", &QUICK, 4).is_empty(),
            "one letter says too little"
        );
    }

    #[test]
    fn the_quick_ones_are_the_last_used_and_then_the_usual() {
        assert_eq!(quick(&[]), QUICK.map(|e| known(e).unwrap()));
        let used = [known("🔥").unwrap(), known("💯").unwrap()];
        let now = quick(&used);
        assert_eq!(&now[..3], &["🔥", "💯", "👍"]);
        assert_eq!(now.len(), 8);
        assert_eq!(now.iter().filter(|e| **e == "💯").count(), 1);
    }

    #[test]
    fn the_picker_lists_every_emoji_and_finds_them_by_name() {
        let screen = Rect::new(0, 0, 100, 40);
        let mut picker = Picker::new((60, 10), screen, &QUICK);
        let offered = emojis::iter().filter(|e| offered(e)).count();
        assert!(offered > 1500, "{offered}");
        assert_eq!(picker.shown.len(), QUICK.len() + offered);
        assert_eq!(picker.rows[0], Row::Heading(MOST_USED));
        assert_eq!(picker.current(), Some("👍"));
        // Down from the most used goes to the first row of faces, under its heading.
        picker.key(Key::Down);
        assert_eq!(picker.current(), Some("😀"));
        picker.key(Key::Right);
        picker.key(Key::Up);
        assert_eq!(picker.current(), Some("💯"));
        for c in "tada".chars() {
            picker.key(Key::Char(c));
        }
        assert_eq!(picker.current(), Some("🎉"));
        // A word of its name finds it too.
        picker.query.clear();
        picker.paste("party");
        assert!(picker.shown.iter().any(|e| e.as_str() == "🎉"));
        picker.query = "zzzz".to_string();
        picker.list();
        assert_eq!(picker.current(), None);
    }

    #[test]
    fn the_picker_stays_on_screen_and_scrolls_with_what_is_picked() {
        let screen = Rect::new(0, 0, 80, 24);
        let picker = Picker::new((78, 20), screen, &QUICK);
        let area = picker.area();
        assert!(area.right() < 80 && area.bottom() <= 24, "{area:?}");
        let mut picker = Picker::new((10, 2), screen, &QUICK);
        for _ in 0..30 {
            picker.key(Key::Down);
        }
        let (row, _) = picker.place_of(picker.selected).unwrap();
        assert!(row >= picker.top && row < picker.top + LIST_ROWS as usize);
        // A click lands on what's drawn there.
        let area = picker.area();
        let y = area.y + HEADER_ROWS + (row - picker.top) as u16;
        let (_, column) = picker.place_of(picker.selected).unwrap();
        let x = area.x + INSET + SQUARE * column as u16;
        assert_eq!(picker.at(x, y), Some(picker.selected));
        assert_eq!(picker.at(area.x, area.y), None);
    }
}
