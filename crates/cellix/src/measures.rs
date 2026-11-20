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
/// Pixels between the buttons by a selection and the selection they're for: their foot this
/// far over its first line, or their top this far under its last where they stand under it.
/// 0.11.0's release check measured them 5 to 45 px from it, scene by scene (P1).
pub const SELECTION_BAR_AIR: f32 = 12.0;
/// Pixels between a dialog's last line, or control, and its row of buttons.
pub const DIALOG_GAP: f32 = 24.0;

/// One anatomy for every tab, a file's and the side pane's, in cells of 17 px, measured to
/// the ink (0.11.0's final review, P12): from its edge to its mark's box, 15 px, so the mark's
/// ink is about 17 px in; to its name, the mark's 25 px box and 8 px after its ink;
pub const TAB_MARK: f32 = 0.9;
pub const TAB_NAME: f32 = 2.85;
/// from its name to where its × is placed, so the ×'s ink is 12 px after the name, whose box
/// starts 2.5 px before that and whose ink is 7 px into the box;
pub const TAB_CLOSE_AIR: f32 = 0.43;
/// and from where the × is placed to the tab's end: its ink, 21 px past that, and as much
/// air after it as before the mark. What a tab's whole columns hold past this is shared in
/// thirds, before the mark, before the × and after it, whatever the tab shows, so each is
/// about 17 to 21 px. A file's tab was 21 | icon | name | 51 to its × | 13, and the space
/// after a pane tab's × ran from 9 px on a working tab to 25 on an idle one.
pub const TAB_CLOSE_END: f32 = 2.26;

/// How far a line of words' measured width runs past its last letter's ink, in pixels, at the
/// interface's size: about 3 px, the letter's own air after it ("README.md" 3.7, "pricing page"
/// 3). What stands in the middle of the air after a name, as the line between two tabs, counts
/// from its ink: from its measure, the line stood 37 px after README.md and 32 before PLAN.md.
pub const WORDS_RUN_ON: f32 = 3.0;

/// `WORDS_RUN_ON` in columns, where there are pixels.
pub fn words_run_on(measure: &dyn Measure) -> f32 {
    columns(WORDS_RUN_ON, measure).unwrap_or(0.0)
}

/// How wide a tab with a mark paints, in cells, for a name `name` cells wide and `more` cells
/// of what follows it before the × (an agent's state, its context).
pub fn tab_painted(name: f32, more: f32) -> f32 {
    TAB_NAME + name + more + TAB_CLOSE_AIR + TAB_CLOSE_END
}
/// Pixels between a card's edge and what it holds, on every side: its words start this far in
/// from its left edge, what stands at its right end ends this far from its right, its first
/// words' capitals stand this far under its top and its last buttons this far over its foot.
/// Every comment card keeps it, and the composer's box stands this far in. It was a column and
/// a half on the left beside the file and two docked, two on the right, and the foot had a
/// row and more where the top had a quarter. See DESIGN.md, Look.
pub const CARD_INSET: f32 = 24.0;
/// Pixels a box inside a card keeps round what it holds, as an agent's rewrite's does, and
/// between it and the buttons under it.
pub const BOX_PADDING: f32 = 16.0;
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

/// Where a row's words stand in it, in pixels from its top: their capitals' tops and their
/// tails' feet. None where there are no pixels.
pub fn words_in_row(measure: &dyn Measure) -> Option<(f32, f32)> {
    let (_, row) = measure.cell_pixels()?;
    let row = row as f32;
    let line = row / measure.row_air().max(1.0);
    let lead = (row - line) / 2.0;
    Some((lead + CAPS * line, lead + TAILS * line))
}

/// A card's parts where a row has room, in rows: how far its words are raised over its
/// buttons, so `DIALOG_GAP` stands between `last` and them, and how far into its first row
/// its panel starts and how far short of its last row's end it stops, so its title's
/// capitals and its buttons' foot have as much air to its edge as its sides have, `side`
/// pixels, kept between 24 and 32. A card is a dialog in a corner: an empty row over its
/// title and under its buttons, the panel inside them. They had 21 px over and under and 17
/// between the words and the buttons, 11 under a command's box, against 34 at their sides.
pub fn card_parts(last: DialogLast, side: f32, measure: &dyn Measure) -> Option<(f32, f32, f32)> {
    let (_, row) = measure.cell_pixels().filter(|_| roomy(measure))?;
    let row = row as f32;
    let line = row / measure.row_air().max(1.0);
    let lead = (row - line) / 2.0;
    let caps = lead + CAPS * line;
    let (top, foot) = cellix_render::button_in_row(row, line);
    let last = match last {
        DialogLast::Words => lead + TAILS * line,
        DialogLast::Control => foot,
    };
    let lift = (DIALOG_GAP - (row - last + top)).max(0.0);
    let air = side.clamp(24.0, 32.0);
    let over = (row + caps - lift - air).clamp(0.0, row);
    let under = (row * 2.0 - foot - air).clamp(0.0, row);
    Some((lift / row, over / row, under / row))
}

/// `CARD_INSET` in columns, where there are pixels. On cells, where a card is its rows and has
/// no edge, its words are a column in.
pub fn card_inset(measure: &dyn Measure) -> f32 {
    columns(CARD_INSET, measure).unwrap_or(1.0)
}

/// `pixels` in columns, where there are pixels.
pub fn columns(pixels: f32, measure: &dyn Measure) -> Option<f32> {
    let (wide, _) = measure.cell_pixels()?;
    Some(pixels / wide.max(1) as f32)
}

/// `pixels` in rows, where there are pixels.
pub fn rows(pixels: f32, measure: &dyn Measure) -> Option<f32> {
    let (_, tall) = measure.cell_pixels()?;
    Some(pixels / tall.max(1) as f32)
}

/// How far a card reaches over its first row and under its last, in rows, so its first
/// words' capitals stand `CARD_INSET` under its top and the foot of the buttons on its last
/// row, dropped `drop` of a row, `CARD_INSET` over its own. A short card's last row says how
/// many answers it has in place of its buttons, its words dropped to their foot, so the card
/// is the same with either. On cells, where nothing's painted, what the design's quarter row
/// was.
pub fn card_air(drop: f32, measure: &dyn Measure) -> (f32, f32) {
    let Some((_, row)) = measure.cell_pixels() else {
        return (0.17, 0.6);
    };
    let row = row as f32;
    let line = row / measure.row_air().max(1.0);
    let (caps, _) = words_in_row(measure).unwrap_or((0.0, row));
    let (_, foot) = cellix_render::button_in_row(row, line);
    let top = (CARD_INSET - caps) / row;
    let under = (foot + drop * row + CARD_INSET - row) / row;
    (top.max(0.0), under.max(0.0))
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
/// How many columns along a panel that floats from column `left` to `right` moves so that
/// neither of its edges stands within `LINE_AIR` of one of the upright `lines` (in columns,
/// with their fractions): nearest first, to the right before the left, and not at all where
/// every letter is a cell and no line is drawn. Its edge on a sheet's, the two lines merged
/// into one (Go to Symbol and a tab's menu on the file's sheet).
pub fn clear_of_lines(lines: &[f32], (left, right): (f32, f32), measure: &dyn Measure) -> i16 {
    let Some((wide, _)) = measure.cell_pixels() else {
        return 0;
    };
    let wide = wide.max(1) as f32;
    let near = (LINE_AIR + line_pixels(measure)) / wide;
    let clear = |by: f32| {
        lines.iter().all(|line| {
            [left + by, right + by]
                .iter()
                .all(|edge| (edge - line).abs() >= near)
        })
    };
    [0, 1, -1, 2, -2]
        .into_iter()
        .find(|by| clear(f32::from(*by)))
        .unwrap_or(0)
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

/// Where what stands at the end of a side pane list's rows ends, in cells: as far in from the
/// sheet's right edge as a row's mark is from its left, two columns into `area`, `edge` being
/// how far into its first column the sheet starts. So the band under the pointer has as much
/// air round what it holds on both sides (0.11.0's review of `next`, m2), and every line
/// that parts a list ends there too, 25 px from the pane's edge as it starts 25 px from the
/// other: they ended 9 px short of the rows (the release review of 0.12.0, 51, 14). On cells,
/// two columns in.
pub fn row_end(area: cellix_render::Rect, edge: f32, measure: &dyn Measure) -> f32 {
    if measure.in_cells() {
        return f32::from(area.right().saturating_sub(2));
    }
    f32::from(area.right()) - (2.0 - edge)
}

/// A chip in a row of tabs, `wide` columns from `left` on row `y`: under the pointer on an
/// icon at the top right, the sidebar's button, the +, a tab behind, and the count of tabs out
/// of view, one height for all. On a row with room, 34 px of the terminal's line, kept at
/// that height as a button on a wash is, which leaves 8 px over and under it in a row of 50
/// and so 8 px over the sheet's line under the row, and 4 px round a 13 pt icon: the icons'
/// and the button's stood at a control's 39 px, 5 px over that line, beside the count's 35
/// (0.11.0's release check, m4). On a row the terminal's line or little more, where nothing
/// keeps 8 px, nine tenths of the row, 3 px round the icon.
pub fn bar_chip(
    grid: &mut cellix_render::Grid,
    (left, y, wide): (f32, u16, f32),
    (radius, fill): (u8, cellix_render::Rgb),
    measure: &dyn Measure,
) {
    let (high, kept) = bar_chip_height(measure);
    let top = f32::from(y) + (1.0 - high) / 2.0;
    grid.paint.push(cellix_render::Paint::Panel {
        bounds: cellix_render::Bounds::cells(left, top, wide, high),
        radius,
        fill: Some(fill),
        border: None,
        shadow: false,
        dashed: false,
        inset: kept,
    });
}

/// How tall `bar_chip` is in rows, and whether that's kept at the terminal's line's height.
pub fn bar_chip_height(measure: &dyn Measure) -> (f32, bool) {
    if roomy(measure) {
        (34.0 / 36.0, true)
    } else {
        (0.89, false)
    }
}

/// How far a button at a row's end (`Grid::row_button`) is lifted back after it goes down onto
/// the words' baseline, in rows, where its row's foot is an edge (the line under peek's header,
/// a band the row's whole height, a suggestion's row in Review): back to the row's middle,
/// where gone down it would stand within `LINE_AIR` of that edge, and nothing where it keeps
/// that. With a capital's height in a button's middle the baseline wants a pixel, which a row of
/// 50 has; on a shorter Airy row the button stays in the middle. The first rule wanted 3 px, and
/// in Review's suggestion row Keep and Dismiss stood 6 px over the next line.
pub fn row_button_foot_lift(measure: &dyn Measure) -> f32 {
    let Some((_, tall)) = measure
        .cell_pixels()
        .filter(|_| roomy(measure) && !measure.in_cells())
    else {
        return 0.0;
    };
    let row = tall as f32;
    let line = row / measure.row_air().max(1.0);
    let (_, foot) = cellix_render::row_button_in_row(row, line);
    if foot <= row - LINE_AIR {
        return 0.0;
    }
    // The way it went down, as paint moves, in whole units.
    let unit = cellix_render::UNIT as f32;
    (cellix_render::WORDS_BASELINE * line / row * unit).round() / unit
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

    /// A panel whose edge would stand on a line, or within 8 px of one, moves a column along;
    /// one clear of every line stays, and on cells nothing moves.
    #[test]
    fn a_floating_panel_keeps_its_edges_off_a_sheets_line() {
        let measure = FontMeasure::new(17, 50).with_type_row(36);
        assert_eq!(clear_of_lines(&[28.0], (28.0, 92.0), &measure), 1);
        assert_eq!(clear_of_lines(&[28.0, 93.0], (28.0, 92.0), &measure), -1);
        assert_eq!(clear_of_lines(&[27.0, 100.5], (28.0, 92.0), &measure), 0);
        assert_eq!(clear_of_lines(&[28.0], (28.0, 92.0), &CellMeasure), 0);
    }

    /// A button at a row's end keeps 8 px from an edge at its row's foot: on a row of 50 it goes
    /// down onto the baseline, a pixel, and keeps 8; on the shortest Airy row, 47, gone down it
    /// would keep 7, so it stays in the middle, lifted back the whole way (Review's suggestion
    /// row stood 6 px over the next line under the first rule).
    #[test]
    fn a_row_button_keeps_8_px_from_its_rows_foot_or_stays_in_the_middle() {
        let tall = FontMeasure::new(17, 50).with_type_row(36);
        assert_eq!(row_button_foot_lift(&tall), 0.0, "a pixel down keeps 8");
        let short = FontMeasure::new(17, 47).with_type_row(36);
        assert!(roomy(&short));
        let lift = row_button_foot_lift(&short);
        let drop = cellix_render::WORDS_BASELINE * 36.0 / 47.0;
        assert!(
            lift > 0.0 && (lift - drop).abs() <= 1.0 / cellix_render::UNIT as f32,
            "lifted back the whole way: {lift} against {drop}"
        );
        assert_eq!(row_button_foot_lift(&CellMeasure), 0.0);
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
