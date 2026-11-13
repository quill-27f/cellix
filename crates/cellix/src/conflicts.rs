//! Merge conflicts in a file, drawn as cards over the code: the `<<<<<<<` line becomes the
//! card's header with its choices, `=======` labels the incoming side, `>>>>>>>` is its foot.
//! Nothing is added between lines, so every row is still a line of the file.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use cellix_render::{
    BOTTOM_LEFT, BOTTOM_RIGHT, Bounds, Grid, Measure, Rect, Rgb, Style, TOP_LEFT, TOP_RIGHT,
};
use cellix_review::conflict::{Conflict, conflicts};

use crate::editor::Editor;
use crate::theme::Theme;

/// Corners as a percentage of a row's height, as Review's cards have.
const CARD_RADIUS: u8 = 30;

/// A choice on a conflict's header, right to left as they stand.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Choice {
    Incoming,
    Current,
    Both,
    Compare,
}

const CHOICES: [(Choice, &str); 4] = [
    (Choice::Incoming, "Incoming"),
    (Choice::Current, "Current"),
    (Choice::Both, "Both"),
    (Choice::Compare, "Compare"),
];

/// A file's conflicts, with the file and the revision they were found at.
type Found = Option<(PathBuf, u64, Rc<Vec<Conflict>>)>;

thread_local! {
    /// Conflicts by file, with the revision they were found at: a file is only searched
    /// again once it changes.
    static FOUND: RefCell<Found> = const { RefCell::new(None) };
}

/// The conflicts in an editor's text, found once per revision.
pub fn found(editor: &Editor) -> Rc<Vec<Conflict>> {
    let path = editor.buffer.path().map(PathBuf::from).unwrap_or_default();
    let revision = editor.buffer.revision();
    FOUND.with(|cell| {
        if let Some((p, r, list)) = &*cell.borrow()
            && *p == path
            && *r == revision
        {
            return list.clone();
        }
        let list = Rc::new(conflicts(&editor.buffer.text()));
        *cell.borrow_mut() = Some((path, revision, list.clone()));
        list
    })
}

/// A colour part of the way from `a` to `b`, out of 100.
fn mix(a: Rgb, b: Rgb, part: u16) -> Rgb {
    let m = |a: u8, b: u8| ((a as u16 * (100 - part) + b as u16 * part) / 100) as u8;
    Rgb(m(a.0, b.0), m(a.1, b.1), m(a.2, b.2))
}

/// The tints: current green, incoming blue, each a stronger shade for its label row.
struct Tints {
    current: Rgb,
    current_head: Rgb,
    incoming: Rgb,
    incoming_head: Rgb,
    base: Rgb,
    foot: Rgb,
}

impl Tints {
    fn new(theme: &Theme) -> Self {
        let ground = theme.text.bg;
        Tints {
            current: mix(ground, theme.git_added, 14),
            current_head: mix(ground, theme.git_added, 26),
            incoming: mix(ground, theme.accent, 14),
            incoming_head: mix(ground, theme.accent, 26),
            base: mix(ground, theme.gutter.fg, 10),
            foot: mix(ground, theme.gutter.fg, 14),
        }
    }
}

/// Where each choice's button starts on a header row, right to left, for the room there is.
fn buttons(text: Rect, measure: &dyn Measure) -> Vec<(Choice, u16, String, &'static str)> {
    let mut placed = Vec::new();
    let mut end = text.right().saturating_sub(1);
    // The header's label wants room too; Compare goes first when there's little.
    let least = text.x + 24;
    for (choice, label) in CHOICES {
        // The room every button has around its words, and its gap to the next.
        let cells = crate::measures::button_cells(label, measure);
        let width = cells.chars().count() as u16;
        // Compare isn't a way to settle the conflict: it stands apart from the three that are.
        if choice == Choice::Compare {
            end = end.saturating_sub(2);
        }
        let Some(start) = end.checked_sub(width) else {
            break;
        };
        if start < least {
            break;
        }
        placed.push((choice, start, cells, label));
        end = start.saturating_sub(crate::measures::BUTTON_GAP);
    }
    placed
}

/// Share of a row a chip's height takes: a button's, as every button has.
const CHIP_HEIGHT: f32 = 0.8;

/// A chip centred in its row. The cell tier shows `cells`.
fn chip(
    grid: &mut Grid,
    (x, y): (u16, u16),
    (cells, label): (&str, &str),
    fill: Option<Rgb>,
    fg: Rgb,
    behind: Rgb,
) {
    let width = cells.chars().count() as u16;
    let style = Style {
        fg,
        // The row keeps its own colour; only the chip, in paint, is filled.
        bg: behind,
        painted: true,
        ..Style::default()
    };
    grid.put_str(x, y, cells, style, x + width);
    if let Some(fill) = fill {
        let top = y as f32 + (1.0 - CHIP_HEIGHT) / 2.0;
        grid.panel(
            Bounds::cells(x as f32, top, width as f32, CHIP_HEIGHT),
            22,
            Some(fill),
            None,
        );
    }
    grid.centred_text(x as f32 + width as f32 / 2.0, y as f32, label, fg, 90);
}

/// The first screen row a line is drawn on, if it's in view.
fn row_of(editor: &Editor, area: Rect, line: usize) -> Option<u16> {
    editor.line_rows_on_screen(area, line).first().copied()
}

/// Draws the cards over the code an editor has just drawn.
pub fn draw(
    grid: &mut Grid,
    editor: &Editor,
    area: Rect,
    (hover, measure): (Option<(u16, u16)>, &dyn Measure),
    theme: &Theme,
) {
    if editor.rendered {
        return;
    }
    let list = found(editor);
    if list.is_empty() {
        return;
    }
    let (text, gutter) = editor.code_layout(area);
    let tints = Tints::new(theme);
    let count = list.len();
    for (index, conflict) in list.iter().enumerate() {
        // A wrapped line tints every row it takes.
        let tint_row = |grid: &mut Grid, line: usize, bg: Rgb| {
            for y in editor.line_rows_on_screen(area, line) {
                grid.set_style(Rect::new(text.x, y, text.w, 1), |s| Style { bg, ..s });
            }
        };
        let label_row = |grid: &mut Grid, line: usize, bg: Rgb, fg: Rgb, words: &str| {
            let y = row_of(editor, area, line)?;
            grid.set_style(Rect::new(text.x, y, text.w, 1), |s| Style { bg, ..s });
            let body = Rect::new(text.x + gutter, y, text.w.saturating_sub(gutter), 1);
            let style = Style {
                bg,
                fg,
                ..theme.text
            };
            grid.fill(body, style);
            grid.label(body.x, y, words, style, body.right());
            Some(y)
        };
        for line in conflict.current_lines() {
            tint_row(grid, line, tints.current);
        }
        if let Some(base) = conflict.base {
            for line in base + 1..conflict.middle {
                tint_row(grid, line, tints.base);
            }
            let words = format!("Base · {}", base_label(conflict, editor, base));
            label_row(grid, base, tints.foot, theme.gutter.fg, &words);
        }
        for line in conflict.incoming_lines() {
            tint_row(grid, line, tints.incoming);
        }
        let current = name(&conflict.current, "the version checked out");
        let incoming = name(&conflict.incoming, "the version coming in");
        let words = format!("Incoming · {incoming}");
        label_row(
            grid,
            conflict.middle,
            tints.incoming_head,
            theme.accent,
            &words,
        );
        let words = format!("end of conflict {}", index + 1);
        let foot = label_row(grid, conflict.end, tints.foot, theme.gutter.fg, &words);
        let words = format!("Current · {current}");
        let head = label_row(
            grid,
            conflict.start,
            tints.current_head,
            theme.git_added,
            &words,
        );
        if let Some(y) = head {
            let end = text.x + gutter + words.chars().count() as u16 + 1;
            // The text's own colour: the quiet one is 2.6 to 1 on the band's tint.
            let dim = Style {
                bg: tints.current_head,
                fg: theme.text.fg,
                ..theme.text
            };
            let placed = buttons(text, measure);
            let first = placed.last().map_or(text.right(), |b| b.1);
            let place = format!("conflict {} of {count}", index + 1);
            grid.label(end, y, &place, dim, first.saturating_sub(1));
            for (choice, x, cells, label) in placed {
                let on =
                    hover.is_some_and(|h| h.1 == y && h.0 >= x && h.0 < x + cells.len() as u16);
                let behind = tints.current_head;
                // One primary, Incoming, as an agent's change usually is; the rest quiet chips
                // that lighten on hover, Compare among them.
                let (fill, fg) = match (choice, on) {
                    (Choice::Incoming, false) => (Some(theme.button), theme.button_text),
                    (Choice::Incoming, true) => {
                        (Some(theme.quiet_button(theme.button)), theme.button_text)
                    }
                    // Toward the sheet, not the text: a chip lighter than the band left
                    // its words at 3.8 to 1.
                    (_, false) => (Some(mix(behind, theme.text.bg, 45)), theme.text.fg),
                    (_, true) => (Some(mix(behind, theme.text.bg, 70)), theme.text.fg),
                };
                chip(grid, (x, y), (&cells, label), fill, fg, behind);
            }
            let card = Bounds::cells(text.x as f32, y as f32, text.w as f32, 1.0);
            grid.corners(card, CARD_RADIUS, theme.text.bg, TOP_LEFT | TOP_RIGHT);
        }
        if let Some(y) = foot {
            let card = Bounds::cells(text.x as f32, y as f32, text.w as f32, 1.0);
            grid.corners(card, CARD_RADIUS, theme.text.bg, BOTTOM_LEFT | BOTTOM_RIGHT);
        }
    }
}

fn name(label: &str, fallback: &str) -> String {
    if label.is_empty() {
        fallback.to_string()
    } else {
        label.to_string()
    }
}

/// What git wrote after the `|||||||` marker.
fn base_label(_conflict: &Conflict, editor: &Editor, line: usize) -> String {
    let text = editor.line_text(line);
    let label = text.trim_start_matches('|').trim();
    name(label, "the common ancestor")
}

/// The conflict and choice under a click, if it's on a header's button.
pub fn hit(
    editor: &Editor,
    area: Rect,
    (x, y): (u16, u16),
    measure: &dyn Measure,
) -> Option<(usize, Choice)> {
    if editor.rendered {
        return None;
    }
    let list = found(editor);
    let (text, _) = editor.code_layout(area);
    let index = list
        .iter()
        .position(|c| row_of(editor, area, c.start) == Some(y))?;
    buttons(text, measure)
        .into_iter()
        .find(|(_, start, cells, _)| x >= *start && x < start + cells.len() as u16)
        .map(|(choice, ..)| (index, choice))
}
