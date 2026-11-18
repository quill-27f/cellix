//! The measures every surface is drawn with, from the end-to-end design (`docs/DESIGN.md`,
//! "One system"). A surface that needs one of these takes it from here, so no two of them
//! can drift apart again.

use cellix_render::{Font, Measure};

/// Columns between buttons side by side, and between a button and the words beside it.
pub const BUTTON_GAP: u16 = 2;
/// The least air between a line, or a sheet's edge, and anything drawn beside it, in the
/// screen's pixels: 8 px of a Retina screen, 4 pt. A control, a floating bar, a tooltip or a
/// menu that opens under a tab row stands at least this far from its line, and a band
/// behind a row stands this far in from both edges of its sheet. See DESIGN.md, Look.
pub const LINE_AIR: f32 = 8.0;
/// Pixels between a dialog's last line, or control, and its row of buttons.
pub const DIALOG_GAP: f32 = 24.0;
/// How far under the top of a line of words their capitals start, and their tails end, in
/// lines: 9 px and 31 of a line of 36, as the interface's font is set.
const CAPS: f32 = 0.25;
const TAILS: f32 = 0.86;

/// What a dialog's last row before its buttons holds: words, whose tails reach down it, or a
/// control a button tall.
#[derive(Clone, Copy)]
pub enum DialogLast {
    Words,
    Control,
}

/// How far a dialog's parts are raised in their rows where a row has room, in rows: its title
/// and what's under it by the first, its buttons by the second. A dialog is whole rows, its
/// title on its second and its buttons on the row after its last control, with none under
/// them; raised so, there's as much air over its title as under its buttons, about what its
/// sides have, and `DIALOG_GAP` over its buttons. Not raised, an empty row over the title and
/// one under the buttons left 65 px and 54, and the buttons stood 17 px under the words (10
/// under New Worktree's choice of agent). Nothing moves on the terminal's line, which has no
/// air to share.
pub fn dialog_lifts(last: DialogLast, measure: &dyn Measure) -> (f32, f32) {
    let Some((_, row)) = measure.cell_pixels().filter(|_| roomy(measure)) else {
        return (0.0, 0.0);
    };
    let row = row as f32;
    let line = row / measure.row_air().max(1.0);
    let lead = (row - line) / 2.0;
    let caps = lead + CAPS * line;
    let (top, foot) = cellix_render::button_in_row(row, line);
    let last = match last {
        DialogLast::Words => lead + TAILS * line,
        DialogLast::Control => foot,
    };
    // Over the title, a row and its capitals' place less the lift; under the buttons, what
    // their row leaves under their foot and the lift they're given more than that.
    let words = (caps + foot - row - top + last + DIALOG_GAP) / 2.0;
    let buttons = words + row + top - last - DIALOG_GAP;
    (words / row, buttons / row)
}

/// How thick a line is on screen, in pixels: what the painter draws a hairline at, a pixel
/// for every 24 of the terminal's line or part of one, so two on a Retina screen. What stands
/// `LINE_AIR` from a line counts from its far side: counted as one pixel, a menu hung under a
/// tab row stood 7 px from it.
pub fn line_pixels(measure: &dyn Measure) -> f32 {
    let Some((_, row)) = measure.cell_pixels() else {
        return 1.0;
    };
    let line = row as f32 / measure.row_air().max(1.0);
    (line / 24.0).ceil().max(1.0)
}
/// How tall a spinner beside words is, in the terminal's lines: in the status bar, a view's
/// heading or a button, 11 pt beside words of 13 or a little less.
pub const WAIT_SPIN: f32 = 0.6;

/// The chip behind a glyph that takes a click, an emoji or a small icon, while it's pointed
/// at or picked: square on screen, nearly a row tall, with the glyph in its middle.
pub fn glyph_chip(middle: f32, y: u16, measure: &dyn Measure) -> cellix_render::Bounds {
    const TALL: f32 = 0.92;
    let wide = TALL * rows_to_columns(measure);
    let top = f32::from(y) + (1.0 - TALL) / 2.0;
    cellix_render::Bounds::cells(middle - wide / 2.0, top, wide, TALL)
}

/// Where an emoji starts so that it stands in the middle of `middle`, at `size` percent of
/// the words' size. Its measure holds the air kept after it in a line of words, which isn't
/// part of what's seen.
pub fn emoji_start(emoji: &str, middle: f32, size: u8, measure: &dyn Measure) -> f32 {
    let font = Font {
        size,
        ..Font::default()
    };
    let air = if measure.in_cells() {
        0.0
    } else {
        let text = cellix_render::INTERFACE_SIZE * f32::from(size) / 100.0;
        cellix_render::EMOJI_AIR * text * rows_to_columns(measure)
    };
    let seen = (measure.width(emoji, font) - air).clamp(0.0, 2.4);
    middle - seen / 2.0
}

/// How many columns are as long as a row is tall: about two, and exactly what the cell's
/// pixels say where there are pixels.
pub fn rows_to_columns(measure: &dyn Measure) -> f32 {
    let (wide, tall) = measure.cell_pixels().unwrap_or((1, 2));
    tall as f32 / wide.max(1) as f32
}
/// How far the words typed in a text box are raised in it, in rows: 2 px. Words stand with
/// their capitals in the middle of their row, so their tails reach 4 px further down than
/// their heads reach up, and in a box nine tenths of a row that put a "g" on its lower edge.
/// Raised, a tail is as far from the box's foot as a tall letter is from its top.
pub const FIELD_LIFT: f32 = 0.055;
/// A row this many times the terminal's line, or taller, has a box with room under a tail,
/// and words stand in its middle as they do in the design: Airy rows. A Comfortable row's box
/// is a pixel taller than the line's, and its words stay raised.
const ROOMY: f32 = 1.3;

/// How far a text box's words are raised in it on rows of this measure: `FIELD_LIFT` where
/// the box is no taller than a tail needs, and nothing where it is.
pub fn field_lift(measure: &dyn Measure) -> f32 {
    if roomy(measure) { 0.0 } else { FIELD_LIFT }
}

/// Whether a row has room for a control to stand in it with its words in its middle.
pub fn roomy(measure: &dyn Measure) -> bool {
    measure.row_air() >= ROOMY
}
/// The band behind a pointed or picked row of a list, `row` from its first column to its
/// last and `inset` rows short of its top and foot: `LINE_AIR` in from the line round the
/// sheet it's on, on both sides. `edge` is how far into the row's first column the sheet
/// starts: half the frame's column for the side pane beside the file, where a band 0.4 of a
/// column in from the row stood 15 px from the sheet's line on its left and 6 on its right.
pub fn band(
    row: cellix_render::Rect,
    edge: f32,
    inset: f32,
    measure: &dyn Measure,
) -> cellix_render::Bounds {
    // The sheet's line, as thick as it's painted, and the air from it: counted as a pixel,
    // a band stood 7 px from a line two pixels thick.
    let line = line_pixels(measure);
    let air = measure
        .cell_pixels()
        .map_or(0.4, |(wide, _)| (LINE_AIR + line) / wide.max(1) as f32);
    let left = f32::from(row.x) + edge + air;
    let right = f32::from(row.right()) - air;
    let top = f32::from(row.y) + inset;
    cellix_render::Bounds::cells(left, top, right - left, f32::from(row.h) - inset * 2.0)
}

/// How tall the band behind a picked or pointed row of a list is, in rows: 33 px of 36,
/// which holds an icon of 16 pt and a letter's tail. A menu's and a palette's are this.
pub const BAND_HEIGHT: f32 = 0.92;

/// Columns of air either side of a button's words inside its chip, where text is drawn in
/// pixels: a cell and a half, about 13 px.
pub const BUTTON_ROOM: f32 = 1.5;
/// The size a button's words are set at, of the interface font's.
pub const BUTTON_TEXT: u8 = 90;

/// The cells a button takes. Where every letter is a cell that's its words between
/// brackets, which say it's a button where no chip can be drawn. In pixels it's as many
/// cells as its words measure with `BUTTON_ROOM` either side, whatever its letters. They're
/// hidden under the paint there, and hold as much of the words as they have room for, so
/// the screen's text still says what the button is.
pub fn button_cells(label: &str, measure: &dyn Measure) -> String {
    if measure.in_cells() {
        return format!("[{label}]");
    }
    let width = button_width(label, measure) as usize;
    let held: String = label.chars().take(width).collect();
    format!("{held:^width$}")
}

/// How many columns a button's chip takes where text is drawn in pixels.
pub fn button_width(label: &str, measure: &dyn Measure) -> u16 {
    let font = Font {
        size: BUTTON_TEXT,
        ..Font::default()
    };
    let width = (measure.width(label, font) + BUTTON_ROOM * 2.0).ceil() as u16;
    width.max(3)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cellix_render::{CellMeasure, FontMeasure};

    #[test]
    fn a_button_keeps_the_same_room_whatever_its_words() {
        let measure = FontMeasure::new(17, 37);
        let font = Font {
            size: BUTTON_TEXT,
            ..Font::default()
        };
        for label in ["OK", "Reply", "Review All Changes"] {
            let cells = button_cells(label, &measure);
            let air = (cells.chars().count() as f32 - measure.width(label, font)) / 2.0;
            assert!(
                (BUTTON_ROOM..BUTTON_ROOM + 0.5).contains(&air),
                "{label}: {air}"
            );
        }
        assert_eq!(button_cells("Reply", &CellMeasure), "[Reply]");
    }

    /// A text box's words are raised for their tails where a row is the terminal's line, or
    /// only a little taller. On an Airy row the box has the room, and they stand in its
    /// middle.
    #[test]
    fn words_are_raised_only_where_a_box_is_too_short_for_their_tails() {
        let line = FontMeasure::new(17, 36);
        let comfortable = FontMeasure::new(17, 43).with_type_row(36);
        let airy = FontMeasure::new(17, 50).with_type_row(36);
        assert_eq!(field_lift(&line), FIELD_LIFT);
        assert_eq!(field_lift(&comfortable), FIELD_LIFT);
        assert_eq!(field_lift(&airy), 0.0);
        assert_eq!(field_lift(&CellMeasure), FIELD_LIFT);
        assert!(roomy(&airy) && !roomy(&comfortable) && !roomy(&line));
    }
}
