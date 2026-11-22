//! Peek: a helper agent, which has no tab of its own, looked at over the side pane. A press on
//! its row in the Agents list, or on its name in a terminal, opens its live terminal on a card
//! a column in from the pane's sides, with a header saying who it is, what it's doing and
//! which agent started it, Keep as Tab, and ×. The keyboard goes to it while it's open; a press
//! outside it, or its ×, closes it, and what's behind it is dimmed meanwhile.
//!
//! A Ctrl+click on an agent's name in any terminal's text looks at it the same way. Ctrl, and
//! not Cmd on macOS: a terminal doesn't tell the program under it of Cmd with a press.

use super::team::Doing;
use super::*;

/// The header's button that gives the helper a tab.
const KEEP: &str = "Keep as Tab";
/// Columns from the card's edge to where the helper's mark is drawn in its header.
const PEEK_MARK: u16 = 2;
/// How far what's around the card is dimmed, out of 255.
const DIM: u8 = 110;
/// Pixels between the header's groups, where there are pictures: the name, what it's doing
/// (its mark and its word, 8 px apart, as one), and who started it.
const GROUP_GAP: f32 = 16.0;
const MARK_GAP: f32 = 8.0;
/// How far into its letter the "·" before who started it has its ink, in pixels.
const DOT_BEARING: f32 = 4.0;
/// How much of its box the spinner's ring and the still dot fill across: where their ink
/// is, which the gaps are measured from.
const RING_INK: f32 = 0.85;
const DOT_INK: f32 = 0.5;

/// `outer` with `inner` taken out of it, as up to four rectangles: over, under, left, right.
fn around(outer: Rect, inner: Rect) -> Vec<Rect> {
    let mut parts = Vec::new();
    if inner.y > outer.y {
        parts.push(Rect::new(outer.x, outer.y, outer.w, inner.y - outer.y));
    }
    if outer.bottom() > inner.bottom() {
        let h = outer.bottom() - inner.bottom();
        parts.push(Rect::new(outer.x, inner.bottom(), outer.w, h));
    }
    if inner.x > outer.x {
        parts.push(Rect::new(outer.x, inner.y, inner.x - outer.x, inner.h));
    }
    if outer.right() > inner.right() {
        let w = outer.right() - inner.right();
        parts.push(Rect::new(inner.right(), inner.y, w, inner.h));
    }
    parts
}

impl App {
    /// The helper peek shows, by the name it was given, and its tab.
    pub(super) fn peeked(&self) -> Option<usize> {
        let name = self.peek.as_deref()?;
        self.member_index(name)
    }

    /// Opens peek on the agent called `name`.
    pub(super) fn open_peek(&mut self, name: &str) {
        if self.member_index(name).is_none() {
            return;
        }
        self.dock.open = true;
        // Over the tab of the agent that started it, or the nearest one up that has a tab, as
        // what it works for.
        let mut up = self.member_index(name);
        while let Some(index) = up.filter(|i| !self.dock.sessions[*i].tabbed) {
            let parent = self.dock.sessions[index].member.as_ref();
            up = parent
                .and_then(|m| m.parent.as_deref())
                .and_then(|p| self.member_index(p));
        }
        if let Some(index) = up {
            self.dock.active = Some(index);
            self.dock.front = Front::Session;
        }
        self.peek = Some(name.to_string());
        self.fit_peek();
    }

    /// A Ctrl+click on `word` in the terminal of session `from`: when it's the name of a live
    /// agent other than that one, a helper opens in peek and one with a tab of its own comes
    /// to the front, as their rows in the Agents list do. Returns whether it was one.
    pub(super) fn look_at_name(&mut self, word: &str, from: usize) -> bool {
        let Some(index) = self.member_index(word) else {
            return false;
        };
        let session = &self.dock.sessions[index];
        if index == from || session.exited {
            return false;
        }
        if session.tabbed {
            self.peek = None;
            self.dock.open = true;
            self.dock.active = Some(index);
            self.dock.front = Front::Session;
            self.dock.focused = true;
            self.dock.agents_tab = false;
            self.see_front_tab();
        } else {
            self.open_peek(word);
        }
        true
    }

    pub(super) fn close_peek(&mut self) {
        self.peek = None;
    }

    /// The card: a cell in from each side of the pane, a column at its sides and a row over
    /// and under, between the row of its tabs and its foot. Where there are pictures its
    /// panel starts `PANE_CARD_LEAD` into its first column, a column from the sheet's edge as
    /// at its right.
    fn peek_card(&self) -> Option<Rect> {
        self.peek.as_ref()?;
        let pane = self.dock_area()?;
        let height = pane.h.checked_sub(3)?;
        let width = pane.w.checked_sub(2)?;
        (height >= 5 && width >= 20).then(|| Rect::new(pane.x + 1, pane.y + 2, width, height))
    }

    /// Where its terminal stands on the card: under the header's row and its line, a column
    /// in from each side of its panel, and so a column and a half from its left where the
    /// panel starts half way into its first.
    fn peek_screen(&self) -> Option<Rect> {
        let card = self.peek_card()?;
        let lead = u16::from(self.pane_card_lead() > 0.0);
        Some(Rect::new(
            card.x + 1 + lead,
            card.y + 1,
            card.w.saturating_sub(2 + lead),
            card.h.saturating_sub(1),
        ))
    }

    /// How far into the row under its cells the card's panel reaches, in rows, so its foot is
    /// as far from the pane's as its right side is from the pane's: a column, less the line
    /// down the pane's side, from that line. Nothing on cells.
    fn peek_reach(&self) -> f32 {
        let Some((wide, row)) = self.measure.cell_pixels() else {
            return 0.0;
        };
        let row = row as f32;
        ((row - wide as f32) / row).clamp(0.0, 1.0)
    }

    /// Sizes the helper's terminal to the card, as it changes.
    pub(super) fn fit_peek(&mut self) {
        let (Some(index), Some(screen)) = (self.peeked(), self.peek_screen()) else {
            return;
        };
        let cell = self.cell_size();
        self.dock.sessions[index].resize((screen.w, screen.h), cell);
    }

    /// The header's Keep as Tab and ×, as (cells, x) and the ×'s column.
    fn peek_buttons(&self, card: Rect) -> ((String, u16), u16) {
        let cells = measures::button_cells(KEEP, &*self.measure);
        let close = self.peek_close(card).floor() as u16;
        let keep = close.saturating_sub(cells.chars().count() as u16 + BUTTON_GAP);
        ((cells, keep), close)
    }

    /// Where the header's × is drawn: as far from the card's right edge, to its ink, as the
    /// helper's mark is from its left. Four columns in, it stood 46 px from the edge against
    /// the mark's 24 (0.11.0's release check, 173-peek). On cells, four columns in.
    fn peek_close(&self, card: Rect) -> f32 {
        let measure = &*self.measure;
        if measure.in_cells() {
            return f32::from(card.right().saturating_sub(4));
        }
        let side = cellix_render::GLYPH * measures::rows_to_columns(measure) / measure.row_air();
        // The mark's box starts `GLYPH_LEAD` before its place, and its ink about 2 of its 24
        // in; the ×'s ink ends 5.6 of its 24 before its box does.
        let lead = cellix_render::GLYPH_LEAD;
        let air = f32::from(PEEK_MARK) - lead + side * 2.2 / 24.0 - self.pane_card_lead();
        f32::from(card.right()) - air - side * (1.0 - 5.6 / 24.0) + lead
    }

    /// The header, the card's first row, its words in the middle of it between the card's edge
    /// and the line under it: the helper's mark, its name in bold, what it's doing as a mark
    /// and a word in that state's colour, and who started it, quieter. Laid out by what each
    /// measures, 8 px apart, so nothing is cut while there's room for it.
    fn draw_peek_header(&self, grid: &mut Grid, (index, card): (usize, Rect), base: Style) {
        let theme = &self.theme;
        let measure = &*self.measure;
        let session = &self.dock.sessions[index];
        let pictures = !measure.in_cells();
        let y = card.y;
        let ((_, keep_x), _) = self.peek_buttons(card);
        let stop = keep_x.saturating_sub(BUTTON_GAP);
        // Pixels as columns, and the height of the terminal's line a mark is sized by; on
        // cells, a column a gap.
        let (cell_wide, cell_tall) = measure.cell_pixels().unwrap_or((1, 1));
        let px = |pixels: f32| {
            if pictures {
                pixels / cell_wide.max(1) as f32
            } else {
                1.0
            }
        };
        let line = cell_tall as f32 / measure.row_air().max(1.0);
        let gap = px(GROUP_GAP);
        // Where the next part starts: painted, and in the cells that hold its words, which
        // keep to whole letters so the screen's text still reads.
        let (mut at, mut cell) = (f32::from(card.x + 4), card.x + 4);
        let next = |wide: f32, letters: usize, at: &mut f32, cell: &mut u16| {
            *at += wide + gap;
            *cell = (*cell + letters as u16 + 1).max(at.floor() as u16);
        };
        let icon = session.icon();
        let colour = crate::dock::icon_colour(icon, theme);
        let fallback = crate::toolbar::fallback(icon);
        grid.glyph(
            f32::from(card.x + PEEK_MARK),
            y,
            icon,
            colour,
            fallback,
            base,
        );
        let name = session.called().unwrap_or_default();
        let strong = Style { bold: true, ..base };
        grid.label_at(at, cell, y, name, strong, stop, 100);
        let bold = Font {
            bold: true,
            ..Font::default()
        };
        let wide = measure.width(name, bold);
        next(wide, name.chars().count(), &mut at, &mut cell);
        // What it's doing: the spinner while it works, a dot when it asks, has finished a
        // turn or stopped on an error, each with its word in the same colour, as its row in
        // the Agents list has it; ready and waiting, its word alone, quieter. The gaps are
        // from the mark's ink, which stands inside its box.
        let doing = self.doing(index);
        let colour = match &doing {
            doing if doing.working() => Some(theme.accent),
            Doing::Asking(_) => Some(theme.attention),
            // Done is done, a dot in the done colour, whatever it said: it was grey words
            // with no mark beside the working one's spinner.
            Doing::Done(_) => Some(theme.git_added),
            Doing::Failed(_) => Some(theme.error),
            _ => None,
        };
        let size = crate::dock::MARK_SIZE;
        let ink = if doing.working() { RING_INK } else { DOT_INK };
        let (box_wide, inset) = if pictures {
            let across = px(size * line);
            (across, across * (1.0 - ink) / 2.0)
        } else {
            (1.0, 0.0)
        };
        if let Some(colour) = colour.filter(|_| at + box_wide < f32::from(stop)) {
            let ground = Style {
                bg: theme.popup,
                ..theme.sidebar
            };
            let start = at - inset;
            if doing.working() {
                grid.spinner(start, y, size, colour, ground);
            } else {
                grid.dot(start, y, size, colour, ground);
            }
            at = start + box_wide - inset + px(MARK_GAP);
            cell = (cell + 2).max(at.floor() as u16);
        }
        let word = doing.word();
        let state = Style {
            fg: colour.unwrap_or(theme.popup_dim),
            ..base
        };
        grid.label_at(at, cell, y, word, state, stop, 100);
        let wide = measure.width(word, Font::default());
        let word_end = at + wide;
        next(wide, word.chars().count(), &mut at, &mut cell);
        let parent = session.member.as_ref().and_then(|m| m.parent.as_deref());
        if let Some(parent) = parent {
            let from = self
                .member_index(parent)
                .map_or_else(|| parent.to_string(), |i| self.dock.sessions[i].label());
            // Whole or not at all: "· from Cl…" said nothing in a narrow pane.
            let from = format!("· from {from}");
            let quiet = Style {
                fg: theme.popup_dim,
                ..base
            };
            // As much air before the "·" as after it, which the space after it has: at the
            // group's gap it stood 20 px from what it's doing and 8 from "from".
            let at = if pictures {
                word_end + px(MARK_GAP - DOT_BEARING)
            } else {
                at
            };
            if at + measure.width(&from, Font::default()) <= f32::from(stop) {
                grid.label_at(at, cell, y, &from, quiet, stop, 100);
            }
        }
    }

    pub(super) fn draw_peek(&self, grid: &mut Grid) {
        let (Some(index), Some(card), Some(screen), Some(pane)) = (
            self.peeked(),
            self.peek_card(),
            self.peek_screen(),
            self.dock_area(),
        ) else {
            return;
        };
        let theme = &self.theme;
        let session = &self.dock.sessions[index];
        // What's around it is dimmed, so the card is what's looked at. Not under it: paint
        // can't go under text, so the card is cells, its terminal's too, and only its corners
        // and its line are painted.
        let behind = Rect::new(pane.x, pane.y + 1, pane.w, pane.h.saturating_sub(1));
        for part in around(behind, card) {
            grid.wash(
                Bounds::inset(part, 0.0, 0.0),
                0,
                (theme.sidebar.bg, DIM),
                None,
            );
        }
        let ground = Style {
            fg: theme.text.fg,
            bg: theme.popup,
            ..theme.bar
        };
        grid.clear_paint_in(card);
        let lead = self.pane_card_lead();
        grid.fill(card, ground);
        if lead > 0.0 {
            // Its first column's left half is the pane's sheet: what stood in that column,
            // as a terminal's cursor, is under the card.
            let column = Rect::new(card.x, card.y, 1, card.h);
            let sheet = Style {
                bg: theme.sidebar.bg,
                ..ground
            };
            grid.fill(column, sheet);
        }
        let base = ground;
        self.draw_peek_header(grid, (index, card), base);
        let ((keep, keep_x), close) = self.peek_buttons(card);
        let y = card.y;
        let hovered = self.over_button((keep_x, y), keep.chars().count() as u16);
        // A quiet button, as every one that isn't the answer is, at its height on the
        // terminal's line (`Grid::row_button`).
        let fill = theme.secondary(theme.popup, hovered);
        // Its cells keep the header's colour: the button is shorter than its row.
        let from = grid.painted_so_far();
        grid.row_button((keep_x, y), &keep, KEEP, fill, base);
        // It goes down onto the name's baseline only as far as keeps `LINE_AIR` from the line
        // under the header, which is its row's foot. A taller header would cost the pane a row.
        grid.lift_since(from, measures::row_button_foot_lift(&*self.measure));
        let shut = Style {
            fg: if self.hover == Some((close, y)) || self.hover == Some((close + 1, y)) {
                theme.text.fg
            } else {
                theme.popup_dim
            },
            ..base
        };
        grid.glyph(self.peek_close(card), y, Icon::Close, shut.fg, "×", shut);
        // Its cursor shows while the keys are its, as a tab's does.
        session.draw_on(
            grid,
            screen,
            theme,
            (self.has_keys(Some(index)), theme.popup),
        );
        // The line under the header, the card's width, and the card's own line round it with
        // its corners, painted over the cells' square ones in what stands behind.
        let (x, w) = (f32::from(card.x) + lead, f32::from(card.w) - lead);
        // Under its last row it reaches down the pane's, to stand as far from the pane's foot
        // as from its sides: a row short of it, it had 50 px under it against 17 beside it.
        let reach = self.peek_reach();
        let tall = f32::from(card.h) + reach;
        if reach > 0.0 {
            let under = Bounds::cells(x, f32::from(card.bottom()), w, reach);
            grid.exact_fill(under, theme.popup);
        }
        if lead > 0.0 {
            // The part of its first column the panel covers.
            let half = Bounds::cells(x, f32::from(card.y), 1.0 - lead, tall);
            grid.panel(half, 0, Some(theme.popup), None);
        }
        let line = Bounds::cells(x, f32::from(card.y) + 1.0, w, 0.0);
        grid.hairline(line, theme.popup_border);
        let edge = Bounds::cells(x, f32::from(card.y), w, tall);
        grid.corners(edge, CARD_RADIUS, theme.sidebar.bg, ALL_CORNERS);
        grid.panel(edge, CARD_RADIUS, None, Some(theme.popup_border));
    }

    /// A press while peek is open: on its header's buttons, into its terminal, or outside it,
    /// which closes it. Returns whether peek took it.
    pub(super) fn peek_mouse(&mut self, mouse: MouseEvent) -> bool {
        let (Some(index), Some(card), Some(screen)) =
            (self.peeked(), self.peek_card(), self.peek_screen())
        else {
            return false;
        };
        let inside = card.contains(mouse.x, mouse.y);
        if !inside {
            if mouse.kind == MouseKind::Left {
                self.close_peek();
                return true;
            }
            return false;
        }
        let y = card.y;
        let ((keep, keep_x), close) = self.peek_buttons(card);
        if mouse.kind == MouseKind::Left && mouse.y == y {
            if mouse.x >= close && mouse.x < close + 2 {
                self.close_peek();
            } else if mouse.x >= keep_x && mouse.x < keep_x + keep.chars().count() as u16 {
                self.keep_peek_as_tab();
            }
            return true;
        }
        if screen.contains(mouse.x, mouse.y) {
            let spot = (mouse.x - screen.x, mouse.y - screen.y);
            // A right click on a link has its menu, over the card.
            if mouse.kind == MouseKind::Right
                && self.session_link_menu(index, spot, (mouse.x, mouse.y))
            {
                return true;
            }
            // Ctrl+click on a web address opens it, on another agent's name looks at it, and
            // on a `file:line` opens that.
            if mouse.kind == MouseKind::Left && mouse.mods.ctrl {
                if let Some(address) = self.dock.sessions[index].link_at(spot) {
                    self.peek = None;
                    self.open_page(address);
                    return true;
                }
                let word = self.dock.sessions[index].word_at(spot);
                if word.is_some_and(|word| self.look_at_name(&word, index)) {
                    return true;
                }
                if let Some((file, line)) = self.dock.sessions[index].path_at(spot) {
                    self.peek = None;
                    self.go_to_place(&file, line);
                }
                return true;
            }
            let session = &mut self.dock.sessions[index];
            match mouse.kind {
                MouseKind::ScrollUp => session.wheel(true, spot),
                MouseKind::ScrollDown => session.wheel(false, spot),
                kind => {
                    if let Some(text) = session.mouse(kind, spot, mouse.mods.shift) {
                        self.copy_text(text, "Copied");
                    }
                }
            }
        }
        true
    }

    /// Keys go to the helper while peek is open. Returns whether it took the key.
    pub(super) fn peek_key(&mut self, key: KeyEvent) -> bool {
        let Some(index) = self.peeked() else {
            return false;
        };
        self.dock.sessions[index].key(key);
        true
    }

    /// What's pasted goes to the helper while peek is open. Returns whether it took it.
    pub(super) fn peek_paste(&mut self, text: &str) -> bool {
        let Some(index) = self.peeked() else {
            return false;
        };
        self.dock.sessions[index].paste(text);
        true
    }

    /// The helper gets a tab of its own, in front, and peek closes.
    fn keep_peek_as_tab(&mut self) {
        let Some(index) = self.peeked() else {
            return;
        };
        self.dock.sessions[index].tabbed = true;
        self.peek = None;
        self.dock.active = Some(index);
        self.dock.front = Front::Session;
        self.dock.focused = true;
        self.fit_dock();
    }

    /// Peek closes when its helper is gone or the pane is.
    pub(super) fn poll_peek(&mut self) -> bool {
        if self.peek.is_some() && (self.peeked().is_none() || self.peek_card().is_none()) {
            self.peek = None;
            return true;
        }
        false
    }
}
