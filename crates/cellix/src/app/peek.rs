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
/// How far what's around the card is dimmed, out of 255.
const DIM: u8 = 110;

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
    fn peeked(&self) -> Option<usize> {
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
    /// and under, between the row of its tabs and its foot.
    fn peek_card(&self) -> Option<Rect> {
        self.peek.as_ref()?;
        let pane = self.dock_area()?;
        let height = pane.h.checked_sub(3)?;
        let width = pane.w.checked_sub(2)?;
        (height >= 5 && width >= 20).then(|| Rect::new(pane.x + 1, pane.y + 2, width, height))
    }

    /// Where its terminal stands on the card: under the header's row and its line, a column
    /// in from each side.
    fn peek_screen(&self) -> Option<Rect> {
        let card = self.peek_card()?;
        Some(Rect::new(
            card.x + 1,
            card.y + 1,
            card.w.saturating_sub(2),
            card.h.saturating_sub(1),
        ))
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
        let close = card.right().saturating_sub(4);
        let keep = close.saturating_sub(cells.chars().count() as u16 + BUTTON_GAP);
        ((cells, keep), close)
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
        grid.fill(card, ground);
        let base = Style {
            fg: theme.text.fg,
            bg: theme.popup,
            ..theme.bar
        };
        let quiet = Style {
            fg: theme.popup_dim,
            ..base
        };
        // The header is the card's first row, its words in the middle of it between the card's
        // edge and the line under it.
        let y = card.y;
        let ((keep, keep_x), close) = self.peek_buttons(card);
        let stop = keep_x.saturating_sub(2);
        // Its mark, its name in bold, what it's doing, and who started it.
        let icon = session.icon();
        let colour = crate::dock::icon_colour(icon, theme);
        let fallback = crate::toolbar::fallback(icon);
        grid.glyph(f32::from(card.x + 2), y, icon, colour, fallback, base);
        let name = session.called().unwrap_or_default();
        let strong = Style { bold: true, ..base };
        let mut x = grid.label(card.x + 4, y, name, strong, stop) + 1;
        let doing = self.doing(index);
        if doing.working() && x + 2 < stop {
            let spin = Style {
                bg: theme.popup,
                ..theme.sidebar
            };
            grid.spinner(f32::from(x), y, crate::dock::MARK_SIZE, theme.accent, spin);
            x += 2;
        }
        let state = match &doing {
            Doing::Asking(_) => Style {
                fg: theme.attention,
                ..base
            },
            _ => base,
        };
        x = grid.label(x, y, doing.word(), state, stop) + 1;
        let parent = session.member.as_ref().and_then(|m| m.parent.as_deref());
        if let Some(parent) = parent {
            let from = self
                .member_index(parent)
                .map_or_else(|| parent.to_string(), |i| self.dock.sessions[i].label());
            // Cut short with an ellipsis where the header is narrow.
            let room = f32::from(stop.saturating_sub(x));
            let from = crate::context::elide(&format!("· from {from}"), room, &*self.measure);
            grid.label(x, y, &from, quiet, stop);
        }
        let hovered = self.over_button((keep_x, y), keep.chars().count() as u16);
        let fill = if hovered {
            theme.quiet_button(theme.quiet_button(theme.popup))
        } else {
            theme.quiet_button(theme.popup)
        };
        // In cells a button like any other. Where there are pictures, its words in the accent
        // and no chip: the header is one row over the line under it, and a chip there would
        // stand within 8 px of that line, so the words are the button, lit under the pointer.
        let wide = keep.chars().count() as u16;
        if self.measure.in_cells() {
            let face = Style { bg: fill, ..base };
            grid.button(keep_x, y, &keep, KEEP, fill, face);
        } else {
            grid.fill(Rect::new(keep_x, y, wide, 1), base);
            let face = Style {
                fg: if hovered { theme.text.fg } else { theme.accent },
                ..base
            };
            let words = f32::from(keep_x) + measures::BUTTON_ROOM;
            let size = measures::BUTTON_TEXT;
            grid.label_at(words, keep_x, y, KEEP, face, keep_x + wide, size);
        }
        let shut = Style {
            fg: if self.hover == Some((close, y)) || self.hover == Some((close + 1, y)) {
                theme.text.fg
            } else {
                theme.popup_dim
            },
            ..base
        };
        grid.glyph(f32::from(close), y, Icon::Close, shut.fg, "×", shut);
        session.draw_on(grid, screen, theme, (true, theme.popup));
        // The line under the header, the card's width, and the card's own line round it with
        // its corners, painted over the cells' square ones in what stands behind.
        let line = Bounds::cells(
            f32::from(card.x),
            f32::from(card.y) + 1.0,
            f32::from(card.w),
            0.0,
        );
        grid.hairline(line, theme.popup_border);
        let (x, w) = (f32::from(card.x), f32::from(card.w));
        let edge = Bounds::cells(x, f32::from(card.y), w, f32::from(card.h));
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
            // Ctrl+click on another agent's name looks at it, and on a `file:line` opens it.
            if mouse.kind == MouseKind::Left && mouse.mods.ctrl {
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
                    if let Some(text) = session.mouse(kind, spot) {
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
