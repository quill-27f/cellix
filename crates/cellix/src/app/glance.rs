//! A comment's card under the pointer while the Comments pane is shut. The pane holds what
//! was opened, so a press on commented words leaves it shut; resting the pointer on them shows
//! their comment instead, on a card like a menu: who made it and its kind, its words, the last
//! answer, and how many answers there are, with "Open in Comments". A press on the card opens
//! the pane on that comment, as a click on its card in the pane goes to its words. The card
//! stands under the words, its own words starting where theirs do, or over them where there's
//! no room under; never on them. It goes when the pointer leaves the words and the card.
//!
//! It's for words a comment tints, which a page shows: code marks a commented line in its
//! gutter instead.

use super::*;

/// The most columns the card's words are set to: about VS Code's hover.
const WIDE: f32 = 40.0;
/// The fewest columns a card's words need, under which there's none.
const NARROWEST: f32 = 16.0;
/// Columns from the card's edge to its words, as a menu's.
const INSET: f32 = 2.0;
/// The most rows a comment and the answer under it take; what's more ends in "…".
const COMMENT_ROWS: usize = 3;
const ANSWER_ROWS: usize = 2;
/// The room the rule over the foot takes at least, painted: a menu's. What whole rows leave
/// over goes to it, as it goes to a menu's rules, so the card's edges keep a menu's air.
const RULE_ROOM: f32 = 0.5;
/// What the foot says the card does.
pub(super) const OPEN: &str = "Open in Comments";

/// The comment the pointer rests on, and the tab its words are in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Glance {
    pub id: u32,
    pub editor: usize,
}

/// A row of the card.
enum Row {
    /// Who made it, and its kind: quiet.
    Head(String),
    Words(String),
    /// A row of the last answer: on its first, its verdict's word where it gives one, in the
    /// verdict's colour, then who gave it in bold, as the Comments pane's card says it ("Agrees"
    /// on a chip, then "Claude"). This card said "Claude agrees:" where the pane said that.
    Answer(Option<(Option<String>, String)>, String),
    /// How many answers, and what a press does.
    Foot(String),
}

/// The card as it stands: its rows, whole, and where its rows' words stand in them.
pub(super) struct Card {
    pub area: Rect,
    rows: Vec<Row>,
    /// Painted, how far down its first row the first words stand.
    pad: f32,
    /// Painted, the room the rule over the foot has.
    rule: f32,
    /// Painted, how far in from the row next to the words the panel stands, and whether
    /// that row is its first (it's under them) or its last (over them).
    drop: f32,
    under: bool,
    fine: bool,
}

/// Rows of text no wider than `width`, the last ending in "…" when there was more.
fn cut(text: &str, width: f32, most: usize, measure: &dyn Measure) -> Vec<String> {
    let mut rows = threads::wrap(text.trim(), width, measure);
    rows.retain(|row| !row.trim().is_empty());
    if rows.len() <= most {
        return rows;
    }
    rows.truncate(most);
    if let Some(last) = rows.last_mut() {
        let font = Font::default();
        while !last.is_empty() && measure.width(&format!("{last}…"), font) > width {
            last.pop();
        }
        *last = format!("{}…", last.trim_end());
    }
    rows
}

impl App {
    /// The comment whose tinted words are under the pointer at `(x, y)`, and its tab: the
    /// shortest, where one's words are inside another's. Only with the pane shut and nothing
    /// over the file.
    fn commented_words_at(&self, x: u16, y: u16, fine_x: f32) -> Option<(u32, usize)> {
        if self.dock.open || !matches!(self.overlay, Overlay::None) || self.review.is_some() {
            return None;
        }
        let pane = self.pane_at(x, y)?;
        let index = *self.panes.get(pane)?;
        let editor = self.editors.get(index).filter(|e| e.rendered)?;
        let file = self.relative(editor.buffer.path()?);
        let area = self.pane_area(pane);
        let ranges: Vec<((usize, usize), u32)> = self
            .threads
            .iter()
            .filter(|t| t.file == file && !t.resolved && t.earlier.is_none())
            .filter_map(|t| Some((quote_range(editor, t)?, t.id)))
            .collect();
        if ranges.is_empty() {
            return None;
        }
        let at = editor.position_at(area, fine_x, y);
        ranges
            .into_iter()
            .filter(|((start, end), _)| at >= *start && at <= *end)
            .filter(|(range, _)| over_words(editor, area, *range, (fine_x, y)))
            .min_by_key(|((start, end), _)| end - start)
            .map(|(_, id)| (id, index))
    }

    /// The pointer moved. On the card it stays, so it can be pressed; on commented words it's
    /// theirs; anywhere else there's none.
    pub(super) fn follow_glance(&mut self, x: u16, y: u16, fine_x: f32) {
        if self
            .glance_card()
            .is_some_and(|card| card.area.contains(x, y))
        {
            return;
        }
        let found = self.commented_words_at(x, y, fine_x);
        self.glance = match (found, self.glance) {
            (Some((id, editor)), Some(glance)) if glance.id == id && glance.editor == editor => {
                Some(glance)
            }
            (Some((id, editor)), _) => Some(Glance { id, editor }),
            (None, _) => None,
        };
    }

    /// A press, the wheel or a drag while the card shows: a press on it opens the Comments
    /// on its comment, and anything else takes the card away. True when it took the press.
    pub(super) fn glance_mouse(&mut self, mouse: MouseEvent) -> bool {
        let card = self.glance_card();
        let id = self.glance.take().map(|glance| glance.id);
        let (Some(card), Some(id)) = (card, id) else {
            return false;
        };
        if !card.area.contains(mouse.x, mouse.y) {
            return false;
        }
        match mouse.kind {
            MouseKind::Left => {
                self.dock.open = true;
                self.dock.show_comments();
                self.dock.focused = false;
                // As a click on its card in the pane does: its words selected, its card lit,
                // the keyboard in the file.
                self.thread_hit(ThreadHit::Open(id));
                self.show_lit_comment(id);
                true
            }
            // The wheel moves the page under it.
            MouseKind::ScrollUp | MouseKind::ScrollDown => false,
            _ => true,
        }
    }

    /// Where the card stands and what it holds, while it's there to show: the pane shut, its
    /// comment still open and its words on screen.
    pub(super) fn glance_card(&self) -> Option<Card> {
        let glance = self.glance?;
        if self.dock.open || !matches!(self.overlay, Overlay::None) {
            return None;
        }
        let pane = self.panes.iter().position(|i| *i == glance.editor)?;
        let editor = self.editors.get(glance.editor).filter(|e| e.rendered)?;
        let file = self.relative(editor.buffer.path()?);
        let thread = self
            .threads
            .iter()
            .find(|t| t.id == glance.id && t.file == file && !t.resolved)?;
        let area = self.pane_area(pane);
        let (start, end) = quote_range(editor, thread)?;
        // The rows its words are on; where some are out of view, the edge they go past.
        let first = editor.cell_of(area, start).map_or(area.y, |(_, row)| row);
        let last = editor
            .cell_of(area, end.saturating_sub(1).max(start))
            .map_or(area.bottom().saturating_sub(1), |(_, row)| row);
        let measure = &*self.measure;
        let fine = !measure.in_cells();
        let room = f32::from(area.w.saturating_sub(2)) - INSET * 2.0;
        let width = WIDE.min(room);
        if width < NARROWEST {
            return None;
        }
        let rows = self.glance_rows(thread, width);
        let font = Font::default();
        let widest = rows
            .iter()
            .map(|row| match row {
                Row::Head(text) | Row::Words(text) | Row::Foot(text) => measure.width(text, font),
                Row::Answer(who, text) => {
                    let who = who.as_ref().map_or(0.0, |(verdict, who)| {
                        let bold = Font { bold: true, ..font };
                        let lead = match verdict {
                            Some(word) => format!("{word} {who}: "),
                            None => format!("{who}: "),
                        };
                        measure.width(&lead, bold)
                    });
                    who + measure.width(text, font)
                }
            })
            .fold(0.0, f32::max);
        // Its words' width and two columns either side, and no spare column: a spare one
        // stood at its right, twice the air at its left.
        let wide = ((widest.min(width).ceil() + INSET * 2.0) as u16).min(area.w);
        let padding = if fine { crate::menu::PADDING } else { 0.0 };
        // Painted, the panel stands clear of the words' tint by the air over the next row's
        // words, up to the 8 px every line keeps: that air is the next row's own, so the
        // panel covers none of its words.
        let drop = measure
            .cell_pixels()
            .filter(|_| fine)
            .map_or(0.0, |(_, row)| {
                let row = row as f32;
                let line = row / measure.row_air().max(1.0);
                let air = ((row - line) / 2.0).clamp(0.0, crate::measures::LINE_AIR);
                let unit = cellix_render::UNIT as f32;
                (air / row * unit).floor() / unit
            });
        let lines = rows.len() as f32 + if fine { RULE_ROOM } else { 0.0 };
        let tall = (lines + padding * 2.0 + drop).ceil() as u16;
        // Under the words where it fits, over them where that fits, else wherever has more.
        let under = area.bottom().saturating_sub(last + 1);
        let over = first.saturating_sub(area.y);
        let (top, tall, below) = if tall <= under {
            (last + 1, tall, true)
        } else if tall <= over {
            (first - tall, tall, false)
        } else if under >= over {
            (last + 1, under, true)
        } else {
            (area.y, over, false)
        };
        if tall < 2 {
            return None;
        }
        // Its words start where the words it's about do, on the row next to it.
        let next_to = if below { last } else { first };
        let starts = if first == next_to {
            editor.cell_of(area, start).map(|(x, _)| x)
        } else {
            let row_start = editor.position_at(area, f32::from(area.x), next_to);
            editor.cell_of(area, row_start).map(|(x, _)| x)
        };
        let starts = starts.unwrap_or(f32::from(area.x) + INSET);
        let left = ((starts - INSET).round().max(0.0) as u16)
            .min(area.right().saturating_sub(wide + 1))
            .max(area.x + 1);
        // A card cut short by the room keeps its foot, losing words over it.
        let room = f32::from(tall) - padding * 2.0 - drop - if fine { RULE_ROOM } else { 0.0 };
        let fits = (room.floor() as usize).max(1);
        let mut rows = rows;
        while rows.len() > fits && rows.len() > 1 {
            rows.remove(rows.len() - 2);
        }
        let spare = f32::from(tall) - drop - padding * 2.0 - rows.len() as f32;
        Some(Card {
            area: Rect::new(left, top, wide, tall),
            rows,
            pad: padding + if below { drop } else { 0.0 },
            rule: spare.max(0.0),
            drop,
            under: below,
            fine,
        })
    }

    /// What the card says about a comment, its words set to `width`.
    fn glance_rows(&self, thread: &Thread, width: f32) -> Vec<Row> {
        let measure = &*self.measure;
        let by = if thread.suggested_by.is_empty() {
            cellix_review::threads::YOU.to_string()
        } else {
            format!("Suggested by {}", thread.suggested_by)
        };
        let head = if thread.kind.is_empty() {
            by
        } else {
            format!("{by} · {}", thread.kind)
        };
        let mut rows = vec![Row::Head(head)];
        rows.extend(
            cut(&thread.text, width, COMMENT_ROWS, measure)
                .into_iter()
                .map(Row::Words),
        );
        if let Some(reply) = thread.replies.last() {
            let (verdict, who, words) = threads::reply_parts(reply);
            let lead = match &verdict {
                Some(word) => format!("{word} {who}:"),
                None => format!("{who}:"),
            };
            let said = format!("{lead} {}", words.replace('\n', " "));
            for (n, row) in cut(&said, width, ANSWER_ROWS, measure)
                .into_iter()
                .enumerate()
            {
                if n == 0 && row.starts_with(&lead) {
                    let rest = row[lead.len()..].trim_start().to_string();
                    rows.push(Row::Answer(Some((verdict.clone(), who.clone())), rest));
                } else {
                    rows.push(Row::Answer(None, row));
                }
            }
        }
        // The last answer is in view, so the foot counts the others: it said "1 reply" over
        // the one reply it showed.
        let others = match thread.replies.len() {
            0 => Some("No replies".to_string()),
            1 => None,
            2 => Some("1 more reply".to_string()),
            n => Some(format!("{} more replies", n - 1)),
        };
        rows.push(Row::Foot(match others {
            Some(others) => format!("{others} · {OPEN}"),
            None => OPEN.to_string(),
        }));
        rows
    }

    /// The card, over the page, under any menu or tip.
    pub(super) fn draw_glance(&self, grid: &mut Grid) {
        let Some(card) = self.glance_card() else {
            return;
        };
        let theme = &self.theme;
        let area = card.area;
        if card.fine {
            ground(grid, &card, theme);
        } else {
            crate::palette::raised(grid, area, theme);
        }
        let base = Style {
            fg: theme.text.fg,
            bg: theme.popup,
            ..theme.bar
        };
        let quiet = Style {
            fg: theme.popup_dim,
            ..base
        };
        let accent = Style {
            fg: theme.accent,
            ..base
        };
        let strong = Style { bold: true, ..base };
        let font = Font::default();
        let x = f32::from(area.x) + INSET;
        let right = area.right().saturating_sub(1);
        let place = |top| Place {
            top,
            right,
            fine: card.fine,
        };
        let mut top = f32::from(area.y) + if card.fine { card.pad } else { 0.0 };
        for row in &card.rows {
            match row {
                Row::Head(text) => {
                    words(grid, (x, 0), text, quiet, place(top));
                }
                Row::Words(text) => {
                    words(grid, (x, 0), text, base, place(top));
                }
                Row::Answer(who, text) => {
                    let (mut at, mut from) = (x, 0);
                    let bold = Font { bold: true, ..font };
                    // Each part in bold, and a space's room after it.
                    let mut part = |grid: &mut Grid, said: &str, style: Style| {
                        from = words(grid, (at, from), said, style, place(top)) + 1;
                        at = if card.fine {
                            at + self.measure.width(&format!("{said} "), bold)
                        } else {
                            f32::from(from)
                        };
                    };
                    if let Some((verdict, who)) = who {
                        if let Some(word) = verdict {
                            let tone = threads::verdict_tone(word, theme);
                            part(grid, word, Style { fg: tone, ..strong });
                        }
                        part(grid, &format!("{who}:"), strong);
                    }
                    words(grid, (at, from), text, base, place(top));
                }
                Row::Foot(text) => {
                    if card.fine {
                        // A rule over it, in the middle of its room.
                        top += card.rule;
                        let across = f32::from(area.w) - INSET * 2.0;
                        let rule = Bounds::cells(x, top - card.rule / 2.0, across, 0.0);
                        grid.hairline(rule, theme.popup_border);
                    }
                    // How many answers, quietly, then what a press does, in the accent.
                    let (count, open) = text.split_at(text.len() - OPEN.len());
                    let from = words(grid, (x, 0), count, quiet, place(top));
                    let after = if card.fine {
                        x + self.measure.width(count, font)
                    } else {
                        f32::from(from)
                    };
                    words(grid, (after, from), open, accent, place(top));
                }
            }
            top += 1.0;
        }
    }
}

/// The card's ground: its cells, but for the row next to the words, whose air the panel
/// stands down into and whose cells keep what they show; and its panel, raised as a menu's.
fn ground(grid: &mut Grid, card: &Card, theme: &Theme) {
    let area = card.area;
    let base = Style {
        fg: theme.text.fg,
        bg: theme.popup,
        ..theme.bar
    };
    let next_to = if card.under {
        area.y
    } else {
        area.bottom().saturating_sub(1)
    };
    for y in area.y..area.bottom() {
        if card.drop > 0.0 && y == next_to {
            continue;
        }
        // The corner cells keep what's behind them, under the round corners.
        let edge = y == area.y || y + 1 == area.bottom();
        let (x, w) = if edge {
            (area.x + 1, area.w.saturating_sub(2))
        } else {
            (area.x, area.w)
        };
        grid.fill(Rect::new(x, y, w, 1), base);
    }
    let top = f32::from(area.y) + if card.under { card.drop } else { 0.0 };
    let bounds = Bounds::cells(
        f32::from(area.x),
        top,
        f32::from(area.w),
        f32::from(area.h) - card.drop,
    );
    grid.popup(bounds, 28, theme.popup, theme.popup_border);
}

/// Where a row of the card's words goes: its top in rows, the column it stops before, and
/// whether it's painted.
#[derive(Clone, Copy)]
struct Place {
    top: f32,
    right: u16,
    fine: bool,
}

/// Words on the card: in cells, on the row holding most of them and no further left than
/// `from`, so what follows other words keeps their letters; and painted where they stand.
/// Gives the column after their cells.
fn words(grid: &mut Grid, (x, from): (f32, u16), text: &str, style: Style, place: Place) -> u16 {
    let row = (place.top + 0.499).floor() as u16;
    let shown = Style {
        painted: place.fine,
        ..style
    };
    let end = grid.put_str((x as u16).max(from), row, text, shown, place.right);
    if place.fine {
        let font = Font {
            bold: style.bold,
            ..Font::default()
        };
        let colour = (style.fg, false);
        grid.text_run((x, place.top), f32::from(place.right), text, colour, font);
    }
    end
}

/// Whether the pointer is on the words from `start` to `end` and not past them: on the rows
/// they're on, from where they start on the first and to where they end on the last.
fn over_words(
    editor: &Editor,
    area: Rect,
    (start, end): (usize, usize),
    (x, y): (f32, u16),
) -> bool {
    if end <= start {
        return false;
    }
    let first = editor.cell_of(area, start);
    let last_letter = editor.cell_of(area, end - 1);
    let first_row = first.map_or(0, |(_, row)| row);
    let last_row = last_letter.map_or(u16::MAX, |(_, row)| row);
    if y < first_row || y > last_row {
        return false;
    }
    if let Some((from, row)) = first
        && row == y
        && x < from - 0.5
    {
        return false;
    }
    // Where the last letter ends: where the place after it is on its row, or a column on.
    let ends = editor
        .cell_of(area, end)
        .filter(|(_, row)| Some(*row) == last_letter.map(|(_, r)| r))
        .map(|(x, _)| x)
        .or(last_letter.map(|(x, _)| x + 1.0));
    match (last_letter, ends) {
        (Some((_, row)), Some(ends)) if row == y => x <= ends + 0.5,
        _ => true,
    }
}
